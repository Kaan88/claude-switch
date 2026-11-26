//! What the window does: its command table, opening a session (with the safeguard and the carry
//! warning), new sessions, and the command palette's entries.

use std::path::{Path, PathBuf};
use std::time::Instant;

use claude_switch_core::{App, Launch, Place, Refusal};
use mdshared_ui::commands::Command;
use mdshared_ui::gfx::Rect;
use mdshared_ui::menu::{Item, Menu};
use mdshared_ui::palette::{Entry, Palette};

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

/// The confirmation before a session the desktop app still lists is opened in the CLI.
pub const DESKTOP_WARNING: &str =
    "The desktop app still lists this session. Do not continue it there while it is open here, or messages are lost.";

/// Palette ids above these name an account or a session by index.
const ACCOUNT_BASE: u32 = 50_000;
const SESSION_BASE: u32 = 100_000;
/// Menu ids of "Open as…": `OPEN_AS_BASE + account index * 2 + (0 CLI, 1 Desktop)`.
const OPEN_AS_BASE: u32 = 1_000;

pub const COMMANDS: &[Command] = &[
    Command::new(NEW_SESSION, "session.new", "New session in a terminal").keys("Ctrl+N"),
    Command::new(NEW_DESKTOP, "session.new_desktop", "New session in the desktop app"),
    Command::new(OPEN_CLI, "session.open_cli", "Open in CLI"),
    Command::new(OPEN_DESKTOP, "session.open_desktop", "Open in Desktop"),
    Command::new(OPEN_AS, "session.open_as", "Open as another account…"),
    Command::new(OPEN_COPY, "session.open_copy", "Open a copy in CLI"),
    Command::new(NEW_SHELL, "tab.new_shell", "New shell tab").keys("Ctrl+Shift+T"),
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
    Command::new(ACCOUNTS, "app.accounts", "Accounts…"),
    Command::new(SETTINGS, "app.settings", "Settings…"),
    Command::new(ABOUT, "app.about", "About"),
];

