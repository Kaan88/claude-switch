//! What the window does: its command table, opening a session (with the safeguard and the carry
//! warning), new sessions, and the command palette's entries.

use std::path::{Path, PathBuf};
use std::time::Instant;

use claude_switch_core::{App, Launch, Place, Refusal};
use mdshared_ui::commands::Command;
use mdshared_ui::gfx::Rect;
use mdshared_ui::menu::{Item, Menu};
use mdshared_ui::palette::{Entry, Palette};

use crate::archive::{
    ARCHIVE_FOLDER, ARCHIVE_OLD, ARCHIVE_ROW, REPORT, UNARCHIVE_ALL, UNARCHIVE_ROW, UNARCHIVE_SELECTED,
};
use crate::dialogs::{self, Dialog, Pending};
use crate::state::{AppState, Focus, MenuKind};

pub const NEW_SESSION: u32 = 1;
pub const NEW_DESKTOP: u32 = 2;
pub const REFRESH: u32 = 3;
pub const ACCOUNTS: u32 = 4;
pub const SETTINGS: u32 = 5;
pub const ABOUT: u32 = 6;
pub const PALETTE: u32 = 7;
pub const OPEN_CLI: u32 = 8;
pub const OPEN_DESKTOP: u32 = 9;
pub const OPEN_AS: u32 = 10;
pub const FIND: u32 = 11;
pub const OPEN_LAST: u32 = 12;
pub const NEW_SHELL: u32 = 13;
pub const NEXT_TAB: u32 = 14;
pub const PREV_TAB: u32 = 15;
pub const CLOSE_TAB: u32 = 16;
pub const OPEN_COPY: u32 = 17;
pub const SESSION_VIEW: u32 = 18;
pub const PASTE: u32 = 19;
pub const SWITCHER: u32 = 20;
const RENAME: u32 = 21;
const PIN: u32 = 22;
const SUGGEST: u32 = 23;
const SUGGEST_BATCH: u32 = 24;
pub const HIDE: u32 = 25;
const HIDE_FOLDER: u32 = 26;
const COPY_ID: u32 = 27;
const COPY_PATH: u32 = 28;
const REVEAL: u32 = 29;
const OPEN_FOLDER: u32 = 32;
const MOVE_FOLDER: u32 = 33;
const CHAT_LAYOUT: u32 = 30;
const SET_TOPIC: u32 = 31;
const MEANING_INDEX: u32 = 240;
const MEANING_ASK: u32 = 241;
pub const NEW_OPENCODE: u32 = 242;

/// The confirmation before a session the desktop app still lists is opened in the CLI.
pub const DESKTOP_WARNING: &str =
    "The desktop app still lists this session. Do not continue it there while it is open here, or messages are lost.";

/// Palette ids above these name an account or a session by index.
const ACCOUNT_BASE: u32 = 50_000;
const SESSION_BASE: u32 = 100_000;
/// Menu ids of "Open as…": `OPEN_AS_BASE + account index * 2 + (0 CLI, 1 Desktop)`.
const OPEN_AS_BASE: u32 = 1_000;

