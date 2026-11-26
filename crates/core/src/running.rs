//! Which sessions are open right now.

use crate::{Config, Running};

/// Every running session of every account: each `<cli_dir>/sessions/<pid>.json` whose process is
/// alive (a folder reached twice counts once). For a session run by a desktop app
/// (`entrypoint` `claude-desktop`) the account is the one whose desktop data folder holds the record
/// named by `hostSessionId`, else the account of the CLI folder. Never fails.
pub fn scan(config: &Config) -> Vec<Running> {
    let _ = config;
    todo!()
}

/// Whether a process with this id exists now.
pub fn pid_alive(pid: u32) -> bool {
    let _ = pid;
    todo!()
}

/// The running entry for a session id, if it is open anywhere.
pub fn find<'a>(running: &'a [Running], session_id: &str) -> Option<&'a Running> {
    running.iter().find(|r| r.session_id == session_id)
}
