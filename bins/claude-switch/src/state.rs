//! The window's whole state in one struct, and the work that changes it without drawing: the
//! scans on worker threads, the filtered and grouped list, the selection and its preview.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use claude_switch_core::{AccountStatus, Config, Running, Session};
use mdshared_app::job::Job;
use mdshared_ui::anim::Stack;
use mdshared_ui::appearance::Appearance;
use mdshared_ui::chrome::WinButton;
use mdshared_ui::commands::Keymap;
use mdshared_ui::dropdown::Dropdown;
use mdshared_ui::gfx::{Rect, Rgb};
use mdshared_ui::host::Proxy;
use mdshared_ui::infobar::{InfoBar, Severity};
use mdshared_ui::menu::Menu;
use mdshared_ui::palette::Palette;
use mdshared_ui::sections::{Entry, Sections};
use mdshared_ui::splitter::{Hold, Splitter};
use mdshared_ui::termview::{CellFont, Metrics, Scheme};
use mdshared_ui::textfield::TextField;
use mdshared_ui::tokens::Tokens;
use mdshared_ui::tooltip::Tooltip;

use crate::dialogs::Dialog;
use crate::list::model::Filter;
use crate::source::{Scan, Source};
use crate::tabs::Tabs;
use crate::winfile::WindowFile;

/// A worker woke the loop: the jobs are looked at (`Woke`), or a terminal's program wrote or
/// ended (`Term`).
pub enum Msg {
    Woke,
    Term,
    SessionView,
    AttachPictures(u64, Vec<std::path::PathBuf>, bool),
    /// The folder chosen in the New session dialog's folder picker.
    FolderPicked(std::path::PathBuf),
    /// The folder chosen to continue a session in: its id and the folder.
    SessionFolderPicked(String, std::path::PathBuf),
    ArchiveReportSaved(crate::archive_report::ReportResult),
    ThemeFile(Result<Option<Box<claude_switch_core::config::theme::NamedTheme>>, String>),
    Tray(crate::tray::Event),
    /// Another process of this user asked to open a handover tab (`handover_pipe`).
    /// The ticket says whether the pipe thread gave up on it first (then it is not acted on).
    Handover(
        Box<crate::compact_cli::Loaded>,
        std::sync::mpsc::Sender<Result<crate::handover_pipe::Outcome, String>>,
        crate::handover_pipe::Ticket,
    ),
    /// A handover from another process failed (a background `/switch-compact`'s reason, or a
    /// request the pipe thread could not load), or opened with a note. Shown as a notice only.
    HandoverNotice(crate::compact_cli::Notice),
    /// A branch's transcript copy was written (or not) off the window's thread.
    BranchWritten(Box<crate::actions::BranchJob>, Result<(), String>),
}

/// The height of a folder heading and of a session row in the list.
pub const HEAD_H: f32 = 30.0;
pub const ROW_H: f32 = 62.0;
/// How often the running sessions are read while the window is shown.
pub const POLL: Duration = Duration::from_secs(5);

/// Which part has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Search,
    Account,
}

/// Something the pointer can be on, recorded while drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Title,
    Caption(WinButton),
    Account,
    UsageLines,
    UsagePopup,
    UsageRefresh,
    UsageOpenAs,
    Search,
    SearchClear,
    SearchConversation,
    NewSession,
    Refresh,
    MenuButton,
    /// An entry of the list (heading or row), by its index in [`AppState::sections`].
    Entry(usize),
    Pin(usize),
    ListMode(claude_switch_core::grouping::Mode),
    ListGrouping,
    ThreadToggle(usize),
    Related(usize),
    ListFilter(Filter),
    ArchiveMenu,
    ArchivePick(usize),
    UndoHide,
    ListBlank,
    Splitter,
    OpenDefault,
    FolderOpen,
    OpenAs,
    OpenCopy,
    Handovers,
    Info(u32),
    InfoClose,
    /// A button of a background handover in the Sessions strip or the details pane, by its id.
    HandoverJob(u64, crate::handover_jobs::Act),
    /// A control of the open dialog, by the dialog's own number.
    Dialog(u32),
    /// A tab of the strip, by its id ([`crate::tabs::SESSIONS`] for the list), and its close button.
    Tab(u64),
    /// A job in the Workers tab's list, by its place, and the selected job's Cancel.
    Worker(usize),
    WorkerCancel,
    TabClose(u64),
    /// The terminal of the tab shown.
    Terminal,
    SessionToggle,
    ChatMode(bool),
    Chat(crate::view::chat::Control),
    RestoreTabs,
    DismissTabs,
    /// The Settings dialog's Peek button, for its tooltip (Settings hits it itself).
    SettingsPeek,
}

