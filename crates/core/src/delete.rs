//! Deleting a session: its transcript, and the folder beside it that belongs only to it
//! (`<session id>/`: subagents, tool results), go to the Windows Recycle Bin, never further.
//! The desktop app's own records are not touched. Marks and kept summaries are the window's to
//! remove (`Marks::forget`, `meaning::Cache`). See DESIGN.md, "Deleting a session".

use std::io;
use std::path::{Path, PathBuf};

use crate::{Config, Refusal, Roots};

/// Where deleted files go. The real one is the Recycle Bin; tests use a folder of their own, so
/// no test ever touches the real Recycle Bin.
pub trait Bin: Send + Sync {
    /// Moves one file or folder (with everything in it) out of the way, recoverably.
    fn recycle(&self, path: &Path) -> io::Result<()>;
}

/// The Windows Recycle Bin, through the shell (`SHFileOperationW` with `FOF_ALLOWUNDO`). Where
/// the shell would have to delete permanently (no Recycle Bin on the drive, or a file too large
/// for it), it asks first (`FOF_WANTNUKEWARNING`); declining keeps the file and is reported.
pub struct RecycleBin;

impl Bin for RecycleBin {
    #[cfg(windows)]
    fn recycle(&self, path: &Path) -> io::Result<()> {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::UI::Shell::{
            FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
            SHFILEOPSTRUCTW, SHFileOperationW,
        };
        let plain = shell_path(path);
        if !plain.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a full path"));
        }
        // A list of paths, each ended by a NUL, the whole list by another.
        let mut from: Vec<u16> = plain.as_os_str().encode_wide().collect();
        if from.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "the path holds a NUL"));
        }
        from.extend([0, 0]);
        let mut op = SHFILEOPSTRUCTW {
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING) as u16,
            ..Default::default()
        };
        // SAFETY: `op` points at `from`, a double-NUL-terminated list that outlives the call.
        let code = unsafe { SHFileOperationW(&mut op) };
        if op.fAnyOperationsAborted != 0 {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "it was not moved (cancelled)"));
        }
        if code != 0 {
            return Err(io::Error::other(format!("the shell refused it (code 0x{code:x})")));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn recycle(&self, _path: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "there is no Recycle Bin here"))
    }
}

/// The shell takes no `\\?\` prefix, which `canonicalize` adds.
fn shell_path(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// What deleting a session moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The one real transcript file, after following links: the same transcript reached from
    /// several accounts through the shared `projects` junction is this one file.
    pub transcript: PathBuf,
    /// `<session id>/` beside the transcript, when it is a real folder.
    pub folder: Option<PathBuf>,
    /// That folder is a link (a junction or symbolic link): left in place, never followed.
    pub linked_folder: Option<PathBuf>,
}

/// What deleting `id`, whose transcript the index found at `file`, would move. Refuses anything
/// that is not a session transcript in a `projects/<folder>/` folder: a different id, a link, a
/// folder, a missing file.
pub fn plan(file: &Path, id: &str) -> Result<Plan, Refusal> {
    crate::launch::validate_session_id(id)?;
    let named = file.file_stem().and_then(|s| s.to_str()).is_some_and(|stem| stem.eq_ignore_ascii_case(id))
        && file.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl"));
    if !named {
        return Err(Refusal::Invalid("That file is not this session's transcript. Nothing was deleted.".into()));
    }
    let gone = |e: io::Error| Refusal::Io(format!("Could not read {}: {e}. Nothing was deleted.", file.display()));
    let meta = std::fs::symlink_metadata(file).map_err(gone)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Refusal::Invalid(format!(
            "{} is not a plain file (a link or a folder). Nothing was deleted.",
            file.display()
        )));
    }
    let transcript = std::fs::canonicalize(file).map_err(gone)?;
    let folder_of =
        transcript.parent().filter(|p| p.parent().and_then(Path::file_name).is_some_and(|n| n == "projects"));
    let Some(parent) = folder_of else {
        return Err(Refusal::Invalid(format!(
            "{} is not in a Claude Code projects folder. Nothing was deleted.",
            transcript.display()
        )));
    };
    let own = parent.join(transcript.file_stem().unwrap_or_default());
    let (folder, linked_folder) = match std::fs::symlink_metadata(&own) {
        Ok(m) if m.file_type().is_symlink() => (None, Some(own)),
        Ok(m) if m.is_dir() => (Some(own), None),
        _ => (None, None),
    };
    Ok(Plan { transcript, folder, linked_folder })
}

/// What a deletion did: the transcript is in the Recycle Bin; the folder may not be.
#[derive(Debug)]
pub struct Outcome {
    pub plan: Plan,
    /// Why the session's own folder was not moved, when it was not.
    pub folder_error: Option<String>,
}

/// Deletes session `id` (transcript at `file`) into `bin`. Under the launch lock, the running
/// check of every account is made again, then `also` (the window's own checks: pins, tabs);
/// any of them refuses and nothing is moved. The transcript goes first: if it does not move,
/// nothing else does. A folder that then does not move is reported in [`Outcome::folder_error`].
pub fn delete(
    roots: &Roots,
    config: &Config,
    id: &str,
    file: &Path,
    bin: &dyn Bin,
    also: impl FnOnce() -> Result<(), String>,
) -> Result<Outcome, Refusal> {
    crate::launch::with_delete_check(roots, config, id, || {
        also().map_err(Refusal::Invalid)?;
        let plan = plan(file, id)?;
        recycle(bin, &plan.transcript).map_err(|e| {
            Refusal::Io(format!("The transcript was not moved to the Recycle Bin ({e}). Nothing was deleted."))
        })?;
        let folder_error = plan.folder.as_ref().and_then(|f| recycle(bin, f).err()).map(|e| e.to_string());
        Ok(Outcome { plan, folder_error })
    })
}

/// Moves `path` and checks it is gone.
fn recycle(bin: &dyn Bin, path: &Path) -> io::Result<()> {
    bin.recycle(path)?;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(io::Error::other("it is still there")),
    }
}

/// A stand-in Recycle Bin for tests: moves into a folder of its own and records what it moved.
/// It refuses paths outside `allowed`, so a test can never move anything else.
#[doc(hidden)]
pub struct FolderBin {
    pub dir: PathBuf,
    pub allowed: PathBuf,
    pub moved: std::sync::Mutex<Vec<PathBuf>>,
    /// Refuse paths whose name is this (a failure to report).
    pub refuse: Option<String>,
}

impl FolderBin {
    pub fn new(dir: PathBuf, allowed: PathBuf) -> Self {
        Self { dir, allowed, moved: Default::default(), refuse: None }
    }

    pub fn moved(&self) -> Vec<PathBuf> {
        self.moved.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Bin for FolderBin {
    fn recycle(&self, path: &Path) -> io::Result<()> {
        let allowed = std::fs::canonicalize(&self.allowed)?;
        if !std::fs::canonicalize(path)?.starts_with(&allowed) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "outside the test folder"));
        }
        if self.refuse.as_deref().is_some_and(|name| path.file_name().is_some_and(|n| n == name)) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused by the test"));
        }
        std::fs::create_dir_all(&self.dir)?;
        let mut moved = self.moved.lock().unwrap_or_else(|e| e.into_inner());
        let to = self.dir.join(format!("{}-{}", moved.len(), path.file_name().unwrap_or_default().to_string_lossy()));
        std::fs::rename(path, &to)?;
        moved.push(path.to_path_buf());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
