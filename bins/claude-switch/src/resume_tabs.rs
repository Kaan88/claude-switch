//! Last-run terminal tabs: identifiers only, never commands, names, or screen contents.

use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use claude_switch_core::{App, Launch, Refusal, Running, Session, sessions::folder_key};
use serde_json::{Value, json};

use crate::dialogs::{Dialog, Pending};
use crate::source::Source;
use crate::state::AppState;
use crate::tabs::{NewTab, SESSIONS, Tabs};
use crate::term::Term;
use crate::winfile::RestoreTabs;

const TOAST_PREFIX: &str = "Tabs not reopened: ";

#[path = "resume_tabs_draw.rs"]
mod drawing;
#[path = "resume_tabs_ownership.rs"]
mod ownership;
#[path = "resume_tabs_process.rs"]
mod process;
pub use drawing::draw_toasts;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    account: String,
    cwd: PathBuf,
    active: bool,
    chat: Option<bool>,
    /// An OpenCode tab in `cwd` (`id` and `account` are empty).
    opencode: bool,
}

#[derive(Default)]
pub struct Restore {
    /// Another Claude Switch window holds the handover channel: that one restores the saved tabs.
    pub second_window: bool,
    /// This window was a second one and took the saved tabs over as the last to close.
    inherited: bool,
    pub save_failed: bool,
    pub window_save_failed: bool,
    baseline: Option<String>,
    pending: VecDeque<Entry>,
    waiting: Option<Entry>,
    failures: Vec<String>,
    active: Option<u64>,
    initial: Option<u64>,
    started: bool,
    verified_owners: HashSet<(u64, String)>,
    external_owners: ownership::PendingOwners,
    /// Folders whose OpenCode tab this restore already reopened with `--continue`.
    opencode_continued: HashSet<String>,
}

fn valid_id(id: &str) -> bool {
    claude_switch_core::launch::validate_session_id(id).is_ok()
}

impl Entry {
    fn opencode(cwd: PathBuf, active: bool) -> Self {
        Self { id: String::new(), account: String::new(), cwd, active, chat: None, opencode: true }
    }
}

fn decode(text: &str) -> VecDeque<Entry> {
    let Ok(Value::Array(rows)) = serde_json::from_str(text) else { return VecDeque::new() };
    let mut ids = HashSet::new();
    rows.into_iter()
        .filter_map(|row| {
            if row["kind"] == "opencode" {
                let cwd = PathBuf::from(row["folder"].as_str()?);
                let active = row["active"].as_bool()?;
                return cwd.is_absolute().then(|| Entry::opencode(cwd, active));
            }
            let id = row["id"].as_str()?;
            let account = row["account"].as_str()?;
            let cwd = PathBuf::from(row["folder"].as_str()?);
            let active = row["active"].as_bool()?;
            (valid_id(id) && !account.is_empty() && cwd.is_absolute() && ids.insert(id.to_ascii_lowercase())).then(
                || Entry {
                    id: id.into(),
                    account: account.into(),
                    cwd,
                    active,
                    chat: row["chat"].as_bool(),
                    opencode: false,
                },
            )
        })
        .collect()
}

fn encode(rows: impl IntoIterator<Item = Entry>) -> String {
    Value::Array(
        rows.into_iter()
            .filter_map(|row| {
                if row.opencode {
                    return Some(json!({"kind": "opencode", "folder": row.cwd.to_str()?, "active": row.active}));
                }
                Some(json!({"id": row.id, "account": row.account, "folder": row.cwd.to_str()?, "active": row.active,
                    "chat":row.chat}))
            })
            .collect(),
    )
    .to_string()
}

fn path(source: &Source) -> Option<PathBuf> {
    source.window_file().map(|p| p.with_file_name("terminal-tabs.json"))
}

