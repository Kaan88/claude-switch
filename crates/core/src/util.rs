//! Small private helpers shared by the modules: tolerant JSON reading, writing a file in one step, and
//! the temporary folders of the tests.

use crate::Refusal;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn lock_beside(path: &Path) -> Result<std::fs::File, Refusal> {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let lock = PathBuf::from(name);
    ensure_parent(&lock)?;
    regular_target(&lock)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .map_err(|e| io_refusal("open the lock", &lock, &e))?;
    file.try_lock().map_err(|e| io_refusal("lock", &lock, &e))?;
    Ok(file)
}

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
pub(crate) fn temp_beside(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    PathBuf::from(tmp)
}

pub(crate) fn publish_new(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    ensure_parent(path)?;
    write_new_atomic(path, bytes)
}

/// Refuse any existing non-file or reparse-point entry, including dangling links.
pub(crate) fn regular_target(path: &Path) -> Result<(), Refusal> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_refusal("inspect", path, &e)),
    };
    if !metadata.is_file() || reparse(&metadata) {
        return Err(io_refusal("replace", path, &"not a regular file"));
    }
    Ok(())
}

pub(crate) fn reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

pub(crate) fn ensure_parent(path: &Path) -> Result<(), Refusal> {
    let dirs: Vec<_> = path.ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()).collect();
    for dir in dirs.into_iter().rev() {
        let check = || std::fs::symlink_metadata(dir);
        let meta = match check() {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(dir) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(io_refusal("create the folder", dir, &e)),
                }
                check().map_err(|e| io_refusal("inspect", dir, &e))?
            }
            Err(e) => return Err(io_refusal("inspect", dir, &e)),
        };
        if !meta.is_dir() || reparse(&meta) {
            return Err(io_refusal("use the folder", dir, &"not a regular directory"));
        }
    }
    Ok(())
}

/// Never resolves the destination; rename replaces the entry, not a link target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    ensure_parent(path)?;
    regular_target(path)?;
    let tmp = temp_beside(path);
    write_through_temp(path, &tmp, bytes)
}

pub fn replace_if_unchanged(path: &Path, baseline: Option<&[u8]>, bytes: &[u8]) -> Result<(), Refusal> {
    let _lock = lock_beside(path)?;
    regular_target(path)?;
    let current = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_refusal("read", path, &e)),
    };
    if current.as_deref() != baseline {
        return Err(Refusal::Invalid("Saved tabs changed in another window; the newer snapshot was kept.".into()));
    }
    write_atomic(path, bytes)
}

fn write_through_temp(path: &Path, tmp: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    let mut file = std::fs::File::create_new(tmp).map_err(|e| io_refusal("create temporary file", tmp, &e))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    written
        .map_err(|e| io_refusal("write", tmp, &e))
        .and_then(|()| regular_target(path))
        .and_then(|()| std::fs::rename(tmp, path).map_err(|e| io_refusal("replace", path, &e)))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(tmp);
        })
}

/// Same-directory staging, but atomically publish only if the user-chosen destination is absent.
pub(crate) fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    let dir = path.parent().ok_or_else(|| io_refusal("save", path, &"missing parent folder"))?;
    let name = path.file_name().ok_or_else(|| io_refusal("save", path, &"missing file name"))?;
    let dir = std::fs::canonicalize(dir).map_err(|e| io_refusal("read the destination folder", dir, &e))?;
    let path = dir.join(name);
    let tmp = temp_beside(&path);
    let mut file = std::fs::File::create_new(&tmp).map_err(|e| io_refusal("create", &tmp, &e))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.map_err(|e| io_refusal("write", &tmp, &e)).and_then(|()| {
        publish_without_replacing(&tmp, &path)
            .map_err(|e| io_refusal("save a new file (choose an unused name)", &path, &e))
    });
    if !cfg!(windows) || result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(windows)]
fn publish_without_replacing(staged: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let (staged, target) = (wide(staged), wide(path));
    // SAFETY: both buffers are live NUL-terminated UTF-16 paths; MoveFileW fails if the
    // destination already exists. Both names resolve within the same canonicalized directory.
    if unsafe { MoveFileW(staged.as_ptr(), target.as_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_without_replacing(staged: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::hard_link(staged, path)
}

/// Writes a JSON value, pretty-printed, with [`write_atomic`].
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), Refusal> {
    write_atomic(path, &json_bytes(path, value)?)
}

pub(crate) fn json_bytes(path: &Path, value: &Value) -> Result<Vec<u8>, Refusal> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| io_refusal("encode", path, &e))?;
    text.push('\n');
    Ok(text.into_bytes())
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
    #[test]
    fn compare_and_replace_preserves_another_windows_snapshot() {
        let t = super::testing::TempDir::new("snapshot-revision");
        let p = t.0.join("terminal-tabs.json");
        super::replace_if_unchanged(&p, None, b"[1]").unwrap();
        assert!(super::replace_if_unchanged(&p, None, b"[]").is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"[1]");
        super::replace_if_unchanged(&p, Some(b"[1]"), b"[]").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"[]");
    }

    #[cfg(windows)]
    #[test]
    fn app_file_writes_refuse_redirected_parent_directories() {
        let t = super::testing::TempDir::new("app-write-junction");
        let real = t.0.join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = t.0.join("alias");
        crate::share::junction::create(&alias, &real).unwrap();
        assert!(super::write_atomic(&alias.join("window.ini"), b"no").is_err());
        assert!(!real.join("window.ini").exists());
        std::fs::remove_dir(alias).unwrap();
    }
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
    fn a_write_refuses_a_symlink_and_leaves_its_target_alone() {
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
        assert!(write_atomic(&link, b"new").is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "old");
        assert_eq!(names(&t.0.join("real")), ["a.json"]);
    }

    #[test]
    fn a_preexisting_temporary_name_is_not_opened_or_removed() {
        let t = TempDir::new("occupied-temp");
        let tmp = t.0.join("tmp");
        put(&tmp, "keep");
        assert!(write_through_temp(&t.0.join("data"), &tmp, b"new").is_err());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"keep");
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
