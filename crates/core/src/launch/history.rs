//! A persistent reservation, serialized with the running check across app instances.

use super::*;
use crate::util::{io_refusal, lock_beside, write_json};
use std::fs::File;
use std::io;

const PENDING_MS: u64 = 60_000;

pub(super) fn previous(entry: &Value) -> &Value {
    entry.get("previous").unwrap_or(entry)
}

pub(super) fn recent(entry: &Value) -> Option<bool> {
    if let Some(at) = entry.get("pending_at").and_then(Value::as_u64) {
        Some(now_ms().abs_diff(at) < PENDING_MS)
    } else {
        entry.get("at")?.as_u64().map(|at| now_ms().abs_diff(at) < JUST_LAUNCHED_MS)
    }
}

pub struct Reservation {
    _lock: File,
    path: PathBuf,
    id: String,
    account: String,
    before: Option<Value>,
    entries: Map<String, Value>,
}

impl Reservation {
    /// Commit only after a successful start, or put the previous entry back.
    pub fn finish(mut self, started: bool) -> Result<(), Refusal> {
        self.entries.remove(&self.id);
        if started {
            self.entries.insert(self.id.clone(), json!({"account": self.account, "at": now_ms()}));
        } else if let Some(before) = self.before {
            self.entries.insert(self.id.clone(), before);
        }
        write_json(&self.path, &Value::Object(self.entries))
    }
}

pub fn reserve(roots: &Roots, config: &Config, id: &str, account: &str) -> Result<Reservation, Refusal> {
    reserve_with(roots, id, account, || {
        let (running, complete) = crate::running::scan_checked(roots, config);
        if let Some(r) = crate::running::find(&running, id) {
            Err(Refusal::AlreadyRunning(r.clone()))
        } else if !complete {
            Err(Refusal::Invalid("Could not verify that the session is closed in every account.".into()))
        } else {
            Ok(())
        }
    })
}

fn valid_previous(entry: &Value) -> bool {
    entry.is_string() || (entry["account"].is_string() && entry.get("at").is_none_or(Value::is_u64))
}

fn valid_entry(entry: &Value) -> bool {
    if let Some(at) = entry.get("pending_at") {
        at.is_u64() && entry.get("previous").is_some_and(|p| p.is_null() || valid_previous(p))
    } else {
        valid_previous(entry)
    }
}

fn read_entries(path: &Path) -> Result<Map<String, Value>, Refusal> {
    crate::util::regular_target(path)?;
    let entries: Map<String, Value> = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| io_refusal("read launch history", path, &e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(io_refusal("read launch history", path, &e)),
    };
    if !entries.values().all(valid_entry) {
        return Err(Refusal::Invalid("Launch history contains an invalid reservation.".into()));
    }
    Ok(entries)
}

/// Recheck activity while preventing Claude Switch from reserving a concurrent start.
pub fn with_archive_check<T>(
    roots: &Roots,
    config: &Config,
    commit: impl FnOnce(&std::collections::HashSet<String>) -> T,
) -> Result<T, Refusal> {
    let path = history_file(roots);
    let _lock = lock_beside(&path)?;
    let (running, complete) = crate::running::scan_checked(roots, config);
    if !complete {
        return Err(Refusal::Invalid("The running-session check was incomplete. Nothing was archived.".into()));
    }
    let mut busy: std::collections::HashSet<_> = running.into_iter().map(|r| r.session_id).collect();
    busy.extend(
        read_entries(&path)?.into_iter().filter_map(|(id, value)| recent(&value).unwrap_or(false).then_some(id)),
    );
    Ok(commit(&busy))
}

fn reserve_with(
    roots: &Roots,
    id: &str,
    account: &str,
    recheck: impl FnOnce() -> Result<(), Refusal>,
) -> Result<Reservation, Refusal> {
    let path = history_file(roots);
    let lock = lock_beside(&path)?;
    recheck()?;
    let mut entries = read_entries(&path)?;
    if entries.iter().any(|(key, value)| key.eq_ignore_ascii_case(id) && recent(value).unwrap_or(false)) {
        return Err(Refusal::Invalid("This session was opened a moment ago. Give it a few seconds to start.".into()));
    }
    let id = entries.keys().find(|key| key.eq_ignore_ascii_case(id)).map_or_else(|| id.to_owned(), Clone::clone);
    let before = entries.get(&id).map(previous).filter(|v| !v.is_null()).cloned();
    entries.insert(id.clone(), json!({"pending_at": now_ms(), "previous": before}));
    let value = Value::Object(entries);
    write_json(&path, &value)?;
    let Value::Object(entries) = value else { unreachable!() };
    Ok(Reservation { _lock: lock, path, id, account: account.into(), before, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, put};

    #[test]
    fn an_unreadable_sessions_directory_or_unresolved_live_pid_blocks_resume() {
        let t = TempDir::new("launch-incomplete-scan");
        let roots = t.roots();
        let config = crate::config::load(&roots);
        let dir = roots.primary_cli_dir().join("sessions");
        assert!(crate::running::scan_checked(&roots, &config).1);
        put(&dir, "not a directory");
        assert!(reserve(&roots, &config, "id", "main").is_err());
        std::fs::remove_file(&dir).unwrap();
        put(&dir.join(format!("{}.json", std::process::id())), "{}");
        assert!(!crate::running::scan_checked(&roots, &config).1);
        assert!(reserve(&roots, &config, "id", "main").is_err());
    }

    #[test]
    fn reservations_exclude_other_handles_and_rollback_keeps_the_previous_account() {
        let t = TempDir::new("launch-lock");
        let roots = t.roots();
        let path = history_file(&roots);
        put(&path, r#"{"id":"main"}"#);
        let first = reserve_with(&roots, "id", "work", || Ok(())).unwrap();
        let competing = roots.clone();
        assert!(std::thread::spawn(move || reserve_with(&competing, "id", "work", || Ok(())).is_err()).join().unwrap());
        assert_eq!(previous(&read_json(&path).unwrap()["id"]), "main");
        first.finish(false).unwrap();
        assert_eq!(read_json(&path).unwrap()["id"], "main");
        reserve_with(&roots, "id", "work", || Ok(())).unwrap().finish(true).unwrap();
        assert_eq!(read_json(&path).unwrap()["id"]["account"], "work");
        assert!(reserve_with(&roots, "ID", "work", || Ok(())).is_err());
    }

    #[test]
    fn archive_rechecks_reservations_and_refuses_an_incomplete_scan() {
        let t = TempDir::new("archive-incomplete-scan");
        let roots = t.roots();
        let config = crate::config::load(&roots);
        let reservation = reserve_with(&roots, "busy", "main", || Ok(())).unwrap();
        assert!(with_archive_check(&roots, &config, |_| panic!("lock held")).is_err());
        reservation.finish(true).unwrap();
        assert!(with_archive_check(&roots, &config, |busy| busy.contains("busy")).unwrap());
        put(&roots.primary_cli_dir().join("sessions"), "not a directory");
        assert!(with_archive_check(&roots, &config, |_| panic!("scan incomplete")).is_err());
    }

    #[test]
    fn broken_history_and_failed_rescan_cannot_authorize_a_launch() {
        let t = TempDir::new("launch-broken");
        let roots = t.roots();
        let path = history_file(&roots);
        for broken in ["broken", r#"{"id":{"at":"bad","account":"main"}}"#] {
            put(&path, broken);
            assert!(reserve_with(&roots, "id", "work", || Ok(())).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        }
        put(&path, "{}");
        assert!(reserve_with(&roots, "id", "work", || Err(Refusal::Invalid("scan".into()))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }
}