fn snapshot(tabs: &Tabs) -> Vec<Entry> {
    let entry = |t: &crate::tabs::Tab| {
        let id = t.resume_id.as_ref().filter(|id| valid_id(id))?.clone();
        let account = t.account.clone()?;
        let cwd = t.cwd.clone()?;
        Some(Entry {
            id,
            account,
            cwd,
            active: tabs.active == Some(t.id),
            chat: t.pane.as_ref().map(|p| p.chat.enabled),
            opencode: false,
        })
    };
    let active = tabs.shown().and_then(entry).map(|t| t.id.to_ascii_lowercase());
    let mut ids = HashSet::new();
    tabs.list
        .iter()
        .filter_map(|t| match &t.opencode {
            Some(folder) if t.term.ended.is_none() => Some(Entry::opencode(folder.clone(), tabs.active == Some(t.id))),
            Some(_) => None,
            None => entry(t),
        })
        .filter(|t| {
            let key = t.id.to_ascii_lowercase();
            t.opencode || (t.active || active.as_ref() != Some(&key)) && ids.insert(key)
        })
        .collect()
}

/// Never use the session pane's heuristic guess to restore a transcript.
fn live_id<'a>(
    running: &'a [Running],
    account: &str,
    cwd: &Path,
    matches_pid: impl Fn(u32) -> bool,
) -> Option<&'a str> {
    let cwd = folder_key(cwd);
    let mut matches = running.iter().filter(|r| {
        r.app == App::Cli
            && r.account == account
            && folder_key(&r.cwd) == cwd
            && matches_pid(r.pid)
            && valid_id(&r.session_id)
    });
    let id = &matches.next()?.session_id;
    matches.all(|other| &other.session_id == id).then_some(id)
}

type Started = (io::Result<Option<u64>>, Result<(), Refusal>);

impl AppState {
    pub fn resume_warning(&self, session: &Session, account: &str, app: App) -> Option<(String, String)> {
        let carry = self.source.carry_warning(&self.config, session, account);
        let desktop = app == App::Cli && (session.desktop_account.is_some() || session.desktop_id.is_some());
        let text = crate::actions::open_warning(carry.as_deref(), desktop)?;
        let title = if carry.is_some() {
            format!("Open as {}?", self.account_name(account))
        } else {
            "Open in the CLI?".into()
        };
        Some((title, text))
    }

    pub fn prepare_resume(&mut self, session: &Session, account: &str, app: App) -> Result<Launch, Refusal> {
        self.running = self.source.running(&self.config);
        self.index_running();
        self.rebuild_view();
        self.source.resume(&self.config, session, account, app, &self.running)
    }

    pub fn start_launch(
        &mut self,
        launch: &Launch,
        note: Option<(&str, &str)>,
        account: Option<&str>,
        in_tab: bool,
    ) -> Result<Started, Refusal> {
        let reservation =
            note.map(|(id, account)| self.source.reserve(&self.config, id, account)).transpose()?.flatten();
        let started = if in_tab && !self.source.is_demo() {
            self.open_tab(launch, note.map(|(id, _)| id), account).map(Some)
        } else {
            self.source.run(launch).map(|()| None)
        };
        if !in_tab && !self.source.is_demo() && started.is_ok() {
            self.restore.external_owners.launched(launch, account, Instant::now());
        }
        let history = reservation.map(|r| r.finish(started.is_ok())).transpose().map(drop);
        Ok((started, history))
    }

    pub fn identify_tabs(&mut self) -> bool {
        let mut processes = None;
        let mut changed = self.verify_external_owners();
        let mut continued = Vec::new();
        for tab in &mut self.tabs.list {
            if !tab.term.running() {
                continue;
            }
            let (Some(pty), Some(account), Some(cwd)) = (&tab.term.pty, &tab.account, &tab.cwd) else { continue };
            let processes = processes.get_or_insert_with(process::Processes::current);
            let found = live_id(&self.running, account, cwd, |pid| pid == pty.pid() || processes.wraps(pty.pid(), pid));
            changed |= apply_live_identity(tab, found);
            continued.extend(tab.pane.as_mut().and_then(|pane| pane.continued.take()));
            if let Source::Real(roots) = &self.source
                && let Some(id) = found.filter(|id| tab.resume_id.as_deref() == Some(*id))
                && let Some(account) = &tab.account
            {
                let key = (tab.id, id.to_owned());
                if !self.restore.verified_owners.contains(&key)
                    && claude_switch_core::launch::record_verified_owner(roots, id, account).is_ok()
                {
                    self.restore.verified_owners.insert(key);
                    if self.selected.as_deref() == Some(id) {
                        self.last_account = Some((id.to_owned(), account.clone()));
                        changed = true;
                    }
                }
            }
        }
        self.restore.verified_owners.retain(|(id, _)| self.tabs.get(*id).is_some());
        // The CLI moved a tab to a new session (`/clear`, compaction): Threads shows it as the
        // continuation, as for a handover.
        if !self.marks_read_only
            && continued.iter().fold(false, |linked, (id, parent)| self.marks.set_parent(id, parent) | linked)
        {
            self.save_marks();
        }
        changed
    }

