//! Drives an in-place handover (`handover_in_place`) through the Chat view's own gate: `/clear`
//! via `chat_confirm_slash`, then the first message via the composer's send. Nothing here
//! writes to the terminal itself, and nothing goes around the gate's checks.
use std::time::{Duration, Instant};

use crate::handover_in_place::Stage;
use crate::state::AppState;

/// The gate refuses every send while Claude Code draws its full-screen view; waiting cannot help.
const FULLSCREEN: &str = "Claude Code draws its full-screen view in this tab, where Chat cannot send \
    (type /tui default there, or set Where it continues to A new tab)";

/// While the tab waits to follow the CLI, the running records are read this often.
const FOLLOW_POLL: Duration = Duration::from_secs(1);

enum Step {
    Wait,
    Fail(&'static str),
    Done(String),
}

impl AppState {
    /// One step for every tab with an in-place handover. True when something changed.
    pub(super) fn in_place_tick(&mut self, now: Instant) -> bool {
        let ids: Vec<u64> = self.tabs.list.iter().filter(|t| t.in_place.is_some()).map(|t| t.id).collect();
        let mut changed = false;
        for id in ids {
            match self.in_place_step(id, now) {
                Step::Wait => {}
                Step::Fail(reason) => {
                    self.in_place_failed(id, reason);
                    changed = true;
                }
                Step::Done(new) => {
                    self.in_place_done(id, &new);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Whether an in-place handover waits (the tick then polls).
    pub(super) fn in_place_waiting(&self) -> bool {
        self.tabs.list.iter().any(|t| t.in_place.is_some())
    }

    fn in_place_step(&mut self, id: u64, now: Instant) -> Step {
        let shown = self.tabs.active == Some(id);
        let demo = self.source.is_demo();
        let Some(tab) = self.tabs.get_mut(id) else { return Step::Wait };
        let Some(job) = &tab.in_place else { return Step::Wait };
        // Demo tabs run no program; their Chat view simulates the writes.
        if !tab.term.running() && !demo {
            return Step::Fail("the tab's program ended");
        }
        if now >= job.deadline() {
            return Step::Fail(job.timeout_reason());
        }
        let (stage, from) = (job.stage, job.from_session.clone());
        let current = tab.resume_id.clone();
        let Some(pane) = &tab.pane else { return Step::Fail("the tab has no Chat view") };
        match stage {
            Stage::Clear => {
                if current.as_deref() != Some(from.as_str()) {
                    return Step::Fail("the tab started another session before /clear");
                }
                if pane.chat.error == Some(super::submission::FULLSCREEN) {
                    return Step::Fail(FULLSCREEN);
                }
                if !shown || !pane.chat.enabled || pane.chat.locked() || pane.chat.error.is_some() {
                    return Step::Wait; // The gate's own retry clears a refusal once the prompt is idle.
                }
                self.chat_confirm_slash(now, id, Some(from), "/clear", false);
                let Some(tab) = self.tabs.get_mut(id) else { return Step::Wait };
                if tab.pane.as_ref().is_some_and(|p| p.chat.error.is_none())
                    && let Some(job) = &mut tab.in_place
                {
                    job.stage = Stage::Follow;
                    job.since = now;
                }
                Step::Wait
            }
            Stage::Follow => {
                let followed = current.filter(|new| !new.eq_ignore_ascii_case(&from));
                match followed {
                    Some(new) if pane.pinned_id().is_some_and(|pin| pin.eq_ignore_ascii_case(&new)) => {
                        if let Some(job) = &mut tab.in_place {
                            job.stage = Stage::Prompt;
                            job.since = now;
                        }
                    }
                    _ => {
                        // Read the CLI's live record sooner than the usual poll.
                        let soon = now + FOLLOW_POLL;
                        self.next_poll = Some(self.next_poll.map_or(soon, |at| at.min(soon)));
                    }
                }
                Step::Wait
            }
            Stage::Prompt => {
                let Some(new) = current.filter(|new| !new.eq_ignore_ascii_case(&from)) else {
                    return Step::Fail("the tab went back to the old session");
                };
                let chat = &pane.chat;
                if chat.error == Some(super::submission::FULLSCREEN) {
                    return Step::Fail(FULLSCREEN);
                }
                if !shown || !chat.enabled || chat.locked() || chat.error.is_some() {
                    return Step::Wait;
                }
                if !chat.editor.text.trim().is_empty() || !chat.chips.is_empty() {
                    return Step::Wait; // Never replace the user's draft; the deadline says why.
                }
                let prompt = tab.in_place.as_ref().map(|j| j.prompt.clone()).unwrap_or_default();
                let pane = tab.pane.as_mut().unwrap();
                pane.chat.editor.set(prompt);
                Self::chat_send(tab, now, false);
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                if chat.error.is_none() && (chat.editor.text.is_empty() || chat.queue.is_some()) {
                    return Step::Done(new);
                }
                if chat.queue.is_none() {
                    chat.editor.set(String::new()); // Refused: leave the box as it was.
                }
                Step::Wait
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handover_in_place::{FOLLOW_WAIT, InPlace};
    use crate::{source::Source, tabs::NewTab, term::Term};
    use std::path::PathBuf;

    fn job(stage: Stage, since: Instant) -> InPlace {
        InPlace {
            from_session: "old".into(),
            from_name: "Old".into(),
            name: "Old · continued".into(),
            prompt: "## Goal\n\nContinue.".into(),
            saved: PathBuf::from("C:/handover/old-1.md"),
            cwd: PathBuf::from("C:/"),
            stage,
            since,
        }
    }

    fn app() -> (AppState, u64) {
        let mut st = AppState::new(Source::Demo, Some(false), Default::default());
        let new = NewTab { label: "t".into(), color: [0; 3], session: Some("old".into()), account: None };
        let id = st.tabs.open(new, Term::empty((40, 8)));
        (st, id)
    }

    #[test]
    fn an_ended_program_or_a_missed_deadline_stops_the_handover_and_names_the_saved_file() {
        let (mut st, id) = app();
        let start = Instant::now();
        st.tabs.get_mut(id).unwrap().in_place = Some(job(Stage::Follow, start));
        assert!(st.in_place_tick(start + FOLLOW_WAIT));
        assert!(st.tabs.get(id).unwrap().in_place.is_none());
        assert!(st.info.is_some(), "the reason is shown");
        st.tabs.get_mut(id).unwrap().in_place = Some(job(Stage::Clear, start));
        st.tabs.get_mut(id).unwrap().resume_id = Some("other".into());
        assert!(st.in_place_tick(start), "a different session before /clear stops it at once");
        let text = crate::handover_in_place::failure_text("x", &PathBuf::from("C:/handover/old-1.md"));
        assert!(text.contains("old-1.md") && text.contains("Nothing else was sent"), "{text}");
    }

    #[test]
    fn clear_then_follow_then_the_handover_as_first_message_all_through_the_gate() {
        let mut st = super::super::tests::demo();
        let start = Instant::now();
        st.win_file.chat_default = false;
        let tab = st.tabs.shown_mut().unwrap();
        let id = tab.id;
        let old = tab.session.clone().unwrap();
        tab.resume_id = Some(old.clone());
        tab.pane.as_mut().unwrap().chat.enabled = true;
        let mut first = job(Stage::Clear, start);
        first.from_session = old.clone();
        tab.in_place = Some(first);
        st.in_place_tick(start);
        let tab = st.tabs.get(id).unwrap();
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert_eq!(chat.writes.first().map(Vec::as_slice), Some(b"/clear".as_slice()), "{:?}", chat.error);
        assert_eq!(tab.in_place.as_ref().map(|j| j.stage), Some(Stage::Follow));
        // Still the old session: the tab waits for the CLI, then a new id that the pane follows.
        assert!(!st.in_place_tick(start));
        let tab = st.tabs.get_mut(id).unwrap();
        let job = tab.in_place.as_mut().unwrap();
        job.stage = Stage::Prompt;
        tab.resume_id = Some("11111111-2222-4333-8444-555555555555".into());
        let pane = tab.pane.as_mut().unwrap();
        pane.chat.writes.clear();
        pane.chat.error = None;
        pane.chat.staging = Default::default(); // As after Claude Code echoed and took /clear.
        pane.chat.editor.set("A draft of mine".into());
        assert!(!st.in_place_tick(start), "never replaces a draft");
        let chat = &mut st.tabs.get_mut(id).unwrap().pane.as_mut().unwrap().chat;
        assert_eq!(chat.editor.text, "A draft of mine");
        chat.editor.set(String::new());
        // The new session's prompt is idle: a quiet screen after a finished turn.
        let pane = st.tabs.get_mut(id).unwrap().pane.as_mut().unwrap();
        pane.chat.fresh = true;
        pane.chat.last_output = Some(start - super::super::QUIET);
        pane.model.chat.latest = Some((100, claude_switch_core::transcript::ChatPhase::EndTurn));
        assert!(st.in_place_tick(start), "{:?}", st.tabs.get(id).unwrap().pane.as_ref().unwrap().chat.error);
        let tab = st.tabs.get(id).unwrap();
        assert!(tab.in_place.is_none(), "done");
        assert_eq!(
            tab.cli_rename,
            Some(("11111111-2222-4333-8444-555555555555".into(), "Old · continued".into())),
            "Claude Code's own name follows with /rename at the next idle prompt"
        );
        let writes = &tab.pane.as_ref().unwrap().chat.writes;
        assert!(writes.iter().any(|w| String::from_utf8_lossy(w).contains("## Goal")), "{writes:?}");
    }

    #[test]
    fn a_tab_in_claude_codes_full_screen_view_stops_at_once() {
        let mut st = super::super::tests::demo();
        let id = st.tabs.shown().unwrap().id;
        let start = Instant::now();
        let tab = st.tabs.get_mut(id).unwrap();
        tab.resume_id = Some("old".into());
        tab.in_place = Some(job(Stage::Clear, start));
        tab.pane.as_mut().unwrap().chat.error = Some(super::super::submission::FULLSCREEN);
        assert!(st.in_place_tick(start), "no waiting for the deadline");
        assert!(st.tabs.get(id).unwrap().in_place.is_none());
        assert!(st.tabs.get(id).unwrap().pane.as_ref().unwrap().chat.writes.is_empty());
    }

    #[test]
    fn chat_for_the_handover_tab_only_leaves_the_saved_default_alone() {
        let mut st = super::super::tests::demo();
        st.win_file.chat_default = false;
        st.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat.enabled = false;
        assert!(st.set_shown_tab_chat(true));
        assert!(st.tabs.shown().unwrap().pane.as_ref().unwrap().chat.enabled);
        assert!(!st.win_file.chat_default, "new tabs still open as the user chose");
        assert!(!st.set_shown_tab_chat(true), "already in Chat");
    }

    #[test]
    fn nothing_is_typed_while_the_tab_is_not_shown_in_the_chat_view() {
        let mut st = super::super::tests::demo();
        let id = st.tabs.shown().unwrap().id;
        let start = Instant::now();
        let tab = st.tabs.get_mut(id).unwrap();
        tab.resume_id = Some("old".into());
        tab.in_place = Some(job(Stage::Clear, start));
        st.show_tab(crate::tabs::SESSIONS);
        assert!(!st.in_place_tick(start));
        let tab = st.tabs.get(id).unwrap();
        assert_eq!(tab.in_place.as_ref().map(|j| j.stage), Some(Stage::Clear));
        assert!(tab.pane.as_ref().unwrap().chat.writes.is_empty());
    }
}
