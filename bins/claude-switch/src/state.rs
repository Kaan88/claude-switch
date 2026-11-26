//! The window's whole state in one struct, and the work that changes it without drawing: the
//! scans on worker threads, the filtered and grouped list, the selection and its preview.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use claude_switch_core::sessions::PreviewLine;
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
    AttachPictures(u64, Vec<std::path::PathBuf>),
}

/// The height of a folder heading and of a session row in the list.
pub const HEAD_H: f32 = 30.0;
pub const ROW_H: f32 = 62.0;
/// How often the running sessions are read while the window is shown.
pub const POLL: Duration = Duration::from_secs(5);
/// How many messages the preview shows.
pub const PREVIEW_LINES: usize = 6;

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
    Search,
    SearchClear,
    NewSession,
    Refresh,
    MenuButton,
    /// An entry of the list (heading or row), by its index in [`AppState::sections`].
    Entry(usize),
    Pin(usize),
    ListMode(bool),
    ListFilter(Filter),
    UndoHide,
    ListBlank,
    Splitter,
    OpenCli,
    OpenDesktop,
    OpenAs,
    OpenCopy,
    Info(u32),
    InfoClose,
    /// A control of the open dialog, by the dialog's own number.
    Dialog(u32),
    /// A tab of the strip, by its id ([`crate::tabs::SESSIONS`] for the list), and its close button.
    Tab(u64),
    TabClose(u64),
    /// The terminal of the tab shown.
    Terminal,
    SessionToggle,
}

/// Which popup menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Main,
    OpenAs,
    Row,
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
    /// The core's groups of all sessions, newest first.
    pub all_groups: Vec<claude_switch_core::sessions::Group>,
    pub groups: Vec<ViewGroup>,
    pub row_locations: HashMap<usize, (usize, usize)>,
    pub filter: Filter,
    pub recent_local_day: Option<(i64, i32)>,
    pub marks: claude_switch_core::marks::Marks,
    pub marks_read_only: bool,
    pub marks_flush_at: Option<Instant>,
    pub undo_hide: Option<crate::session_marks::UndoHide>,
    pub naming_job: Option<Job<crate::session_panel::NameResults>>,
    pub sections: Sections,
    pub collapsed: HashSet<String>,
    pub selected: Option<String>,
    pub search: TextField,
    pub search_stops: Vec<f32>,
    pub focus: Focus,
    /// The keyboard moved the focus last: the focus ring shows.
    pub key_used: bool,
    pub accounts_dd: Dropdown<String>,
    pub splitter: Splitter,
    pub preview: Option<(String, Vec<PreviewLine>)>,
    pub preview_job: Option<(String, Job<Vec<PreviewLine>>)>,
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
    pub menu: Option<(MenuKind, Menu<u32>)>,
    pub context_folder: Option<String>,
    pub palette: Option<Palette>,
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
    pub debug_list: bool,
}

