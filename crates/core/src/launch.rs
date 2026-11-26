//! What to start to open a session, a new one, a login or the desktop app. Nothing here starts
//! anything: every function returns a [`Launch`] for the window to run.

use crate::{App, Config, Launch, Refusal, Roots, Running, Session};
use std::path::{Path, PathBuf};

/// The desktop app's executable, from the command registered for the `claude:` protocol
/// (`HKCU\Software\Classes\claude\shell\open\command`). `None` when it is not registered or the file
/// is gone.
pub fn desktop_exe() -> Option<PathBuf> {
    todo!()
}

/// Continues `session` as `account` in `app`. Refuses with [`Refusal::AlreadyRunning`] when `running`
/// (read just before by the caller) holds the session, [`Refusal::NotLoggedIn`] when the account's CLI
/// has no login. CLI: `claude --resume <id>` in the session's folder (the home folder when that is
/// gone), [`crate::Place::Terminal`]. Desktop: `claude --desktop --resume <id>`,
/// [`crate::Place::Detached`]. Either way `CLAUDE_CONFIG_DIR` is set for a further account and
/// removed for the first; a further account's shared links and MCP list are brought up to date first
/// ([`crate::share::apply`], [`crate::share::sync_mcp`]; their failure does not stop the launch).
pub fn resume(
    roots: &Roots,
    config: &Config,
    session: &Session,
    account: &str,
    app: App,
    running: &[Running],
) -> Result<Launch, Refusal> {
    let _ = (roots, config, session, account, app, running);
    todo!()
}

/// A new session as `account` in `cwd`: `claude` in a terminal, or the desktop app for
/// [`App::Desktop`] (the same as [`open_desktop`]; the folder is chosen there).
pub fn new_session(roots: &Roots, config: &Config, account: &str, app: App, cwd: &Path) -> Result<Launch, Refusal> {
    let _ = (roots, config, account, app, cwd);
    todo!()
}

/// `claude auth login` for the account, in a terminal.
pub fn login(roots: &Roots, config: &Config, account: &str) -> Result<Launch, Refusal> {
    let _ = (roots, config, account);
    todo!()
}

/// The desktop app as `account`: the executable alone for the first account, with
/// `--user-data-dir=<desktop_dir>` for a further one. [`Refusal::NoDesktopApp`] when it is not found.
pub fn open_desktop(config: &Config, account: &str) -> Result<Launch, Refusal> {
    let _ = (config, account);
    todo!()
}

/// A plain shell (`pwsh.exe`, else `powershell.exe`, else `cmd.exe`, whichever is on the `PATH`) in `cwd`.
pub fn shell(cwd: &Path) -> Launch {
    let _ = cwd;
    todo!()
}

/// Records that `session_id` was last opened as `account` (in `<appdata>/claude-switch/history.json`),
/// so that a later carry to another account can be warned about. The window calls it after a launch
/// started. Failure is silent.
pub fn note_launch(roots: &Roots, session_id: &str, account: &str) {
    let _ = (roots, session_id, account);
    todo!()
}

/// The account a session was last opened as: what [`note_launch`] recorded, else the desktop record's
/// account, else the first account.
pub fn last_account(roots: &Roots, config: &Config, session: &Session) -> String {
    let _ = (roots, config, session);
    todo!()
}

/// The warning to show before opening `session` as `account`, when that is another account than the
/// one it last ran as: one or two plain sentences saying that the conversation continues but the
/// other account has nothing of it cached, so the first message re-reads all of it (with the
/// transcript's size as a rough measure). `None` when the account is the same.
pub fn carry_warning(roots: &Roots, config: &Config, session: &Session, account: &str) -> Option<String> {
    let _ = (roots, config, session, account);
    todo!()
}
