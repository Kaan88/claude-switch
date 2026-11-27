//! A handover that continues in place: in the tab and CLI process that run the session, Switch
//! sends `/clear` at an idle prompt, waits until the tab has followed the CLI to its new session,
//! then sends the handover as that session's first message, always through the Chat view's gate
//! (`view::chat::in_place`). The new session is linked as the old one's continuation and the old
//! one is marked handed over. Every step has a deadline; a step that does not happen in time
//! stops the handover with its reason. The handover file is kept either way.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::Session;

use crate::state::AppState;

/// From the start until Claude Code takes `/clear` (waiting for an idle prompt included).
pub const CLEAR_WAIT: Duration = Duration::from_secs(90);
/// From `/clear` until the tab has followed the CLI to the new session.
pub const FOLLOW_WAIT: Duration = Duration::from_secs(30);
/// From following until the first message is taken.
pub const PROMPT_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Clear,
    Follow,
    Prompt,
    /// The gate took the first message and is staging it (a message of several lines waits for
    /// the input line to show it before Enter); done once it has been entered.
    Sending,
}

#[derive(Debug)]
pub struct InPlace {
    pub from_session: String,
    pub from_name: String,
    pub name: String,
    pub prompt: String,
    pub saved: PathBuf,
    pub cwd: PathBuf,
    /// The transcript's length when the handover was made: a prompt recorded at or past it means
    /// the user sent something since, and the handover no longer covers the session.
    pub basis: u64,
    pub stage: Stage,
    pub since: Instant,
    /// The handover dialog's "Delete the original session afterwards": once the first message
    /// is taken and the tab follows the new session, the old one is deleted (`delete`).
    pub delete_original: bool,
}

impl InPlace {
    pub fn deadline(&self) -> Instant {
        self.since
            + match self.stage {
                Stage::Clear => CLEAR_WAIT,
                Stage::Follow => FOLLOW_WAIT,
                Stage::Prompt | Stage::Sending => PROMPT_WAIT,
            }
    }

    /// Why this step stopped, for the notice.
    pub fn timeout_reason(&self) -> &'static str {
        match self.stage {
            Stage::Clear => "Claude Code's prompt did not become idle for /clear in time",
            Stage::Follow => "the tab did not follow Claude Code to its new session after /clear",
            Stage::Prompt => "the handover could not be sent as the new session's first message in time",
            Stage::Sending => "Claude Code did not show the handover on its input line in time, so it was not entered",
        }
    }
}

pub fn failure_text(reason: &str, saved: &std::path::Path) -> String {
    format!(
        "Stopped because {reason}. Nothing else was sent. The handover is saved at {}; open it from the session's Handovers list.",
        saved.display()
    )
}

impl AppState {
    /// The tab a handover of `session` continues in: one that runs it (verified), with the
    /// Chat view's pane, as `account` when one is named; none when Settings say "A new tab".
    pub fn in_place_target(&self, session: &str, account: Option<&str>) -> Option<u64> {
        if self.config.compact.new_tab {
            return None;
        }
        self.tabs
            .list
            .iter()
            .find(|tab| {
                tab.resume_id.as_deref() == Some(session)
                    && tab.term.running()
                    && tab.pane.is_some()
                    && tab.in_place.is_none()
                    && account.is_none_or(|a| tab.account.as_deref() == Some(a))
            })
            .map(|tab| tab.id)
    }

    /// Starts a handover in `tab`, which must still run `session`.
    pub fn continue_in_place(
        &mut self,
        tab: u64,
        session: &Session,
        (from, name): (String, String),
        text: &str,
        (saved, basis): (PathBuf, u64),
    ) -> Result<(), String> {
        let prompt = crate::compact_open::first_prompt(text, &saved)?;
        if self.marks_read_only {
            return Err("Repair marks.json before starting a handover session.".into());
        }
        if self.recheck_tab_identity(tab).as_deref() != Some(session.id.as_str()) {
            return Err("The tab no longer runs this session, or it could not be verified.".into());
        }
        let Some(t) = self.tabs.get_mut(tab) else { return Err("The tab was closed.".into()) };
        t.in_place = Some(InPlace {
            from_session: session.id.clone(),
            from_name: from,
            name,
            prompt,
            saved,
            cwd: session.cwd.clone(),
            basis,
            stage: Stage::Clear,
            since: Instant::now(),
            delete_original: false,
        });
        self.show_tab(tab);
        // Chat for this tab only; the saved default for new tabs stays as it was.
        self.set_shown_tab_chat(true);
        self.toast(
            "Continuing here: Claude Code clears the conversation, then gets the handover.".into(),
            Instant::now(),
        );
        Ok(())
    }

    pub(crate) fn in_place_failed(&mut self, tab: u64, reason: &str) {
        let Some(job) = self.tabs.get_mut(tab).and_then(|t| t.in_place.take()) else { return };
        // An automatic handover that stopped here asks next time (compaction is still off).
        self.auto_ask_again(tab, &job.from_session);
        self.warn("The handover did not continue here", failure_text(reason, &job.saved));
    }