/// Which popup menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuKind {
    Main,
    OpenAs,
    Row,
    Tab(u64),
    Chat(u64, crate::view::chat::MenuMode),
    /// Cut, Copy, Paste, Select all on a text field.
    Field(crate::fieldmouse::FieldId),
    /// Copy, Select all, Copy message, Copy as Markdown on the conversation of a tab.
    Conversation(u64),
    /// Copy and Save as… from the Review handover dialog's “More ▾”.
    HandoverMore,
}

/// A folder of the list as it is shown: the rows left after the filter.
#[derive(Clone, Debug)]
pub struct ViewGroup {
    pub name: String,
    /// The folder, lower case, as the key of its collapsed state.
    pub key: String,
    pub rows: Vec<usize>,
}

pub struct AppState {
    pub source: Source,
    pub config: Config,
    config_baseline: Config,
    pub config_damaged: bool,
    pub desktop_job: Option<mdshared_app::job::Job<Result<String, String>>>,
    pub desktop_folder_open: Option<(std::path::PathBuf, String, claude_switch_core::App)>,
    pub tokens: Tokens,
    pub high_contrast: bool,
    pub appearance: Appearance,
    pub term_colours: (Scheme, Rgb),
    /// `--theme light|dark`: the system's choice is not followed.
    pub forced_dark: Option<bool>,
    pub sessions: Vec<Session>,
    pub by_id: HashMap<String, usize>,
    pub running: Vec<Running>,
    pub statuses: HashMap<String, AccountStatus>,
    pub usage: crate::usage::Usage,
    pub usage_warnings: crate::usage_warnings::Warnings,
    pub notifications: crate::attention::Notifications,
    /// The core's groups of all sessions, newest first.
    pub all_groups: Vec<claude_switch_core::sessions::Group>,
    pub groups: Vec<ViewGroup>,
    pub grouping: crate::list::grouping::Grouping,
    pub details_scroll: crate::details::Scroll,
    pub row_locations: HashMap<usize, (usize, usize)>,
    pub filter: Filter,
    pub recent_local_day: Option<(i64, i32)>,
    pub marks: claude_switch_core::marks::Marks,
    pub marks_read_only: bool,
    /// What the New session list knows about the sessions' folders.
    pub folders: crate::recent_folders::Disk,
    pub marks_flush_at: Option<Instant>,
    /// Saves left to retry for marks no one will set again (a handover's links); see `save_marks`.
    pub marks_retries: u8,
    pub undo_hide: Option<crate::session_marks::UndoHide>,
    pub archive_picks: HashSet<String>,
    pub archive_count: usize,
    /// Where deleted sessions go: the Recycle Bin (in tests, a folder of the test's own).
    pub recycle: std::sync::Arc<dyn claude_switch_core::delete::Bin>,
    /// Copies whose original is deleted once the copy's transcript is written (`delete`).
    pub copy_deletes: Vec<crate::delete::AfterCopy>,
    pub naming_job: Option<Job<crate::session_panel::NameResults>>,
    pub meaning: crate::meaning::Meaning,
    pub sections: Sections,
    pub collapsed: HashSet<String>,
    pub selected: Option<String>,
    pub search: TextField,
    pub conversation_search: crate::conversation_search::Search,
    pub session_size: crate::session_size::Size,
    pub search_hit: Option<mdshared_ui::textfield::FieldHit>,
    /// Mouse selection in whichever single-line field the pointer pressed.
    pub field_mouse: crate::fieldmouse::FieldMouse,
    pub focus: Focus,
    /// The keyboard moved the focus last: the focus ring shows.
    pub key_used: bool,
    pub accounts_dd: Dropdown<String>,
    pub splitter: Splitter,
    /// The selected session's conversation in the details, read-only.
    pub details_reader: Option<crate::view::reader::Reader>,
    /// When the shown details conversation and the Workers tab are read again.
    pub feed_poll: Option<Instant>,
    /// The OpenCode bridge's jobs (the Workers tab).
    pub workers: crate::workers_tab::WorkersTab,
    pub last_account: Option<(String, String)>,
    pub scan_job: Option<Job<Scan>>,
    pub running_job: Option<Job<crate::source::ActivityScan>>,
    pub running_ids: HashSet<String>,
    pub scanned: bool,
    pub last_scan: Option<Instant>,
    pub next_poll: Option<Instant>,
    pub toasts: Stack<String>,
    pub info: Option<InfoBar>,
    pub dialog: Option<Dialog>,
    pub settings_category: Option<&'static str>,
    pub confirm_scroll: f32,
    pub compact: crate::compact_dialog::Cache,
    pub opencode: crate::opencode_workers::Workers,
    pub menu: Option<(MenuKind, Menu<u32>)>,
    pub context_folder: Option<String>,
    pub palette: Option<Palette>,
    pub palette_sessions: HashMap<u32, String>,
    pub keymap: Keymap,
    pub tip: Tooltip<Target>,
    pub hits: Vec<(Rect, Target)>,
    pub hover: Option<Target>,
    pub pressed: Option<Target>,
    pub mouse: (f32, f32),
    pub blink: Option<Duration>,
    pub win_file: WindowFile,
    pub proxy: Option<Arc<Mutex<Proxy<Msg>>>>,
    pub list_area: Rect,
    pub main_top: f32,
    pub list_scroll_track: Option<Rect>,
    /// The terminal tabs, and what the last frame laid out for them.
    pub tabs: Tabs,
    pub restore: crate::resume_tabs::Restore,
    pub session_view: crate::view::Window,
    pub cell_font: CellFont,
    pub metrics: Option<Metrics>,
    pub term_area: Rect,
    /// The grid that fits the terminal area, for a tab opened next.
    pub grid: (u16, u16),
    pub window_focused: bool,
    /// The window is to close (asked and confirmed).
    pub quit: bool,
    /// `--shell` and `--type`: a program to open in a tab at start, and text for it at times.
    pub debug_shell: Option<String>,
    pub debug_type: Vec<(Instant, Vec<u8>)>,
    pub debug_tab: Option<u64>,
    pub debug_list: bool,
}

