//! Scan-local parsed entries, reusable only when the open file still matches their stamp.

use super::{CHUNK, Lines, TitleFrom, prompts, read_inner, scan_cached, str_field, transcript_id};
use crate::{Config, Roots, Session};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    identity: Option<(u64, u64)>,
    size: u64,
    modified: u128,
}

impl Stamp {
    #[cfg(windows)]
    fn read(file: &File) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};

        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the file owns the open handle and info is writable for the duration of the call.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return None;
        }
        let join = |hi, lo| u64::from(hi) << 32 | u64::from(lo);
        let id = join(info.nFileIndexHigh, info.nFileIndexLow);
        Some(Self {
            identity: (id != 0).then_some((u64::from(info.dwVolumeSerialNumber), id)),
            size: join(info.nFileSizeHigh, info.nFileSizeLow),
            modified: u128::from(join(info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime)),
        })
    }

    #[cfg(not(windows))]
    fn read(file: &File) -> Option<Self> {
        let meta = file.metadata().ok()?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            Some((meta.dev(), meta.ino()))
        };
        #[cfg(not(unix))]
        let identity = None;
        let time = meta.modified().ok()?;
        // Store the sign in the low bit to distinguish pre-epoch timestamps too.
        let modified = match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => after.as_nanos() << 1,
            Err(before) => (before.duration().as_nanos() << 1) | 1,
        };
        Some(Self { identity, size: meta.len(), modified })
    }

    pub fn size(self) -> u64 {
        self.size
    }

    pub fn modified_ms(self) -> u64 {
        #[cfg(windows)]
        let ms = self.modified.saturating_sub(116_444_736_000_000_000) / 10_000;
        #[cfg(not(windows))]
        let ms = if self.modified & 1 == 0 { (self.modified >> 1) / 1_000_000 } else { 0 };
        ms as u64
    }
}

/// The second sample must come from the same handle, not whatever now lives at its path.
fn checked<T>(file: &mut File, before: Stamp, read: impl FnOnce(&mut File) -> Option<T>) -> Option<T> {
    let body = read(file)?;
    (Stamp::read(file)? == before).then_some(body)
}

pub(super) fn transcript_snapshot(path: &Path) -> Option<(Stamp, Lines)> {
    transcript_id(path)?;
    let mut file = File::open(path).ok()?;
    let stamp = Stamp::read(&file)?;
    let lines = checked(&mut file, stamp, |f| Lines::read_from(f, stamp.size, CHUNK))?;
    Some((stamp, lines))
}

/// Read a changed row's tail only. A changing or unreadable snapshot never advances the caller's
/// baseline, so a later poll can try again.
pub fn changed_row_activity(path: &Path, previous: Stamp) -> Option<(Stamp, String, bool)> {
    changed_row_activity_with(path, previous, |f, size| Lines::read_from(f, size, 0))
}

fn changed_row_activity_with(
    path: &Path,
    previous: Stamp,
    read: impl FnOnce(&mut File, u64) -> Option<Lines>,
) -> Option<(Stamp, String, bool)> {
    let mut file = File::open(path).ok()?;
    let stamp = Stamp::read(&file)?;
    if stamp == previous {
        return None;
    }
    let lines = checked(&mut file, stamp, |f| read(f, stamp.size))?;
    let (prompt, ended) = prompts::activity(&lines);
    Some((stamp, prompt, ended))
}

type Entries<T> = HashMap<PathBuf, (Stamp, Option<T>)>;

#[derive(Default)]
pub struct ScanCache {
    transcripts: Entries<(Session, TitleFrom)>,
    desktop: Entries<Value>,
    #[cfg(test)]
    body_reads: usize,
}

impl ScanCache {
    /// Only entries visited during this scan survive; errors and unstable reads are not cached.
    pub fn scan(&mut self, roots: &Roots, config: &Config) -> Vec<Session> {
        let mut next = Self::default();
        let sessions = scan_cached(roots, config, self, &mut next);
        *self = next;
        sessions
    }

    pub fn transcript_stamp(&self, path: &Path) -> Option<Stamp> {
        self.transcripts.get(path).map(|(stamp, _)| *stamp)
    }

    pub(super) fn transcript(&mut self, path: PathBuf, next: &mut Self) -> Option<(Session, TitleFrom)> {
        transcript_id(&path)?;
        #[cfg(test)]
        let reads = &mut next.body_reads;
        cached_entry(path, &mut self.transcripts, &mut next.transcripts, |path, f, stamp| {
            #[cfg(test)]
            {
                *reads += 1;
            }
            let lines = checked(f, stamp, |f| Lines::read_from(f, stamp.size, CHUNK))?;
            Some(read_inner(path, stamp, lines))
        })
    }

    pub(super) fn desktop(&mut self, path: PathBuf, next: &mut Self) -> Option<Value> {
        #[cfg(test)]
        let reads = &mut next.body_reads;
        cached_entry(path, &mut self.desktop, &mut next.desktop, |_, f, stamp| {
            #[cfg(test)]
            {
                *reads += 1;
            }
            let bytes = checked(f, stamp, |f| {
                let mut bytes = Vec::new();
                f.take(stamp.size).read_to_end(&mut bytes).ok()?;
                Some(bytes)
            })?;
            Some(serde_json::from_slice(&bytes).ok().filter(|v| str_field(v, "cliSessionId").is_some()))
        })
    }
}

fn cached_entry<T: Clone>(
    path: PathBuf,
    previous: &mut Entries<T>,
    next: &mut Entries<T>,
    read: impl FnOnce(&Path, &mut File, Stamp) -> Option<Option<T>>,
) -> Option<T> {
    let mut file = File::open(&path).ok()?;
    let stamp = Stamp::read(&file)?;
    let value = match previous.remove(&path) {
        Some((old, value)) if stamp.identity.is_some() && old == stamp => value,
        _ => read(&path, &mut file, stamp)?,
    };
    let result = value.clone();
    next.insert(path, (stamp, value));
    result
}

#[cfg(test)]
mod tests;