    pub fn load_tab_restore(&mut self) {
        if let Some(file) = path(&self.source)
            && let Err(error) = self.restore.load_snapshot(&file)
        {
            self.restore.save_failed = true;
            self.error("Could not read saved terminal tabs", format!("{error} The saved file was left unchanged."));
        }
        if self.win_file.restore_tabs == RestoreTabs::Never || self.restore.second_window {
            return;
        }
        if self.source.is_demo() {
            self.restore.pending = crate::demo::scan(&self.config)
                .sessions
                .into_iter()
                .filter(|s| s.desktop_id.is_none())
                .take(3)
                .enumerate()
                .map(|(i, s)| Entry {
                    id: s.id,
                    account: "main".into(),
                    cwd: s.cwd,
                    active: i == 1,
                    chat: None,
                    opencode: false,
                })
                .collect();
        } else if let Some(text) = &self.restore.baseline {
            self.restore.pending = decode(text);
        }
    }

    pub fn recheck_tab_identity(&mut self, tab: u64) -> Option<String> {
        if let Source::Real(roots) = &self.source {
            let (running, complete) = claude_switch_core::running::scan_checked(roots, &self.config);
            if !complete {
                return None;
            }
            self.running = running;
            self.index_running();
        }
        self.tabs.get(tab).filter(|t| self.source.is_demo() || t.term.running())?.resume_id.clone()
    }

    pub fn save_tab_restore(&mut self) -> Result<(), Refusal> {
        // The saved tabs are the first window's: a second one neither restores nor replaces them
        // while the first runs, so they survive whichever window closes first.
        if self.restore.second_window {
            return Ok(());
        }
        let Some(file) = path(&self.source) else { return Ok(()) };
        if self.win_file.restore_tabs == RestoreTabs::Never {
            return claude_switch_core::replace_if_unchanged(
                &file,
                self.restore.baseline.as_deref().map(str::as_bytes),
                b"[]",
            );
        }
        if self.tabs.list.iter().any(|tab| tab.cwd.is_some())
            && let Source::Real(roots) = &self.source
        {
            let (running, complete) = claude_switch_core::running::scan_checked(roots, &self.config);
            if !complete {
                return Err(Refusal::Invalid("The current terminal sessions could not be verified.".into()));
            }
            self.running = running;
            self.identify_tabs();
        }
        let rows = snapshot(&self.tabs);
        if rows.is_empty() && self.restore.inherited {
            return Ok(());
        }
        if !rows.is_empty() || self.restore.pending.is_empty() && self.restore.waiting.is_none() {
            return self.restore.save_snapshot(&file, rows);
        }
        Ok(())
    }

    /// A second window that closes after the first has gone (it could claim the handover channel)
    /// is the last one: its tabs become the saved ones, over whatever the first window left. With
    /// no tabs of its own it leaves the first window's (`save_tab_restore`).
    pub fn inherit_tab_restore(&mut self) {
        let Some(file) = path(&self.source) else { return };
        // The snapshot as the first window left it; unreadable: stay a second window, write nothing.
        if self.restore.second_window && self.restore.load_snapshot(&file).is_ok() {
            self.restore.second_window = false;
            self.restore.inherited = true;
        }
    }

    pub fn dismiss_tab_restore(&mut self) {
        let baseline = self.restore.baseline.take();
        let (second_window, inherited) = (self.restore.second_window, self.restore.inherited);
        self.restore = Restore { baseline, second_window, inherited, ..Restore::default() };
        if second_window {
            return;
        }
        if let Some(file) = path(&self.source) {
            match claude_switch_core::replace_if_unchanged(
                &file,
                self.restore.baseline.as_deref().map(str::as_bytes),
                b"[]",
            ) {
                Ok(()) => self.restore.baseline = Some("[]".into()),
                Err(e) => self.error("Could not clear the saved tabs", e.to_string()),
            }
        }
    }