impl AppState {
    pub fn new(source: Source, forced_dark: Option<bool>, win_file: WindowFile) -> AppState {
        let (config, config_damaged) = source.load_config();
        // Changes made in one account since the app last ran (a /model there) reach the others.
        let _ = source.settings_sync(&config);
        let usage = crate::usage::Usage::new(&source);
        let usage_warnings = crate::usage_warnings::Warnings::new(&source);
        let marks = match &source {
            Source::Real(roots) => claude_switch_core::marks::Marks::load(roots),
            Source::Demo => Ok(Default::default()),
        };
        let marks_error = marks.as_ref().err().map(ToString::to_string);
        let high_contrast = mdshared_ui::platform::system_high_contrast();
        let (tokens, appearance, term_colours) = appearance(&config, forced_dark, high_contrast);
        let mut splitter = Splitter::new(Hold::A(440.0)).min(300.0, 340.0);
        if !win_file.split.is_empty() {
            splitter.apply_setting(&win_file.split);
        }
        let mut s = AppState {
            source,
            config_baseline: config.clone(),
            config,
            config_damaged,
            desktop_job: None,
            desktop_folder_open: None,
            tokens,
            high_contrast,
            appearance,
            term_colours,
            forced_dark,
            sessions: Vec::new(),
            by_id: HashMap::new(),
            running: Vec::new(),
            statuses: HashMap::new(),
            usage,
            usage_warnings,
            notifications: Default::default(),
            all_groups: Vec::new(),
            groups: Vec::new(),
            grouping: Default::default(),
            details_scroll: Default::default(),
            row_locations: HashMap::new(),
            filter: Filter::All,
            recent_local_day: None,
            marks: marks.unwrap_or_default(),
            marks_read_only: marks_error.is_some(),
            folders: Default::default(),
            marks_flush_at: None,
            marks_retries: 0,
            undo_hide: None,
            archive_picks: HashSet::new(),
            archive_count: 0,
            recycle: crate::delete::default_bin(),
            copy_deletes: Vec::new(),
            naming_job: None,
            meaning: Default::default(),
            sections: Sections::new(400.0, ROW_H),
            collapsed: HashSet::new(),
            selected: None,
            search: TextField::new(),
            conversation_search: Default::default(),
            session_size: Default::default(),
            search_hit: None,
            field_mouse: Default::default(),
            focus: Focus::List,
            key_used: false,
            accounts_dd: Dropdown::new(Vec::<(String, String)>::new(), 0),
            splitter,
            details_reader: None,
            feed_poll: None,
            workers: Default::default(),
            last_account: None,
            scan_job: None,
            running_job: None,
            running_ids: HashSet::new(),
            scanned: false,
            last_scan: None,
            next_poll: None,
            toasts: Stack::new(),
            info: None,
            dialog: None,
            settings_category: None,
            confirm_scroll: 0.0,
            compact: Default::default(),
            opencode: Default::default(),
            menu: None,
            context_folder: None,
            palette: None,
            palette_sessions: HashMap::new(),
            keymap: Keymap::new(crate::actions::COMMANDS),
            tip: Tooltip::new(),
            hits: Vec::new(),
            hover: None,
            pressed: None,
            mouse: (-1.0, -1.0),
            blink: mdshared_ui::platform::caret_blink_time(),
            session_view: crate::view::Window::new(&win_file),
            win_file,
            proxy: None,
            list_area: Rect::default(),
            main_top: 0.0,
            list_scroll_track: None,
            tabs: Tabs::default(),
            restore: Default::default(),
            cell_font: CellFont::new(),
            metrics: None,
            term_area: Rect::default(),
            grid: crate::term::FIRST_SIZE,
            window_focused: true,
            quit: false,
            debug_shell: None,
            debug_type: Vec::new(),
            debug_tab: None,
            debug_list: false,
        };
        if let Some(error) = marks_error {
            s.error("Session marks are read-only", error);
        }
        s.rebuild_accounts();
        s.load_meaning();
        s
    }