impl AppState {
    /// Runs a command of the table (or a palette entry's id).
    pub fn command(&mut self, now: Instant, id: u32, anchor: Option<Rect>) {
        match id {
            NEW_SESSION => self.ask_new_session(),
            NEW_DESKTOP => self.new_session_now(now, App::Desktop, &self.current().slug.clone(), &self.start_folder()),
            REFRESH => self.refresh(now),
            ACCOUNTS => self.dialog = Some(dialogs::accounts(self, now)),
            SETTINGS => self.dialog = Some(dialogs::settings(self)),
            ABOUT => self.dialog = Some(Dialog::About),
            PALETTE => self.open_palette(),
            SWITCHER => self.quick_switcher(),
            RENAME => self.ask_rename_session(),
            SUGGEST | SUGGEST_BATCH => self.ask_naming(id == SUGGEST_BATCH),
            HIDE_FOLDER if let Some(key) = self.context_folder.take() => self.toggle_group_hidden(now, key),
            HIDE | HIDE_FOLDER => self.toggle_hidden(now, id == HIDE_FOLDER),
            PIN => {
                if let Some(&i) = self.selected.as_ref().and_then(|id| self.by_id.get(id)) {
                    self.toggle_pin(i);
                    self.reveal_selected();
                }
            }
            COPY_ID | COPY_PATH | REVEAL => self.session_file_action(now, id),
            SESSION_VIEW => self.toggle_session_view(),
            PASTE => self.paste(),
            NEW_SHELL => self.new_shell(now),
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
            OPEN_LAST => {
                if let Some(s) = self.selected_session() {
                    let (id, app) = (s.id.clone(), s.last_app);
                    let account = self.current().slug.clone();
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
                if let Some(s) = self.sessions.get((x - SESSION_BASE) as usize) {
                    let id = s.id.clone();
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

    /// The items of "Open as…": every other account, in each app.
    fn open_as_items(&self) -> Vec<Item<u32>> {
        let current = &self.config.current;
        let mut items = Vec::new();
        for (k, a) in self.config.accounts.iter().enumerate() {
            if &a.slug == current {
                continue;
            }
            let id = OPEN_AS_BASE + k as u32 * 2;
            items.push(Item::new(id, format!("Open in CLI as {}", a.name)));
            items.push(Item::new(id + 1, format!("Open in Desktop as {}", a.name)));
        }
        if items.is_empty() {
            items.push(Item::new(0, "There is no other account").enabled(false));
        }
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
        let carry = self.source.carry_warning(&self.config, &session, &account);
        let desktop = app == App::Cli && (session.desktop_account.is_some() || session.desktop_id.is_some());
        match open_warning(carry.as_deref(), desktop) {
            Some(text) => {
                let title = match carry {
                    Some(_) => format!("Open as {}?", self.account_name(&account)),
                    None => "Open in the CLI?".to_string(),
                };
                let then = Pending::Open { id: id.to_string(), account, app };
                self.dialog = Some(Dialog::Confirm { title, text, ok: "Continue".into(), then });
            }
            None => self.open_now(now, id, &account, app),
        }
    }

    /// Opens a copy of a session (a fork, a new session) in the CLI as the current account: safe
    /// while the original runs anywhere. The carry warning applies to it too.
    pub fn open_copy(&mut self, now: Instant, id: &str) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        let account = self.current().slug.clone();
        match self.source.carry_warning(&self.config, &session, &account) {
            Some(text) => {
                let title = format!("Open a copy as {}?", self.account_name(&account));
                let then = Pending::Copy { id: id.to_string(), account };
                self.dialog = Some(Dialog::Confirm { title, text, ok: "Continue".into(), then });
            }
            None => self.copy_now(now, id, &account),
        }
    }

    pub fn copy_now(&mut self, now: Instant, id: &str, account: &str) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        match self.source.fork(&self.config, &session, account) {
            Err(r) => self.error("Not opened", r.to_string()),
            Ok(l) => {
                let name = self.account_name(account);
                let done = format!("Opened a copy of \u{201c}{}\u{201d} as {name}", self.title_of(&session));
                self.start(now, &l, None, Some(account), done);
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
        self.running = self.source.running(&self.config);
        self.index_running();
        self.rebuild_view();
        match self.source.resume(&self.config, &session, account, app, &self.running) {
            Err(r @ Refusal::AlreadyRunning(_)) => self.warn("Already open", r.to_string()),
            Err(r) => self.error("Not opened", r.to_string()),
            Ok(l) => {
                let where_ = if app == App::Cli { "a terminal" } else { "the desktop app" };
                let done = format!(
                    "Opened \u{201c}{}\u{201d} in {where_} as {}",
                    self.title_of(&session),
                    self.account_name(account)
                );
                self.start(now, &l, Some((id, account)), Some(account), done);
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
        // Noted before the start, not after: the session is not in the running list for a few
        // seconds, and the note is what refuses a second start in that time.
        if let Some((id, account)) = note {
            self.source.note_launch(id, account);
        }
        let in_tab = self.config.embedded_terminal && l.place == Place::Terminal && !self.source.is_demo();
        let started =
            if in_tab { self.open_tab(l, note.map(|(id, _)| id), account).map(drop) } else { self.source.run(l) };
        match started {
            Ok(()) => {
                let done = if self.source.is_demo() { format!("{done} (demo: nothing started)") } else { done };
                self.toast(done, now);
                self.info = None;
            }
            Err(e) => self.error("Could not start it", format!("{}: {e}", l.program.display())),
        }
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
            Ok(l) => {
                let folder = cwd.file_name().map_or_else(|| cwd.display().to_string(), |n| n.to_string_lossy().into());
                let done = format!("Started a new session in {folder} as {}", self.account_name(account));
                self.start(now, &l, None, Some(account), done);
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
        let mut entries: Vec<Entry> = self.keymap.entries().map(|e| e.group(0)).collect();
        if self.terminal_shown() {
            entries.push(Entry::new(PASTE, "Paste").group(0));
        }
        if self.tabs.shown().is_some_and(|tab| tab.pane.is_some()) {
            entries.push(Entry::new(SESSION_VIEW, "Toggle session view · Ctrl+Shift+E").group(0));
        }
        for (k, a) in self.config.accounts.iter().enumerate() {
            entries.push(Entry::new(ACCOUNT_BASE + k as u32, format!("Switch to account {}", a.name)).group(1));
        }
        for (i, s) in self.sessions.iter().enumerate() {
            if !self.listed(s) {
                continue;
            }
            let folder = s.cwd.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            entries
                .push(Entry::new(SESSION_BASE + i as u32, format!("Open {}", self.title_of(s))).hint(folder).group(2));
        }
        let mut p = Palette::new();
        p.set_groups(vec![(0, "Commands".into()), (1, "Accounts".into()), (2, "Sessions".into())]);
        p.set_entries(entries);
        p.set_placeholder("Type a command or a session");
        p.open("");
        self.palette = Some(p);
    }

    pub fn row_menu(&mut self, entry: usize, anchor: Rect) {
        if let Some(mdshared_ui::sections::Entry::Heading(g)) = self.sections.entry(entry) {
            if let Some(key) = self.groups[g].key.strip_prefix("folder:") {
                let label = if self.marks.folder_hidden(key) { "Unhide folder" } else { "Hide folder" };
                self.context_folder = Some(key.to_string());
                self.menu = Some((
                    MenuKind::Row,
                    Menu::new(vec![Item::new(HIDE_FOLDER, label)], 0.0, 0.0, false).around(anchor),
                ));
            }
            return;
        }
        let Some(i) = self.session_at(entry) else { return };
        let id = self.sessions[i].id.clone();
        self.context_folder = None;
        self.select(&id);
        let s = &self.sessions[i];
        let pin = if self.marks.pinned(&id) { "Unpin" } else { "Pin" };
        let hide = if self.marks.hidden(&id) { "Unhide session" } else { "Hide session" };
        let folder = if self.marks.folder_hidden(&claude_switch_core::sessions::folder_key(&s.cwd)) {
            "Unhide folder"
        } else {
            "Hide folder"
        };
        let items = vec![
            Item::new(OPEN_CLI, "Open in CLI"),
            Item::new(OPEN_DESKTOP, "Open in Desktop"),
            Item::new(OPEN_AS, "Open as…"),
            Item::new(OPEN_COPY, "Open a copy"),
            Item::new(RENAME, "Rename").hint("F2").sep(),
            Item::new(SUGGEST, "Suggest name"),
            Item::new(PIN, pin).hint("Ctrl+D"),
            Item::new(HIDE, hide).hint("Delete"),
            Item::new(HIDE_FOLDER, folder).enabled(!s.cwd.as_os_str().is_empty()),
            Item::new(COPY_ID, "Copy session id").sep(),
            Item::new(COPY_PATH, "Copy folder path").enabled(!s.cwd.as_os_str().is_empty()),
            Item::new(REVEAL, "Reveal transcript in Explorer"),
        ];
        self.menu = Some((MenuKind::Row, Menu::new(items, 0.0, 0.0, false).around(anchor).min_width(228.0)));
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
            REVEAL if self.source.is_demo() => self.toast("Demo: Explorer was not started".into(), now),
            REVEAL => {
                let path = s.file.clone();
                if !path.is_file() {
                    self.warn("Transcript not found", path.display().to_string());
                } else if let Err(e) = std::process::Command::new("explorer.exe").arg("/select,").arg(path).spawn() {
                    self.error("Could not open Explorer", e.to_string());
                }
            }
            _ => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_confirmation_holds_whichever_warnings_apply() {
        assert_eq!(open_warning(None, false), None);
        assert_eq!(open_warning(Some("Cold cache."), false).as_deref(), Some("Cold cache."));
        assert_eq!(open_warning(None, true).as_deref(), Some(DESKTOP_WARNING));
        let both = open_warning(Some("Cold cache."), true).unwrap();
        assert!(both.starts_with("Cold cache.") && both.ends_with(DESKTOP_WARNING));
    }

    #[test]
    fn a_desktop_session_opened_in_the_cli_asks_first_and_a_cli_one_does_not() {
        let mut s = crate::state::AppState::new(
            crate::source::Source::Demo,
            Some(false),
            crate::winfile::WindowFile::default(),
        );
        s.apply_scan(crate::demo::scan(&s.config));
        let running: Vec<String> = s.running.iter().map(|r| r.session_id.clone()).collect();
        let free = |want: App| {
            s.sessions
                .iter()
                .find(|x| {
                    x.last_app == want && x.desktop_id.is_none() == (want == App::Cli) && !running.contains(&x.id)
                })
                .filter(|x| crate::demo::last_account(x) == "main")
                .map(|x| x.id.clone())
        };
        let (desk, cli) = (free(App::Desktop).unwrap(), free(App::Cli).unwrap());
        s.open(Instant::now(), &desk, "main".into(), App::Cli);
        match &s.dialog {
            Some(Dialog::Confirm { text, then: Pending::Open { .. }, .. }) => assert_eq!(text, DESKTOP_WARNING),
            _ => panic!("no desktop warning"),
        }
        s.dialog = None;
        s.open(Instant::now(), &cli, "main".into(), App::Cli);
        assert!(s.dialog.is_none());

        s.open(Instant::now(), &desk, "work".into(), App::Cli);
        match &s.dialog {
            Some(Dialog::Confirm { text, then: Pending::Open { id, account, app: App::Cli }, .. }) => {
                assert_eq!((id, account.as_str()), (&desk, "work"));
                assert!(text.contains("full price") && text.ends_with(DESKTOP_WARNING));
            }
            _ => panic!("expected one confirmation containing both warnings"),
        }
        crate::dialogs::click(&mut s, Instant::now(), crate::dialogs::OK, 0.0);
        assert!(s.dialog.is_none(), "Continue does not open a second confirmation");

        s.open(Instant::now(), &desk, "main".into(), App::Desktop);
        assert!(s.dialog.is_none(), "the Desktop warning only applies when opening in the CLI");
        let i = s.by_id[&desk];
        s.sessions[i].desktop_id = None;
        s.sessions[i].desktop_account = None;
        s.open(Instant::now(), &desk, "main".into(), App::Cli);
        assert!(s.dialog.is_none(), "a Desktop entrypoint without a Desktop record is not a listing");

        // A copy of a running session is not refused.
        s.open_copy(Instant::now(), &running[0]);
        assert!(s.info.is_none());
        // Enter chooses the primary button: Cancel for closing, Continue for opening.
        assert!(!Pending::Quit.ok_is_primary() && !Pending::CloseTab(1).ok_is_primary());
        assert!(Pending::Copy { id: cli.clone(), account: "main".into() }.ok_is_primary());
    }
}