    /// The first message was taken: link the new session as the old one's continuation.
    pub(crate) fn in_place_done(&mut self, tab: u64, new: &str) {
        let Some(job) = self.tabs.get_mut(tab).and_then(|t| t.in_place.take()) else { return };
        let account = self.tabs.get(tab).and_then(|t| t.account.clone());
        let file = account
            .as_deref()
            .and_then(|a| claude_switch_core::account(&self.config, a).ok())
            .map(|a| crate::view::handover_transcript(&a.cli_dir, &job.cwd, new));
        self.marks.set_handover_from(new, &job.from_name);
        self.marks.set_parent(new, &job.from_session);
        self.marks.set_handed_over_to(&job.from_session, new);
        if let Some(file) = &file {
            self.marks.rename(new, file, &job.name);
        }
        // The session is already continued: nothing would set these links again, so a failed
        // save keeps them and tries again (and any later save writes them).
        self.marks_retries = crate::session_marks::MARKS_RETRIES + 1;
        self.save_marks();
        // Claude Code's own name follows (item 39): `/rename` at the next idle prompt, through the
        // Chat view's slash gate, once the first message has gone.
        if let Some(t) = self.tabs.get_mut(tab) {
            t.cli_rename = Some((new.to_owned(), job.name.clone()));
        }
        self.rebuild_view();
        self.toast("Handed over: this tab continues from the summary.".into(), Instant::now());
        if job.delete_original {
            self.delete_after_in_place(tab, &job.from_session, &job.from_name, new);
        }
    }

    /// The original of a handover that continued here, once the tab follows the new session:
    /// deleted, unless it is not the one the tab left (or still runs, is open, is pinned).
    pub(crate) fn delete_after_in_place(&mut self, tab: u64, old: &str, title: &str, new: &str) {
        let after = Some("The handover continued here.");
        let follows = self.tabs.get(tab).is_some_and(|t| {
            t.resume_id.as_deref().or(t.session.as_deref()).is_some_and(|id| id.eq_ignore_ascii_case(new))
        });
        let result = if old.eq_ignore_ascii_case(new) || !follows {
            crate::delete::Deleted::Kept("the tab does not show that it follows the new session yet.".into())
        } else {
            self.delete_session(old)
        };
        self.report_delete(Instant::now(), title, result, after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_has_its_own_deadline_and_reason() {
        let now = Instant::now();
        let mut job = InPlace {
            from_session: "a".into(),
            from_name: "A".into(),
            name: "A · continued".into(),
            prompt: "## Goal".into(),
            saved: PathBuf::from("h.md"),
            cwd: PathBuf::from("."),
            basis: 0,
            stage: Stage::Clear,
            since: now,
            delete_original: false,
        };
        assert_eq!(job.deadline(), now + CLEAR_WAIT);
        job.stage = Stage::Follow;
        assert_eq!(job.deadline(), now + FOLLOW_WAIT);
        assert!(job.timeout_reason().contains("follow"));
        job.stage = Stage::Prompt;
        assert_eq!(job.deadline(), now + PROMPT_WAIT);
    }

    #[test]
    fn a_failed_save_keeps_the_continuation_marks_and_tries_again() {
        use crate::{source::Source, tabs::NewTab, term::Term};
        use claude_switch_core::{Roots, marks::Marks};
        let root = std::env::temp_dir().join(format!("cs-in-place-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let roots = Roots { home: root.join("home"), appdata: root.join("appdata") };
        let mut st = AppState::new(Source::Real(roots.clone()), None, Default::default());
        let file = roots.config_file().with_file_name("marks.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // Another window leaves the file unreadable: the save refuses.
        std::fs::write(&file, b"{\"sessions\":1}").unwrap();
        let (old, new) = ("aaaaaaaa-0000-4000-8000-000000000001", "bbbbbbbb-0000-4000-8000-000000000002");
        let blank = NewTab { label: "t".into(), color: [0; 3], session: None, account: None };
        let tab = st.tabs.open(blank, Term::empty((8, 4)));
        let now = Instant::now();
        st.tabs.get_mut(tab).unwrap().in_place = Some(InPlace {
            from_session: old.into(),
            from_name: "Old".into(),
            name: "Old · continued".into(),
            prompt: "## Goal".into(),
            saved: PathBuf::from("h.md"),
            cwd: PathBuf::from("."),
            basis: 0,
            stage: Stage::Sending,
            since: now,
            delete_original: false,
        });
        st.in_place_done(tab, new);
        assert!(st.info.is_some(), "the failure is reported");
        assert_eq!(st.marks.handed_over_to(old), Some(new), "kept in memory, not rolled back");
        assert_eq!(st.marks.parent(new), Some(old));
        assert!(st.marks_flush_at.is_some(), "and tried again");
        // Each retry that fails schedules the next, a bounded number of times.
        for _ in 0..crate::session_marks::MARKS_RETRIES {
            assert!(!st.save_marks());
            assert!(st.marks_flush_at.is_some());
        }
        assert!(!st.save_marks());
        assert!(st.marks_flush_at.is_none(), "then it waits for the next save or the exit");
        assert_eq!(st.marks.handed_over_to(old), Some(new), "still kept for that save");
        std::fs::write(&file, b"{\"otherWindow\":true}").unwrap();
        assert!(st.save_marks(), "the next save writes them, merged with the other window's");
        let disk = Marks::load(&roots).unwrap();
        assert_eq!((disk.handed_over_to(old), disk.parent(new)), (Some(new), Some(old)));
        assert_eq!(disk.handover_from(new), Some("Old"));
        assert!(std::fs::read_to_string(&file).unwrap().contains("otherWindow"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_tab_setting_or_another_account_means_no_in_place_target() {
        use crate::{source::Source, tabs::NewTab, term::Term};
        let mut st = AppState::new(Source::Demo, Some(false), Default::default());
        let new = NewTab { label: "t".into(), color: [0; 3], session: Some("s".into()), account: Some("main".into()) };
        let tab = st.tabs.open(new, Term::empty((8, 4)));
        assert_eq!(st.in_place_target("s", None), None, "no Chat pane, not running");
        let _ = tab;
        st.config.compact.new_tab = true;
        assert_eq!(st.in_place_target("s", Some("main")), None);
    }
}
