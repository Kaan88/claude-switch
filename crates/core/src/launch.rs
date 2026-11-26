//! What to start to open a session, a new one, a login or the desktop app. Nothing here starts
//! anything: every function returns a [`Launch`] for the window to run.

use crate::sessions::folder_name;
use crate::util::read_json;
#[cfg(test)]
use crate::util::write_json;
#[cfg(windows)]
mod desktop;
mod history;
pub use history::{Reservation, record_verified_owner, reserve, with_archive_check};
#[cfg(test)]
mod tests;
use crate::{Account, App, Config, Launch, Place, Refusal, Roots, Running, Session, account as find_account};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
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

/// Shared by every child launcher. Apply removals before the app's explicit settings.
pub fn child_removals(launch: &Launch) -> impl Iterator<Item = OsString> + '_ {
    launch
        .env_remove
        .iter()
        .map(OsString::from)
        .chain(std::env::vars_os().map(|(name, _)| name).filter(|name| scrub_name(name)))
}

fn scrub_name(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    let starts = |prefix: &[u8]| name.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix));
    starts(b"ANTHROPIC_") || starts(b"CLAUDE_") || name.eq_ignore_ascii_case(b"CLAUDECODE")
}

/// How long after a launch the same session is not opened again: it may not be in `running` yet.
const JUST_LAUNCHED_MS: u64 = 20_000;

/// The verified desktop executable from a known installed layout, never a shell command or alias.
pub fn desktop_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    return desktop::resolve();
    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
pub use desktop::guard_desktop_exe;

#[cfg(windows)]
pub fn is_desktop_request(launch: &Launch) -> bool {
    launch.place == Place::Detached && launch.program == Path::new(desktop::REQUEST)
}

