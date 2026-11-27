//! Where the window's data comes from: the core over the real folders of this computer, or the
//! made-up data of `--demo` ([`crate::demo`]). Every call the window makes into the core goes
//! through here, so the demo doesn't touch personal files. Its session-view fixture uses only temp.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use claude_switch_core::grouping::{Project, projects_for_scan};
use claude_switch_core::sessions::{self, PreviewLine, ScanCache, Stamp};
use claude_switch_core::share::{self, ShareState};
use claude_switch_core::{
    Account, AccountStatus, App, Config, Launch, Refusal, Roots, Running, Session, accounts, config, launch, running,
};

use crate::demo;
use crate::list::grouping::RelatedIndex;

/// The real computer, or the demo.
#[derive(Clone, Debug)]
pub enum Source {
    Real(Roots),
    Demo,
}

/// What one full scan finds.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub sessions: Vec<Session>,
    pub running: Vec<Running>,
    pub statuses: Vec<(String, AccountStatus)>,
    pub projects: HashMap<String, Project>,
    pub transcript_stamps: HashMap<String, Stamp>,
    pub related: Arc<RelatedIndex>,
}

impl Scan {
    pub fn with_related(mut self) -> Self {
        self.related = Arc::new(RelatedIndex::new(&self.sessions, &self.projects));
        self
    }
}

pub struct ActivityScan {
    pub running: Vec<Running>,
    pub updates: Vec<ActivityUpdate>,
}

pub struct ActivityUpdate {
    pub id: String,
    pub file: PathBuf,
    pub previous: Stamp,
    pub stamp: Stamp,
    pub prompt: String,
    pub ended: bool,
}

impl Source {
    pub fn is_demo(&self) -> bool {
        matches!(self, Source::Demo)
    }

    pub fn load_config(&self) -> (Config, bool) {
        match self {
            Source::Real(roots) => config::load_checked(roots),
            Source::Demo => (demo::config(), false),
        }
    }

    pub fn save_config(&self, baseline: &Config, c: &Config) -> Result<Config, Refusal> {
        match self {
            Source::Real(roots) => config::save_changes(roots, baseline, c),
            Source::Demo => Ok(c.clone()),
        }
    }

    /// Sessions, running sessions and the accounts' login states. Runs on a worker thread.
    pub fn scan(&self, c: &Config, cache: &mut ScanCache) -> Scan {
        match self {
            Source::Real(roots) => {
                let sessions = cache.scan(roots, c);
                let projects = projects_for_scan(sessions.iter().map(|s| s.cwd.as_path()));
                let transcript_stamps =
                    sessions.iter().filter_map(|s| Some((s.id.clone(), cache.transcript_stamp(&s.file)?))).collect();
                Scan {
                    sessions,
                    projects,
                    transcript_stamps,
                    running: running::scan(roots, c),
                    statuses: c.accounts.iter().map(|a| (a.slug.clone(), accounts::status(roots, a))).collect(),
                    ..Scan::default()
                }
                .with_related()
            }
            Source::Demo => demo::scan(c),
        }
    }

    pub fn running(&self, c: &Config) -> Vec<Running> {
        match self {
            Source::Real(roots) => running::scan(roots, c),
            Source::Demo => demo::running(),
        }
    }

    pub fn activity(
        &self,
        c: &Config,
        mut files: HashMap<String, (PathBuf, Stamp)>,
        mut watch: HashSet<String>,
    ) -> ActivityScan {
        let running = self.running(c);
        let mut updates = Vec::new();
        if !self.is_demo() {
            watch.extend(running.iter().map(|r| r.session_id.clone()));
            for id in watch {
                let Some((file, previous)) = files.remove(&id) else { continue };
                if let Some((stamp, prompt, ended)) = sessions::changed_row_activity(&file, previous) {
                    updates.push(ActivityUpdate { id, file, previous, stamp, prompt, ended });
                }
            }
        }
        ActivityScan { running, updates }
    }

    /// The last few messages. Runs on a worker thread.
    pub fn preview(&self, s: &Session, max: usize) -> Vec<PreviewLine> {
        match self {
            Source::Real(_) => sessions::preview(&s.file, max),
            Source::Demo => demo::preview(s, max),
        }
    }

    pub fn last_account(&self, c: &Config, s: &Session) -> Option<String> {
        match self {
            Source::Real(roots) => launch::last_account(roots, c, s),
            Source::Demo => Some(demo::last_account(s)),
        }
    }

    pub fn carry_warning(&self, c: &Config, s: &Session, account: &str) -> Option<String> {
        match self {
            Source::Real(roots) => launch::carry_warning(roots, c, s, account),
            Source::Demo => demo::carry_warning(s, account),
        }
    }

