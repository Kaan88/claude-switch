//! Where the window's data comes from: the core over the real folders of this computer, or the
//! made-up data of `--demo` ([`crate::demo`]). Every call the window makes into the core goes
//! through here, so the demo doesn't touch personal files. Its session-view fixture uses only temp.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;

use claude_switch_core::sessions::{self, PreviewLine};
use claude_switch_core::share::{self, ShareState};
use claude_switch_core::{
    Account, AccountStatus, App, Config, Launch, Refusal, Roots, Running, Session, accounts, config, launch, running,
};

use crate::demo;

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
}

pub struct ActivityScan {
    pub running: Vec<Running>,
    pub updates: Vec<(String, u64, String, bool)>,
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

    pub fn save_config(&self, c: &Config) -> Result<(), Refusal> {
        match self {
            Source::Real(roots) => config::save(roots, c),
            Source::Demo => Ok(()),
        }
    }

    pub fn write_workers(&self, c: &Config) -> Result<(), Refusal> {
        match self {
            Source::Real(roots) => config::write_workers(roots, c),
            Source::Demo => Ok(()),
        }
    }

    /// Sessions, running sessions and the accounts' login states. Runs on a worker thread.
    pub fn scan(&self, c: &Config) -> Scan {
        match self {
            Source::Real(roots) => Scan {
                sessions: sessions::scan(roots, c),
                running: running::scan(roots, c),
                statuses: c.accounts.iter().map(|a| (a.slug.clone(), accounts::status(roots, a))).collect(),
            },
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
        files: HashMap<String, (PathBuf, u64)>,
        mut watch: HashSet<String>,
    ) -> ActivityScan {
        let running = self.running(c);
        let mut updates = Vec::new();
        if !self.is_demo() {
            watch.extend(running.iter().map(|r| r.session_id.clone()));
            for id in watch {
                let Some((file, previous)) = files.get(&id) else { continue };
                let modified = claude_switch_core::modified_ms(file);
                if modified != 0
                    && modified != *previous
                    && let Some((prompt, ended)) = sessions::row_activity(file)
                {
                    updates.push((id, modified, prompt, ended));
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

    pub fn last_account(&self, c: &Config, s: &Session) -> String {
        match self {
            Source::Real(roots) => launch::last_account(roots, c, s),
            Source::Demo => demo::last_account(s),
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

    pub fn fork(&self, c: &Config, s: &Session, account: &str) -> Result<Launch, Refusal> {
        match self {
            Source::Real(roots) => launch::fork(roots, c, s, account),
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
            Source::Real(roots) => Some(roots.appdata.join("claude-switch").join("window.ini")),
            Source::Demo => None,
        }
    }
}
