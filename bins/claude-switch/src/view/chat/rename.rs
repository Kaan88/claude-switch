//! Claude Code's own session name after a tab rename or a handover: `/rename <name>`, handed only
//! to the Chat view's slash gate (`chat_confirm_slash`), which refuses a running turn, a menu, a
//! full-screen view or a prompt that is not idle and empty. The app never edits the transcript.
//!
//! A handover's rename goes once Chat has read the new session's own first prompt (after `/clear`
//! the gate cannot tell that first turn from an idle input before that, and keys typed then were
//! left on the input under a permission prompt), waited for at most `PROMPT_WAIT`; then the box
//! says to rename in the terminal. Every rename is followed until Claude Code
//! took it: a line that was not entered comes off the input again (`submission::follow_owned`)
//! and is typed again, at most `TRIES` times; then the box says to rename in the terminal.
use std::time::{Duration, Instant};

use super::submission::{COMMAND_INTERRUPTED, COMMAND_LEFT, COMMAND_TIMEOUT, COMMAND_WITHDRAWN, CommandResult};
use crate::state::AppState;

/// Attempts before Chat leaves the rename to the user.
const TRIES: u8 = 3;
pub(super) const GAVE_UP: &str =
    "Claude Code did not take the session's new name. Type /rename and the name in the terminal to set it.";
/// How long a handover's rename waits for Chat to read the new session's first prompt (the
/// transcript's tail Chat reads may hold none, after a first turn of many records).
pub(crate) const PROMPT_WAIT: Duration = Duration::from_secs(120);
pub(super) const NO_PROMPT: &str = "Chat could not read the new session's first message, so it did not set Claude \
    Code's name for it. Type /rename and the name in the terminal to set it.";

pub(super) fn line(name: &str) -> String {
    format!("/rename {}", name.trim())
}

impl AppState {
    /// One step for the shown tab's rename, if it waits for one. True when keys were written.
    pub(super) fn send_cli_rename(&mut self, now: Instant) -> bool {
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let Some(rename) = tab.cli_rename.clone() else { return false };
        // A session followed after `/clear` has no transcript (so no verified ID) until its first
        // message is written: wait for it rather than drop the rename.
        let awaiting = tab.session.is_none()
            && tab.pane.as_ref().is_some_and(|pane| {
                pane.chat.awaiting_transcript
                    && pane.pinned_id().is_some_and(|pin| pin.eq_ignore_ascii_case(&rename.session))
            });
        if awaiting {
            return false;
        }
        if tab.session.as_deref() != Some(rename.session.as_str()) {
            tab.cli_rename = None; // Another session now runs here; never rename that one.
            return false;
        }
        let Some(pane) = tab.pane.as_mut() else { return false };
        if rename.sent {
            let chat = &mut pane.chat;
            let following = chat.command_in_flight();
            let result = if following { None } else { chat.staging.result.take() };
            let next = match result {
                _ if following => return false,
                Some(CommandResult::Taken) => None,
                // Following stopped after its Enter (terminal input): most likely taken; nothing
                // is claimed either way, and it is not typed again over what the user did.
                Some(CommandResult::Unknown) => None,
                // Refused before a key went (the gate's notice says why): it goes again.
                None => Some(rename.clone()),
                Some(CommandResult::Withdrawn) if rename.tries + 1 < TRIES => {
                    // Its notice is about an attempt that goes again.
                    if matches!(chat.error, Some(e) if e == COMMAND_WITHDRAWN || e == COMMAND_INTERRUPTED || e == COMMAND_TIMEOUT)
                    {
                        chat.error = None;
                    }
                    Some(crate::tabs::CliRename { tries: rename.tries + 1, ..rename.clone() })
                }
                Some(CommandResult::Withdrawn | CommandResult::Left) => {
                    if chat.error != Some(COMMAND_LEFT) {
                        chat.error = Some(GAVE_UP);
                        chat.retry = None;
                    }
                    None
                }
            };
            tab.cli_rename = next.map(|r| crate::tabs::CliRename { sent: false, ..r });
            return false;
        }
        // After a handover's `/clear`, the session's own transcript is read: until its first
        // prompt is, a first turn that runs reads as an idle input to the gate. Not for ever: the
        // tail Chat reads may hold no prompt at all.
        if let Some(by) = rename.prompt_by
            && (!pane.model.loaded || pane.model.chat.prompt.is_none())
        {
            if now >= by {
                tab.cli_rename = None;
                pane.chat.error = Some(NO_PROMPT);
                pane.chat.retry = None;
            }
            return false;
        }
        if !pane.model.loaded {
            return false;
        }
        if !pane.chat.enabled || pane.chat.locked() || pane.chat.error.is_some() {
            return false;
        }
        pane.chat.staging.result = None;
        let id = tab.id;
        self.chat_confirm_slash(now, id, Some(rename.session.clone()), &line(&rename.name), false);
        let Some(tab) = self.tabs.get_mut(id) else { return false };
        let Some(pane) = tab.pane.as_ref() else { return false };
        let staged = pane.chat.command_in_flight();
        if let Some(rename) = tab.cli_rename.as_mut().filter(|_| staged) {
            rename.sent = true;
        }
        staged
    }