pub const COMMANDS: &[Command] = &[
    Command::new(NEW_SESSION, "session.new", "New session…").keys("Ctrl+N"),
    Command::new(NEW_DESKTOP, "session.new_desktop", "New session in the desktop app"),
    Command::new(OPEN_CLI, "session.open_cli", "Open in CLI"),
    Command::new(OPEN_DESKTOP, "session.open_desktop", "Open in Desktop"),
    Command::new(OPEN_AS, "session.open_as", "Open as…"),
    Command::new(OPEN_COPY, "session.open_copy", "Open a copy in CLI"),
    Command::new(crate::compact_dialog::COMMAND, "session.compact", crate::compact_dialog::LABEL),
    Command::new(NEW_SHELL, "tab.new_shell", "New shell tab").keys("Ctrl+Shift+T"),
    Command::new(NEW_OPENCODE, "tab.new_opencode", "New OpenCode tab…"),
    Command::new(NEXT_TAB, "tab.next", "Next tab").keys("Ctrl+Tab"),
    Command::new(PREV_TAB, "tab.previous", "Previous tab").keys("Ctrl+Shift+Tab"),
    Command::new(CLOSE_TAB, "tab.close", "Close terminal tab").keys("Ctrl+Shift+W"),
    Command::new(REFRESH, "view.refresh", "Refresh").keys("F5"),
    Command::new(FIND, "view.find", "Search sessions").keys("Ctrl+F"),
    Command::new(PALETTE, "view.palette", "Command palette").keys("Ctrl+K"),
    Command::new(SWITCHER, "session.switch", "Quick switcher").keys("Ctrl+P"),
    Command::new(RENAME, "session.rename", "Rename session").keys("F2"),
    Command::new(PIN, "session.pin", "Pin / unpin session").keys("Ctrl+D"),
    Command::new(SUGGEST, "session.suggest", "Suggest names for the selected session"),
    Command::new(SUGGEST_BATCH, "session.suggest_recent", "Suggest names for all unnamed recent sessions"),
    Command::new(MEANING_INDEX, "session.meaning_index", "Summarize sessions for search by meaning…"),
    Command::new(MEANING_ASK, "session.meaning_ask", "Ask about past sessions…"),
    Command::new(ACCOUNTS, "app.accounts", "Accounts…"),
    Command::new(SETTINGS, "app.settings", "Settings…"),
    Command::new(ABOUT, "app.about", "About"),
    Command::new(ARCHIVE_OLD, "session.archive_old", "Archive older sessions…"),
];

impl AppState {
    /// Runs a command of the table (or a palette entry's id).
    pub fn command(&mut self, now: Instant, id: u32, anchor: Option<Rect>) {
        match id {
            NEW_SESSION => self.ask_new_session(),
            crate::compact_dialog::COMMAND => {
                if let Some(tab) = self.tabs.shown() {
                    self.ask_tab_compact(tab.id, None);
                } else {
                    self.ask_compact(false);
                }
            }
            NEW_DESKTOP => {
                let folder = self.start_folder();
                let account = self.folder_open(&folder, App::Desktop).0.to_owned();
                self.new_session_now(now, App::Desktop, &account, &folder);
            }
            crate::delete::DELETE_ROW | crate::delete::COPY_DELETE => self.delete_action(id),
            ARCHIVE_OLD | ARCHIVE_ROW | ARCHIVE_FOLDER | UNARCHIVE_ROW | UNARCHIVE_SELECTED | UNARCHIVE_ALL
            | REPORT => {
                self.archive_action(now, id);
            }
            REFRESH => self.refresh(now),
            ACCOUNTS => self.dialog = Some(dialogs::accounts(self, now)),
            SETTINGS => self.opencode_settings(false),
            ABOUT => self.dialog = Some(Dialog::About),
            PALETTE => self.open_palette(),
            SWITCHER => self.quick_switcher(),
            RENAME => self.ask_rename_session(),
            SET_TOPIC => self.ask_set_topic(),
            SUGGEST | SUGGEST_BATCH => self.ask_naming(id == SUGGEST_BATCH),
            MEANING_INDEX => self.ask_meaning_index(),
            MEANING_ASK => self.ask_meaning_question(),
            HIDE_FOLDER if let Some(key) = self.context_folder.take() => self.toggle_group_hidden(now, key),
            HIDE | HIDE_FOLDER => self.toggle_hidden(now, id == HIDE_FOLDER),
            PIN => {
                if let Some(&i) = self.selected.as_ref().and_then(|id| self.by_id.get(id)) {
                    self.toggle_pin(i);
                    self.reveal_selected();
                }
            }
            COPY_ID | COPY_PATH | REVEAL | OPEN_FOLDER => self.session_file_action(now, id),
            SESSION_VIEW => self.toggle_session_view(),
            CHAT_LAYOUT => self.set_chat_layout(!self.chat_active()),
            PASTE => self.paste(),
            NEW_SHELL => self.new_shell(now),
            NEW_OPENCODE => self.ask_new_opencode_tab(),
            NEXT_TAB | PREV_TAB => self.step_tab(id == NEXT_TAB),
            CLOSE_TAB => {
                if let Some(t) = self.tabs.active {
                    self.ask_close_tab(t);
                }
            }
            FIND => {
                self.show_sessions();
                self.focus = Focus::Search;
                self.search.select_all();
            }
            OPEN_CLI | OPEN_DESKTOP => {
                let app = if id == OPEN_CLI { App::Cli } else { App::Desktop };
                let account = self.current().slug.clone();
                if let Some(s) = self.selected.clone() {
                    self.open(now, &s, account, app);
                }
            }
            OPEN_COPY => {
                if let Some(s) = self.selected.clone() {
                    self.open_copy(now, &s);
                }
            }
            MOVE_FOLDER => {
                if let Some(s) = self.selected.clone() {
                    self.pick_session_folder(now, &s);
                }
            }
            OPEN_LAST => {
                if let Some(s) = self.selected_session() {
                    let id = s.id.clone();
                    let (account, app) = self.folder_open(&s.cwd, s.last_app);
                    let account = account.to_owned();
                    self.open(now, &id, account, app);
                }
            }
            OPEN_AS => {
                if self.selected.is_some() {
                    let anchor = anchor.unwrap_or(Rect::new(self.mouse.0, self.mouse.1, 0.0, 0.0));
                    self.menu =
                        Some((MenuKind::OpenAs, Menu::new(self.open_as_items(), 0.0, 0.0, false).around(anchor)));
                }
            }
            x if (OPEN_AS_BASE..OPEN_AS_BASE + 200).contains(&x) => {
                let k = ((x - OPEN_AS_BASE) / 2) as usize;
                let app = if (x - OPEN_AS_BASE).is_multiple_of(2) { App::Cli } else { App::Desktop };
                if let (Some(a), Some(s)) = (self.config.accounts.get(k), self.selected.clone()) {
                    let slug = a.slug.clone();
                    self.open(now, &s, slug, app);
                }
            }
            x if (ACCOUNT_BASE..SESSION_BASE).contains(&x) => {
                if let Some(a) = self.config.accounts.get((x - ACCOUNT_BASE) as usize) {
                    self.switch_account(&a.slug.clone());
                }
            }
            x if x >= SESSION_BASE => {
                if let Some(id) = self.palette_session(x).map(str::to_owned) {
                    self.select(&id);
                    self.reveal_selected();
                    self.command(now, OPEN_LAST, None);
                }
            }
            _ => {}
        }
    }

