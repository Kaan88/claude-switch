//! What to start to open a session, a new one, a login or the desktop app. Nothing here starts
//! anything: every function returns a [`Launch`] for the window to run.

use crate::sessions::folder_name;
use crate::util::{read_json, write_json};
#[cfg(test)]
mod tests;
use crate::{Account, App, Config, Launch, Place, Refusal, Roots, Running, Session, account as find_account};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The variable that gives the CLI another config folder.
const CONFIG_VAR: &str = "CLAUDE_CONFIG_DIR";

/// Variables that carry a Claude session's identity or login. A window started from inside such a
/// session has them, and no launch may pass them on.
const SESSION_VARS: [&str; 13] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_ACCOUNT_UUID",
    "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_EXECPATH",
];

/// How long after a launch the same session is not opened again: it may not be in `running` yet.
const JUST_LAUNCHED_MS: u64 = 20_000;

/// The program path out of a registered shell command such as `"C:\…\Claude.exe" -- "%1"`.
fn program_of_command(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let program = match command.strip_prefix('"') {
        Some(rest) => &rest[..rest.find('"')?],
        None => command.split_whitespace().next()?,
    };
    Some(PathBuf::from(program)).filter(|p| !p.as_os_str().is_empty())
}

/// The default value of `HKCU\Software\Classes\claude\shell\open\command`.
#[cfg(windows)]
fn registered_command() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = "Software\\Classes\\claude\\shell\\open\\command\0".encode_utf16().collect();
    let mut size: u32 = 0;
    // SAFETY: `key` is a NUL-terminated UTF-16 string that outlives the call; a null value name asks
    // for the default value; a null buffer with `size` asks only for the size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 || size > 64 * 1024 {
        return None;
    }
    let mut buf: Vec<u16> = vec![0; (size as usize).div_ceil(2)];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `size` bytes, as the call is told, and lives until after it returns.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(not(windows))]
fn registered_command() -> Option<String> {
    None
}

/// The desktop app's executable, from the command registered for the `claude:` protocol
/// (`HKCU\Software\Classes\claude\shell\open\command`). `None` when it is not registered or the file
/// is gone.
pub fn desktop_exe() -> Option<PathBuf> {
    program_of_command(&registered_command()?).filter(|p| p.is_file())
}

/// The environment changes that make the CLI use the account's folder.
fn account_env(account: &Account) -> (BTreeMap<String, String>, Vec<String>) {
    if account.primary {
        (BTreeMap::new(), vec![CONFIG_VAR.to_string()])
    } else {
        (BTreeMap::from([(CONFIG_VAR.to_string(), account.cli_dir.to_string_lossy().into_owned())]), Vec::new())
    }
}

/// Brings a further account's links and MCP list up to date; failures do not stop a launch.
fn refresh_shared(roots: &Roots, account: &Account) {
    if !account.primary {
        let _ = crate::share::apply(roots, account);
        let _ = crate::share::sync_mcp(roots, account);
    }
}

pub(crate) fn cli_launch(account: &Account, args: &[&str], cwd: PathBuf, place: Place, label: String) -> Launch {
    let (env, mut env_remove) = account_env(account);
    env_remove.extend(SESSION_VARS.map(str::to_string));
    Launch {
        program: PathBuf::from("claude"),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: Some(cwd),
        env,
        env_remove,
        place,
        label,
    }
}

/// Continues `session` as `account` in `app`. Refuses with [`Refusal::AlreadyRunning`] when `running`
/// (read just before by the caller) holds the session, [`Refusal::Invalid`] when it was launched less
/// than 20 seconds ago ([`note_starting`]), [`Refusal::NotLoggedIn`] when the account's CLI has no
/// login. CLI: `claude --resume <id>` in the session's folder (the home folder when that is
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
    if let Some(r) = crate::running::find(running, &session.id) {
        return Err(Refusal::AlreadyRunning(r.clone()));
    }
    if launched_a_moment_ago(roots, &session.id) {
        return Err(Refusal::Invalid("This session was opened a moment ago. Give it a few seconds to start.".into()));
    }
    let account = find_account(config, account)?;
    if !crate::accounts::status(roots, account).cli_logged_in {
        return Err(Refusal::NotLoggedIn(account.name.clone()));
    }
    refresh_shared(roots, account);
    let cwd = if session.cwd.is_dir() { session.cwd.clone() } else { roots.home.clone() };
    let folder = folder_name(&session.cwd);
    Ok(match app {
        App::Cli => cli_launch(
            account,
            &["--resume", &session.id],
            cwd,
            Place::Terminal,
            format!("{} · {folder} · Resume", account.name),
        ),
        App::Desktop => cli_launch(
            account,
            &["--desktop", "--resume", &session.id],
            cwd,
            Place::Detached,
            format!("{} · {folder} · Resume in the desktop app", account.name),
        ),
    })
}

