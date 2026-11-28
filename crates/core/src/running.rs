//! Which sessions are open right now.

use crate::util::{read_json, str_field};
use crate::{Account, App, Config, Roots, Running};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often, and how far apart, the session file of a live process is read again when it does not
/// name a session: it is being written.
const RETRIES: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(40);

/// The account whose desktop data folder holds the record `<host_id>.json`.
fn desktop_owner<'a>(accounts: &'a [Account], host_id: &str) -> Option<&'a Account> {
    if host_id.is_empty() || host_id.contains(['/', '\\', '.']) {
        return None;
    }
    let file = format!("{host_id}.json");
    accounts.iter().find(|a| {
        let base = a.desktop_dir.join("claude-code-sessions");
        let subdirs = |dir: &std::path::Path| -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
                .unwrap_or_default()
        };
        subdirs(&base).iter().flat_map(|acc| subdirs(acc)).any(|org| org.join(&file).is_file())
    })
}

/// The CLI folders to look in, each with the slug it stands for: every configured account's, then
/// every `<accounts folder>/<name>/cli` on disk (a removed account's sessions may still run), with the
/// folder's name as the slug.
fn cli_dirs(roots: &Roots, config: &Config, complete: &mut bool) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = config.accounts.iter().map(|a| (a.cli_dir.clone(), a.slug.clone())).collect();
    match std::fs::read_dir(roots.accounts_dir()) {
        Ok(entries) => {
            let mut found = Vec::new();
            for entry in entries {
                match entry {
                    Ok(e) => match e.file_type() {
                        Ok(t) if t.is_dir() || t.is_symlink() => {
                            found.push((e.path().join("cli"), e.file_name().to_string_lossy().into_owned()))
                        }
                        Ok(_) => {}
                        Err(_) => *complete = false,
                    },
                    Err(_) => *complete = false,
                }
            }
            found.sort();
            out.extend(found);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => *complete = false,
    }
    out
}

/// The content of the session file of process `pid` (from its name), when it names a session. A file
/// of a live process that does not is read again up to [`RETRIES`] times, calling `wait` before each.
fn read_entry(path: &Path, pid: u32, mut wait: impl FnMut()) -> Option<Value> {
    let read = || {
        read_json(path).filter(|v| {
            v.get("pid").is_none_or(|p| p.as_u64() == Some(pid.into()))
                && str_field(v, "sessionId").is_some_and(|id| crate::launch::validate_session_id(id).is_ok())
        })
    };
    let mut entry = read();
    if entry.is_none() && pid_alive(pid) {
        for _ in 0..RETRIES {
            wait();
            entry = read();
            if entry.is_some() {
                break;
            }
        }
    }
    entry
}

/// Every running session of every account: each `<cli_dir>/sessions/<pid>.json` whose process is
/// alive (a folder reached twice counts once), also in the CLI folders under the accounts folder that
/// no configured account has. A live process's file that names no session is read again a few times
/// and left out only when it still names none. For a session run by a desktop app
/// (`entrypoint` `claude-desktop`) the account is the one whose desktop data folder holds the record
/// named by `hostSessionId`, else the account of the CLI folder. Never fails.
pub fn scan(roots: &Roots, config: &Config) -> Vec<Running> {
    scan_checked(roots, config).0
}

pub fn scan_checked(roots: &Roots, config: &Config) -> (Vec<Running>, bool) {
    let (found, complete) = scan_folders(roots, config);
    (found.into_iter().map(|(running, _)| running).collect(), complete)
}

/// The `sessions` folders (canonical paths) that hold the record of a live process, whatever
/// account the session is shown under: a desktop app's session of a further account runs in the
/// first account's CLI folder and holds that folder's files. False with them when some folder or
/// record could not be read, or a live process's record names no session yet: the list may then
/// miss one.
pub fn live_folders(roots: &Roots, config: &Config) -> (Vec<PathBuf>, bool) {
    let (found, complete) = scan_folders(roots, config);
    let mut folders: Vec<PathBuf> = found.into_iter().map(|(_, folder)| folder).collect();
    folders.sort();
    folders.dedup();
    (folders, complete)
}