    pub fn resume(&self, c: &Config, s: &Session, account: &str, app: App, run: &[Running]) -> Result<Launch, Refusal> {
        match self {
            Source::Real(roots) => launch::resume(roots, c, s, account, app, run),
            Source::Demo => demo::resume(s, account, app, run),
        }
    }

    pub fn fork(&self, c: &Config, s: &Session, account: &str, new_id: &str) -> Result<Launch, Refusal> {
        match self {
            Source::Real(roots) => launch::fork(roots, c, s, account, new_id),
            Source::Demo => Ok(demo::fork(s, account)),
        }
    }

    pub fn new_session(&self, c: &Config, account: &str, app: App, cwd: &std::path::Path) -> Result<Launch, Refusal> {
        match self {
            Source::Real(roots) => launch::new_session(roots, c, account, app, cwd),
            Source::Demo => Ok(demo::launch(account, "New session")),
        }
    }

    pub fn login(&self, c: &Config, account: &str) -> Result<Launch, Refusal> {
        match self {
            Source::Real(roots) => launch::login(roots, c, account),
            Source::Demo => Ok(demo::launch(account, "Sign in")),
        }
    }

    pub fn open_desktop(&self, c: &Config, account: &str) -> Result<Launch, Refusal> {
        match self {
            Source::Real(_) => launch::open_desktop(c, account),
            Source::Demo => Ok(demo::launch(account, "Desktop")),
        }
    }

    pub fn reserve(
        &self,
        config: &Config,
        session_id: &str,
        account: &str,
    ) -> Result<Option<launch::Reservation>, Refusal> {
        match self {
            Source::Real(roots) => launch::reserve(roots, config, session_id, account).map(Some),
            Source::Demo => Ok(None),
        }
    }

    pub fn share_status(&self, a: &Account) -> Vec<ShareState> {
        match self {
            Source::Real(roots) => share::status(roots, a),
            Source::Demo => demo::share_status(a),
        }
    }

    /// Brings the sharing accounts' Claude Code settings in line (`settings_sync`).
    pub fn settings_sync(&self, c: &Config) -> Result<claude_switch_core::settings_sync::Report, Refusal> {
        match self {
            Source::Real(roots) => claude_switch_core::settings_sync::sync(roots, c, &c.settings_share),
            Source::Demo => Ok(Default::default()),
        }
    }

    /// A Claude Code setting as the master account has it (`settings_sync::get`).
    pub fn claude_setting(
        &self,
        c: &Config,
        s: claude_switch_core::settings_sync::Setting,
    ) -> Option<serde_json::Value> {
        match self {
            Source::Real(roots) => claude_switch_core::settings_sync::get(roots, c, s),
            Source::Demo => None,
        }
    }

    /// Saves a Claude Code setting in the master account and shares it (`settings_sync::set`).
    pub fn set_claude_setting(
        &self,
        c: &Config,
        s: claude_switch_core::settings_sync::Setting,
        value: Option<serde_json::Value>,
    ) -> Result<claude_switch_core::settings_sync::Report, Refusal> {
        match self {
            Source::Real(roots) => claude_switch_core::settings_sync::set(roots, c, s, value),
            Source::Demo => Ok(Default::default()),
        }
    }

    pub fn share_apply(&self, a: &Account) -> Result<Vec<ShareState>, Refusal> {
        match self {
            Source::Real(roots) => share::apply(roots, a),
            Source::Demo => Ok(demo::share_status(a)),
        }
    }

    pub fn add_account(&self, c: &mut Config, name: &str) -> Result<Account, Refusal> {
        match self {
            Source::Real(roots) => accounts::add(roots, c, name),
            Source::Demo => demo::add_account(c, name),
        }
    }

    pub fn rename(&self, c: &mut Config, slug: &str, name: &str) -> Result<(), Refusal> {
        match self {
            Source::Real(_) => accounts::rename(c, slug, name),
            Source::Demo => demo::rename(c, slug, name),
        }
    }

    pub fn remove(&self, c: &mut Config, slug: &str) -> Result<(), Refusal> {
        match self {
            Source::Real(_) => accounts::remove(c, slug),
            Source::Demo => demo::remove(c, slug),
        }
    }

    /// Starts what a launch says ([`crate::run::run`]); the demo starts nothing.
    pub fn run(&self, l: &Launch) -> io::Result<()> {
        match self {
            Source::Real(_) => crate::run::run(l),
            Source::Demo => Ok(()),
        }
    }

    /// The folder a new session starts in when nothing is selected.
    pub fn home(&self) -> PathBuf {
        match self {
            Source::Real(roots) => roots.home.clone(),
            Source::Demo => PathBuf::from(r"C:\Users\demo"),
        }
    }

    /// The window's own settings file; none in the demo, which writes nothing.
    pub fn window_file(&self) -> Option<PathBuf> {
        match self {
            Source::Real(roots) => Some(roots.config_file().with_file_name("window.ini")),
            Source::Demo => None,
        }
    }
}