    pub fn pal(&self) -> &mdshared_ui::theme::Palette {
        &self.tokens.palette
    }

    /// The account new launches use.
    pub fn current(&self) -> &claude_switch_core::Account {
        claude_switch_core::account(&self.config, &self.config.current).unwrap_or(&self.config.accounts[0])
    }

    pub fn account_name(&self, slug: &str) -> String {
        claude_switch_core::account(&self.config, slug).map_or_else(|_| slug.to_string(), |a| a.name.clone())
    }

    pub fn account_color(&self, slug: &str) -> [u8; 3] {
        claude_switch_core::account(&self.config, slug).map_or(self.pal().muted, |a| a.color)
    }

    /// The dropdown's options again, from the accounts and their login states.
    pub fn rebuild_accounts(&mut self) {
        let options: Vec<(String, String)> = self
            .config
            .accounts
            .iter()
            .map(|a| {
                let signed = self.statuses.get(&a.slug).map(|s| s.cli_logged_in);
                let label = match signed {
                    Some(true) => format!("{} \u{2013} signed in", a.name),
                    Some(false) => format!("{} \u{2013} not signed in", a.name),
                    None => a.name.clone(),
                };
                (a.slug.clone(), label)
            })
            .collect();
        let at = self.config.accounts.iter().position(|a| a.slug == self.config.current).unwrap_or(0);
        self.accounts_dd = Dropdown::new(options, at);
    }

