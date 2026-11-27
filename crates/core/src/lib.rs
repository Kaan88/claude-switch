//! Everything of claude-switch that is not a window: the accounts, the sessions found on disk, which
//! of them are running, what to start to open one, and the links that let two accounts share their
//! settings. Plain data and functions over folders that are passed in, so every one of them is tested
//! against a temporary folder. `docs/DESIGN.md` says what the files look like and why.
//!
//! Deliberately not here: any window or drawing, starting a terminal or the desktop app (the caller
//! runs a [`Launch`]), and networking. Only [`usage`] opens the credentials file, read-only.
//!
//! THE CONTRACT: the public items of this file are fixed. The window is written against them while
//! the bodies are filled in. Add private items and modules freely; do not change a public name, field
//! or signature without saying so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod accounts;
pub mod archive;
pub mod clock;
pub mod compact;
pub mod config;
pub mod delete;
pub mod folder_defaults;
pub mod fulltext;
pub mod grouping;
pub mod launch;
pub mod marks;
pub mod meaning;
mod merge;
pub mod naming;
mod prompt_clean;
pub mod running;
pub mod sessions;
pub mod share;
pub mod stats;
pub mod transcript;
pub mod usage;
mod util;
pub use util::write_atomic as write_app_file;
pub use util::{APP_FILE_LIMIT, read_bounded, replace_if_unchanged};

/// Where the files of this computer are, so that tests can point everything at a temporary folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    /// The user's home folder (`%USERPROFILE%`).
    pub home: PathBuf,
    /// The roaming application data folder (`%APPDATA%`).
    pub appdata: PathBuf,
}

impl Roots {
    /// The real folders of the current user, from the environment. `None` when either is unset.
    pub fn from_env() -> Option<Roots> {
        Some(Roots { home: std::env::var_os("USERPROFILE")?.into(), appdata: std::env::var_os("APPDATA")?.into() })
    }
    /// The first account's CLI folder: `<home>/.claude`.
    pub fn primary_cli_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }
    /// The first account's desktop data folder: `<appdata>/Claude`.
    pub fn primary_desktop_dir(&self) -> PathBuf {
        self.appdata.join("Claude")
    }
    /// Where further accounts live: `<home>/.claude-accounts`.
    pub fn accounts_dir(&self) -> PathBuf {
        self.home.join(".claude-accounts")
    }
    /// This app's own settings file: `<appdata>/claude-switch/config.json`.
    pub fn config_file(&self) -> PathBuf {
        crate::util::resolve_profile(&self.appdata).join("claude-switch").join("config.json")
    }
    /// The bridge's worker list: `<home>/.config/opencode-bridge/workers.json`.
    pub fn workers_file(&self) -> PathBuf {
        crate::util::resolve_profile(&self.home).join(".config").join("opencode-bridge").join("workers.json")
    }
}

/// One Claude login the owner uses, with the folders that hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// A short name of lower-case letters, digits and `-`, unique among the accounts and never
    /// changed afterwards (it names the folder). The first account's is `"main"`.
    pub slug: String,
    /// What the owner calls it. Free text, shown in the window.
    pub name: String,
    /// Its colour in the window, as red, green, blue.
    pub color: [u8; 3],
    /// True for the first account, whose folders are the standard ones and are never linked.
    pub primary: bool,
    /// The CLI's config folder for this account (`CLAUDE_CONFIG_DIR`).
    pub cli_dir: PathBuf,
    /// The desktop app's data folder for this account (`--user-data-dir`).
    pub desktop_dir: PathBuf,
}

/// What the files say about an account's login. Read by [`accounts::status`]; nothing here is a secret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountStatus {
    /// The CLI has a saved login (`.credentials.json` exists in the CLI folder and `oauthAccount` is
    /// in its `.claude.json`).
    pub cli_logged_in: bool,
    /// The desktop app has been run with this account's data folder (the folder holds a
    /// `claude-code-sessions` folder or a `Local State` file).
    pub desktop_used: bool,
    /// `oauthAccount.accountUuid`, when the CLI is logged in.
    pub account_uuid: Option<String>,
    /// `oauthAccount.emailAddress`.
    pub email: Option<String>,
    /// `oauthAccount.displayName`.
    pub display_name: Option<String>,
    /// `oauthAccount.organizationName`.
    pub organization: Option<String>,
    /// `oauthAccount.planDisplayName`.
    pub plan: Option<String>,
}