    pub fn switch_account(&mut self, slug: &str) {
        if self.config.current != slug {
            self.config.current = slug.to_string();
            self.save_config();
            self.rebuild_accounts();
        }
    }

    /// The items of "Open as…": every account, in each app, independent of the folder default.
    fn open_as_items(&self) -> Vec<Item<u32>> {
        let mut items = Vec::new();
        for (k, a) in self.config.accounts.iter().enumerate() {
            let id = OPEN_AS_BASE + k as u32 * 2;
            items.push(Item::new(id, format!("Open in CLI as {}", a.name)));
            items.push(Item::new(id + 1, format!("Open in Desktop as {}", a.name)));
        }
        if items.is_empty() {
            items.push(Item::new(0, "There is no other account").enabled(false));
        }
        items.push(Item::new(crate::compact_dialog::COMMAND, crate::compact_dialog::LABEL).sep());
        items
    }

    /// Opens a session: asks first for a cold cache or a Desktop record when opening in the CLI.
    pub fn open(&mut self, now: Instant, id: &str, account: String, app: App) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        if self.focus_tab_of(now, id) {
            return;
        }
        if let Some(r) = self.running_of(id) {
            let text = Refusal::AlreadyRunning(r.clone()).to_string();
            self.warn("Already open", text);
            return;
        }
        match self.resume_warning(&session, &account, app) {
            Some((title, text)) => {
                let then = Pending::Open { id: id.to_string(), account, app };
                self.dialog = Some(Dialog::Confirm { title, text, ok: "Continue".into(), then });
            }
            None => self.open_now(now, id, &account, app),
        }
    }

    /// Opens a copy of a session (a fork, a new session) in the CLI as the current account: safe
    /// while the original runs anywhere. The carry warning applies to it too.
    pub fn open_copy(&mut self, now: Instant, id: &str) {
        self.open_copy_as(now, id, self.current().slug.clone());
    }

    pub fn open_copy_as(&mut self, now: Instant, id: &str, account: String) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        match self.source.carry_warning(&self.config, &session, &account) {
            Some(text) => {
                let title = format!("Open a copy as {}?", self.account_name(&account));
                let then = Pending::Copy { id: id.to_string(), account, delete: false };
                self.dialog = Some(Dialog::Confirm { title, text, ok: "Continue".into(), then });
            }
            None => self.copy_now(now, id, &account),
        }
    }

    pub fn copy_now(&mut self, now: Instant, id: &str, account: &str) {
        self.copy_now_then(now, id, account, false);
    }

    /// Opens a copy; with `delete`, the original is deleted once the copy's transcript is written.
    pub fn copy_now_then(&mut self, now: Instant, id: &str, account: &str, delete: bool) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        let copy_id = match crate::compact_open::session_id() {
            Ok(id) => id,
            Err(e) => return self.error("Not opened", e),
        };
        match self.source.fork(&self.config, &session, account, &copy_id) {
            Err(r) => self.error("Not opened", r.to_string()),
            Ok(l) => {
                if !self.source.is_demo() && !self.record_copy(&copy_id, id) {
                    return;
                }
                let name = self.account_name(account);
                let done = format!("Opened a copy of \u{201c}{}\u{201d} as {name}", self.title_of(&session));
                if self.start_with_result(now, &l, None, Some(account), done, Some((&session.cwd, App::Cli))) {
                    self.remember_folder_open(&session.cwd, account, App::Cli);
                    if delete && self.source.is_demo() {
                        self.toast("Demo: the original was not deleted".into(), now);
                    } else if delete {
                        let tab = self.config.embedded_terminal.then_some(self.tabs.active).flatten();
                        self.delete_after_copy(&session, &copy_id, account, tab);
                    }
                } else if !self.source.is_demo() {
                    self.marks.clear_parent(&copy_id);
                    self.save_marks();
                }
            }
        }
    }

    /// Branch at a message of the conversation in tab `tab`: a new session, as the tab's account,
    /// that continues from a copy of the transcript cut there (`claude_switch_core::branch`). At
    /// a prompt the copy keeps that prompt's whole turn; at an answer it ends with the answer.
    pub fn branch_at(&mut self, now: Instant, tab: u64, item: claude_switch_core::transcript::ItemId) {
        let Some(t) = self.tabs.get(tab) else { return };
        let Some(pane) = t.pane.as_ref() else { return };
        let account = t.account.clone().unwrap_or_else(|| self.current().slug.clone());
        let Some(session) = t.session.as_ref().and_then(|id| self.by_id.get(id)).map(|&i| self.sessions[i].clone())
        else {
            return self
                .error("Not branched", "The session is not in the Sessions list yet. Try again in a moment.".into());
        };
        let Some(cut) = pane.branch_cut(item) else { return };
        let new_id = match crate::compact_open::session_id() {
            Ok(id) => id,
            Err(e) => return self.error("Not branched", e),
        };
        match self.source.branch(&self.config, &session, &account, &new_id, cut) {
            Err(r) => self.error("Not branched", r.to_string()),
            Ok(l) => {
                if !self.source.is_demo() && !self.record_copy(&new_id, &session.id) {
                    let _ = std::fs::remove_file(session.file.with_file_name(format!("{new_id}.jsonl")));
                    return;
                }
                let name = self.account_name(&account);
                let done = format!("Opened a branch of \u{201c}{}\u{201d} as {name}", self.title_of(&session));
                if self.start_with_result(now, &l, None, Some(&account), done, Some((&session.cwd, App::Cli))) {
                    self.remember_folder_open(&session.cwd, &account, App::Cli);
                } else if !self.source.is_demo() {
                    self.marks.clear_parent(&new_id);
                    self.save_marks();
                    let _ = std::fs::remove_file(session.file.with_file_name(format!("{new_id}.jsonl")));
                }
            }
        }
    }

    /// Reads the running sessions again, then resumes and starts.
    pub fn open_now(&mut self, now: Instant, id: &str, account: &str, app: App) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        if self.focus_tab_of(now, id) {
            return;
        }
        match self.prepare_resume(&session, account, app) {
            Err(r @ Refusal::AlreadyRunning(_)) => self.warn("Already open", r.to_string()),
            Err(r) => self.error("Not opened", r.to_string()),
            Ok(l) => {
                let done = resume_status(app, self.title_of(&session), &self.account_name(account));
                if self.start_with_result(now, &l, Some((id, account)), Some(account), done, Some((&session.cwd, app)))
                {
                    self.remember_folder_open(&session.cwd, account, app);
                }
            }
        }
    }

    /// The safeguard for the window's own tabs: a session open in one counts as running at
    /// once, before any file says so. Shows that tab instead. True when there was one.
    fn focus_tab_of(&mut self, now: Instant, id: &str) -> bool {
        let Some(tab) = self.tab_of_session(id) else { return false };
        self.show_tab(tab);
        self.toast("Already open in a tab".into(), now);
        true
    }

    /// Runs a launch; on success notes it and says so. A launch that needs a terminal opens a
    /// tab of the window when the setting says so (the demo starts nothing either way).
    /// `account` gives the tab its colour.
    pub fn start(&mut self, now: Instant, l: &Launch, note: Option<(&str, &str)>, account: Option<&str>, done: String) {
        self.start_with_result(now, l, note, account, done, None);
    }

    fn start_with_result(
        &mut self,
        now: Instant,
        l: &Launch,
        note: Option<(&str, &str)>,
        account: Option<&str>,
        done: String,
        folder: Option<(&Path, App)>,
    ) -> bool {
        if !self.source.is_demo() && crate::run::needs_desktop_worker(l) {
            if self.desktop_job.is_some() {
                self.warn("Desktop handoff pending", "Wait for the current handoff to finish.".into());
                return false;
            }
            self.start_desktop_handoff(l.clone(), note.map(|(id, account)| (id.to_owned(), account.to_owned())), done);
            self.desktop_folder_open = self
                .desktop_job
                .as_ref()
                .and(folder.zip(account))
                .map(|((cwd, app), slug)| (cwd.to_owned(), slug.to_owned(), app));
            return false; // Pending is not success; do not persist a folder preference yet.
        }
        let in_tab = self.config.embedded_terminal && l.place == Place::Terminal;
        let (started, history) = match self.start_launch(l, note, account, in_tab) {
            Ok(outcome) => outcome,
            Err(r) => {
                self.error("Not opened", r.to_string());
                return false;
            }
        };
        let success = started.is_ok();
        match started {
            Ok(_) => {
                let done = if self.source.is_demo() { format!("{done} (demo: nothing started)") } else { done };
                self.toast(done, now);
                self.info = None;
            }
            Err(e) => self.error("Could not start it", format!("{}: {e}", l.program.display())),
        }
        if let Err(error) = history {
            self.error("Could not finish the launch record", error.to_string());
        }
        success
    }

    fn start_desktop_handoff(&mut self, launch: Launch, note: Option<(String, String)>, done: String) {
        let source = self.source.clone();
        let config = self.config.clone();
        match mdshared_app::job::Job::start(self.wake_fn(), move |cancel, _| {
            let reservation = note
                .as_ref()
                .map(|(id, account)| source.reserve(&config, id, account))
                .transpose()
                .map_err(|e| e.to_string())?
                .flatten();
            let result = if crate::run::needs_terminal(&launch) {
                crate::run::run_unseen(&launch, || cancel.cancelled())
            } else {
                crate::run::run_cancellable(&launch, || cancel.cancelled())
            }
            .map(|()| done);
            let history = reservation
                .map(|r| match &result {
                    Ok(_) => r.finish_desktop(),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {
                        Ok(())
                    }
                    Err(_) => r.finish(false),
                })
                .transpose();
            match (result, history) {
                (Ok(done), Ok(_)) => Ok(done),
                (Err(error), Ok(_)) => Err(error.to_string()),
                (Ok(done), Err(history)) => Err(format!("{done}; could not update launch history: {history}")),
                (Err(error), Err(history)) => Err(format!("{error}; could not roll back launch history: {history}")),
            }
        }) {
            Ok(job) => self.desktop_job = Some(job),
            Err(error) => self.error("Could not start desktop handoff", error.to_string()),
        }
    }

    pub fn take_desktop_job(&mut self, now: Instant) -> bool {
        if !self.desktop_job.as_ref().is_some_and(|job| !job.running()) {
            return false;
        }
        let folder = self.desktop_folder_open.take();
        match self.desktop_job.take().and_then(|job| job.take()) {
            Some(Ok(done)) => {
                self.toast(done, now);
                if let Some((cwd, account, app)) = folder {
                    self.remember_folder_open(&cwd, &account, app);
                }
            }
            Some(Err(error)) => self.error("Desktop handoff", error),
            None => self.error("Desktop handoff", "The handoff worker stopped.".into()),
        }
        true
    }

    /// A plain shell in a tab, in the selected session's folder or the home folder.
    pub fn new_shell(&mut self, now: Instant) {
        let cwd = self.start_folder();
        let l = claude_switch_core::launch::shell(&cwd);
        if self.source.is_demo() {
            self.toast(format!("{} (demo: nothing started)", l.label), now);
            return;
        }
        if let Err(e) = self.open_tab(&l, None, None) {
            self.error("Could not start it", format!("{}: {e}", l.program.display()));
        }
    }

    /// An existing folder for a shell, or the home folder.
    fn start_folder(&self) -> PathBuf {
        self.selected_session()
            .map(|s| s.cwd.clone())
            .filter(|c| !c.as_os_str().is_empty() && (self.source.is_demo() || c.is_dir()))
            .unwrap_or_else(|| self.source.home())
    }

    pub fn new_session_now(&mut self, now: Instant, app: App, account: &str, cwd: &Path) {
        match self.source.new_session(&self.config, account, app, cwd) {
            Ok(mut l) => {
                if app == App::Cli && !self.config.embedded_terminal && !self.source.is_demo() {
                    match crate::compact_open::session_id() {
                        Ok(id) => l.args.extend(["--session-id".into(), id]),
                        Err(error) => return self.error("Not started", error),
                    }
                }
                let folder = cwd.file_name().map_or_else(|| cwd.display().to_string(), |n| n.to_string_lossy().into());
                let where_ = crate::folder_defaults::app_label(app);
                let done = format!("Started a new session in {folder} as {} in {where_}", self.account_name(account));
                if self.start_with_result(now, &l, None, Some(account), done, Some((cwd, app))) {
                    self.remember_folder_open(cwd, account, app);
                }
            }
            Err(r) => self.error("Not started", r.to_string()),
        }
    }

    pub fn main_menu(&mut self, anchor: Rect) {
        let k = |id| self.keymap.hint(id);
        let items = vec![
            Item::new(SWITCHER, "Quick switcher").hint(if self.terminal_shown() { "Ctrl+Shift+P" } else { "Ctrl+P" }),
            Item::new(ACCOUNTS, "Accounts…"),
            Item::new(SETTINGS, "Settings…"),
            Item::new(PALETTE, "Command palette").hint(k(PALETTE)).sep(),
            Item::new(ABOUT, "About"),
        ];
        let mut menu = Menu::new(items, 0.0, 0.0, true).around(anchor).min_width(200.0);
        menu.set_high_contrast(false);
        self.menu = Some((MenuKind::Main, menu));
    }

    pub fn open_palette(&mut self) {
        let mut session_entries = std::collections::HashMap::new();
        let mut entries: Vec<Entry> = self.keymap.entries().map(|e| e.group(0)).collect();
        if self.archive_count != 0 {
            entries.push(Entry::new(REPORT, "Archive summary report…").group(0));
        }
        if self.terminal_shown() {
            entries.push(Entry::new(PASTE, "Paste").group(0));
        }
        if self.tabs.shown().is_some_and(|tab| tab.pane.is_some()) {
            entries.push(Entry::new(CHAT_LAYOUT, "Switch Chat / Terminal · F6 in palette").group(0));
            if !self.chat_active() {
                entries.push(Entry::new(SESSION_VIEW, "Toggle session view · Ctrl+Shift+E").group(0));
            }
        }
        for (k, a) in self.config.accounts.iter().enumerate() {
            entries.push(Entry::new(ACCOUNT_BASE + k as u32, format!("Switch to account {}", a.name)).group(1));
        }
        for (i, s) in self.sessions.iter().enumerate() {
            if !self.listed(s) {
                continue;
            }
            let folder = s.cwd.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let entry = SESSION_BASE + i as u32;
            session_entries.insert(entry, s.id.clone());
            entries.push(Entry::new(entry, format!("Open {}", self.title_of(s))).hint(folder).group(2));
        }
        let mut p = Palette::new();
        p.set_groups(vec![(0, "Commands".into()), (1, "Accounts".into()), (2, "Sessions".into())]);
        p.set_entries(entries);
        p.set_placeholder("Type a command or a session");
        p.open("");
        self.palette = Some(p);
        self.palette_sessions = session_entries;
    }

    fn palette_session(&self, entry: u32) -> Option<&str> {
        let id = self.palette_sessions.get(&entry)?;
        let s = self.sessions.get(*self.by_id.get(id)?)?;
        (s.id == *id && self.listed(s)).then_some(id.as_str())
    }

    pub fn row_menu(&mut self, entry: usize, anchor: Rect) {
        if let Some(mdshared_ui::sections::Entry::Heading(g)) = self.sections.entry(entry) {
            if let Some(key) = self.groups[g].key.strip_prefix("folder:") {
                self.context_folder = Some(key.to_string());
                let mut items = vec![
                    Item::new(HIDE_FOLDER, "Folder hidden"),
                    Item::new(ARCHIVE_FOLDER, "Archive folder…").enabled(!key.is_empty()),
                ];
                self.archive_view_items(&mut items);
                self.menu = Some((MenuKind::Row, Menu::new(items, 0.0, 0.0, false).around(anchor).min_width(244.0)));
            }
            return;
        }
        let Some(i) = self.session_at(entry) else { return };
        let id = self.sessions[i].id.clone();
        self.context_folder = None;
        self.select(&id);
        let s = &self.sessions[i];
        let mut items = vec![
            Item::new(OPEN_CLI, "Open in CLI"),
            Item::new(OPEN_DESKTOP, "Open in Desktop"),
            Item::new(OPEN_AS, "Open as…"),
            Item::new(OPEN_COPY, "Open a copy"),
            crate::delete::copy_item(self.delete_block(&s.id)),
            Item::new(MOVE_FOLDER, "Continue in another folder…"),
            Item::new(crate::compact_dialog::COMMAND, crate::compact_dialog::LABEL),
            Item::new(RENAME, "Rename").hint("F2").sep(),
            Item::new(SET_TOPIC, "Set topic…"),
            Item::new(SUGGEST, "Suggest name"),
            Item::new(PIN, "Pinned").hint("Ctrl+D").enabled(!self.is_archived(s)),
            Item::new(HIDE, "Session hidden").hint("Delete"),
            Item::new(HIDE_FOLDER, "Folder hidden").enabled(!s.cwd.as_os_str().is_empty()),
            archive_row_item(self.is_archived(s), self.row_flags(s)),
            crate::delete::row_item(self.delete_block(&s.id)),
            Item::new(ARCHIVE_FOLDER, "Archive folder…").enabled(!s.cwd.as_os_str().is_empty()),
            Item::new(OPEN_FOLDER, "Open working folder").enabled(!s.cwd.as_os_str().is_empty()).sep(),
            Item::new(COPY_ID, "Copy session id"),
            Item::new(COPY_PATH, "Copy folder path").enabled(!s.cwd.as_os_str().is_empty()),
            Item::new(REVEAL, "Reveal transcript in Explorer"),
        ];
        self.archive_view_items(&mut items);
        self.menu = Some((MenuKind::Row, Menu::new(items, 0.0, 0.0, false).around(anchor).min_width(244.0)));
    }

    pub(crate) fn row_switches(&self) -> [(u32, bool); 3] {
        let s = self.selected_session();
        let folder =
            self.context_folder.clone().or_else(|| s.map(|s| claude_switch_core::sessions::folder_key(&s.cwd)));
        [
            (PIN, s.is_some_and(|s| self.marks.pinned(&s.id))),
            (HIDE, s.is_some_and(|s| self.marks.hidden(&s.id))),
            (HIDE_FOLDER, folder.is_some_and(|f| self.marks.folder_hidden(&f))),
        ]
    }

    fn session_file_action(&mut self, now: Instant, action: u32) {
        use mdshared_ui::platform::set_clipboard_text;
        let Some(s) = self.selected_session() else { return };
        match action {
            COPY_ID => {
                set_clipboard_text(&s.id);
                self.toast("Session id copied".into(), now);
            }
            COPY_PATH => {
                set_clipboard_text(&s.cwd.to_string_lossy());
                self.toast("Folder path copied".into(), now);
            }
            OPEN_FOLDER => {
                let folder = s.cwd.clone();
                self.open_working_folder(now, &folder);
            }
            REVEAL if self.source.is_demo() => self.toast("Demo: Explorer was not started".into(), now),
            REVEAL => {
                let path = s.file.clone();
                if !path.is_file() {
                    self.warn("Transcript not found", path.display().to_string());
                } else if let Err(e) = crate::run::reveal(&path) {
                    self.error("Could not open Explorer", e.to_string());
                }
            }
            _ => {}
        }
    }
}