    pub fn wake_fn(&self) -> impl Fn() + Send + Sync + 'static {
        let proxy = self.proxy.clone();
        move || {
            if let Some(p) = &proxy
                && let Ok(p) = p.lock()
            {
                p.send(Msg::Woke);
            }
        }
    }

    /// Starts a full scan on a worker, unless one runs.
    pub fn refresh(&mut self, now: Instant) {
        if self.scan_job.is_some() {
            return;
        }
        let (source, config) = (self.source.clone(), self.config.clone());
        self.last_scan = Some(now);
        let cache = self.grouping.scan_cache.clone();
        self.scan_job = Job::start(self.wake_fn(), move |_, _| {
            source.scan(&config, &mut cache.lock().unwrap_or_else(|e| e.into_inner()))
        })
        .ok();
    }

    /// Starts the cheap scan of running sessions, unless a scan runs.
    pub fn poll_running(&mut self) {
        if self.scan_job.is_some() || self.running_job.is_some() {
            return;
        }
        let (source, config) = (self.source.clone(), self.config.clone());
        let stamps = &self.grouping.transcript_stamps;
        let files =
            self.sessions.iter().filter_map(|s| Some((s.id.clone(), (s.file.clone(), *stamps.get(&s.id)?)))).collect();
        let watch = self
            .running
            .iter()
            .map(|r| r.session_id.clone())
            .chain(self.tabs.list.iter().filter_map(|tab| tab.session.clone()))
            .chain(self.selected.clone())
            .collect();
        self.running_job = Job::start(self.wake_fn(), move |_, _| source.activity(&config, files, watch)).ok();
    }

    /// Takes what finished jobs give. True when something changed.
    pub fn take_jobs(&mut self) -> bool {
        let mut changed = false;
        if let Some(job) = &self.scan_job
            && !job.running()
        {
            let scan = job.take().unwrap_or_default();
            self.scan_job = None;
            self.apply_scan(scan);
            changed = true;
        }
        if let Some(job) = &self.running_job
            && !job.running()
        {
            let r = job.take();
            self.running_job = None;
            if let Some(r) = r {
                changed |= self.apply_activity(r);
            }
        }
        if let Some(reader) = &mut self.details_reader {
            changed |= reader.collect();
        }
        changed |= self.take_conversation_search();
        changed |= self.take_related();
        changed |= self.take_folder_check();
        let selected = self.selected_session().map(|s| s.file.clone());
        self.take_naming_job() | self.take_meaning_job() | self.session_size.take(selected.as_deref()) | changed
    }

    pub fn apply_scan(&mut self, scan: Scan) {
        let search_changed = self.conversation_search.on
            && (scan.sessions.len() != self.sessions.len()
                || scan.sessions.iter().any(|s| {
                    self.by_id.get(&s.id).is_none_or(|&i| {
                        let old = &self.sessions[i];
                        (s.bytes, s.transcript_ms, &s.file) != (old.bytes, old.transcript_ms, &old.file)
                    })
                }));
        self.sessions = scan.sessions;
        // A session continued in another folder works there now (`Marks::folder`).
        for s in &mut self.sessions {
            if let Some(folder) = self.marks.folder(&s.id) {
                s.cwd = folder.into();
            }
        }
        self.grouping.projects = scan.projects;
        self.grouping.transcript_stamps = scan.transcript_stamps;
        self.cancel_related();
        self.grouping.related_index = scan.related;
        self.running = scan.running;
        self.index_running();
        self.statuses = scan.statuses.into_iter().collect();
        self.by_id = self.sessions.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
        self.all_groups = claude_switch_core::sessions::group_by_folder(&self.sessions);
        if !self.scanned && self.source.is_demo() {
            crate::demo::seed_marks(&mut self.marks, &self.sessions);
        }
        self.prune_marks();
        self.rebuild_threads();
        self.scanned = true;
        self.rebuild_accounts();
        if search_changed {
            self.restart_conversation_search(Instant::now());
        }
        self.rebuild_view();
        self.check_folders();
        self.reload_new_session();
        let keep = self.selected.clone();
        self.last_account = None;
        match keep {
            Some(id)
                if self.by_id.contains_key(&id)
                    && !search_changed
                    && (self.marks.archived_at(&id).is_none() || self.entry_of(&id).is_some()) =>
            {
                self.see_shown();
                self.load_details(&id, true);
            }
            _ => {
                self.select_first();
                if self.grouping.related_id.is_none()
                    && let Some(&i) = self.selected.as_deref().and_then(|id| self.by_id.get(id))
                {
                    self.load_related(i);
                }
            }
        }
    }

    /// The session of a list entry, when it is a row.
    pub fn session_at(&self, entry: usize) -> Option<usize> {
        match self.sections.entry(entry)? {
            Entry::Row { group, row } => self.groups.get(group)?.rows.get(row).copied(),
            Entry::Heading(_) => None,
        }
    }

    /// The list entry that shows a session.
    pub fn entry_of(&self, id: &str) -> Option<usize> {
        let i = *self.by_id.get(id)?;
        let &(group, row) = self.row_locations.get(&i)?;
        self.sections.index(Entry::Row { group, row })
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.selected.as_ref().and_then(|id| self.by_id.get(id)).map(|&i| &self.sessions[i])
    }

    pub fn select_first(&mut self) {
        match self.sections.nav(None, mdshared_ui::rows::Nav::Home).and_then(|e| self.session_at(e)) {
            Some(i) => {
                let id = self.sessions[i].id.clone();
                self.select(&id);
            }
            None => {
                self.selected = None;
                self.details_reader = None;
                self.session_size.cancel();
                self.cancel_related();
            }
        }
    }

    /// Selects a session and starts reading its preview.
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        self.details_scroll.offset = 0.0;
        self.load_details(id, false);
        self.see_shown();
    }

    pub(crate) fn load_details(&mut self, id: &str, again: bool) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        self.load_related(i);
        self.load_handover_history(id);
        let wake = self.wake_fn();
        self.session_size.start(&session, self.source.is_demo(), wake);
        self.last_account = self.source.last_account(&self.config, &session).map(|account| (id.to_string(), account));
        if let Some(reader) = self.details_reader.as_ref().filter(|r| r.id == id && r.file == session.file) {
            // The same session again (a rescan): the reader goes on from where it read.
            if again {
                reader.poll();
            }
            return;
        }
        let wake = self.wake_fn();
        self.details_reader = if self.source.is_demo() {
            crate::view::reader::Reader::demo(id, &session.file, &session.title, wake).ok()
        } else {
            Some(crate::view::reader::Reader::new(id, session.file.clone(), session.cwd.clone(), wake))
        };
    }

    pub fn running_of(&self, id: &str) -> Option<&Running> {
        if !self.running_ids.contains(id) {
            return None;
        }
        claude_switch_core::running::find(&self.running, id)
    }

    pub fn toast(&mut self, text: String, now: Instant) {
        self.toasts.push(text, Some(now + Duration::from_secs(4)));
    }

    pub fn error(&mut self, title: &str, text: String) {
        self.info = Some(InfoBar::new(Severity::Error, title, text).closable(true));
    }

    pub fn note(&mut self, title: &str, text: String) {
        self.info = Some(InfoBar::new(Severity::Info, title, text).closable(true));
    }

    pub fn warn(&mut self, title: &str, text: String) {
        self.info = Some(InfoBar::new(Severity::Warning, title, text).closable(true));
    }

    /// The look again, from the system (or the forced choice).
    pub fn retheme(&mut self) {
        self.high_contrast = mdshared_ui::platform::system_high_contrast();
        (self.tokens, self.appearance, self.term_colours) =
            appearance(&self.config, self.forced_dark, self.high_contrast);
    }
}

