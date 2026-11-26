//! Native notification-area integration. No installer, shortcut, COM activator or external process.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::mem::size_of;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

const CALLBACK: u32 = WM_APP + 71;
const SUBCLASS: usize = 71;
const ICON: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1; // SDK: NIN_SELECT | NINF_KEY

#[derive(Clone, Copy)]
pub enum Event {
    Menu(Option<(i32, i32)>),
    Open,
    BalloonClick,
    BalloonDone,
    ExplorerRestarted,
}

type Callback = Box<dyn Fn(Event)>;
thread_local! {
    static CALLBACKS: RefCell<HashMap<usize, (u32, Callback)>> = RefCell::default();
}

struct Balloon {
    title: String,
    text: String,
    tab: Option<u64>,
}

#[derive(Default)]
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
    requested_tip: Option<String>,
    queue: VecDeque<Balloon>,
    active: Option<Balloon>,
}

impl Tray {
    pub fn attach(&mut self, window: &Window, send: impl Fn(Event) + 'static) {
        let Some(hwnd) = hwnd(window) else { return };
        if !self.hwnd.is_null() {
            return;
        }
        // SAFETY: HWND belongs to this UI thread; the callback and registered name have static lifetime.
        unsafe {
            let restart = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            if restart == 0 || SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0) == 0 {
                return;
            }
            CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(hwnd as usize, (restart, Box::new(send))));
            self.hwnd = hwnd;
            self.icon = app_icon();
        }
    }

    pub fn enabled(&self) -> bool {
        self.added
    }

    fn needs_sync(&self, tip: &str, force: bool) -> bool {
        force || self.requested_tip.as_deref() != Some(tip)
    }

    pub fn sync(&mut self, enabled: bool, tip: String, force: bool) {
        if self.hwnd.is_null() {
            return;
        }
        if !enabled {
            self.remove_icon();
            self.requested_tip = None;
            return;
        }
        if !self.needs_sync(&tip, force) {
            return;
        }
        let mut data = self.data();
        data.uFlags = NIF_TIP | NIF_SHOWTIP;
        fill(&mut data.szTip, &tip);
        // SAFETY: data has the matching cbSize and backed NUL-terminated fields; the shell copies it.
        unsafe {
            if self.added {
                if Shell_NotifyIconW(NIM_MODIFY, &data) != 0 {
                    self.requested_tip = Some(tip);
                    return;
                }
                self.explorer_restarted();
            }
            data.uFlags |= NIF_MESSAGE | NIF_ICON;
            data.uCallbackMessage = CALLBACK;
            data.hIcon = self.icon;
            if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                self.added = Shell_NotifyIconW(NIM_SETVERSION, &data) != 0;
                if !self.added {
                    Shell_NotifyIconW(NIM_DELETE, &data);
                }
            }
        }
        // Even a failed registration must not retry on every terminal wake.
        self.requested_tip = Some(tip);
    }

    pub fn explorer_restarted(&mut self) {
        self.added = false;
        self.requested_tip = None;
        self.queue.clear();
        self.active = None;
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON,
            ..Default::default()
        }
    }

    fn remove_icon(&mut self) {
        if self.added {
            // SAFETY: notification data identifies only our window's icon.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data()) };
            self.explorer_restarted();
        }
    }

    /// Keep the click destination paired with the one balloon handed to the shell.
    pub fn notice(&mut self, title: &str, text: &str, tab: Option<u64>) -> bool {
        if !self.added || self.queue.len() >= 16 {
            return false;
        }
        self.queue.push_back(Balloon { title: title.into(), text: text.into(), tab });
        self.next_balloon();
        self.active.is_some()
    }

    pub fn discard_usage_notices(&mut self) {
        self.queue.retain(|balloon| balloon.title != "Claude Switch usage");
    }

    fn next_balloon(&mut self) {
        if self.active.is_some() {
            return;
        }
        while let Some(balloon) = self.queue.pop_front() {
            let mut data = self.data();
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
            fill(&mut data.szInfoTitle, &balloon.title);
            fill(&mut data.szInfo, &balloon.text);
            // SAFETY: data is initialized, identifies our icon, and its string buffers remain live for the call.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0 {
                self.active = Some(balloon);
                break;
            }
        }
    }

    pub fn balloon_done(&mut self) -> Option<Option<u64>> {
        let target = self.active.take().map(|balloon| balloon.tab);
        self.next_balloon();
        target
    }

    pub fn menu(&self, at: Option<(i32, i32)>, labels: &[String]) -> Option<usize> {
        if !self.added {
            return None;
        }
        let mut point = POINT::default();
        // SAFETY: the menu is created, used and destroyed on the owning window thread; AppendMenu
        // copies each label and TrackPopupMenu returns a command rather than calling app handlers.
        unsafe {
            if let Some((x, y)) = at {
                point = POINT { x, y };
            } else if GetCursorPos(&mut point) == 0 {
                return None;
            }
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            for (i, label) in labels.iter().enumerate() {
                let wide = wide(&label.replace('&', "&&"));
                AppendMenuW(menu, MF_STRING, i + 1, wide.as_ptr());
            }
            SetForegroundWindow(self.hwnd);
            let choice = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                self.hwnd,
                std::ptr::null(),
            );
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            usize::try_from(choice).ok()?.checked_sub(1)
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let bound = CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&(self.hwnd as usize)).is_some());
        if bound {
            self.remove_icon();
        }
        // SAFETY: the binding was for a still-live window on this thread; the privately owned icon
        // came from CreateIconFromResourceEx and is no longer in the shell.
        unsafe {
            if bound {
                RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS);
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

// SAFETY: Comctl32 calls this only for the HWND and owning thread registered in `attach`.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&(hwnd as usize)));
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON,
            ..Default::default()
        };
        // SAFETY: this callback is on the window thread, during destruction of this very window.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
        }
    } else {
        CALLBACKS.with(|callbacks| {
            let callbacks = callbacks.borrow();
            let Some((restart, send)) = callbacks.get(&(hwnd as usize)) else { return };
            let event = if message == *restart && *restart != 0 {
                Some(Event::ExplorerRestarted)
            } else if message == CALLBACK && (lparam as u32 >> 16) == ICON {
                match lparam as u32 & 0xffff {
                    WM_CONTEXTMENU => {
                        let point = (wparam as i16 as i32, (wparam >> 16) as i16 as i32);
                        Some(Event::Menu((point != (-1, -1)).then_some(point)))
                    }
                    NIN_SELECT | NIN_KEYSELECT => Some(Event::Open),
                    NIN_BALLOONUSERCLICK => Some(Event::BalloonClick),
                    NIN_BALLOONHIDE | NIN_BALLOONTIMEOUT => Some(Event::BalloonDone),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(event) = event {
                send(event);
            }
        });
    }
    // SAFETY: forward the untouched message to the next handler in this subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn wide(text: &str) -> Vec<u16> {
    text.chars().filter(|&c| c != '\0').collect::<String>().encode_utf16().chain([0]).collect()
}

fn fill(buffer: &mut [u16], text: &str) {
    let mut index = 0;
    for ch in text.chars().filter(|&c| c != '\0') {
        let end = index + ch.len_utf16();
        if end >= buffer.len() {
            break;
        }
        ch.encode_utf16(&mut buffer[index..end]);
        index = end;
    }
    buffer[index] = 0;
}

fn hwnd(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

fn app_icon() -> HICON {
    // This committed ICO starts with a 16×16 RT_ICON image (the third and fourth directory
    // DWORDs give its length and offset). Only the image, not the ICO directory, goes to User32.
    let ico = include_bytes!("../../../assets/icon.ico");
    let size = u32::from_le_bytes(ico[14..18].try_into().unwrap());
    let offset = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
    let image = ico[offset..offset + size as usize].to_vec();
    // SAFETY: image points at a live, complete icon resource from a bundled, trusted ICO.
    unsafe { CreateIconFromResourceEx(image.as_ptr(), size, 1, 0x0003_0000, 0, 0, LR_DEFAULTCOLOR) }
}

pub fn flash(window: &Window) {
    if let Some(hwnd) = hwnd(window) {
        let data =
            FLASHWINFO { cbSize: size_of::<FLASHWINFO>() as u32, hwnd, dwFlags: FLASHW_TRAY, uCount: 1, dwTimeout: 0 };
        // SAFETY: hwnd is the live app window and data lives through the call.
        unsafe { FlashWindowEx(&data) };
    }
}

pub fn show(window: &Window) {
    window.set_visible(true);
    if window.is_minimized() == Some(true) {
        window.set_minimized(false);
    }
    window.focus_window();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_queued_usage_preserves_waiting_tab_balloons() {
        let mut tray = Tray::default();
        tray.queue.push_back(Balloon { title: "Claude Switch usage".into(), text: "usage".into(), tab: None });
        tray.queue.push_back(Balloon { title: "Session waiting".into(), text: "waiting".into(), tab: Some(9) });
        tray.discard_usage_notices();
        assert_eq!(tray.queue.iter().map(|balloon| balloon.tab).collect::<Vec<_>>(), [Some(9)]);
    }

    #[test]
    fn fixed_shell_buffers_keep_terminators_and_whole_surrogates() {
        let mut buffer = [0; 4];
        fill(&mut buffer, "a🌟z");
        assert_eq!(String::from_utf16(&buffer[..3]).unwrap(), "a🌟");
        assert_eq!(buffer[3], 0);
        let mut buffer = [0; 3];
        fill(&mut buffer, "a🌟");
        assert_eq!(buffer, [b'a' as u16, 0, 0]);
    }

    #[test]
    fn unchanged_tray_tip_skips_shell_until_restart_or_forced_sync() {
        let mut tray = Tray::default();
        tray.requested_tip.replace("same".into());
        assert!(!tray.needs_sync("same", false), "including after failed registration");
        assert!(tray.needs_sync("changed", false));
        assert!(tray.needs_sync("same", true), "before hiding the window");
        tray.explorer_restarted();
        assert!(tray.needs_sync("same", false));
    }
}