impl AppState {
    /// "Continue in another folder…": the system's folder picker, starting at the session's.
    fn pick_session_folder(&mut self, now: Instant, id: &str) {
        if self.source.is_demo() {
            self.toast("Demo: the folder picker was not opened".into(), now);
            return;
        }
        let Some(&i) = self.by_id.get(id) else { return };
        let start = Some(self.sessions[i].cwd.clone()).filter(|cwd| cwd.is_dir());
        let (proxy, id) = (self.proxy.clone(), id.to_owned());
        mdshared_ui::platform::pick_folder_async(start.as_deref(), move |folder| {
            if let Some(folder) = folder
                && let Some(proxy) = proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(crate::state::Msg::SessionFolderPicked(id, folder));
            }
        });
    }

    /// Continues session `id` in `folder` as the current account: Claude Code resumes it there and
    /// writes on into the same transcript, naming the new folder in what it writes from now on
    /// (Claude Code 2.1.332). The app remembers the folder for the session, so the list, its
    /// group and the next Open follow it. Only a session that is not open anywhere: it moves when
    /// it starts again.
    pub(crate) fn continue_in_folder(&mut self, now: Instant, id: &str, folder: &std::path::Path) {
        let Some(&i) = self.by_id.get(id) else { return };
        if !folder.is_dir() {
            self.warn("Folder not found", folder.display().to_string());
            return;
        }
        let open_here =
            self.tabs.list.iter().any(|t| t.session.as_deref() == Some(id) || t.resume_id.as_deref() == Some(id));
        if open_here || self.running_of(id).is_some() {
            self.warn(
                "Close the session first",
                "A session continues in another folder when it starts again. Close it, then try again.".into(),
            );
            return;
        }
        if self.sessions[i].cwd != folder && !self.set_session_folder(id, folder) {
            return;
        }
        let account = self.current().slug.clone();
        self.open(now, id, account, App::Cli);
    }

    /// Opens a session's or tab's working folder in Explorer, or says why not.
    pub(crate) fn open_working_folder(&mut self, now: Instant, folder: &std::path::Path) {
        if self.source.is_demo() {
            self.toast("Demo: Explorer was not started".into(), now);
        } else if folder.as_os_str().is_empty() || !folder.is_dir() {
            self.warn("Working folder not found", folder.display().to_string());
        } else if let Err(e) = crate::run::open_folder(folder) {
            self.error("Could not open the folder", e.to_string());
        }
    }
}