    pub fn restore_tabs_if_always(&mut self, now: Instant) -> bool {
        if self.scanned
            && !self.quit
            && self.nothing_over()
            && !self.picture_overlay()
            && !self.restore.started
            && !self.restore.pending.is_empty()
            && self.win_file.restore_tabs == RestoreTabs::Always
        {
            self.begin_tab_restore(now);
            return true;
        }
        false
    }

    pub fn begin_tab_restore(&mut self, now: Instant) {
        if !self.scanned || self.restore.started || self.restore.pending.is_empty() {
            return;
        }
        self.restore.initial = self.tabs.active;
        self.restore.started = true;
        self.next_restore_tab(now);
    }

    fn next_restore_tab(&mut self, now: Instant) {
        while let Some(entry) = self.restore.pending.pop_front() {
            if entry.opencode {
                self.restore_opencode(&entry);
                continue;
            }
            match self.check_restore(&entry) {
                Ok(session) => {
                    if let Some((title, text)) = self.resume_warning(&session, &entry.account, App::Cli) {
                        if let Some(r) =
                            self.source.running(&self.config).into_iter().find(|r| r.session_id == entry.id)
                        {
                            self.restore_failed(&entry, &Refusal::AlreadyRunning(r).to_string());
                            continue;
                        }
                        self.restore.waiting = Some(entry);
                        self.dialog =
                            Some(Dialog::Confirm { title, text, ok: "Continue".into(), then: Pending::RestoreTab });
                        return;
                    }
                    self.try_restore(&entry, &session);
                }
                Err(reason) => self.restore_failed(&entry, &reason),
            }
        }
        let active = self.restore.active.or(self.restore.initial).unwrap_or(SESSIONS);
        self.show_tab(active);
        let failures = std::mem::take(&mut self.restore.failures);
        self.restore = Restore { baseline: self.restore.baseline.take(), ..Restore::default() };
        if !failures.is_empty() {
            self.toast(format!("{TOAST_PREFIX}{}", failures.join("; ")), now);
        }
    }

    pub fn confirm_restore_tab(&mut self, now: Instant, proceed: bool) {
        if let Some(entry) = self.restore.waiting.take() {
            if proceed {
                match self.check_restore(&entry) {
                    Ok(session) => self.try_restore(&entry, &session),
                    Err(reason) => self.restore_failed(&entry, &reason),
                }
            } else {
                self.restore_failed(&entry, "cancelled");
            }
        }
        self.next_restore_tab(now);
    }

    pub fn restore_prompt_after_cancelled_quit(&mut self, now: Instant) {
        if let Some(entry) = self.restore.waiting.take() {
            self.restore.pending.push_front(entry);
            self.next_restore_tab(now);
        }
    }

    fn check_restore(&self, entry: &Entry) -> Result<Session, String> {
        let session = self
            .by_id
            .get(&entry.id)
            .and_then(|&i| self.sessions.get(i))
            .filter(|s| self.source.is_demo() || s.file.is_file())
            .ok_or("session is gone")?;
        if self.tab_of_session(&entry.id).is_some() {
            return Err("already open in a tab".into());
        }
        // The saved folder is descriptive. Resume uses the indexed folder and its usual fallback.
        Ok(session.clone())
    }

    fn try_restore(&mut self, item: &Entry, session: &Session) {
        let result = self
            .prepare_resume(session, &item.account, App::Cli)
            .and_then(|launch| self.start_launch(&launch, Some((&item.id, &item.account)), Some(&item.account), true));
        let id = match result {
            Ok((Ok(id), history)) => {
                if let Err(error) = history {
                    self.error("Could not finish the launch record", error.to_string());
                }
                id.or_else(|| self.source.is_demo().then(|| self.fake_restored_tab(item, session)))
            }
            Ok((Err(e), _)) => {
                self.restore_failed(item, &e.to_string());
                None
            }
            Err(e) => {
                self.restore_failed(item, &e.to_string());
                None
            }
        };
        if let (Some(id), Some(chat)) = (id, item.chat)
            && let Some(pane) = self.tabs.get_mut(id).and_then(|tab| tab.pane.as_mut())
        {
            pane.restore_chat_layout(chat);
        }
        if item.active && id.is_some() {
            self.restore.active = id;
        }
    }

