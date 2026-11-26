//! Claude Code's own session name after a tab rename: `/rename <name>`, handed only to the Chat
//! view's slash gate (`chat_confirm_slash`), which refuses a running turn, a menu, a full-screen
//! view or a prompt that is not idle and empty. The app never edits the transcript.
use std::time::Instant;

use crate::state::AppState;

pub(super) fn line(name: &str) -> String {
    format!("/rename {}", name.trim())
}

impl AppState {
    /// One attempt for the shown tab, if it waits for one. A refusal stays shown by the gate; its
    /// own retry clears it once the prompt is idle, and the next tick sends. True when written.
    pub(super) fn send_cli_rename(&mut self, now: Instant) -> bool {
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let Some((session, name)) = tab.cli_rename.clone() else { return false };
        // A session followed after `/clear` has no transcript (so no verified ID) until its first
        // message is written: wait for it rather than drop the rename.
        let awaiting = tab.session.is_none()
            && tab.pane.as_ref().is_some_and(|pane| {
                pane.chat.awaiting_transcript && pane.pinned_id().is_some_and(|pin| pin.eq_ignore_ascii_case(&session))
            });
        if awaiting {
            return false;
        }
        if tab.session.as_deref() != Some(session.as_str()) {
            tab.cli_rename = None; // Another session now runs here; never rename that one.
            return false;
        }
        let Some(pane) = &tab.pane else { return false };
        if !pane.chat.enabled || pane.chat.locked() || pane.chat.error.is_some() {
            return false;
        }
        let id = tab.id;
        self.chat_confirm_slash(now, id, Some(session), &line(&name), false);
        let Some(tab) = self.tabs.get_mut(id) else { return false };
        let sent = tab.pane.as_ref().is_some_and(|pane| pane.chat.error.is_none());
        if sent {
            tab.cli_rename = None;
        }
        sent
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
        st.tabs.get_mut(id).unwrap().cli_rename = Some(("s-1".into(), "toolkit".into()));
        let now = Instant::now();
        assert!(!st.send_cli_rename(now), "no Chat view in a plain tab: nothing is typed");
        assert!(st.tabs.get(id).unwrap().cli_rename.is_some(), "still waiting");
        st.tabs.get_mut(id).unwrap().session = Some("s-2".into());
        assert!(!st.send_cli_rename(now));
        assert!(st.tabs.get(id).unwrap().cli_rename.is_none(), "never renames another session");
    }
}
