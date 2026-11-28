//! Deleting a session from the list, and the original after "Open a copy" or a handover: the
//! transcript and its own folder go to the Recycle Bin (`claude_switch_core::delete`), its own
//! marks and kept summary are removed. Refused while it runs, is open in a tab or is pinned,
//! checked again at the moment of deletion. See DESIGN.md, "Deleting a session".

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use claude_switch_core::Session;
use claude_switch_core::delete::Bin;
use mdshared_ui::menu::Item;

use crate::dialogs::{Dialog, Pending};
use crate::source::Source;
use crate::state::AppState;

pub const DELETE_ROW: u32 = 67;
/// The row menu's "Open a copy, then delete the original…".
pub const COPY_DELETE: u32 = 68;
/// The handover dialog's checkbox.
pub const CHECKBOX: &str = "Delete the original session afterwards";

/// The real Recycle Bin; in tests a bin that refuses everything, so that a test that forgets
/// to give its own folder can never touch the real Recycle Bin.
pub fn default_bin() -> Arc<dyn Bin> {
    #[cfg(not(test))]
    return Arc::new(claude_switch_core::delete::RecycleBin);
    #[cfg(test)]
    return Arc::new(NoBin);
}

#[cfg(test)]
struct NoBin;

#[cfg(test)]
impl Bin for NoBin {
    fn recycle(&self, _: &std::path::Path) -> std::io::Result<()> {
        Err(std::io::Error::other("tests have no Recycle Bin"))
    }
}

/// A copy whose original is deleted once the copy's transcript has been written.
pub struct AfterCopy {
    pub original: String,
    pub title: String,
    pub copy: String,
    /// Where the copy's transcript appears.
    pub copy_file: PathBuf,
    /// The tab the copy runs in, when it runs in one: closed before the copy was written, the
    /// original is kept.
    pub tab: Option<u64>,
}

/// How a deletion went.
#[derive(Debug, PartialEq, Eq)]
pub enum Deleted {
    /// The transcript (and its folder, if it had one) are in the Recycle Bin.
    Done,
    /// The transcript is in the Recycle Bin; something else was not done (the reason).
    Partly(String),
    /// Nothing was deleted (the reason).
    Kept(String),
    /// `--demo`: nothing was deleted.
    Demo,
}

/// The row menu's item, disabled with the reason as its hint.
pub fn row_item(block: Option<&'static str>) -> Item<u32> {
    Item::new(DELETE_ROW, "Delete session…").enabled(block.is_none()).hint(block.unwrap_or(""))
}

/// The row menu's copy-then-delete item, disabled with the reason the original could not be
/// deleted (a running original can still be copied, but not deleted: so this item waits).
pub fn copy_item(block: Option<&'static str>) -> Item<u32> {
    Item::new(COPY_DELETE, "Open a copy, then delete the original…").enabled(block.is_none()).hint(block.unwrap_or(""))
}