    fn fake_restored_tab(&mut self, item: &Entry, session: &Session) -> u64 {
        let mut term = Term::empty(self.grid);
        term.feed(b"Restored demo session (no program started)\r\n");
        let id = self.tabs.open(
            NewTab {
                label: self.title_of(session).into(),
                color: self.account_color(&item.account),
                session: Some(item.id.clone()),
                account: Some(item.account.clone()),
            },
            term,
        );
        self.tabs.get_mut(id).unwrap().cwd = Some(item.cwd.clone());
        id
    }

    /// OpenCode keeps its own sessions: the tab reopens in its folder with `--continue` (that
    /// folder's last OpenCode session) and never with the prompt it may have started with.
    fn restore_opencode(&mut self, item: &Entry) {
        // `--continue` is the folder's last session: only one tab per folder may take it, or two
        // tabs would show the same session. The others start fresh.
        let start = if self.restore.opencode_continued.insert(folder_key(&item.cwd)) {
            crate::opencode_tab::Start::Continue
        } else {
            crate::opencode_tab::Start::Fresh
        };
        match self.open_opencode_tab(&item.cwd, start) {
            Ok(id) if item.active => self.restore.active = Some(id),
            Ok(_) => {}
            Err(e) => {
                let label = crate::opencode_tab::label(&item.cwd);
                self.restore.failures.push(format!("{label} ({e})"));
            }
        }
    }

    fn restore_failed(&mut self, item: &Entry, reason: &str) {
        let name = self
            .by_id
            .get(&item.id)
            .and_then(|&i| self.sessions.get(i))
            .map(|s| self.title_of(s))
            .filter(|name| !name.trim().is_empty());
        let text = name.map_or_else(
            || {
                let folder = claude_switch_core::sessions::folder_name(&item.cwd);
                format!("{folder} · {} ({reason})", item.id)
            },
            |name| format!("{name} ({reason})"),
        );
        self.restore.failures.push(text);
    }
}

fn apply_live_identity(tab: &mut crate::tabs::Tab, found: Option<&str>) -> bool {
    tab.apply_live_identity(found)
}

