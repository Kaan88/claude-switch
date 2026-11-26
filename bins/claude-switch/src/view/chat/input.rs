use std::io;
use std::path::PathBuf;
use std::time::Instant;

use claude_switch_core::transcript::ItemKind;
use mdshared_ui::host::{Cx, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::{Chat, Chip, Drawer, Gate, Reason, Side, recall, send_bytes};
use crate::state::{AppState, Msg, Target};
use crate::tabs::Tab;

pub fn layout_shortcut(name: &KeyName, mods: Mods, palette: bool, session_tab: bool) -> bool {
    palette && session_tab && *name == KeyName::F(6) && mods.none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Drawer,
    Open,
    Grip,
    Panel,
    Side(Side),
    Editor,
    Attach,
    Send,
    Override,
    SlashModel,
    Stop,
    Remove(usize),
    MoreChips,
}

impl AppState {
    pub fn chat_paste(&mut self) -> bool {
        if !self.chat_takes_input() {
            return false;
        }
        self.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat.paste_clipboard();
        true
    }

    pub fn chat_active(&self) -> bool {
        self.tabs.shown().and_then(|tab| tab.pane.as_ref()).is_some_and(|pane| pane.chat.enabled)
    }

    pub fn chat_takes_input(&self) -> bool {
        self.tabs
            .shown()
            .and_then(|tab| tab.pane.as_ref())
            .is_some_and(|pane| pane.chat.enabled && !(pane.chat.drawer_focus && pane.chat.drawer != Drawer::Closed))
    }

    pub fn set_chat_layout(&mut self, enabled: bool) {
        self.clear_session_text_focus();
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        if pane.chat.enabled != enabled {
            tab.term.stop_pointer();
            pane.chat.enabled = enabled;
            pane.chat.drawer_focus = false;
            pane.chat.resizing = None;
            if enabled {
                pane.ui.chat_conversation(&mut pane.model);
            } else {
                pane.ui.set_chat(false);
            }
            self.win_file.chat_default = enabled;
            self.save_window();
        }
    }

    pub fn chat_input_tick(&mut self, cx: &mut Cx<'_, Msg>) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        if pane.model.error.is_some() {
            chat.invalidate(&pane.model.chat);
        } else if pane.model.session_id().is_some() {
            chat.observe(&pane.model.chat, cx.now());
        }
        if chat.enabled {
            let decision = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, cx.now());
            if chat.react(decision.1) | (chat.last_gate.replace(decision.0) != Some(decision.0)) {
                cx.request_redraw_all();
            }
        }
    }

    /// A key over Chat is either for the editor or for the explicitly focused drawer.
    pub fn chat_key(&mut self, cx: &mut Cx<'_, Msg>, k: &KeyPress) -> bool {
        if !self.chat_active() || !self.nothing_over() || self.picture_overlay() {
            return false;
        }
        let mods = k.mods;
        // Reuse the app's already-reserved pane shortcut; no new chord steals PTY input.
        if crate::termkeys::shortcut(k.name.as_ref().unwrap_or(&KeyName::Escape), mods)
            == Some(crate::actions::SESSION_VIEW)
        {
            self.chat_control(cx, Control::Drawer);
            return true;
        }
        if !self.chat_takes_input() {
            return false;
        }
        self.clear_session_text_focus();
        let Some(tab) = self.tabs.shown_mut() else { return true };
        let pane = tab.pane.as_mut().expect("Chat pane");
        let chat = &mut pane.chat;
        match k.name.as_ref() {
            Some(KeyName::Enter) if !mods.shift && !mods.ctrl && !mods.alt => self.chat_control(cx, Control::Send),
            Some(KeyName::Char('v' | 'V')) if mods.ctrl && !mods.alt => {
                chat.paste_clipboard();
            }
            Some(KeyName::Up | KeyName::Down)
                if !mods.shift && !mods.ctrl && !mods.alt && (chat.editor.text.is_empty() || chat.recall.is_some()) =>
            {
                let history = pane.model.items.values().rev().filter_map(|item| match &item.kind {
                    ItemKind::Prompt { text, synthetic: false, .. } => Some(text.as_str()),
                    _ => None,
                });
                if let Some(prompt) = recall(history, &mut chat.recall, k.name == Some(KeyName::Up)) {
                    chat.editor.set(prompt.to_owned());
                    chat.editor.move_to(prompt.len(), false);
                    chat.error = None;
                }
            }
            _ if k.text.as_deref() == Some("/") && chat.editor.text.is_empty() && !mods.ctrl && !mods.alt => {
                self.chat_slash(cx, false);
            }
            Some(name) => {
                chat.editor.key(name, mods, k.text.as_deref());
                chat.recall = None;
                chat.error = None;
            }
            None => {
                if let Some(text) = &k.text {
                    chat.editor.insert(text);
                    chat.error = None;
                }
            }
        }
        true
    }

    pub fn chat_text(&mut self, cx: &mut Cx<'_, Msg>, text: &str) -> bool {
        if !self.chat_takes_input() {
            return false;
        }
        let chat = &mut self.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
        if text == "/" && chat.editor.text.is_empty() {
            self.chat_slash(cx, false);
        } else {
            chat.editor.insert(text);
            chat.recall = None;
            chat.error = None;
        }
        true
    }

    pub fn note_chat_terminal_input(&mut self) {
        if let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) {
            pane.note_terminal_input();
        }
    }

    pub fn chat_press(&mut self, cx: &Cx<'_, Msg>, target: Option<Target>, x: f32, y: f32, clicks: u32) -> bool {
        if !self.chat_active() || self.picture_overlay() {
            return false;
        }
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.drawer_focus = target == Some(Target::Terminal);
        match target {
            Some(Target::Chat(Control::Grip)) => {
                chat.resizing = Some((y, chat.layout.drawer.h));
                chat.drawer_focus = true;
                true
            }
            Some(Target::Chat(Control::Editor)) => {
                let at = chat.editor_at(x, y);
                if clicks > 1 {
                    chat.editor.key(&KeyName::Char('a'), Mods { ctrl: true, ..Mods::default() }, None);
                } else {
                    chat.editor.move_to(at, cx.mods().shift);
                    chat.editor_drag = true;
                }
                true
            }
            Some(Target::Chat(Control::Open)) => {
                self.pressed = target;
                true
            }
            _ => false,
        }
    }

    pub fn chat_move(&mut self, x: f32, y: f32) -> Option<bool> {
        if !self.chat_active() || self.picture_overlay() || !self.nothing_over() {
            return None;
        }
        let chat = &mut self.tabs.shown_mut()?.pane.as_mut()?.chat;
        if chat.resize_drawer(y) {
            return Some(true);
        }
        if chat.editor_drag {
            let at = chat.editor_at(x, y);
            let changed = at != chat.editor.caret;
            chat.editor.move_to(at, true);
            return Some(changed);
        }
        None
    }

    pub fn chat_end_drag(&mut self, y: Option<f32>) -> bool {
        self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()).is_some_and(|pane| pane.chat.end_drag(y))
    }

    pub fn chat_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) else { return false };
        let chat = &mut pane.chat;
        if !chat.enabled {
            return false;
        }
        if chat.layout.editor.contains(x, y) {
            chat.editor.wheel(dy);
        } else if chat.layout.side.w > 0.0 && chat.layout.side.contains(x, y) {
            chat.side_scroll =
                if dy > 0.0 { chat.side_scroll.saturating_sub(1) } else { (chat.side_scroll + 1).min(chat.side_max) };
        } else {
            return false;
        }
        true
    }

    pub fn chat_control(&mut self, cx: &mut Cx<'_, Msg>, control: Control) {
        if control == Control::SlashModel {
            self.chat_slash(cx, true);
            return;
        }
        if matches!(control, Control::Send | Control::Override | Control::Stop) {
            self.term_woke(cx);
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        if !chat.enabled {
            return;
        }
        match control {
            Control::Drawer => chat.toggle_drawer(),
            Control::Open if chat.drawer == Drawer::Closed => chat.open_drawer(),
            Control::Open => chat.drawer_focus = true,
            Control::Panel => chat.side_hidden = !chat.side_hidden,
            Control::Side(side) => {
                chat.side = side;
                chat.side_scroll = 0;
            }
            Control::Remove(i) if i < chat.chips.len() => {
                chat.chips.remove(i);
                chat.chip_page = chat.chip_page.min(chat.chips.len().saturating_sub(1) / 3);
                chat.error = None;
            }
            Control::MoreChips => chat.chip_page = (chat.chip_page + 1) % chat.chips.len().max(1).div_ceil(3),
            Control::Attach => {
                let cwd = pane.cwd.clone();
                self.attach_picker(Some(&cwd), true);
            }
            Control::Send | Control::Override => Self::chat_send(tab, cx.now(), control == Control::Override),
            Control::Stop => {
                let decision = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, cx.now());
                if decision == (Gate::Running, None) && chat.can_stop(&pane.model.chat) {
                    chat.stopped = pane.model.chat.prompt.map(|p| (pane.model.chat.generation, p));
                    if let Err(e) = write(tab, b"\x1b", "Stopped") {
                        tab.pane.as_mut().unwrap().chat.error = Some(e);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn chat_send(tab: &mut Tab, now: Instant, force: bool) {
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.editor.text.trim().is_empty() && chat.chips.is_empty() {
            return;
        }
        let (gate, reason) = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
        let modes = tab.term.screen.modes();
        chat.error = match gate {
            Gate::Ended => Some("The session ended. Your draft is still here."),
            _ if !chat.demo && (!modes.bracketed_paste || modes.alt_screen) => {
                Some("Bracketed paste is unavailable. Open the terminal to check.")
            }
            Gate::Running => Some("A turn is running. Your draft is still here."),
            Gate::Check if reason.is_some() => reason.map(Reason::label),
            Gate::Check if !force => Some("Open the terminal to check"),
            _ => None,
        };
        if chat.error.is_some() {
            if reason.is_some() {
                chat.open_drawer();
            }
            return;
        }
        if chat.chips.iter().any(|chip| matches!(chip, Chip::Clipboard(_))) {
            chat.error =
                Some("Open the terminal to attach and preview the clipboard picture. Remove this chip before sending.");
            chat.open_drawer();
            return;
        }
        let Some(bytes) = send_bytes(&chat.chips, &chat.editor.text) else {
            chat.error = Some("The draft has unsupported control characters.");
            return;
        };
        let demo_text: String = chat.editor.text.chars().take(400).collect();
        if let Err(error) = write(tab, &bytes, &demo_text) {
            tab.pane.as_mut().unwrap().chat.error = Some(error);
            return;
        }
        let pane = tab.pane.as_mut().unwrap();
        pane.chat.invalidate(&pane.model.chat);
        pane.chat.editor.set(String::new());
        pane.chat.chips.clear();
        pane.chat.chip_page = 0;
        pane.chat.recall = None;
    }

    fn chat_slash(&mut self, cx: &mut Cx<'_, Msg>, model: bool) {
        self.term_woke(cx);
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        let (gate, _) = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, cx.now());
        if gate != Gate::Ready {
            if !model {
                chat.editor.insert("/");
            }
            chat.open_drawer();
            chat.error = Some("Open the terminal to check. Enter slash commands in the terminal.");
            return;
        }
        let bytes: &[u8] = if model { b"/model\r" } else { b"/" };
        if let Err(error) = write(tab, bytes, if model { "/model" } else { "/" }) {
            tab.pane.as_mut().unwrap().chat.error = Some(error);
            return;
        }
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        chat.invalidate(&pane.model.chat);
        chat.slash = true;
        chat.suppressed = None;
        chat.react(Some(Reason::Slash));
        chat.drawer_focus = true;
    }

    pub fn chat_attach(&mut self, id: u64, files: &[PathBuf]) {
        if let Some(pane) = self.tabs.get_mut(id).and_then(|t| t.pane.as_mut()) {
            let chat = &mut pane.chat;
            for path in files {
                if super::super::attach::paths(std::slice::from_ref(path)).is_empty() {
                    chat.error = Some("A file path could not be attached.");
                    continue;
                }
                let chip = Chip::Path(path.clone());
                if !chat.chips.contains(&chip) {
                    chat.chips.push(chip);
                }
            }
            chat.chip_page = chat.chips.len().saturating_sub(1) / 3;
        }
    }
}

impl Chat {
    pub(super) fn resize_drawer(&mut self, y: f32) -> bool {
        let Some((start, height)) = self.resizing else { return false };
        self.drawer_height = Some((height + start - y).max(52.0));
        true
    }

    pub(super) fn end_drag(&mut self, y: Option<f32>) -> bool {
        if let Some(y) = y {
            self.resize_drawer(y);
        }
        self.resizing.take().is_some() | std::mem::take(&mut self.editor_drag)
    }

    fn paste_clipboard(&mut self) {
        if let Some(seq) = clipboard_picture() {
            self.chips.retain(|c| !matches!(c, Chip::Clipboard(_)));
            self.chips.push(Chip::Clipboard(seq));
            self.chip_page = (self.chips.len() - 1) / 3;
        } else if let Some(text) = platform::clipboard_text() {
            self.editor.insert(&text);
        }
        self.recall = None;
        self.error = None;
    }

    fn editor_at(&self, x: f32, y: f32) -> usize {
        let rect = self.layout.editor;
        let row = self.editor.scroll + ((y - rect.y - 4.0).max(0.0) / 17.0) as usize;
        let col = ((x - rect.x - 8.0).max(0.0) / self.cell_width.max(1.0)).round() as usize;
        self.editor.row_byte(row, col)
    }
}

fn write(tab: &mut Tab, bytes: &[u8], demo_text: &str) -> Result<(), &'static str> {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    if chat.demo {
        let demo_text: String = demo_text.chars().filter(|c| !c.is_control() || *c == '\n').collect();
        tab.term.feed(
            format!("\r\n> {}\r\n[Demo input — no process started]\r\n", demo_text.replace('\n', "\r\n")).as_bytes(),
        );
        chat.demo_progress = true;
    } else {
        let pty =
            tab.term.pty.as_ref().filter(|p| tab.term.ended.is_none() && p.running()).ok_or("The session ended.")?;
        pty.write(bytes).map_err(|_: io::Error| "The terminal write failed. Check the drawer before retrying.")?;
    }
    tab.term.scroll = 0;
    tab.term.selection = None;
    Ok(())
}

#[cfg(windows)]
fn clipboard_picture() -> Option<u32> {
    use windows_sys::Win32::System::DataExchange::{
        GetClipboardSequenceNumber, IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    // SAFETY: The registered name is a static, NUL-terminated UTF-16 string; the queries take no pointers.
    unsafe {
        let before = GetClipboardSequenceNumber();
        let png = RegisterClipboardFormatW(windows_sys::w!("PNG"));
        let bitmap = [2, 8, 17, png].into_iter().any(|format| format != 0 && IsClipboardFormatAvailable(format) != 0);
        (bitmap && before != 0 && before == GetClipboardSequenceNumber()).then_some(before)
    }
}

#[cfg(not(windows))]
fn clipboard_picture() -> Option<u32> {
    None
}