impl AppState {
    /// Why session `id` cannot be deleted now, as a short hint; `None` when it can.
    pub fn delete_block(&self, id: &str) -> Option<&'static str> {
        if self.running_ids.contains(id) {
            Some("Running")
        } else if self.tab_holds(id) {
            Some("Open in a tab")
        } else if self.marks.pinned(id) {
            Some("Pinned")
        } else {
            None
        }
    }

    /// A tab of this window runs `id` (its routed session or its verified resume identity).
    fn tab_holds(&self, id: &str) -> bool {
        self.tabs.list.iter().any(|t| {
            (t.session.as_deref() == Some(id) || t.resume_id.as_deref() == Some(id))
                && (t.term.running() || self.source.is_demo() && t.term.ended.is_none())
        })
    }

    pub fn delete_action(&mut self, action: u32) {
        let Some(s) = self.selected_session().cloned() else { return };
        if let Some(block) = self.delete_block(&s.id) {
            self.warn("Not deleted", block_text(block));
            return;
        }
        let account = self.current().slug.clone();
        let mut text = self.delete_text(&s, action == COPY_DELETE);
        let (title, ok, then) = match action {
            DELETE_ROW => ("Delete session?", "Delete", Pending::Delete { id: s.id.clone() }),
            COPY_DELETE => {
                // The carry warning that "Open a copy" would show is part of this one confirmation.
                if let Some(carry) = self.source.carry_warning(&self.config, &s, &account) {
                    text = format!(
                        "{carry}

{text}"
                    );
                }
                (
                    "Open a copy, then delete the original?",
                    "Open copy",
                    Pending::Copy { id: s.id.clone(), account, delete: true },
                )
            }
            _ => return,
        };
        self.confirm_scroll = 0.0;
        self.dialog = Some(Dialog::Confirm { title: title.into(), text, ok: ok.into(), then });
    }

    /// The confirmation's text: which session, where, what happens to it.
    pub fn delete_text(&self, s: &Session, after_copy: bool) -> String {
        let folder =
            if s.cwd.as_os_str().is_empty() { "no folder recorded".to_owned() } else { s.cwd.display().to_string() };
        let mut text = format!("\u{201c}{}\u{201d}\nFolder: {folder}\n\n", self.title_of(s));
        if after_copy {
            text.push_str(
                "A copy opens in the CLI. Once Claude Code has written the copy's transcript, the original's \
                 transcript and its own folder are moved to the Recycle Bin. If the original still runs or is \
                 open in a tab then, it is kept.",
            );
        } else {
            text.push_str("Its transcript and its own folder (subagents, tool results) are moved to the Recycle Bin.");
        }
        text.push_str(
            " You can restore them from there. Its Claude Switch name, topic, marks and summary are removed.",
        );
        if s.desktop_id.is_some() {
            text.push_str("\n\nThe desktop app keeps its own record of this session and may still list it.");
        }
        if self.source.is_demo() {
            text.push_str("\n\nDemo: nothing is deleted.");
        }
        text
    }

    /// The list's Delete button: checks again, deletes, says how it went.
    pub fn delete_confirmed(&mut self, now: Instant, id: &str) {
        let title = self.by_id.get(id).map(|&i| self.title_of(&self.sessions[i]).to_owned()).unwrap_or_default();
        let result = self.delete_session(id);
        self.report_delete(now, &title, result, None);
    }

    /// Says how a deletion went. `after` names what came first (the copy, the handover), which
    /// went well either way.
    pub fn report_delete(&mut self, now: Instant, title: &str, result: Deleted, after: Option<&str>) {
        let lead = after.map_or_else(String::new, |a| format!("{a} "));
        match result {
            Deleted::Done => self.toast(format!("Moved \u{201c}{title}\u{201d} to the Recycle Bin"), now),
            Deleted::Demo => self.toast(format!("Demo: \u{201c}{title}\u{201d} was not deleted"), now),
            Deleted::Partly(why) => self.warn(
                "Session partly deleted",
                format!("{lead}The transcript of \u{201c}{title}\u{201d} is in the Recycle Bin, but {why}"),
            ),
            Deleted::Kept(why) if after.is_some() => self
                .warn("The original session was kept", format!("{lead}\u{201c}{title}\u{201d} was not deleted: {why}")),
            Deleted::Kept(why) => self.error("Not deleted", why),
        }
    }

    /// Deletes session `id`, checking again first: not running in any account, not open in a
    /// tab, not pinned (in this window or as saved). Then its own marks and kept summary go.
    pub fn delete_session(&mut self, id: &str) -> Deleted {
        let Some(s) = self.by_id.get(id).map(|&i| self.sessions[i].clone()) else {
            return Deleted::Kept("It is no longer in the list. Refresh and try again.".into());
        };
        if let Some(block) = self.delete_block(id) {
            return Deleted::Kept(block_text(block));
        }
        let Source::Real(roots) = &self.source else { return Deleted::Demo };
        let roots = roots.clone();
        let saved_pin = || {
            claude_switch_core::marks::Marks::load(&roots)
                .map_err(|e| e.to_string())?
                .pinned(id)
                .then(|| block_text("Pinned"))
                .map_or(Ok(()), Err)
        };
        let outcome = match claude_switch_core::delete::delete(
            &roots,
            &self.config,
            id,
            &s.file,
            self.recycle.as_ref(),
            saved_pin,
        ) {
            Ok(outcome) => outcome,
            Err(e) => return Deleted::Kept(e.to_string()),
        };
        let mut problems = Vec::new();
        if let Some(e) = &outcome.folder_error {
            let folder = outcome.plan.folder.as_deref().unwrap_or(&s.file).display();
            problems.push(format!("its folder {folder} was not moved ({e})."));
        }
        if let Some(link) = &outcome.plan.linked_folder {
            problems.push(format!("{} is a link and was left in place.", link.display()));
        }
        if self.marks.forget(id) {
            if self.marks_read_only {
                problems.push("its marks were not removed: marks.json is read-only in this window.".into());
            } else if let Err(e) = self.marks.save(&roots) {
                problems.push(format!("its marks were not removed yet ({e})."));
            }
        }
        if let Err(e) = self.forget_meaning_of(id) {
            problems.push(format!("its kept summary was not removed ({e})."));
        }
        self.drop_session(id);
        if problems.is_empty() { Deleted::Done } else { Deleted::Partly(problems.join(" ")) }
    }

    /// Removes a deleted session's kept summary, in memory and in `meaning.json`.
    fn forget_meaning_of(&mut self, id: &str) -> Result<(), String> {
        self.meaning.cache.digests.remove(id);
        let Source::Real(roots) = &self.source else { return Ok(()) };
        let path = claude_switch_core::meaning::Cache::path(roots);
        let mut cache = claude_switch_core::meaning::Cache::load(&path);
        if cache.digests.remove(id).is_some() {
            cache.save(&path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Takes a deleted session out of the list now; the rescan that follows confirms it.
    fn drop_session(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
        self.by_id = self.sessions.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
        self.all_groups = claude_switch_core::sessions::group_by_folder(&self.sessions);
        self.archive_picks.remove(id);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
            self.details_reader = None;
        }
        self.rebuild_threads();
        self.rebuild_view();
        if self.selected.is_none() {
            self.select_first();
        }
        self.refresh(Instant::now());
    }

    /// After "Open a copy" with the option: waits for the copy's transcript, then deletes.
    pub fn delete_after_copy(&mut self, original: &Session, copy: &str, account: &str, tab: Option<u64>) {
        let Ok(a) = claude_switch_core::account(&self.config, account) else { return };
        let copy_file = crate::view::handover_transcript(&a.cli_dir, &original.cwd, copy);
        self.copy_deletes.push(AfterCopy {
            original: original.id.clone(),
            title: self.title_of(original).to_owned(),
            copy: copy.to_owned(),
            copy_file,
            tab,
        });
    }

    /// Deletes the originals whose copies have been written; keeps those whose copy's tab
    /// closed first. True when something was shown.
    pub fn check_copy_deletes(&mut self, now: Instant) -> bool {
        let mut shown = false;
        for job in std::mem::take(&mut self.copy_deletes) {
            if copy_written(&job.copy_file, &job.copy) {
                let result = self.delete_session(&job.original);
                self.report_delete(now, &job.title, result, Some("The copy is open."));
                shown = true;
            } else if job.tab.is_some_and(|t| self.tabs.get(t).is_none_or(|t| !t.term.running())) {
                let why = "its copy's tab closed before Claude Code wrote the copy.".to_owned();
                self.report_delete(now, &job.title, Deleted::Kept(why), Some("The copy was not written."));
                shown = true;
            } else {
                self.copy_deletes.push(job);
            }
        }
        shown
    }
}

/// The copy's transcript holds a record of the copy itself: Claude Code has written it. Only
/// the head of the file is read.
fn copy_written(file: &std::path::Path, copy: &str) -> bool {
    use std::io::Read as _;
    let mut head = Vec::new();
    let Ok(f) = std::fs::File::open(file) else { return false };
    if f.take(256 * 1024).read_to_end(&mut head).is_err() {
        return false;
    }
    String::from_utf8_lossy(&head).lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .is_ok_and(|v| v["sessionId"].as_str().is_some_and(|id| id.eq_ignore_ascii_case(copy)))
    })
}

fn block_text(block: &str) -> String {
    match block {
        "Running" => "It is running. Close it first.".into(),
        "Open in a tab" => "It is open in a tab. Close the tab first.".into(),
        "Pinned" => "It is pinned. Unpin it first.".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests;