/// Which program a session was last run by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum App {
    /// The Claude Code CLI in a terminal.
    Cli,
    /// The Code tab of the Claude desktop app.
    Desktop,
}

/// One session found on disk: a transcript, joined with the desktop app's record of it when there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The transcript's id: the `.jsonl` file's stem. What `claude --resume` takes.
    pub id: String,
    /// The best title there is: the custom title, else the generated one, else the desktop record's,
    /// else the first prompt cut to 80 characters, else the id's first eight characters.
    pub title: String,
    /// The last recorded prompt, for display and search.
    pub last_prompt: String,
    /// Bounded transcript-head evidence for local grouping.
    pub grouping: grouping::Transcript,
    /// The last meaningful transcript record appears to end an assistant turn.
    pub turn_ended: bool,
    /// Transcript modification time, independent of Desktop activity.
    pub transcript_ms: u64,
    /// The working folder the session ran in. Empty when no line of the transcript named one.
    pub cwd: PathBuf,
    /// The git branch named by the newest line that has one.
    pub branch: Option<String>,
    /// The time of the last activity, in milliseconds since 1970: the newest of the transcript file's
    /// modification time and the desktop record's `lastActivityAt`.
    pub last_active_ms: u64,
    /// The transcript file's size in bytes: a rough measure of how much a cold cache will cost.
    pub bytes: u64,
    /// Which program ran it last, from the newest line that has an `entrypoint`.
    pub last_app: App,
    /// The desktop record's own id (`local_…`), when a desktop app knows this session.
    pub desktop_id: Option<String>,
    /// The slug of the account whose desktop data folder holds that record.
    pub desktop_account: Option<String>,
    /// The desktop record says it is archived.
    pub archived: bool,
    /// Started by a tool or a subagent, not by the owner: the newest `entrypoint` in the
    /// transcript starts with `sdk` (`sdk-cli`, `sdk-ts`).
    pub worker: bool,
    /// The transcript file.
    pub file: PathBuf,
}

/// A session that is running now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    /// The transcript id it has open.
    pub session_id: String,
    /// The process, which was alive when this was read.
    pub pid: u32,
    /// Which program runs it.
    pub app: App,
    /// The slug of the account whose CLI folder holds the `sessions/<pid>.json` that says so.
    pub account: String,
    /// The folder it runs in.
    pub cwd: PathBuf,
}

/// Something to start. The core builds it; the window decides whether it runs in a terminal tab, an
/// external terminal or as a detached process, by [`Launch::place`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// The program: a full path, or a name to be found on the `PATH` (`claude`).
    pub program: PathBuf,
    /// Its arguments, one per element, not quoted.
    pub args: Vec<String>,
    /// The folder to start it in. `None`: the caller's own.
    pub cwd: Option<PathBuf>,
    /// Variables to set on top of the caller's environment.
    pub env: BTreeMap<String, String>,
    /// Variables to remove from the caller's environment (`CLAUDE_CONFIG_DIR` for the first account).
    pub env_remove: Vec<String>,
    /// Where it belongs.
    pub place: Place,
    /// A line for the window's tab or toast, such as `main · kweb-mail · Resume`.
    pub label: String,
}

/// Where a [`Launch`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// It needs a terminal: a tab in the window, or an external terminal window.
    Terminal,
    /// It is a windowed program or exits at once: start it detached, with no console.
    Detached,
}

