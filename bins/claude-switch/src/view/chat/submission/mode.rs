//! The permission mode chip. The mode is read from the indicator under the CLI's input box; a
//! click writes one Shift+Tab (`CSI Z`, the CLI's own cycle) at a verified, quiet, empty prompt,
//! and the next click is taken only after the indicator changed. An unreadable indicator stops.
use std::time::{Duration, Instant};

use mdshared_term::Screen;

use super::screen::clean_prompt;
use super::{Intent, blocked, write};
use crate::state::AppState;
use crate::tabs::Tab;
use crate::view::chat::Chat;
use crate::view::chat::question::screens;

const STEP: Duration = Duration::from_secs(2);
const SHIFT_TAB: &[u8] = b"\x1b[Z";
/// No indicator under a clean prompt: the default mode shows none (to confirm in a live session).
const DEFAULT: &str = "Default";
const UNREADABLE: &str = "The permission mode cannot be read in the terminal. Check the terminal.";
const NOT_CHANGED: &str = "The permission mode did not change. Check the terminal.";

/// A Shift+Tab in flight: the mode it left and when it must have changed.
#[derive(Clone, Copy, Debug)]
pub(in crate::view::chat) struct ModeStep {
    from: &'static str,
    due: Instant,
}

/// The mode the screen shows, `None` when it cannot be read.
pub(in crate::view::chat) fn read(screen: &Screen) -> Option<&'static str> {
    screens::mode(screen).or_else(|| clean_prompt(screen).then_some(DEFAULT))
}

impl Chat {
    /// Follows the screen's indicator; true when the chip changed.
    pub(super) fn observe_mode(&mut self, screen: &Screen) -> bool {
        let mode = read(screen);
        std::mem::replace(&mut self.mode_shown, mode) != mode
    }

    pub(in crate::view::chat) fn mode_label(&self) -> String {
        match (self.mode_step, self.mode_shown) {
            (Some(_), _) => "Mode …".to_owned(),
            (None, Some(mode)) => format!("Mode: {mode}"),
            (None, None) => "Mode —".to_owned(),
        }
    }

    pub(in crate::view::chat) fn mode_deadline(&self) -> Option<Instant> {
        self.mode_step.map(|step| step.due)
    }
}

impl AppState {
    /// A click on the mode chip.
    pub(in crate::view::chat) fn chat_mode(tab: &mut Tab, now: Instant) {
        let error = blocked(tab, now, Intent::Slash);
        let mode = read(&tab.term.screen);
        let chat = &mut tab.pane.as_mut().expect("Chat pane").chat;
        if chat.mode_step.is_some() || chat.locked() || chat.waiting().is_some() {
            return;
        }
        let Some(from) = mode else {
            chat.refused(UNREADABLE, None);
            return;
        };
        if let Some(error) = error {
            chat.refused(error, None);
            return;
        }
        if chat.demo {
            // The demo has no CLI whose mode could change.
            #[cfg(test)]
            chat.writes.push(SHIFT_TAB.to_vec());
            return;
        }
        match write(tab, SHIFT_TAB, "") {
            Ok(()) => {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.error = None;
                chat.mode_step = Some(ModeStep { from, due: now + STEP });
            }
            Err(error) => tab.pane.as_mut().unwrap().chat.refused(error, None),
        }
    }
}

/// Ends a Shift+Tab in flight once the indicator changed; true when the chip changed.
pub(super) fn advance(tab: &mut Tab, now: Instant) -> bool {
    let mode = read(&tab.term.screen);
    let chat = &mut tab.pane.as_mut().expect("Chat pane").chat;
    let Some(step) = chat.mode_step else { return false };
    match mode {
        Some(mode) if mode != step.from => chat.mode_step = None,
        _ if now >= step.due => {
            chat.mode_step = None;
            chat.refused(if mode.is_none() { UNREADABLE } else { NOT_CHANGED }, None);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::tabs::NewTab;
    use crate::term::Term;
    use crate::view::chat::QUIET;

    fn prompt(indicator: &str) -> String {
        let rule = "─".repeat(20);
        format!("\x1b[2J\x1b[H\x1b[?2004h{rule}\r\n❯ \r\n{rule}\r\n  {indicator}\x1b[2;3H")
    }

    #[test]
    fn the_mode_is_read_below_the_prompt_only() {
        let mut screen = Screen::new(60, 8, 0);
        screen.feed(prompt("⏸ plan mode on (shift+tab to cycle)").as_bytes());
        assert_eq!(read(&screen), Some("Plan"));
        screen.feed(prompt("⏵⏵ accept edits on (shift+tab to cycle)").as_bytes());
        assert_eq!(read(&screen), Some("Accept edits"));
        screen.feed(prompt("? for shortcuts").as_bytes());
        assert_eq!(read(&screen), Some(DEFAULT));
        let mut busy = Screen::new(60, 8, 0);
        busy.feed("Plan mode on is what I said\r\n❯ typing".as_bytes());
        assert_eq!(read(&busy), None, "text above or a busy prompt is not an indicator");
    }

    fn chat(tab: &mut Tab) -> &mut Chat {
        &mut tab.pane.as_mut().unwrap().chat
    }

    #[test]
    fn a_click_writes_one_shift_tab_and_the_next_waits_for_the_change() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let root = std::env::temp_dir();
        let mut pane = app.new_session_pane(crate::view::model::Source::File(root.join("mode.jsonl")), root);
        pane.chat.enabled = true;
        pane.chat.demo = true;
        pane.chat.identity_unverified = false;
        let mut term = Term::empty((60, 8));
        term.feed(prompt("⏸ plan mode on").as_bytes());
        let id = app.tabs.open(NewTab { label: "M".into(), color: [0; 3], session: None, account: None }, term);
        let tab = app.tabs.get_mut(id).unwrap();
        tab.pane = Some(pane);
        let now = Instant::now();
        chat(tab).last_output = Some(now - QUIET);
        AppState::chat_mode(tab, now);
        assert_eq!(chat(tab).error, None);
        assert_eq!(chat(tab).writes, [SHIFT_TAB.to_vec()]);

        // As if the write had gone: the next click waits for the indicator.
        chat(tab).error = None;
        chat(tab).mode_step = Some(ModeStep { from: "Plan", due: now + STEP });
        chat(tab).writes.clear();
        AppState::chat_mode(tab, now);
        assert!(chat(tab).writes.is_empty());
        assert!(!advance(tab, now), "unchanged before the deadline");
        tab.term.feed(prompt("⏵⏵ accept edits on").as_bytes());
        assert!(advance(tab, now));
        assert!(chat(tab).mode_step.is_none() && chat(tab).error.is_none());

        chat(tab).mode_step = Some(ModeStep { from: "Accept edits", due: now + STEP });
        assert!(advance(tab, now + STEP));
        assert_eq!(chat(tab).error, Some(NOT_CHANGED));
    }
}