impl AppState {
    pub fn new(source: Source, forced_dark: Option<bool>, win_file: WindowFile) -> AppState {
        let config = source.load_config();
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
            config,
            tokens,
            high_contrast,
            appearance,
            term_colours,
            forced_dark,
            sessions: Vec::new(),
            by_id: HashMap::new(),
            running: Vec::new(),
            statuses: HashMap::new(),
            all_groups: Vec::new(),
            groups: Vec::new(),
            row_locations: HashMap::new(),
            filter: Filter::All,
            recent_local_day: None,
            marks: marks.unwrap_or_default(),
            marks_read_only: marks_error.is_some(),
            marks_flush_at: None,
            undo_hide: None,
            naming_job: None,
            sections: Sections::new(400.0, ROW_H),
            collapsed: HashSet::new(),
            selected: None,
            search: TextField::new(),
            search_stops: Vec::new(),
            focus: Focus::List,
            key_used: false,
            accounts_dd: Dropdown::new(Vec::<(String, String)>::new(), 0),
            splitter,
            preview: None,
            preview_job: None,
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
            menu: None,
            context_folder: None,
            palette: None,
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
            cell_font: CellFont::new(),
            metrics: None,
            term_area: Rect::default(),
            grid: crate::term::FIRST_SIZE,
            window_focused: true,
            quit: false,
            debug_shell: None,
            debug_type: Vec::new(),
            debug_list: false,
        };
        if let Some(error) = marks_error {
            s.error("Session marks are read-only", error);
        }
        s.rebuild_accounts();
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
        self.scan_job = Job::start(self.wake_fn(), move |_, _| source.scan(&config)).ok();
    }

    /// Starts the cheap scan of running sessions, unless a scan runs.
    pub fn poll_running(&mut self) {
        if self.scan_job.is_some() || self.running_job.is_some() {
            return;
        }
        let (source, config) = (self.source.clone(), self.config.clone());
        let files = self.sessions.iter().map(|s| (s.id.clone(), (s.file.clone(), s.transcript_ms))).collect();
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
        if let Some((id, job)) = &self.preview_job
            && !job.running()
        {
            let lines = job.take().unwrap_or_default();
            if self.selected.as_ref() == Some(id) {
                self.preview = Some((id.clone(), lines));
            }
            self.preview_job = None;
            changed = true;
        }
        self.take_naming_job() | changed
    }

    pub fn apply_scan(&mut self, scan: Scan) {
        self.sessions = scan.sessions;
        self.running = scan.running;
        self.index_running();
        self.statuses = scan.statuses.into_iter().collect();
        self.by_id = self.sessions.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
        self.all_groups = claude_switch_core::sessions::group_by_folder(&self.sessions);
        if !self.scanned && self.source.is_demo() {
            crate::demo::seed_marks(&mut self.marks, &self.sessions);
        }
        self.prune_marks();
        self.scanned = true;
        self.rebuild_accounts();
        self.rebuild_view();
        let keep = self.selected.clone();
        self.last_account = None;
        match keep {
            Some(id) if self.by_id.contains_key(&id) => {
                self.see_shown();
                self.load_details(&id, true);
            }
            _ => self.select_first(),
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
                self.preview = None;
            }
        }
    }

    /// Selects a session and starts reading its preview.
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        self.load_details(id, false);
        self.see_shown();
    }

    pub(crate) fn load_details(&mut self, id: &str, again: bool) {
        let Some(&i) = self.by_id.get(id) else { return };
        let session = self.sessions[i].clone();
        self.last_account = Some((id.to_string(), self.source.last_account(&self.config, &session)));
        if !again && self.preview.as_ref().is_some_and(|(p, _)| p == id) {
            return;
        }
        if !again {
            self.preview = None;
        }
        if let Some((_, job)) = &self.preview_job {
            job.cancel();
        }
        let source = self.source.clone();
        let job = Job::start(self.wake_fn(), move |_, _| source.preview(&session, PREVIEW_LINES)).ok();
        self.preview_job = job.map(|j| (id.to_string(), j));
    }

    /// Brings the selected row into view.
    pub fn reveal_selected(&mut self) {
        if let Some(e) = self.selected.clone().and_then(|id| self.entry_of(&id)) {
            self.sections.scroll = self.sections.reveal(e);
        }
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

    pub fn warn(&mut self, title: &str, text: String) {
        self.info = Some(InfoBar::new(Severity::Warning, title, text).closable(true));
    }

    /// The look again, from the system (or the forced choice).
    pub fn retheme(&mut self) {
        self.high_contrast = mdshared_ui::platform::system_high_contrast();
        (self.tokens, self.appearance, self.term_colours) =
            appearance(&self.config, self.forced_dark, self.high_contrast);
    }

    pub fn save_config(&mut self) {
        if let Err(e) = self.source.save_config(&self.config) {
            self.error("The settings were not saved", e.to_string());
        }
    }
}

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
        let mut tokens = Tokens::new(dark());
        tokens.palette = look.palette(&tokens.palette, platform::system_accent(), None);
        tokens
    };
    let terminal =
        crate::termtheme::terminal(&config.terminal_scheme, &tokens.palette, &config.appearance, high_contrast);
    (tokens, look, terminal)
}

#[cfg(test)]
mod tests;
