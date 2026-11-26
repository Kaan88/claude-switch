//! The window's size and the splitter's place, kept between runs in `window.ini` through the
//! toolkit's settings layer. A missing or damaged file gives the defaults; nothing is written in
//! the demo (it has no file).

use std::path::Path;

use mdshared_app::settings::{self, Writer};
use mdshared_app::sister::Rect as ScreenRect;
use mdshared_ui::place;
use mdshared_ui::window_state::{self, Monitor, WindowState};
use winit::window::Window;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestoreTabs {
    #[default]
    Ask,
    Always,
    Never,
}

impl RestoreTabs {
    pub const ALL: [Self; 3] = [Self::Ask, Self::Always, Self::Never];
    pub const LABELS: [&str; 3] = ["Ask", "Always", "Never"];

    fn key(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0)
    }
}

/// What is remembered.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFile {
    pub w: f32,
    pub h: f32,
    pub place: Option<WindowState>,
    pub restore_tabs: RestoreTabs,
    /// [`mdshared_ui::splitter::Splitter::to_setting`].
    pub split: String,
    /// `None` follows the window-width default.
    pub session_view: Option<bool>,
    pub session_split: String,
    pub chat_default: bool,
}

impl Default for WindowFile {
    fn default() -> WindowFile {
        WindowFile {
            w: 1100.0,
            h: 720.0,
            place: None,
            restore_tabs: RestoreTabs::Ask,
            split: String::new(),
            session_view: None,
            session_split: String::new(),
            chat_default: false,
        }
    }
}

fn size(v: &str, default: f32) -> f32 {
    match v.trim().parse::<f32>() {
        Ok(n) if n.is_finite() && (200.0..=20_000.0).contains(&n) => n,
        _ => default,
    }
}

/// Reads the text of the file.
pub fn parse(text: &str) -> WindowFile {
    let mut out = WindowFile::default();
    for (k, v) in settings::pairs(text) {
        match k {
            "width" => out.w = size(v, out.w),
            "height" => out.h = size(v, out.h),
            "placement" => out.place = WindowState::from_setting(v),
            "restore_tabs" => {
                out.restore_tabs = RestoreTabs::ALL.into_iter().find(|mode| mode.key() == v.trim()).unwrap_or_default();
            }
            "split" => out.split = settings::unescape(v),
            "session_view" => out.session_view = v.trim().parse().ok(),
            "session_split" => out.session_split = settings::unescape(v),
            "chat_default" => out.chat_default = v.trim().parse().unwrap_or(false),
            _ => {}
        }
    }
    out
}

pub fn to_text(f: &WindowFile) -> String {
    let mut w = Writer::new("window");
    w.line("width", &format!("{:.0}", f.w));
    w.line("height", &format!("{:.0}", f.h));
    if let Some(place) = &f.place {
        w.line("placement", &place.to_setting());
    }
    w.line("restore_tabs", f.restore_tabs.key());
    w.line("split", &settings::escape(&f.split));
    if let Some(open) = f.session_view {
        w.line("session_view", if open { "true" } else { "false" });
    }
    w.line("session_split", &settings::escape(&f.session_split));
    w.line("chat_default", if f.chat_default { "true" } else { "false" });
    w.finish()
}

pub fn load(path: Option<&Path>) -> WindowFile {
    path.and_then(|p| settings::read(p).ok().flatten()).map_or_else(WindowFile::default, |t| parse(&t))
}

pub fn save(path: Option<&Path>, f: &WindowFile) -> Result<(), claude_switch_core::Refusal> {
    if let Some(p) = path {
        claude_switch_core::write_app_file(p, to_text(f).as_bytes())?;
    }
    Ok(())
}

pub fn apply(window: &Window, saved: &WindowState) {
    if let Some(rect) = restore_rect(saved, &window_state::monitors(window))
        && place::set(window, rect)
    {
        window.set_maximized(saved.maximized);
    }
}

pub fn capture(window: &Window) -> Option<WindowState> {
    let place = window_state::capture(window)?;
    #[cfg(windows)]
    let place = {
        let mut place = place;
        if place.maximized
            && let Some(work) = mdshared_ui::place::work_area(window)
        {
            // GetWindowPlacement gives a maximized window's normal position in workspace
            // coordinates. SetWindowPos on restart needs the corresponding screen coordinates.
            let (x, y) = place.monitor.position;
            place.rect.x += (work.x - x) as f32 / place.scale;
            place.rect.y += (work.y - y) as f32 / place.scale;
        }
        place
    };
    Some(place)
}