fn scan_folders(roots: &Roots, config: &Config) -> (Vec<(Running, PathBuf)>, bool) {
    let mut out: Vec<(Running, PathBuf)> = Vec::new();
    let mut complete = true;
    let mut seen = HashSet::new();
    for (cli_dir, slug) in cli_dirs(roots, config, &mut complete) {
        let dir = cli_dir.join("sessions");
        let canonical = match std::fs::canonicalize(&dir) {
            Ok(path) => path,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound || std::fs::symlink_metadata(&dir).is_ok() {
                    complete = false;
                }
                continue;
            }
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem_pid = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok());
            let Some(stem_pid) = stem_pid else { continue };
            let Some(v) = read_entry(&path, stem_pid, || std::thread::sleep(RETRY_PAUSE)) else {
                complete &= pid_state(stem_pid) == Some(false);
                continue;
            };
            let pid = stem_pid;
            match record_pid_state(pid, &path) {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    complete = false;
                    continue;
                }
            }
            let Some(session_id) = str_field(&v, "sessionId") else { continue };
            if out.iter().any(|(r, _)| r.pid == pid && r.session_id == session_id) {
                continue;
            }
            let app = if str_field(&v, "entrypoint") == Some("claude-desktop") { App::Desktop } else { App::Cli };
            let owner = match (app, str_field(&v, "hostSessionId")) {
                (App::Desktop, Some(host)) => desktop_owner(&config.accounts, host),
                _ => None,
            };
            let running = Running {
                session_id: session_id.to_string(),
                pid,
                app,
                account: owner.map_or_else(|| slug.clone(), |a| a.slug.clone()),
                cwd: str_field(&v, "cwd").map(PathBuf::from).unwrap_or_default(),
            };
            out.push((running, canonical.clone()));
        }
    }
    out.sort_by(|(a, _), (b, _)| a.account.cmp(&b.account).then(a.pid.cmp(&b.pid)));
    (out, complete)
}

/// Whether a process with this id exists now.
pub fn pid_alive(pid: u32) -> bool {
    pid_state(pid) != Some(false)
}

#[cfg(windows)]
fn pid_state(pid: u32) -> Option<bool> {
    windows_pid_state(pid, None)
}

#[cfg(windows)]
fn record_pid_state(pid: u32, path: &Path) -> Option<bool> {
    use std::os::windows::fs::MetadataExt as _;
    let last_write = std::fs::metadata(path).ok()?.last_write_time();
    (last_write != 0).then_some(())?;
    windows_pid_state(pid, Some(last_write))
}

#[cfg(not(windows))]
fn record_pid_state(pid: u32, _path: &Path) -> Option<bool> {
    pid_state(pid)
}

#[cfg(windows)]
fn windows_pid_state(pid: u32, record_time: Option<u64>) -> Option<bool> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };
    fn matches_record(handle: HANDLE, record_time: Option<u64>) -> Option<bool> {
        let Some(record_time) = record_time else { return Some(true) };
        let (mut creation, mut exit, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: the process handle is live and each distinct FILETIME is writable until return.
        let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
        let created = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        (ok && created != 0).then_some(created <= record_time)
    }
    const STILL_ACTIVE: u32 = 259;
    if pid == 0 {
        return Some(false);
    }
    // SAFETY: OpenProcess takes plain values and returns a handle or null; nothing is borrowed.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid) };
    if !handle.is_null() {
        // A process handle is signalled once the process has ended, whatever its exit code.
        // SAFETY: `handle` was opened above with the right to wait on it; a zero timeout returns at once.
        let alive = match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_TIMEOUT => matches_record(handle, record_time),
            WAIT_OBJECT_0 => Some(false),
            _ => None,
        };
        // SAFETY: `handle` was opened above and is closed exactly once.
        unsafe { CloseHandle(handle) };
        return alive;
    }
    // SAFETY: reads the calling thread's last error code; no pointers involved.
    match unsafe { GetLastError() } {
        ERROR_INVALID_PARAMETER => return Some(false),
        ERROR_ACCESS_DENIED => {}
        _ => return None,
    }
    // Waiting was denied: ask with the right to query alone, which cannot wait.
    // SAFETY: OpenProcess takes plain values and returns a handle or null; nothing is borrowed.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads the calling thread's last error code; no pointers involved.
        return None;
    }
    let mut code: u32 = 0;
    // SAFETY: `handle` is a valid process handle opened above and `code` is a live u32 to write to.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
    let alive = match (ok, code) {
        (true, STILL_ACTIVE) => matches_record(handle, record_time),
        (true, _) => Some(false),
        _ => None,
    };
    // SAFETY: `handle` was opened above and is closed exactly once.
    unsafe { CloseHandle(handle) };
    alive
}