impl Restore {
    fn load_snapshot(&mut self, file: &Path) -> io::Result<()> {
        self.baseline = match claude_switch_core::read_bounded(file, claude_switch_core::APP_FILE_LIMIT) {
            Ok(bytes) => Some(String::from_utf8(bytes).map_err(io::Error::other)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(())
    }

    fn save_snapshot(&self, file: &Path, rows: Vec<Entry>) -> Result<(), Refusal> {
        claude_switch_core::replace_if_unchanged(
            file,
            self.baseline.as_deref().map(str::as_bytes),
            encode(rows).as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "11111111-0000-0000-0000-000000000000";
    const B: &str = "22222222-0000-0000-0000-000000000000";

    #[test]
    fn oversized_saved_tabs_are_not_loaded_or_overwritten() {
        let path = std::env::temp_dir().join(format!("cs-w3-tabs-cap-{}.json", std::process::id()));
        let len = claude_switch_core::APP_FILE_LIMIT as u64 + 1;
        std::fs::File::create_new(&path).unwrap().set_len(len).unwrap();
        let mut restore = Restore::default();
        assert_eq!(restore.load_snapshot(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(restore.save_snapshot(&path, vec![]).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), len);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn finish_restore_then_save_changed_tabs_keeps_the_loaded_revision() {
        let root = std::env::temp_dir().join(format!("cs-restore-exit-{}", std::process::id()));
        let disk = Source::Real(claude_switch_core::Roots { home: root.clone(), appdata: root.clone() });
        let file = path(&disk).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut app = AppState::new(Source::Demo, None, Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let original = Entry {
            id: app.sessions[2].id.clone(),
            account: "main".into(),
            cwd: app.sessions[2].cwd.clone(),
            active: true,
            chat: None,
            opencode: false,
        };
        std::fs::write(&file, encode([original.clone()])).unwrap();
        app.restore.load_snapshot(&file).unwrap();
        app.restore.pending = decode(app.restore.baseline.as_deref().unwrap());
        app.begin_tab_restore(Instant::now());
        assert!(!app.restore.started);
        assert_eq!(snapshot(&app.tabs).len(), 1);
        app.tabs.list.clear();
        app.source = disk;
        app.save_tab_restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
        assert!(app.restore.save_snapshot(&file, vec![original]).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn records_round_trip_and_untrusted_or_duplicate_rows_are_skipped() {
        let entry = Entry {
            id: A.into(),
            account: "main".into(),
            cwd: std::env::temp_dir().join("folder"),
            active: true,
            chat: Some(true),
            opencode: false,
        };
        let invalid = Entry { id: "--bogus".into(), ..entry.clone() };
        let text = encode([entry.clone(), invalid, entry.clone()]);
        assert_eq!(decode(&text), VecDeque::from([entry]));
        assert!(decode("oops").is_empty());
    }

    #[test]
    fn opencode_tabs_are_saved_by_folder_and_reopen_with_continue_never_a_prompt() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let folder = app.sessions[0].cwd.clone();
        let first = app.open_opencode_tab(&folder, crate::opencode_tab::Start::Prompt("## Goal\nx".into())).unwrap();
        app.open_opencode_tab(&folder, crate::opencode_tab::Start::Fresh).unwrap();
        app.show_tab(first);
        let rows = snapshot(&app.tabs);
        assert_eq!(rows.len(), 2, "two tabs in one folder are both kept");
        assert!(rows.iter().all(|r| r.opencode && r.cwd == folder && r.id.is_empty() && r.account.is_empty()));
        assert!(rows[0].active && !rows[1].active);
        let claude = Entry {
            id: A.into(),
            account: "main".into(),
            cwd: std::env::temp_dir().join("folder"),
            active: false,
            chat: None,
            opencode: false,
        };
        let text = encode(rows.iter().cloned().chain([claude.clone()]));
        assert!(!text.contains("--prompt") && !text.contains("Goal"), "the prompt is not saved: {text}");
        let decoded = decode(&text);
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[2], claude);
        assert!(decode(r#"[{"kind":"opencode","folder":"relative","active":true}]"#).is_empty());

        app.tabs.list.clear();
        app.restore.pending = decoded.into_iter().take(1).collect();
        app.begin_tab_restore(Instant::now());
        let tab = app.tabs.shown().expect("reopened and shown");
        assert_eq!(tab.opencode.as_deref(), Some(folder.as_path()));
    }

    #[test]
    fn only_one_opencode_tab_per_folder_reopens_with_continue_the_others_start_fresh() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let a = app.sessions[0].cwd.clone();
        let b = app.sessions.iter().map(|s| s.cwd.clone()).find(|c| folder_key(c) != folder_key(&a)).unwrap();
        app.restore.pending =
            [Entry::opencode(a.clone(), false), Entry::opencode(a.clone(), true), Entry::opencode(b.clone(), false)]
                .into();
        app.begin_tab_restore(Instant::now());
        let starts: Vec<_> = app
            .tabs
            .list
            .iter()
            .map(|t| (t.opencode.clone().unwrap(), t.term.screen.row_text(0).contains("--continue")))
            .collect();
        assert_eq!(starts, [(a.clone(), true), (a, false), (b, true)]);
        // A later restore (another window run) starts its own count.
        assert!(app.restore.opencode_continued.is_empty());
    }

    #[test]
    fn snapshot_ignores_shells_and_unverified_guesses_and_preserves_order() {
        let mut tabs = Tabs::default();
        let mut open = |label: &str, id: Option<&str>| {
            tabs.open(
                NewTab {
                    label: label.into(),
                    color: [0; 3],
                    session: id.map(str::to_owned),
                    account: Some("main".into()),
                },
                Term::empty((8, 4)),
            )
        };
        let a = open("secret - not stored", Some(A));
        let shell = open("shell", None);
        let b = open("b", Some(B));
        let unknown = open("unknown", None);
        tabs.get_mut(unknown).unwrap().session = Some("33333333-0000-0000-0000-000000000000".into());
        for id in [a, b, unknown] {
            tabs.get_mut(id).unwrap().cwd = Some(r"C:\fake".into());
        }
        tabs.list.swap(0, 2);
        tabs.activate(a);
        let rows = snapshot(&tabs);
        assert_eq!(rows.iter().map(|t| (t.id.as_str(), t.active)).collect::<Vec<_>>(), [(B, false), (A, true)]);
        assert!(!encode(rows).contains("secret"));
        tabs.activate(shell);
        assert!(snapshot(&tabs).iter().all(|t| !t.active));
    }

    #[test]
    fn the_active_incarnation_wins_if_the_same_session_has_two_tabs() {
        let mut tabs = Tabs::default();
        for account in ["main", "work"] {
            let tab = tabs.open(
                NewTab { label: "tab".into(), color: [0; 3], session: Some(A.into()), account: Some(account.into()) },
                Term::empty((8, 4)),
            );
            tabs.get_mut(tab).unwrap().cwd = Some(std::env::temp_dir());
        }
        let rows = snapshot(&tabs);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].active);
        assert_eq!(rows[0].account, "work");
    }

    #[test]
    fn always_restores_available_demo_tabs_in_order_and_skips_running_and_missing_sessions() {
        let prefs = crate::winfile::WindowFile { restore_tabs: RestoreTabs::Always, ..Default::default() };
        let mut app = AppState::new(Source::Demo, Some(false), prefs);
        let now = Instant::now();
        app.load_tab_restore();
        assert!(!app.restore_tabs_if_always(now));
        let scan = crate::demo::scan(&app.config);
        let mut missing = app.restore.pending[0].clone();
        missing.id = A.into();
        app.restore.pending.push_back(missing);
        let expected: Vec<_> = app.restore.pending.iter().skip(1).take(2).map(|r| r.id.clone()).collect();
        app.apply_scan(scan);
        assert!(app.restore_tabs_if_always(now));
        assert!(!app.restore_tabs_if_always(now));
        assert!(app.restore.pending.is_empty() && app.dialog.is_none());
        assert_eq!(app.tabs.list.iter().map(|t| t.session.clone().unwrap()).collect::<Vec<_>>(), expected);
        assert_eq!(app.tabs.active, Some(app.tabs.list[0].id));
    }

    #[test]
    fn a_window_that_did_not_claim_the_handover_channel_restores_no_tabs() {
        let prefs = crate::winfile::WindowFile { restore_tabs: RestoreTabs::Always, ..Default::default() };
        let mut app = AppState::new(Source::Demo, Some(false), prefs);
        app.restore.second_window = true;
        app.load_tab_restore();
        assert!(app.restore.pending.is_empty(), "the window holding the channel restores them");
        app.apply_scan(crate::demo::scan(&app.config));
        assert!(!app.restore_tabs_if_always(Instant::now()));
        assert!(app.tabs.list.is_empty());
    }

    #[test]
    fn a_second_window_never_replaces_the_first_windows_saved_tabs_in_any_close_order() {
        let root = std::env::temp_dir().join(format!("cs-restore-second-{}", std::process::id()));
        let disk = Source::Real(claude_switch_core::Roots { home: root.clone(), appdata: root.clone() });
        let file = path(&disk).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let saved = Entry {
            id: A.into(),
            account: "main".into(),
            cwd: "C:/a".into(),
            active: true,
            chat: None,
            opencode: false,
        };
        std::fs::write(&file, encode([saved.clone()])).unwrap();
        let open = |second: bool| {
            let mut app = AppState::new(Source::Demo, None, Default::default());
            app.restore.second_window = second;
            app.restore.load_snapshot(&file).unwrap();
            app.source = disk.clone();
            app
        };
        // The second window closes first, then again after the first one closed.
        for never in [false, true] {
            let mut second = open(true);
            if never {
                second.win_file.restore_tabs = RestoreTabs::Never;
            }
            second.save_tab_restore().unwrap();
            second.dismiss_tab_restore();
            assert!(second.restore.second_window, "dismissing keeps it a second window");
            second.save_tab_restore().unwrap();
            assert_eq!(decode(&std::fs::read_to_string(&file).unwrap()), vec![saved.clone()]);
        }
        let mut first = open(false);
        let mut second = open(true);
        first.restore.save_snapshot(&file, vec![saved.clone(), Entry { id: B.into(), ..saved.clone() }]).unwrap();
        let written = std::fs::read_to_string(&file).unwrap();
        second.save_tab_restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written, "the first window's close survives");
        first.restore.baseline = Some(written);
        first.save_tab_restore().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_second_window_that_closes_last_saves_its_tabs_and_one_without_tabs_keeps_the_firsts() {
        let root = std::env::temp_dir().join(format!("cs-restore-last-{}", std::process::id()));
        let disk = Source::Real(claude_switch_core::Roots { home: root.clone(), appdata: root.clone() });
        let file = path(&disk).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let first = Entry {
            id: A.into(),
            account: "main".into(),
            cwd: "C:/a".into(),
            active: true,
            chat: None,
            opencode: false,
        };
        std::fs::write(&file, encode([first.clone()])).unwrap();
        let open = |tab: bool| {
            let mut app = AppState::new(Source::Demo, None, Default::default());
            app.restore.second_window = true;
            app.restore.load_snapshot(&file).unwrap();
            app.source = disk.clone();
            if tab {
                let new = NewTab { label: "b".into(), color: [0; 3], session: None, account: Some("main".into()) };
                let id = app.tabs.open(new, Term::empty((8, 4)));
                let tab = app.tabs.get_mut(id).unwrap();
                tab.resume_id = Some(B.into());
                tab.cwd = Some("C:/b".into());
                app.tabs.active = Some(id);
            }
            app
        };
        // The first window closed meanwhile and saved other tabs than this window loaded.
        let mut second = open(true);
        let closed =
            encode([first.clone(), Entry { id: "33333333-0000-0000-0000-000000000000".into(), ..first.clone() }]);
        std::fs::write(&file, &closed).unwrap();
        second.save_tab_restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), closed, "still a second window: nothing written");
        // Closing as the last window (it now holds the channel): its tabs are the ones to restore.
        second.inherit_tab_restore();
        second.save_tab_restore().unwrap();
        let saved = decode(&std::fs::read_to_string(&file).unwrap());
        assert_eq!(saved.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [B]);

        // A last window with no tabs of its own leaves the first window's.
        std::fs::write(&file, &closed).unwrap();
        let mut empty = open(false);
        empty.inherit_tab_restore();
        empty.save_tab_restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), closed);
        // Unless tabs are never restored: then the snapshot is cleared as for any window.
        empty.win_file.restore_tabs = RestoreTabs::Never;
        empty.save_tab_restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ask_waits_for_a_choice_and_warning_cancel_continues_the_batch() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let now = Instant::now();
        app.load_tab_restore();
        app.restore.pending[1].account = "work".into();
        let remaining = app.restore.pending[2].id.clone();
        app.apply_scan(crate::demo::scan(&app.config));
        assert!(!app.restore_tabs_if_always(now));
        assert!(app.tabs.is_empty());
        app.begin_tab_restore(now);
        assert!(matches!(app.dialog, Some(Dialog::Confirm { then: Pending::RestoreTab, .. })));
        assert!(app.tabs.is_empty());
        crate::dialogs::click(&mut app, now, crate::dialogs::CANCEL, 0.0);
        assert_eq!(app.tabs.list.len(), 1);
        assert_eq!(app.tabs.list[0].session.as_deref(), Some(remaining.as_str()));
        app.dismiss_tab_restore();
        app.win_file.restore_tabs = RestoreTabs::Never;
        app.load_tab_restore();
        assert!(app.restore.pending.is_empty());
    }

    #[test]
    fn live_identity_requires_matching_process_folder_and_account_and_rejects_ambiguity() {
        let cwd = PathBuf::from(r"C:\test");
        let running = |id: &str, pid| Running {
            session_id: id.into(),
            pid,
            app: App::Cli,
            account: "main".into(),
            cwd: cwd.clone(),
        };
        let records = [running(A, 10), running(B, 11)];
        assert_eq!(live_id(&records, "main", &cwd, |pid| pid == 10), Some(A));
        assert_eq!(live_id(&records, "elsewhere", &cwd, |_| true), None);
        assert_eq!(live_id(&records, "main", Path::new(r"C:\other"), |_| true), None);
        assert_eq!(live_id(&records, "main", &cwd, |_| true), None);
    }
}