/// Why something was not done. Every variant's `Display` is one plain sentence for the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The session is open already; starting it again would lose messages.
    AlreadyRunning(Running),
    /// No account has this slug.
    NoSuchAccount(String),
    /// No session has this id.
    NoSuchSession(String),
    /// The account's CLI is not logged in, so a CLI launch would only ask to log in.
    NotLoggedIn(String),
    /// The desktop app's executable was not found.
    NoDesktopApp,
    /// Installed candidate exists, but it could not be verified for launch.
    UnverifiedDesktopApp,
    /// A file or folder could not be read or written; the text names it and says why.
    Io(String),
    /// A name or value that cannot be used (an empty account name, a slug that exists).
    Invalid(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::AlreadyRunning(r) => write!(
                f,
                "This session is already open in {} (account {}, process {}). Close it there first.",
                match r.app {
                    App::Cli => "a terminal",
                    App::Desktop => "the desktop app",
                },
                r.account,
                r.pid
            ),
            Refusal::NoSuchAccount(s) => write!(f, "There is no account \"{s}\"."),
            Refusal::NoSuchSession(s) => write!(f, "There is no session {s}."),
            Refusal::NotLoggedIn(s) => write!(f, "The account \"{s}\" is not signed in to the CLI yet."),
            Refusal::NoDesktopApp => write!(f, "The Claude desktop app was not found on this computer."),
            Refusal::UnverifiedDesktopApp => {
                write!(f, "The Claude desktop app was found, but its installation could not be verified.")
            }
            Refusal::Io(s) | Refusal::Invalid(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Refusal {}

/// Which account the bridge's Claude Code workers run on (`docs/DESIGN.md`, "Workers").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkersUse {
    /// The account of the session that started the job (the bridge's behaviour without a setting).
    Same,
    /// The other account than that session's; with more than two accounts, the next one in order.
    Other,
    /// Always this account (its slug).
    Account(String),
}

/// This app's own settings, kept in [`Roots::config_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The accounts, the first one first. Never empty: [`config::load`] makes the first account.
    pub accounts: Vec<Account>,
    /// The account new launches use (a slug). Falls back to the first when it names none.
    pub current: String,
    /// Which account the workers use.
    pub workers: WorkersUse,
    /// Open CLI sessions in a tab of the window (true) or in an external terminal window (false).
    pub embedded_terminal: bool,
    /// Start Claude tabs with `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` (the CLI's classic
    /// renderer); the Chat view cannot send while the CLI draws its full-screen view.
    pub classic_terminal_view: bool,
    /// Show archived desktop sessions in the list.
    pub show_archived: bool,
    /// Show the sessions started by tools and subagents ([`Session::worker`]) in the list.
    pub show_workers: bool,
    /// Read and display subscription usage.
    pub show_usage: bool,
    pub notifications: config::Notifications,
    /// Show hidden sessions and folders.
    pub show_hidden: bool,
    /// Use the folder-grouped view instead of Recent.
    pub sessions_mode: grouping::Mode,
    pub group_by_folder: bool,
    pub show_sittings: bool,
    pub suggest_topics: bool,
    /// Terminal colour scheme id; unknown ids are treated as automatic by the window.
    pub terminal_scheme: String,
    /// Terminal font size in logical pixels.
    pub terminal_font_size: u8,
    /// Window theme: "system", "light" or "dark".
    pub window_theme: String,
    /// Optional visual preferences beyond the theme and font size.
    pub appearance: config::AppearancePrefs,
    /// Settings › Chat: quick-command chips and whether Chat answers the CLI's menus.
    pub chat: config::ChatPrefs,
    /// Handover model and per-provider permission to send session data.
    pub compact: compact::Preferences,
    /// Opt-in search, questions and grouping by meaning (off by default).
    pub meaning: meaning::Prefs,
}

/// Convenience for callers: the account with this slug.
pub fn account<'a>(config: &'a Config, slug: &str) -> Result<&'a Account, Refusal> {
    config.accounts.iter().find(|a| a.slug == slug).ok_or_else(|| Refusal::NoSuchAccount(slug.to_string()))
}

/// Milliseconds since 1970 for a file's modification time, 0 when it cannot be read.
pub fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}
