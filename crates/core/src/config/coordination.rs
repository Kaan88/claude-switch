//! Cross-window permits and publication from the settings on disk.

use super::{parse, read_settings};
use crate::usage::Error;
use crate::util::{LockError, io_refusal, lock_beside, lock_beside_until, regular_target, write_json};
use crate::{Config, Refusal, Roots, WorkersUse};
use serde_json::{Map, Value};
use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Instant;

/// Try to hold the configuration lock across credential access or request submission.
pub fn usage_permit(roots: &Roots, dir: &Path) -> Option<File> {
    usage_permit_until(roots, dir, Instant::now(), || true).ok()
}

pub fn usage_permit_until(
    roots: &Roots,
    dir: &Path,
    deadline: Instant,
    enabled: impl Fn() -> bool,
) -> Result<File, Error> {
    let path = roots.config_file();
    let lock = lock_beside_until(&path, deadline, enabled).map_err(|error| match error {
        LockError::Busy => Error::SettingsBusy,
        LockError::Refused(_) => Error::SettingsUnavailable,
    })?;
    let settings = read_settings(&path).map_err(|_| Error::SettingsUnavailable)?;
    let config = parse(roots, settings, false);
    if !config.show_usage || !config.accounts.iter().any(|a| a.cli_dir == dir) {
        return Err(Error::SettingsUnavailable);
    }
    Ok(lock)
}

/// Read committed opt-in without opening credentials or the usage cache.
pub fn usage_enabled(roots: &Roots) -> bool {
    read_settings(&roots.config_file()).is_ok_and(|v| v["showUsage"].as_bool().unwrap_or(true))
}

/// Update only the two worker-account keys in the bridge from the committed app settings.
pub fn sync_workers(roots: &Roots) -> Result<(), Refusal> {
    let settings = roots.config_file();
    let _lock = lock_beside(&settings)?;
    let committed = read_settings(&settings)?;
    write_workers(roots, &parse(roots, committed, false))
}

fn bridge_bytes(path: &Path) -> Result<Option<Vec<u8>>, Refusal> {
    regular_target(path)?;
    match crate::read_bounded(path, crate::APP_FILE_LIMIT) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_refusal("read", path, &e)),
    }
}

pub(super) fn write_workers(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let path = roots.workers_file();
    let _lock = lock_beside(&path)?;
    let baseline = bridge_bytes(&path)?;
    let mut object = match &baseline {
        Some(bytes) => {
            serde_json::from_slice::<Map<String, Value>>(bytes).map_err(|e| io_refusal("update", &path, &e))?
        }
        None => Map::new(),
    };
    let dirs: Vec<Value> =
        config.accounts.iter().map(|a| Value::String(a.cli_dir.to_string_lossy().into_owned())).collect();
    let uses = match &config.workers {
        WorkersUse::Same => "same".to_string(),
        WorkersUse::Other => "other".to_string(),
        WorkersUse::Account(slug) => match config.accounts.iter().find(|a| &a.slug == slug) {
            Some(a) => a.cli_dir.to_string_lossy().into_owned(),
            None => "same".to_string(),
        },
    };
    object.insert("claudeAccounts".into(), Value::Array(dirs));
    object.insert("claudeWorkersUse".into(), Value::String(uses));
    // Non-Switch writers may not honor this lock. Refuse a change we can observe rather than
    // replacing it with a stale merge; the retry will load the newer bridge file.
    if bridge_bytes(&path)? != baseline {
        return Err(Refusal::Invalid("The worker bridge changed during sync.".into()));
    }
    write_json(&path, &Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_checked, reset, save};
    use crate::util::read_json;
    use crate::util::testing::{TempDir, config_with, put};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn usage_permits_wait_for_the_same_settings_lock_without_reporting_network_failure() {
        let t = TempDir::new("usage-lock-contention");
        let roots = t.roots();
        let cfg = config_with(&roots, &[("work", "Work")]);
        save(&roots, &cfg).unwrap();
        let first = usage_permit(&roots, &cfg.accounts[0].cli_dir).unwrap();
        assert_eq!(
            usage_permit_until(&roots, &cfg.accounts[1].cli_dir, Instant::now(), || true).unwrap_err(),
            Error::SettingsBusy
        );
        std::thread::scope(|scope| {
            let (contending, started) = mpsc::channel();
            let roots = &roots;
            let dir = &cfg.accounts[1].cli_dir;
            let second = scope.spawn(move || {
                let checks = std::cell::Cell::new(0);
                // The second callback is reached only after the first lock attempt was blocked.
                usage_permit_until(roots, dir, Instant::now() + Duration::from_secs(5), || {
                    checks.set(checks.get() + 1);
                    if checks.get() == 2 {
                        contending.send(()).unwrap();
                    }
                    true
                })
            });
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(first);
            assert!(second.join().unwrap().is_ok());
        });
    }

    #[test]
    fn worker_sync_uses_committed_settings_and_is_retryable_after_reset() {
        let t = TempDir::new("bridge-committed");
        let roots = t.roots();
        let mut config = config_with(&roots, &[("work", "Work")]);
        config.workers = WorkersUse::Other;
        save(&roots, &config).unwrap();
        put(&roots.workers_file(), r#"{"foreign":42}"#);
        sync_workers(&roots).unwrap();
        assert_eq!(read_json(&roots.workers_file()).unwrap()["claudeWorkersUse"], "other");
        put(&roots.config_file(), "broken settings");
        assert!(sync_workers(&roots).is_err());
        assert_eq!(read_json(&roots.workers_file()).unwrap()["claudeWorkersUse"], "other");
        reset(&roots).unwrap();
        let lock = lock_beside(&roots.workers_file()).unwrap();
        assert!(sync_workers(&roots).is_err());
        drop(lock);
        sync_workers(&roots).unwrap();
        let bridge = read_json(&roots.workers_file()).unwrap();
        assert_eq!(bridge["foreign"], 42);
        assert_eq!(bridge["claudeWorkersUse"], "same");
        assert_eq!(bridge["claudeAccounts"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn oversized_settings_and_marks_refuse_recovery_without_replacement() {
        let t = TempDir::new("oversized-app-metadata");
        let roots = t.roots();
        let config = roots.config_file();
        let marks = config.with_file_name("marks.json");
        let len = crate::APP_FILE_LIMIT as u64 + 1;
        for path in [&config, &marks] {
            put(path, "{}");
            std::fs::OpenOptions::new().write(true).open(path).unwrap().set_len(len).unwrap();
        }
        assert!(load_checked(&roots).1);
        assert!(reset(&roots).unwrap_err().to_string().contains("size limit"));
        assert!(crate::marks::Marks::load(&roots).unwrap_err().to_string().contains("size limit"));
        assert_eq!(std::fs::metadata(config).unwrap().len(), len);
        assert_eq!(std::fs::metadata(marks).unwrap().len(), len);
    }
}
