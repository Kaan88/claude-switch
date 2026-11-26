//! Shared paste bytes for the clipboard, the file picker and file drops.

use std::path::PathBuf;

use mdshared_term::Modes;
use mdshared_ui::host::Cx;
use mdshared_ui::platform;

use crate::state::{AppState, Msg};
use crate::termkeys;

pub fn clipboard(text: Option<&str>, modes: &Modes) -> Vec<u8> {
    match text.filter(|text| !text.is_empty()) {
        Some(text) => termkeys::paste(text, modes),
        None => b"\x1bv".to_vec(),
    }
}

pub fn paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .filter_map(|path| path.to_str())
        .filter(|path| !path.is_empty() && !path.contains('"') && !path.chars().any(char::is_control))
        .map(|path| if path.chars().any(char::is_whitespace) { format!("\"{path}\"") } else { path.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
pub struct DropBatch {
    next: Option<(std::time::Instant, u64, Vec<PathBuf>)>,
}

impl AppState {
    pub fn attach_picker(&self, cwd: Option<&std::path::Path>, chat: bool) {
        let Some(id) = self.tabs.active else { return };
        let proxy = self.proxy.clone();
        platform::pick_files_async(cwd, "Pictures", &["png", "jpg", "jpeg", "webp", "gif"], move |files| {
            if !files.is_empty()
                && let Some(proxy) = proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(Msg::AttachPictures(id, files, chat));
            }
        });
    }

    pub fn paste_paths(&mut self, id: u64, files: &[PathBuf]) -> bool {
        let shown = self.tabs.active == Some(id);
        if self.tabs.get(id).and_then(|tab| tab.pane.as_ref()).is_some_and(|pane| pane.chat.enabled) {
            self.chat_attach(id, files);
            return shown;
        }
        if let Some(tab) = self.tabs.get_mut(id) {
            let text = paths(files);
            if !text.is_empty() {
                if let Some(pane) = &mut tab.pane {
                    pane.chat.invalidate(&pane.model.chat);
                }
                let redraw = tab.term.scroll != 0 && shown;
                tab.term.scroll = 0;
                tab.term.write(&termkeys::paste(&text, &tab.term.screen.modes()));
                return redraw;
            }
        }
        false
    }

    pub fn terminal_drop(&mut self, cx: &mut Cx<'_, Msg>, file: PathBuf) {
        // Winit emits one event per file, without a batch delimiter.
        if !self.nothing_over() || self.picture_overlay() {
            return;
        }
        let Some(id) = self.tabs.active else { return };
        if self.chat_active() {
            self.chat_attach(id, &[file]);
            cx.request_redraw_all();
            return;
        }
        let now = cx.now();
        if self.session_view.drops.next.as_ref().is_some_and(|&(_, to, _)| to != id) {
            self.finish_terminal_drop();
        }
        let at = now + std::time::Duration::from_millis(30);
        let pending = self.session_view.drops.next.get_or_insert_with(|| (at, id, Vec::new()));
        pending.0 = at;
        pending.2.push(file);
        cx.wake_at(at);
    }

    fn finish_terminal_drop(&mut self) -> bool {
        self.session_view.drops.next.take().is_some_and(|(_, id, files)| self.paste_paths(id, &files))
    }

    pub fn flush_drop_before_input(&mut self) {
        if self.session_view.drops.next.as_ref().is_some_and(|(_, id, _)| self.tabs.active == Some(*id)) {
            self.finish_terminal_drop();
        }
    }

    pub fn terminal_drop_due(&mut self, cx: &mut Cx<'_, Msg>, now: std::time::Instant) {
        if let Some((at, _, _)) = &self.session_view.drops.next {
            if *at <= now {
                if self.finish_terminal_drop() {
                    if self.chat_active() {
                        cx.request_redraw_all();
                    } else {
                        cx.request_redraw(self.term_area);
                    }
                }
            } else {
                cx.wake_at(*at);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pastes_image_keystroke_without_wrapping_and_preserves_text_paste() {
        let modes = Modes { bracketed_paste: true, ..Modes::default() };
        assert_eq!(clipboard(None, &modes), b"\x1bv");
        assert_eq!(clipboard(Some(""), &modes), b"\x1bv");
        assert_eq!(clipboard(Some(" "), &modes), b"\x1b[200~ \x1b[201~");
    }

    #[test]
    fn quotes_only_whitespace_and_rejects_controls_and_quotes() {
        let files = [r"C:\one.png", r"C:\my photo.jpg", "bad\rname", "bad\"name", r"C:\Ü\photo.png"].map(PathBuf::from);
        assert_eq!(paths(&files), r#"C:\one.png "C:\my photo.jpg" C:\Ü\photo.png"#);
        let modes = Modes { bracketed_paste: true, ..Modes::default() };
        assert_eq!(termkeys::paste(&paths(&files[..2]), &modes), b"\x1b[200~C:\\one.png \"C:\\my photo.jpg\"\x1b[201~");
    }

    #[test]
    fn sending_paths_resets_only_the_target_scroll_and_reports_visible_damage() {
        use crate::{source::Source, tabs::NewTab, term::Term};
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let mut tab = || {
            let t = Term::empty((10, 10));
            app.tabs.open(NewTab { label: "demo".into(), color: [0; 3], session: None, account: None }, t)
        };
        let old = tab();
        let shown = tab();
        app.tabs.get_mut(old).unwrap().term.scroll = 2;
        app.tabs.get_mut(shown).unwrap().term.scroll = 3;
        let paths = [PathBuf::from("photo.png")];
        assert!(!app.paste_paths(old, &paths));
        assert_eq!(app.tabs.get(old).unwrap().term.scroll, 0);
        assert_eq!(app.tabs.get(shown).unwrap().term.scroll, 3);
        app.session_view.drops.next = Some((std::time::Instant::now(), old, paths.to_vec()));
        app.flush_drop_before_input();
        assert!(app.session_view.drops.next.is_some(), "input in another tab must not consume a drop");
        app.session_view.drops.next = Some((std::time::Instant::now(), shown, paths.to_vec()));
        app.flush_drop_before_input();
        assert!(app.session_view.drops.next.is_none(), "the drop arrives before terminal input");
        assert_eq!(app.tabs.get(shown).unwrap().term.scroll, 0);
        app.tabs.get_mut(shown).unwrap().term.scroll = 1;
        assert!(app.paste_paths(shown, &paths));
    }
}