/// A copy of `session` as `account`: `claude --resume <id> --fork-session` in the session's folder
/// (the home folder when that is gone), [`crate::Place::Terminal`], with the same environment as
/// [`resume`]. The copy is a new session, so this is safe while the original runs anywhere and does
/// not refuse a running session, nor note a start of the original's id; it refuses
/// [`Refusal::NotLoggedIn`] when the account's CLI has no login.
pub fn fork(roots: &Roots, config: &Config, session: &Session, account: &str) -> Result<Launch, Refusal> {
    let account = find_account(config, account)?;
    if !crate::accounts::status(roots, account).cli_logged_in {
        return Err(Refusal::NotLoggedIn(account.name.clone()));
    }
    refresh_shared(roots, account);
    let cwd = if session.cwd.is_dir() { session.cwd.clone() } else { roots.home.clone() };
    let label = format!("{} · {} · Copy", account.name, folder_name(&session.cwd));
    Ok(cli_launch(account, &["--resume", &session.id, "--fork-session"], cwd, Place::Terminal, label))
}

/// A new session as `account` in `cwd`: `claude` in a terminal, or the desktop app for
/// [`App::Desktop`] (the same as [`open_desktop`]; the folder is chosen there).
pub fn new_session(roots: &Roots, config: &Config, account: &str, app: App, cwd: &Path) -> Result<Launch, Refusal> {
    if app == App::Desktop {
        return open_desktop(config, account);
    }
    let account = find_account(config, account)?;
    refresh_shared(roots, account);
    let label = format!("{} · {} · New", account.name, folder_name(cwd));
    Ok(cli_launch(account, &[], cwd.to_path_buf(), Place::Terminal, label))
}

/// `claude auth login` for the account, in a terminal.
pub fn login(roots: &Roots, config: &Config, account: &str) -> Result<Launch, Refusal> {
    let account = find_account(config, account)?;
    let label = format!("{} · Sign in", account.name);
    Ok(cli_launch(account, &["auth", "login"], roots.home.clone(), Place::Terminal, label))
}

/// The launch of the desktop executable `exe` as `account`.
fn desktop_launch(exe: PathBuf, account: &Account) -> Launch {
    let args = if account.primary {
        Vec::new()
    } else {
        vec![format!("--user-data-dir={}", account.desktop_dir.to_string_lossy())]
    };
    Launch {
        program: exe,
        args,
        cwd: None,
        env: BTreeMap::new(),
        // The desktop app's Code tab always works in the standard CLI folder (docs/DESIGN.md).
        env_remove: without_cli_env(),
        place: Place::Detached,
        label: format!("{} · Desktop app", account.name),
    }
}

/// The desktop app as `account`: the executable alone for the first account, with
/// `--user-data-dir=<desktop_dir>` for a further one. [`Refusal::NoDesktopApp`] when it is not found.
pub fn open_desktop(config: &Config, account: &str) -> Result<Launch, Refusal> {
    let account = find_account(config, account)?;
    let exe = desktop_exe().ok_or(Refusal::NoDesktopApp)?;
    Ok(desktop_launch(exe, account))
}