    /// Whether the shown tab still waits to send a `/rename` (the tick then polls).
    pub(super) fn cli_rename_waiting(&self) -> bool {
        self.tabs
            .shown()
            .is_some_and(|tab| tab.cli_rename.is_some() && tab.pane.as_ref().is_some_and(|pane| pane.chat.enabled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, tabs::NewTab, term::Term};

    #[test]
    fn the_rename_is_one_valid_slash_command_for_the_gate() {
        let line = line("  Toolkit review 😺 ");
        assert_eq!(line, "/rename Toolkit review 😺");
        assert!(super::super::submission::slash_bytes(&line).is_some());
    }

    #[test]
    fn a_pending_rename_waits_for_chat_and_is_dropped_when_the_session_changes() {
        let mut st = AppState::new(Source::Demo, Some(false), Default::default());
        let new = NewTab { label: "tab".into(), color: [0; 3], session: Some("s-1".into()), account: None };
        let id = st.tabs.open(new, Term::empty((40, 8)));
        st.tabs.get_mut(id).unwrap().cli_rename = Some(crate::tabs::CliRename::new("s-1", "toolkit"));
        let now = Instant::now();
        assert!(!st.send_cli_rename(now), "no Chat view in a plain tab: nothing is typed");
        assert!(st.tabs.get(id).unwrap().cli_rename.is_some(), "still waiting");
        st.tabs.get_mut(id).unwrap().session = Some("s-2".into());
        assert!(!st.send_cli_rename(now));
        assert!(st.tabs.get(id).unwrap().cli_rename.is_none(), "never renames another session");
    }

    /// The CLI's input at rest holding `text`, drawn now.
    fn input_holds(f: &mut Live, text: &str) {
        let now = f.now;
        let tab = f.tab();
        tab.term = Term::empty((100, 40));
        tab.term.feed(format!("\x1b[?2004h\u{276f} {text}").as_bytes());
        f.chat().output(now, true, false);
    }

    fn live() -> Live {
        let mut f = Live::new();
        let id = f.tab().id;
        f.app.tabs.active = Some(id);
        f.tab().session = Some("s-1".into());
        f.tab().resume_id = Some("s-1".into());
        f.tab().pane.as_mut().unwrap().model.loaded = true;
        input_holds(&mut f, "");
        f.pass(SECOND);
        f
    }

    /// The window's tick: the gate's stages, then the rename's own step.
    fn tick(f: &mut Live, time: Duration) -> bool {
        f.pass(time);
        let now = f.now;
        f.app.send_cli_rename(now)
    }

    use super::super::submission::ready_tests::Live;
    use crate::tabs::CliRename;
    use std::time::Duration;
    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_rename_waits_for_the_sessions_first_prompt_then_is_followed_until_taken() {
        let mut f = live();
        f.tab().pane.as_mut().unwrap().model.chat.prompt = None;
        let by = f.now + PROMPT_WAIT;
        f.tab().cli_rename = Some(CliRename::after_handover("s-1", "Notes", by));
        assert!(!tick(&mut f, SECOND));
        assert!(f.writes().is_empty(), "not before Chat has read the session's first prompt");
        f.tab().pane.as_mut().unwrap().model.chat.prompt = Some(10);
        assert!(tick(&mut f, SECOND));
        assert_eq!(f.writes(), [b"/rename Notes".to_vec()]);
        input_holds(&mut f, "/rename Notes");
        tick(&mut f, SECOND / 10);
        assert_eq!(f.writes(), [b"\r".to_vec()]);
        assert!(f.tab().cli_rename.as_ref().is_some_and(|r| r.sent), "followed");
        input_holds(&mut f, "");
        tick(&mut f, SECOND / 10);
        tick(&mut f, SECOND / 10);
        assert!(f.tab().cli_rename.is_none(), "taken");
        assert!(f.chat().error.is_none() && f.writes().is_empty());
    }

    #[test]
    fn a_handovers_rename_gives_up_with_a_word_when_no_first_prompt_is_read_in_time() {
        let mut f = live();
        f.tab().pane.as_mut().unwrap().model.chat.prompt = None;
        let by = f.now + PROMPT_WAIT;
        f.tab().cli_rename = Some(CliRename::after_handover("s-1", "Notes", by));
        assert!(!tick(&mut f, PROMPT_WAIT / 2));
        assert!(f.tab().cli_rename.is_some(), "still waiting");
        assert!(!tick(&mut f, PROMPT_WAIT));
        assert!(f.writes().is_empty() && f.tab().cli_rename.is_none(), "given up, nothing typed");
        assert_eq!(f.chat().error, Some(NO_PROMPT), "the box says to rename in the terminal");
    }

    #[test]
    fn a_tab_rename_does_not_wait_for_a_first_prompt() {
        let mut f = live();
        f.tab().pane.as_mut().unwrap().model.chat.prompt = None;
        f.tab().cli_rename = Some(CliRename::new("s-1", "Notes"));
        assert!(tick(&mut f, SECOND));
        assert_eq!(f.writes(), [b"/rename Notes".to_vec()]);
    }

    #[test]
    fn terminal_input_after_the_renames_enter_drops_it_without_claiming_it_failed() {
        let mut f = live();
        f.tab().cli_rename = Some(CliRename::new("s-1", "Notes"));
        assert!(tick(&mut f, SECOND));
        assert_eq!(f.writes(), [b"/rename Notes".to_vec()]);
        input_holds(&mut f, "/rename Notes");
        tick(&mut f, SECOND / 10);
        assert_eq!(f.writes(), [vec![13u8]], "Enter");
        // A wheel or a key in the terminal right after Enter.
        f.chat().interrupt_staging();
        assert!(!tick(&mut f, SECOND / 10));
        assert!(f.tab().cli_rename.is_none(), "not typed again over what the user did");
        assert!(f.chat().error.is_none(), "no word that Claude Code did not take it");
        assert!(f.writes().is_empty());
    }

    #[test]
    fn a_rename_that_is_not_taken_comes_off_the_input_goes_again_and_is_given_up_with_a_word() {
        let mut f = live();
        f.tab().cli_rename = Some(CliRename::new("s-1", "Notes"));
        for attempt in 0..TRIES {
            assert!(tick(&mut f, SECOND), "attempt {attempt}");
            assert_eq!(f.writes(), [b"/rename Notes".to_vec()], "attempt {attempt}");
            input_holds(&mut f, "/rename Notes");
            tick(&mut f, SECOND / 10);
            assert_eq!(f.writes(), [b"\r".to_vec()]);
            // Claude Code does not take it: the line stays.
            tick(&mut f, SECOND * 4);
            input_holds(&mut f, "/rename Notes");
            tick(&mut f, SECOND);
            assert_eq!(f.writes(), [b"\x15".to_vec()], "exactly the line comes off");
            input_holds(&mut f, "");
            assert!(!tick(&mut f, SECOND));
        }
        assert!(f.tab().cli_rename.is_none(), "given up after {TRIES} attempts");
        assert_eq!(f.chat().error, Some(GAVE_UP), "the box says to rename in the terminal");
        assert!(!f.chat().locked() && !f.chat().command_in_flight());
        assert!(!tick(&mut f, SECOND * 5));
        assert!(f.writes().is_empty(), "nothing more is typed");
    }
}