/// Whether a process with this id exists now.
#[cfg(not(windows))]
fn pid_state(pid: u32) -> Option<bool> {
    Some(pid != 0 && std::path::Path::new("/proc").join(pid.to_string()).exists())
}

/// The running entry for a session id, if it is open anywhere.
pub fn find<'a>(running: &'a [Running], session_id: &str) -> Option<&'a Running> {
    running.iter().find(|r| r.session_id.eq_ignore_ascii_case(session_id))
}

#[cfg(test)]
mod tests {
    #[test]
    fn mismatched_pid_and_invalid_uuid_cannot_prove_scan_complete() {
        let t = TempDir::new("run-contradictory");
        let roots = t.roots();
        let config = config_with(&roots, &[]);
        let me = std::process::id();
        let path = roots.primary_cli_dir().join("sessions").join(format!("{me}.json"));
        for bad in [
            json!({"pid":DEAD,"sessionId":"00000000-0000-4000-a000-000000000001"}),
            json!({"pid":me,"sessionId":"invalid"}),
        ] {
            put(&path, &bad.to_string());
            let (running, complete) = scan_checked(&roots, &config);
            assert!(running.is_empty() && !complete);
        }
    }
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};
    use serde_json::json;

    const DEAD: u32 = 4_000_000_000;

    #[test]
    fn this_process_is_alive_and_a_made_up_one_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(DEAD));
        assert!(!pid_alive(0));
    }

    #[test]
    fn a_running_file_with_a_dead_pid_is_ignored() {
        let t = TempDir::new("run-dead");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let me = std::process::id();
        let sessions = roots.primary_cli_dir().join("sessions");
        put(&sessions.join(format!("{DEAD}.json")), &json!({"pid":DEAD,"sessionId":"dead","cwd":"C:\\x"}).to_string());
        put(
            &sessions.join(format!("{me}.json")),
            &json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000001","cwd":"C:\\x","entrypoint":"cli"})
                .to_string(),
        );
        put(&sessions.join("notes.json"), &json!({"pid":me,"sessionId":"odd"}).to_string());
        put(&sessions.join(format!("{me}.abc.key")), "x");
        let running = scan(&roots, &config);
        assert_eq!(running.len(), 1);
        let r = &running[0];
        assert_eq!(
            (r.session_id.as_str(), r.pid, r.app, r.account.as_str()),
            ("00000000-0000-4000-a000-000000000001", me, App::Cli, "main")
        );
        assert_eq!(r.cwd, PathBuf::from("C:\\x"));
        assert!(find(&running, "00000000-0000-4000-a000-000000000001").is_some());
        assert!(find(&running, "dead").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_live_pid_does_not_validate_a_record_written_before_that_process_started() {
        let t = TempDir::new("run-reused");
        let roots = t.roots();
        let config = config_with(&roots, &[]);
        let pid = std::process::id();
        let path = roots.primary_cli_dir().join("sessions").join(format!("{pid}.json"));
        put(&path, &json!({"pid":pid,"sessionId":"00000000-0000-4000-a000-000000000001"}).to_string());
        assert_eq!(scan_checked(&roots, &config).0.len(), 1);
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        let (running, complete) = scan_checked(&roots, &config);
        assert!(running.is_empty() && complete);
    }

    #[cfg(windows)]
    #[test]
    fn a_process_that_ended_with_code_259_is_not_alive() {
        // The `Child` keeps the ended process's id and handle in being for the check.
        let mut child = std::process::Command::new("cmd").args(["/c", "exit", "259"]).spawn().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(259));
        assert!(!pid_alive(child.id()));
    }

    #[test]
    fn a_half_written_file_of_a_live_process_is_read_again() {
        let t = TempDir::new("run-half");
        let me = std::process::id();
        let file = t.0.join(format!("{me}.json"));
        let whole = json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000002"}).to_string();
        for half in ["{\"pid\":", "{}"] {
            put(&file, half);
            let mut waits = 0;
            let entry = read_entry(&file, me, || {
                waits += 1;
                if waits == 3 {
                    put(&file, &whole);
                }
            });
            assert_eq!(
                (entry.unwrap()["sessionId"].as_str(), waits),
                (Some("00000000-0000-4000-a000-000000000002"), 3)
            );
        }
        put(&file, "{");
        let mut waits = 0;
        assert!(read_entry(&file, me, || waits += 1).is_none());
        assert_eq!(waits, 3, "then it is left out: there is no id to report");
        assert!(read_entry(&file, DEAD, || panic!("a dead process's file is not waited for")).is_none());
    }

    #[test]
    fn sessions_in_a_cli_folder_no_account_has_are_found_under_its_folder_name() {
        let t = TempDir::new("run-unconfigured");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let me = std::process::id();
        put(
            &roots.accounts_dir().join("removed").join("cli").join("sessions").join(format!("{me}.json")),
            &json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000003","entrypoint":"cli"}).to_string(),
        );
        put(
            &config.accounts[1].cli_dir.join("sessions").join(format!("{me}.json")),
            &json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000004","entrypoint":"cli"}).to_string(),
        );
        let running = scan(&roots, &config);
        let got: Vec<(&str, &str)> = running.iter().map(|r| (r.session_id.as_str(), r.account.as_str())).collect();
        assert_eq!(
            got,
            [("00000000-0000-4000-a000-000000000003", "removed"), ("00000000-0000-4000-a000-000000000004", "work")]
        );
    }

    #[test]
    fn the_last_removed_accounts_sessions_are_still_found() {
        let t = TempDir::new("run-last-removed");
        let roots = t.roots();
        let mut config = config_with(&roots, &[("work", "Work")]);
        let me = std::process::id();
        put(
            &config.accounts[1].cli_dir.join("sessions").join(format!("{me}.json")),
            &json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000003","entrypoint":"cli"}).to_string(),
        );
        crate::accounts::remove(&mut config, "work").unwrap();
        assert_eq!(config.accounts.len(), 1);
        let running = scan(&roots, &config);
        assert_eq!(running.len(), 1);
        assert_eq!(
            (running[0].session_id.as_str(), running[0].account.as_str(), running[0].pid),
            ("00000000-0000-4000-a000-000000000003", "work", me)
        );
    }

    #[test]
    fn a_desktop_session_belongs_to_the_account_holding_its_record() {
        let t = TempDir::new("run-desktop");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let me = std::process::id();
        put(
            &roots.primary_cli_dir().join("sessions").join(format!("{me}.json")),
            &json!({"pid":me,"sessionId":"00000000-0000-4000-a000-000000000005","entrypoint":"claude-desktop","hostSessionId":"local_9"}).to_string(),
        );
        put(
            &config.accounts[1].desktop_dir.join("claude-code-sessions").join("a").join("o").join("local_9.json"),
            "{}",
        );
        let running = scan(&roots, &config);
        assert_eq!(running.len(), 1);
        assert_eq!((running[0].app, running[0].account.as_str()), (App::Desktop, "work"));
        std::fs::remove_dir_all(config.accounts[1].desktop_dir.join("claude-code-sessions")).unwrap();
        assert_eq!(scan(&roots, &config)[0].account, "main", "without a record it is the CLI folder's account");
    }
}