/// The first of `names` found in a folder of `path` (a `PATH`-style list).
fn first_on_path(path: Option<&std::ffi::OsStr>, names: &[&'static str]) -> Option<&'static str> {
    let dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    names.iter().copied().find(|name| dirs.iter().any(|d| d.join(name).is_file()))
}

fn without_cli_env() -> Vec<String> {
    std::iter::once(CONFIG_VAR).chain(SESSION_VARS).map(str::to_string).collect()
}

/// A plain shell (`pwsh.exe`, else `powershell.exe`, else `cmd.exe`, whichever is on the `PATH`) in `cwd`.
/// It gets no Claude session's identity or login and no `CLAUDE_CONFIG_DIR`: a `claude` typed in it
/// is the first account's, as in any new terminal.
pub fn shell(cwd: &Path) -> Launch {
    let path = std::env::var_os("PATH");
    let program = first_on_path(path.as_deref(), &["pwsh.exe", "powershell.exe", "cmd.exe"]).unwrap_or("cmd.exe");
    Launch {
        program: PathBuf::from(program),
        args: Vec::new(),
        cwd: Some(cwd.to_path_buf()),
        env: BTreeMap::new(),
        env_remove: without_cli_env(),
        place: Place::Terminal,
        label: format!("Shell · {}", folder_name(cwd)),
    }
}

fn history_file(roots: &Roots) -> PathBuf {
    roots.appdata.join("claude-switch").join("history.json")
}

/// The system clock, in milliseconds since 1970.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Whether `history.json` says the session was launched within the last [`JUST_LAUNCHED_MS`].
fn launched_a_moment_ago(roots: &Roots, session_id: &str) -> bool {
    read_json(&history_file(roots))
        .and_then(|v| v.get(session_id)?.get("at")?.as_u64())
        // Either way round, so that a clock set back a little still counts and one set far back does
        // not lock the session.
        .is_some_and(|at| now_ms().abs_diff(at) < JUST_LAUNCHED_MS)
}

/// Records that `session_id` is being opened as `account`, now: what [`note_launch`] records. The
/// window calls it before it starts the process, so that [`resume`] refuses a second start while the
/// first is not yet in `running`. Failure is silent.
pub fn note_starting(roots: &Roots, session_id: &str, account: &str) {
    let path = history_file(roots);
    let mut map = match read_json(&path) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    map.insert(session_id.to_string(), json!({"account": account, "at": now_ms()}));
    let _ = write_json(&path, &Value::Object(map));
}

/// Records that `session_id` was last opened as `account` and when (in
/// `<appdata>/claude-switch/history.json`), so that a later carry to another account can be warned
/// about. The window calls it after a launch started. Failure is silent.
pub fn note_launch(roots: &Roots, session_id: &str, account: &str) {
    note_starting(roots, session_id, account);
}

/// The account a session was last opened as: what [`note_launch`] recorded, else the desktop record's
/// account, else the first account.
pub fn last_account(roots: &Roots, config: &Config, session: &Session) -> String {
    let known = |s: &str| config.accounts.iter().any(|a| a.slug == s);
    // An entry is `{"account": …, "at": …}`; an older one is the account's slug alone.
    let noted = read_json(&history_file(roots))
        .and_then(|v| {
            let entry = v.get(&session.id)?;
            entry.as_str().or_else(|| entry.get("account")?.as_str()).map(str::to_string)
        })
        .filter(|s| known(s));
    noted
        .or_else(|| session.desktop_account.clone().filter(|s| known(s)))
        .or_else(|| config.accounts.first().map(|a| a.slug.clone()))
        .unwrap_or_else(|| "main".to_string())
}

/// A size in plain words: `850 KB`, `12.4 MB`.
fn size_words(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 * 1024 { format!("{} KB", bytes.div_ceil(1024)) } else { format!("{:.1} MB", bytes as f64 / MB) }
}

/// The warning to show before opening `session` as `account`, when that is another account than the
/// one it last ran as: one or two plain sentences saying that the conversation continues but the
/// other account has nothing of it cached, so the first message re-reads all of it (with the
/// transcript's size as a rough measure). `None` when the account is the same.
pub fn carry_warning(roots: &Roots, config: &Config, session: &Session, account: &str) -> Option<String> {
    let last = last_account(roots, config, session);
    if last == account {
        return None;
    }
    let name = |slug: &str| find_account(config, slug).map_or_else(|_| slug.to_string(), |a| a.name.clone());
    Some(format!(
        "This conversation last ran as {} and continues as {}. {} has none of it cached, so the first message \
         re-reads the whole conversation at full price (the transcript is {}).",
        name(&last),
        name(account),
        name(account),
        size_words(session.bytes)
    ))
}
