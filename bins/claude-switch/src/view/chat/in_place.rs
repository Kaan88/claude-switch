//! Drives an in-place handover (`handover_in_place`) through the Chat view's own gate: `/clear`
//! via `chat_confirm_slash`, then the first message via the composer's send. Nothing here
//! writes to the terminal itself, and nothing goes around the gate's checks.
use std::time::{Duration, Instant};

use crate::handover_in_place::Stage;
use crate::state::AppState;

/// The user sent something after the handover was made: clearing now would lose that exchange.
const STALE: &str = "a message was sent in this session after the handover was made, so it would leave that \
    out (make the handover again)";

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
                if pane.model.chat.prompt.is_some_and(|at| at >= job.basis) {
                    return Step::Fail(STALE);
                }
                if !shown || !pane.chat.enabled || pane.chat.locked() || pane.chat.error.is_some() {
                    return Step::Wait; // The gate's own retry clears a refusal once the prompt is idle.
                }
                if pane.chat.has_draft() {
                    // A draft would become the cleared session's first message: never /clear under
                    // it; wait for it to go (the deadline says why if it stays).
                    return Step::Wait;
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
                if pane.chat.error.is_some() {
                    // The gate refused or interrupted /clear (its notice says why).
                    return Step::Fail("Claude Code did not take /clear");
                }
                match followed(pane, current.as_deref(), &from) {
                    Some(_) => {
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
            Stage::Prompt | Stage::Sending => {
                let Some(new) = followed(pane, current.as_deref(), &from) else {
                    return Step::Fail("the tab went back to the old session");
                };
                if stage == Stage::Sending {
                    let chat = &pane.chat;
                    if chat.error.is_some() {
                        // The draft stays in the box for the gate's own recovery.
                        return Step::Fail("Claude Code did not take the handover as its first message");
                    }
                    if !chat.locked() && chat.editor.text.is_empty() {
                        return Step::Done(new);
                    }
                    return Step::Wait;
                }
                let chat = &pane.chat;
                if chat.error == Some(super::submission::FULLSCREEN) {
                    return Step::Fail(FULLSCREEN);
                }
                if !shown || !chat.enabled || chat.locked() || chat.error.is_some() {
                    return Step::Wait;
                }
                if chat.has_draft() {
                    return Step::Wait; // Never replace the user's draft; the deadline says why.
                }
                let prompt = tab.in_place.as_ref().map(|j| j.prompt.clone()).unwrap_or_default();
                let pane = tab.pane.as_mut().unwrap();
                pane.chat.editor.set(prompt);
                Self::chat_send(tab, now, false);
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                if chat.error.is_none() && chat.editor.text.is_empty() {
                    return Step::Done(new);
                }
                if chat.error.is_none() && chat.locked() {
                    // Taken: the gate stages it (or queues it behind a turn) and enters it itself.
                    if let Some(job) = &mut tab.in_place {
                        job.stage = Stage::Sending;
                        job.since = now;
                    }
                    return Step::Wait;
                }
                chat.editor.set(String::new()); // Refused: leave the box as it was.
                Step::Wait
            }
        }
    }
}