pub fn demo_offscreen() -> WindowState {
    WindowState {
        rect: mdshared_ui::gfx::Rect::new(60.0, 80.0, 900.0, 620.0),
        maximized: true,
        scale: 1.0,
        monitor: window_state::MonitorId::new(Some("Demo disconnected monitor".into()), (-100_000, 0), (1920, 1080)),
    }
}

fn work_rect(m: &Monitor) -> ScreenRect {
    ScreenRect {
        x: m.position.0.saturating_add((m.work.x * m.scale).round() as i32),
        y: m.position.1.saturating_add((m.work.y * m.scale).round() as i32),
        w: (m.work.w * m.scale).round() as u32,
        h: (m.work.h * m.scale).round() as u32,
    }
}

fn restore_rect(saved: &WindowState, monitors: &[Monitor]) -> Option<ScreenRect> {
    let primary = monitors.first()?;
    let scale = monitors.iter().find(|m| m.id() == saved.monitor).map_or(saved.scale, |m| m.scale);
    let rect = ScreenRect {
        x: saved.monitor.position.0.saturating_add((saved.rect.x * scale).round() as i32),
        y: saved.monitor.position.1.saturating_add((saved.rect.y * scale).round() as i32),
        w: (saved.rect.w * scale).round() as u32,
        h: (saved.rect.h * scale).round() as u32,
    };
    let overlap = |a: i32, len_a: u32, b: i32, len_b: u32| {
        let (a, b) = (i64::from(a), i64::from(b));
        (a + i64::from(len_a)).min(b + i64::from(len_b)) - a.max(b)
    };
    if monitors
        .iter()
        .map(work_rect)
        .any(|work| overlap(rect.x, rect.w, work.x, work.w) >= 100 && overlap(rect.y, rect.h, work.y, work.h) >= 100)
    {
        return Some(rect);
    }
    let work = work_rect(primary);
    let w = ((saved.rect.w * primary.scale).round() as u32).clamp(1, work.w.max(1));
    let h = ((saved.rect.h * primary.scale).round() as u32).clamp(1, work.h.max(1));
    Some(ScreenRect {
        x: work.x.saturating_add((work.w.saturating_sub(w) / 2) as i32),
        y: work.y.saturating_add((work.h.saturating_sub(h) / 2) as i32),
        w,
        h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ini_save_reports_a_refused_destination() {
        assert!(save(Some(std::env::temp_dir().as_path()), &WindowFile::default()).is_err());
        assert!(save(None, &WindowFile::default()).is_ok());
    }

    #[test]
    fn the_text_reads_back_as_what_was_written() {
        let f = WindowFile {
            w: 1280.0,
            h: 800.0,
            split: "a:420".into(),
            session_view: Some(false),
            session_split: "b:450".into(),
            chat_default: true,
            place: Some(saved(240)),
            restore_tabs: RestoreTabs::Always,
        };
        assert_eq!(parse(&to_text(&f)), f);
    }

    #[test]
    fn nonsense_gives_the_defaults() {
        assert_eq!(parse("width=abc\nheight=-5\nplacement=oops\nrestore_tabs=bad"), WindowFile::default());
        assert_eq!(load(None), WindowFile::default());
    }

    fn monitor(x: i32) -> Monitor {
        Monitor {
            name: None,
            position: (x, 0),
            size: (1200, 900),
            scale: 1.0,
            work: mdshared_ui::gfx::Rect::new(0.0, 20.0, 1200.0, 800.0),
        }
    }

    fn saved(x: i32) -> WindowState {
        WindowState {
            rect: mdshared_ui::gfx::Rect::new(x as f32, 50.0, 800.0, 500.0),
            maximized: true,
            scale: 1.0,
            monitor: monitor(-1200).id(),
        }
    }

    #[test]
    fn a_partial_window_stays_only_when_at_least_100_px_are_visible_on_both_axes() {
        let mut item = saved(-700);
        assert_eq!(restore_rect(&item, &[monitor(0), monitor(-1200)]).unwrap().x, -1900);
        item.rect.x -= 1.0;
        let centered = ScreenRect { x: 200, y: 170, w: 800, h: 500 };
        assert_eq!(restore_rect(&item, &[monitor(0), monitor(-1200)]), Some(centered));
        assert_eq!(restore_rect(&saved(0), &[monitor(0)]), Some(centered));
        let mut item = saved(200);
        item.rect.y = 721.0;
        assert_eq!(restore_rect(&item, &[monitor(0), monitor(-1200)]), Some(centered));
    }
}
