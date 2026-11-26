//! Small private helpers shared by the modules: tolerant JSON reading, writing a file in one step, and
//! the temporary folders of the tests.

use crate::Refusal;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The JSON in a file, `None` when it is missing or does not parse.
pub(crate) fn read_json(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A string field of a JSON object, `None` when missing, not a string or empty.
pub(crate) fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A [`Refusal::Io`] naming the path and the reason.
pub(crate) fn io_refusal(what: &str, path: &Path, err: &dyn std::fmt::Display) -> Refusal {
    Refusal::Io(format!("Could not {what} {}: {err}.", path.display()))
}

/// A name beside `path` that no other call, of this process or another running one, uses.
fn temp_beside(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    PathBuf::from(tmp)
}

/// Writes `bytes` to a temporary file beside `path`, flushes it to disk and renames it over `path`,
/// creating the folder. When `path` is a symlink to a file that exists, that file is replaced and the
/// link stays.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    let linked = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let resolved = linked.then(|| std::fs::canonicalize(path).ok()).flatten();
    let path = resolved.as_deref().unwrap_or(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_refusal("create the folder", dir, &e))?;
    }
    let tmp = temp_beside(path);
    let written = std::fs::File::create(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    written
        .map_err(|e| io_refusal("write", &tmp, &e))
        .and_then(|()| std::fs::rename(&tmp, path).map_err(|e| io_refusal("replace", path, &e)))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// Writes a JSON value, pretty-printed, with [`write_atomic`].
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), Refusal> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| io_refusal("encode", path, &e))?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// White space runs as one space, trimmed, cut to `max` characters.
pub(crate) fn squeeze(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        if count > 0 {
            if count >= max {
                break;
            }
            out.push(' ');
            count += 1;
        }
        for c in word.chars() {
            if count >= max {
                break;
            }
            out.push(c);
            count += 1;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

#[cfg(test)]
mod tests {
    use super::testing::{TempDir, put};
    use super::*;

    /// The names in a folder, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn every_write_has_its_own_temporary_name_and_leaves_none_behind() {
        let t = TempDir::new("utl-write");
        let file = t.0.join("a.json");
        let (first, second) = (temp_beside(&file), temp_beside(&file));
        assert_ne!(first, second);
        assert!(first.to_string_lossy().contains(&format!(".{}.", std::process::id())));
        assert_eq!(first.parent(), file.parent());
        // A file under the old fixed name is not this write's to touch.
        put(&t.0.join("a.json.tmp"), "another's");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(t.0.join("a.json.tmp")).unwrap(), "another's");
        assert_eq!(names(&t.0), ["a.json", "a.json.tmp"]);
    }

    #[test]
    fn a_failed_rename_removes_the_temporary_file() {
        let t = TempDir::new("utl-fail");
        let target = t.0.join("taken");
        put(&target.join("inside"), "x");
        assert!(matches!(write_atomic(&target, b"new"), Err(Refusal::Io(_))));
        assert_eq!(names(&t.0), ["taken"]);
    }

    #[test]
    fn a_write_to_a_symlink_goes_to_the_file_it_points_at() {
        let t = TempDir::new("utl-link");
        let real = t.0.join("real").join("a.json");
        put(&real, "old");
        let link = t.0.join("link.json");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&real, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real, &link);
        // Windows lets only some users make a file symlink; without one there is nothing to check.
        if let Err(e) = made {
            eprintln!("skipped: no file symlink can be made here ({e})");
            return;
        }
        write_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(names(&t.0.join("real")), ["a.json"]);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::{Account, Config, Roots};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A folder of its own under the system's temporary folder, removed when dropped.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> TempDir {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
            let dir = std::env::temp_dir().join(format!("cs-core-{tag}-{}-{n}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        pub(crate) fn roots(&self) -> Roots {
            Roots { home: self.0.join("home"), appdata: self.0.join("appdata") }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a file, creating its folders.
    pub(crate) fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A further account's value, with the standard folders under `roots`, without touching the disk.
    pub(crate) fn further(roots: &Roots, slug: &str, name: &str) -> Account {
        let base = roots.accounts_dir().join(slug);
        Account {
            slug: slug.into(),
            name: name.into(),
            color: [1, 2, 3],
            primary: false,
            cli_dir: base.join("cli"),
            desktop_dir: base.join("desktop"),
        }
    }

    /// The default config for `roots` plus further accounts.
    pub(crate) fn config_with(roots: &Roots, further_accounts: &[(&str, &str)]) -> Config {
        let mut config = crate::config::load(roots);
        for (slug, name) in further_accounts {
            config.accounts.push(further(roots, slug, name));
        }
        config
    }

    /// Marks an account as logged in, with a fake `.claude.json` and an empty `.credentials.json`.
    pub(crate) fn log_in(roots: &Roots, account: &Account) {
        put(&account.cli_dir.join(".credentials.json"), "{}");
        put(
            &crate::accounts::claude_json(roots, account),
            r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"a@example.invalid","displayName":"A"}}"#,
        );
    }
}