fn resume_status(app: App, title: &str, account: &str) -> String {
    match app {
        App::Cli => format!("Opened “{title}” in a terminal as {account}"),
        App::Desktop => format!("“{title}” handed to the desktop app"),
    }
}

/// The text of the confirmation before a session is opened: the carry warning, the desktop
/// warning, or both in one; `None` when neither applies.
pub fn open_warning(carry: Option<&str>, desktop_lists_it: bool) -> Option<String> {
    match (carry, desktop_lists_it) {
        (Some(c), true) => Some(format!("{c}\n\n{DESKTOP_WARNING}")),
        (Some(c), false) => Some(c.to_string()),
        (None, true) => Some(DESKTOP_WARNING.to_string()),
        (None, false) => None,
    }
}

fn archive_row_item(archived: bool, flags: crate::list::model::Flags) -> Item<u32> {
    if archived {
        return Item::new(UNARCHIVE_ROW, "Unarchive session");
    }
    let hint = match (flags.pinned, flags.running) {
        (false, false) => "",
        (true, false) => "Pinned",
        (false, true) => "Running",
        (true, true) => "Pinned and running",
    };
    Item::new(ARCHIVE_ROW, "Archive session…")
        .enabled(claude_switch_core::archive::eligible(flags.pinned, flags.running, archived))
        .hint(hint)
}

#[cfg(test)]
mod tests;