/// The environment changes that make the CLI use the account's folder.
fn account_env(account: &Account) -> (BTreeMap<String, String>, Vec<String>) {
    if account.primary {
        (BTreeMap::new(), vec![CONFIG_VAR.to_string()])
    } else {
        (BTreeMap::from([(CONFIG_VAR.to_string(), account.cli_dir.to_string_lossy().into_owned())]), Vec::new())
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
/// removed for the first; missing shared links and initial MCP settings are provisioned first
/// without replacing existing directories.
pub fn resume(
    roots: &Roots,
    config: &Config,
    session: &Session,
    account: &str,
    app: App,
    running: &[Running],
) -> Result<Launch, Refusal> {
    validate_session_id(&session.id)?;
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
    crate::share::prepare_cli(roots, account, Some(session))?;
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
pub fn fork(roots: &Roots, config: &Config, session: &Session, account: &str, new_id: &str) -> Result<Launch, Refusal> {
    validate_session_id(&session.id)?;
    validate_session_id(new_id)?;
    if session.id.eq_ignore_ascii_case(new_id) {
        return Err(Refusal::Invalid("A copy requires a different session ID.".into()));
    }
    let account = find_account(config, account)?;
    if !crate::accounts::status(roots, account).cli_logged_in {
        return Err(Refusal::NotLoggedIn(account.name.clone()));
    }
    crate::share::prepare_cli(roots, account, Some(session))?;
    let cwd = if session.cwd.is_dir() { session.cwd.clone() } else { roots.home.clone() };
    let label = format!("{} · {} · Copy", account.name, folder_name(&session.cwd));
    Ok(cli_launch(
        account,
        &["--resume", &session.id, "--fork-session", "--session-id", new_id],
        cwd,
        Place::Terminal,
        label,
    ))
}

pub fn validate_session_id(id: &str) -> Result<(), Refusal> {
    let valid = id.len() == 36
        && id.bytes().enumerate().all(
            |(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
            },
        );
    if valid { Ok(()) } else { Err(Refusal::Invalid("The transcript does not have a UUID session ID.".into())) }
}

/// Explicit agent for a new Claude Code CLI session. Each namespace segment is a name, never a flag.
pub fn session_agent_args(name: &str) -> Result<[&str; 2], Refusal> {
    let valid = name.len() <= 128
        && name.split(':').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        });
    if !valid {
        return Err(Refusal::Invalid("The selected CLI agent does not have a supported name.".into()));
    }
    Ok(["--agent", name])
}

/// A new session as `account` in `cwd`: `claude` in a terminal, or the desktop app for
/// [`App::Desktop`] (the same as [`open_desktop`]; the folder is chosen there).
pub fn new_session(roots: &Roots, config: &Config, account: &str, app: App, cwd: &Path) -> Result<Launch, Refusal> {
    if app == App::Desktop {
        return open_desktop(config, account);
    }
    let account = find_account(config, account)?;
    crate::share::prepare_cli(roots, account, None)?;
    let label = format!("{} · {} · New", account.name, folder_name(cwd));
    Ok(cli_launch(account, &[], cwd.to_path_buf(), Place::Terminal, label))
}

/// `claude auth login` for the account, in a terminal.
pub fn login(roots: &Roots, config: &Config, account: &str) -> Result<Launch, Refusal> {
    let account = find_account(config, account)?;
    crate::share::prepare_cli(roots, account, None)?;
    let label = format!("{} · Sign in", account.name);
    Ok(cli_launch(account, &["auth", "login"], roots.home.clone(), Place::Terminal, label))
}

/// Distinguishes a sign-in terminal from a session or shell without looking at its title or output.
pub fn is_login(launch: &Launch) -> bool {
    launch.place == Place::Terminal && launch.program == Path::new("claude") && launch.args == ["auth", "login"]
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
/// `--user-data-dir=<desktop_dir>` for a further one. The runner resolves and locks the
/// installed executable at the point of process creation, never as a separate preflight.
pub fn open_desktop(config: &Config, account: &str) -> Result<Launch, Refusal> {
    let account = find_account(config, account)?;
    #[cfg(windows)]
    return Ok(desktop_launch(PathBuf::from(desktop::REQUEST), account));
    #[cfg(not(windows))]
    {
        let _ = account;
        Err(Refusal::NoDesktopApp)
    }
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
    roots.config_file().with_file_name("history.json")
}

/// The system clock, in milliseconds since 1970.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Whether `history.json` says the session was launched within the last [`JUST_LAUNCHED_MS`].
fn launched_a_moment_ago(roots: &Roots, session_id: &str) -> bool {
    read_json(&history_file(roots)).is_some_and(|v| {
        v.as_object().is_some_and(|entries| {
            entries
                .iter()
                .any(|(id, entry)| id.eq_ignore_ascii_case(session_id) && history::recent(entry).unwrap_or(false))
        })
    })
}

#[cfg(test)]
fn note_starting(roots: &Roots, session_id: &str, account: &str) {
    let path = history_file(roots);
    let mut map = match read_json(&path) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    map.insert(session_id.to_string(), json!({"account": account, "at": now_ms()}));
    let _ = write_json(&path, &Value::Object(map));
}

#[cfg(test)]
fn note_launch(roots: &Roots, session_id: &str, account: &str) {
    note_starting(roots, session_id, account);
}

/// The verified account in launch history or the Desktop index; otherwise ownership is unknown.
pub fn last_account(roots: &Roots, config: &Config, session: &Session) -> Option<String> {
    let known = |s: &str| config.accounts.iter().any(|a| a.slug == s);
    // An entry is `{"account": …, "at": …}`; an older one is the account's slug alone.
    let noted = read_json(&history_file(roots))
        .and_then(|v| {
            let entries = v.as_object()?;
            let entry = entries.get(&session.id).or_else(|| {
                entries.iter().find_map(|(id, entry)| id.eq_ignore_ascii_case(&session.id).then_some(entry))
            })?;
            let entry = history::previous(entry);
            entry.as_str().or_else(|| entry.get("account")?.as_str()).map(str::to_string)
        })
        .filter(|s| known(s));
    noted.or_else(|| session.desktop_account.clone().filter(|s| known(s)))
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
    if last.as_deref() == Some(account) {
        return None;
    }
    let name = |slug: &str| find_account(config, slug).map_or_else(|_| slug.to_string(), |a| a.name.clone());
    let Some(last) = last else {
        return Some(format!(
            "The previous account is unknown. If this conversation last ran under a different account, \
             {} must re-read the conversation ({}) on the first message.",
            name(account),
            size_words(session.bytes)
        ));
    };
    Some(format!(
        "This conversation last ran as {} and continues as {}. {} has none of it cached, so the first message \
         re-reads the whole conversation at full price (the transcript is {}).",
        name(&last),
        name(account),
        name(account),
        size_words(session.bytes)
    ))
}
