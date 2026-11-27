//! Folders for tests under the system's temporary folder.
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// `<tag>-<pid>-<n>-<nanos>` under the temporary folder, not created. No other test has it: not
/// one of this run, nor of a run beside it, nor of an earlier run whose process had the same id
/// (Windows soon gives an ended process's id to a new one).
pub(crate) fn unique(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("{tag}-{}-{n}-{nanos}", std::process::id()))
}

/// A [`unique`] folder (not created) that goes, with all it holds, when the test ends, also when
/// it fails.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(tag: &str) -> Scratch {
        Scratch(unique(tag))
    }

    /// The same, created.
    pub(crate) fn created(tag: &str) -> Scratch {
        let scratch = Scratch::new(tag);
        std::fs::create_dir_all(&scratch.0).unwrap();
        scratch
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_scratch_folder_is_its_own_and_goes_when_dropped() {
    let (a, b) = (Scratch::created("cs-scratch"), Scratch::new("cs-scratch"));
    assert_ne!(*a, *b);
    std::fs::write(a.join("file"), "x").unwrap();
    let path = a.to_path_buf();
    drop(a);
    assert!(!path.exists());
}