mod recovery;

fn appearance(config: &Config, forced_dark: Option<bool>, high_contrast: bool) -> (Tokens, Appearance, (Scheme, Rgb)) {
    use mdshared_ui::platform;
    let look = crate::termtheme::appearance(&config.appearance);
    let dark = || {
        forced_dark
            .or_else(|| crate::termtheme::window_dark(&config.window_theme))
            .unwrap_or_else(platform::system_is_dark)
    };
    let tokens = if high_contrast {
        Tokens::high_contrast_with(platform::system_contrast_colors().as_ref(), platform::system_is_dark())
    } else {
        let dark = dark();
        let a = &config.appearance;
        let base = crate::termtheme::palettes::get(dark, if dark { &a.dark_theme } else { &a.light_theme });
        let mut tokens = Tokens::new(dark);
        let system = platform::system_accent();
        tokens.palette = match crate::termtheme::named::selected(a, dark) {
            Some(t) => crate::termtheme::named::palette(t, a, system),
            None => crate::windowlook::palette(&base.palette, &look, system),
        };
        tokens
    };
    let terminal =
        crate::termtheme::terminal(&config.terminal_scheme, &tokens.palette, &config.appearance, high_contrast);
    (tokens, look, terminal)
}

#[cfg(test)]
mod tests;