/// The session the tab followed the CLI to after `/clear`: the pane's pinned ID once it differs
/// from the old one. Until its first message writes the transcript, the pane awaits it and the
/// tab has no resumable ID yet; after that the tab's verified ID must agree.
fn followed(pane: &crate::view::Pane, resume: Option<&str>, from: &str) -> Option<String> {
    let pin = pane.pinned_id().filter(|pin| !pin.eq_ignore_ascii_case(from))?;
    let agrees = resume.is_some_and(|id| id.eq_ignore_ascii_case(pin));
    (pane.chat.awaiting_transcript && !pane.chat.identity_unverified || agrees).then(|| pin.to_owned())
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
            basis: u64::MAX,
            stage,
            since,
            delete_original: false,
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

    const OLD: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const NEW: &str = "bbbbbbbb-0000-4000-8000-000000000002";

    /// A Claude tab on a real transcript, as the session branch's identity tests build it: the
    /// gate, the live-record following and the first-message rules are the real ones; only the
    /// writes are recorded instead of reaching a process.
    struct Real {
        st: AppState,
        root: PathBuf,
        id: u64,
    }

    impl Drop for Real {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Real {
        fn new(name: &str) -> Self {
            let root = crate::testing::unique(&format!("cs-in-place-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let mut st = AppState::new(Source::Demo, Some(false), Default::default());
            st.win_file.chat_default = false;
            let file = Self::transcript_at(&root, OLD);
            let mut pane = st.new_session_pane(crate::view::model::Source::File(file), root.clone());
            pane.chat.demo = true;
            pane.chat.demo_progress = true;
            pane.chat.enabled = true;
            let mut term = Term::empty((80, 12));
            term.feed("\x1b[?2004h❯ ".as_bytes());
            let id = st.tabs.open(NewTab { label: "t".into(), color: [0; 3], session: None, account: None }, term);
            st.tabs.get_mut(id).unwrap().pane = Some(pane);
            st.tabs.active = Some(id);
            let mut real = Self { st, root, id };
            assert!(real.tab().apply_live_identity(Some(OLD)));
            assert_eq!(real.tab().resume_id.as_deref(), Some(OLD));
            real
        }

        fn transcript_at(root: &std::path::Path, id: &str) -> PathBuf {
            let file = crate::view::handover_transcript(root, root, id);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            let record = serde_json::json!({ "type": "user", "cwd": root }).to_string();
            std::fs::write(&file, format!("{record}\n")).unwrap();
            file
        }

        fn tab(&mut self) -> &mut crate::tabs::Tab {
            self.st.tabs.get_mut(self.id).unwrap()
        }

        fn chat(&mut self) -> &mut super::super::Chat {
            &mut self.tab().pane.as_mut().unwrap().chat
        }

        /// What the chat tick does between steps: the gate's own staging, on a quiet screen.
        fn settle(&mut self, now: Instant) {
            self.chat().last_output = Some(now - super::super::QUIET);
            AppState::advance_submission(self.tab(), now, true);
        }

        fn stage(&mut self) -> Option<Stage> {
            self.tab().in_place.as_ref().map(|j| j.stage)
        }
    }

    #[test]
    fn clear_then_follow_then_the_handover_as_first_message_all_through_the_gate() {
        let mut r = Real::new("flow");
        let now = Instant::now();
        let mut first = job(Stage::Clear, now);
        first.from_session = OLD.into();
        first.cwd = r.root.clone();
        r.tab().in_place = Some(first);
        r.chat().last_output = Some(now - super::super::QUIET);

        // 1. `/clear` through the slash gate, which re-checks the session right before writing.
        r.st.in_place_tick(now);
        let error = r.chat().error;
        assert_eq!(r.chat().writes.first().map(Vec::as_slice), Some(b"/clear".as_slice()), "{error:?}");
        assert_eq!(r.stage(), Some(Stage::Follow));
        r.settle(now);
        assert!(r.chat().writes.iter().any(|w| w == b"\r"), "entered once the input line shows it");

        // 2. The CLI names a new session whose transcript it has yet to write: the real follow.
        assert!(!r.st.in_place_tick(now), "still the old session");
        assert!(r.tab().apply_live_identity(Some(NEW)));
        let tab = r.tab();
        assert_eq!(tab.resume_id, None, "nothing to resume before the first message");
        assert!(tab.pane.as_ref().unwrap().chat.awaiting_transcript);
        r.st.in_place_tick(now);
        assert_eq!(r.stage(), Some(Stage::Prompt));

        // A draft of the user's own is never replaced.
        r.chat().editor.set("A draft of mine".into());
        r.chat().writes.clear();
        assert!(!r.st.in_place_tick(now));
        assert_eq!(r.chat().editor.text, "A draft of mine");
        assert!(r.chat().writes.is_empty());
        r.chat().editor.set(String::new());
        // Nor is shell mode turned on with no command yet: the handover would have gone as a
        // shell command, refused as several lines, with the session already cleared.
        r.chat().shell_mode = true;
        assert!(!r.st.in_place_tick(now));
        assert!(r.chat().editor.text.is_empty() && r.chat().writes.is_empty() && r.chat().shell_mode);
        r.chat().shell_mode = false;

        // 3. The handover as the new session's genuine first message: an ordinary Send of several
        // lines, staged until the input line shows them, then entered by the gate.
        r.chat().last_output = Some(now - super::super::QUIET);
        let taken = r.st.in_place_tick(now);
        let error = r.chat().error;
        assert!(!taken, "{error:?}");
        assert_eq!(r.stage(), Some(Stage::Sending));
        assert!(String::from_utf8_lossy(&r.chat().writes[0]).contains("## Goal"));
        assert!(!r.chat().writes.iter().any(|w| w == b"\r"), "no Enter before the input line shows it");
        r.settle(now);
        let error = r.chat().error;
        assert!(r.chat().writes.iter().any(|w| w == b"\r"), "{error:?}");
        assert!(r.chat().first_sent, "the gate counts it as the new chat's first message");
        assert!(r.st.in_place_tick(now));
        let tab = r.tab();
        assert!(tab.in_place.is_none(), "done");
        assert_eq!(tab.cli_rename, Some((NEW.into(), "Old · continued".into())));

        // 4. `/rename` waits while the new transcript does not exist yet, then goes through the gate.
        r.chat().writes.clear();
        r.chat().last_output = Some(now - super::super::QUIET);
        assert!(!r.st.send_cli_rename(now));
        assert!(r.tab().cli_rename.is_some(), "kept, not dropped, while the session awaits its transcript");
        Real::transcript_at(&r.root.clone(), NEW);
        r.tab().apply_live_identity(Some(NEW));
        assert_eq!(r.tab().session.as_deref(), Some(NEW));
        r.chat().last_output = Some(now - super::super::QUIET);
        let sent = r.st.send_cli_rename(now);
        let error = r.chat().error;
        assert!(sent, "{error:?}");
        assert_eq!(r.chat().writes.first().map(Vec::as_slice), Some(b"/rename Old \xc2\xb7 continued".as_slice()));
        assert!(r.tab().cli_rename.is_none());
    }

    #[test]
    fn a_handover_the_gate_could_not_enter_stops_and_leaves_the_draft_for_recovery() {
        let mut r = Real::new("lines-timeout");
        let now = Instant::now();
        r.tab().apply_live_identity(Some(NEW));
        let mut sending = job(Stage::Prompt, now);
        sending.from_session = OLD.into();
        r.tab().in_place = Some(sending);
        r.chat().last_output = Some(now - super::super::QUIET);
        r.st.in_place_tick(now);
        assert_eq!(r.stage(), Some(Stage::Sending));
        // The input line never shows the paste: the gate times out without Enter.
        r.tab().term.feed("\x1b[2J\x1b[H\x1b[?2004h❯ ".as_bytes());
        AppState::advance_submission(r.tab(), now + std::time::Duration::from_secs(10), true);
        assert!(r.chat().error.is_some());
        assert!(r.st.in_place_tick(now));
        assert!(r.tab().in_place.is_none(), "stopped with the reason");
        assert!(r.chat().editor.text.contains("## Goal"), "the gate keeps the draft for its recovery");
        assert!(!r.chat().writes.iter().any(|w| w == b"\r"), "never entered");
    }

    #[test]
    fn a_draft_in_the_composer_holds_the_clear_so_the_session_is_never_cleared_without_the_handover() {
        // The reviewer's case: a draft, then a tick. Before, /clear went out and the send stage
        // then waited on the draft until its deadline, leaving the session cleared.
        let mut r = Real::new("draft-before-clear");
        let now = Instant::now();
        let mut first = job(Stage::Clear, now);
        first.from_session = OLD.into();
        r.tab().in_place = Some(first);
        r.chat().last_output = Some(now - super::super::QUIET);
        r.chat().editor.set("my next question".into());
        assert!(!r.st.in_place_tick(now));
        assert!(r.chat().writes.is_empty(), "no /clear under a draft");
        assert_eq!(r.stage(), Some(Stage::Clear));
        assert!(r.st.in_place_tick(now + crate::handover_in_place::CLEAR_WAIT), "stops at the deadline");
        assert!(r.chat().writes.is_empty() && r.tab().in_place.is_none());
        assert_eq!(r.chat().editor.text, "my next question", "the draft is the user's");
        assert_eq!(r.tab().resume_id.as_deref(), Some(OLD), "the session was never cleared");
    }

    #[test]
    fn a_message_sent_after_the_handover_was_made_stops_it_before_clear() {
        let mut r = Real::new("stale");
        let now = Instant::now();
        let mut first = job(Stage::Clear, now);
        first.from_session = OLD.into();
        first.basis = 500;
        r.tab().in_place = Some(first);
        r.chat().last_output = Some(now - super::super::QUIET);
        // The newest prompt starts before the handover's basis: it is covered.
        r.tab().pane.as_mut().unwrap().model.chat.prompt = Some(120);
        r.st.in_place_tick(now);
        assert_eq!(r.chat().writes.first().map(Vec::as_slice), Some(b"/clear".as_slice()));

        let mut r = Real::new("stale-newer");
        let mut first = job(Stage::Clear, now);
        first.from_session = OLD.into();
        first.basis = 500;
        r.tab().in_place = Some(first);
        r.chat().last_output = Some(now - super::super::QUIET);
        r.tab().pane.as_mut().unwrap().model.chat.prompt = Some(500);
        assert!(r.st.in_place_tick(now), "stops at once");
        assert!(r.chat().writes.is_empty(), "never clears with a stale handover");
        assert!(r.tab().in_place.is_none());
        assert!(r.st.info.is_some() || r.st.dialog.is_some(), "says why");
    }

    #[test]
    fn a_cli_menu_holds_the_clear_until_it_is_closed() {
        let mut r = Real::new("menu");
        let now = Instant::now();
        let mut first = job(Stage::Clear, now);
        first.from_session = OLD.into();
        r.tab().in_place = Some(first);
        r.chat().error = Some(super::super::Reason::Menu.label());
        assert!(!r.st.in_place_tick(now));
        assert_eq!(r.stage(), Some(Stage::Clear), "waits for the gate's own retry");
        assert!(r.chat().writes.is_empty());
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
