//! The folders the New session dialog offers: those of the sessions that are neither archived nor
//! hidden and that still exist, one per folder, newest activity first; and the filter over them.
//! Existence is read on a worker, so opening the dialog waits for no disk. No drawing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use claude_switch_core::Session;
use claude_switch_core::sessions::{folder_key, folder_name};
use mdshared_app::fuzzy;
use mdshared_app::job::Job;

use crate::state::AppState;

/// How many folders the list holds at once; the filter searches all of them.
pub const LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub struct Folder {
    /// The spelling of the newest session, cleaned.
    pub path: PathBuf,
    /// What two spellings of one folder share ([`folder_key`] of the cleaned path).
    pub key: String,
    pub name: String,
    /// The path above `name`; empty for a root.
    pub parent: String,
    pub last_active_ms: u64,
}

/// What the last look at the disk found.
#[derive(Default)]
pub struct Existence {
    /// Keys of the folders that were not there. A folder not yet looked at is taken to exist.
    missing: HashSet<String>,
    job: Option<Job<HashSet<String>>>,
}

/// One spelling for a Windows path: backslashes, no verbatim prefix, no repeated or trailing
/// separator and no `.` component. The disk is not read.
pub fn clean(cwd: &Path) -> PathBuf {
    let text = cwd.to_string_lossy().replace('/', "\\");
    let text = match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        Some(rest) => rest.strip_prefix(r"UNC\").map_or_else(|| text.clone(), |share| format!(r"\\{share}")),
        None => text,
    };
    Path::new(&text).components().collect()
}

/// The keys of those `folders` that are not directories now. Reads the disk.
fn missing_of(folders: HashMap<String, PathBuf>) -> HashSet<String> {
    folders.into_iter().filter(|(_, path)| !path.is_dir()).map(|(key, _)| key).collect()
}

/// The indices of the folders `query` asks for, at most [`LIMIT`]: all, newest first, for a blank
/// query; else those whose path holds every word of it, newest first; when none does, those it
/// matches fuzzily, best first.
pub fn matching(all: &[Folder], query: &str) -> Vec<usize> {
    let query = query.trim().replace('/', "\\").to_lowercase();
    let query = query.trim_end_matches('\\');
    let words: Vec<&str> = query.split_whitespace().collect();
    let paths: Vec<String> = all.iter().map(|f| f.path.to_string_lossy().to_lowercase()).collect();
    let mut found: Vec<usize> = (0..all.len()).filter(|&i| words.iter().all(|w| paths[i].contains(w))).collect();
    if found.is_empty() {
        let letters = words.concat();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        found = fuzzy::rank(&letters, &paths, LIMIT).into_iter().map(|(i, _)| i).collect();
    }
    found.truncate(LIMIT);
    found
}

impl AppState {
    /// Every folder the New session dialog may offer, newest activity first. A session counts
    /// unless it is archived here, archived in the desktop app while the setting does not show
    /// those, or hidden, itself or by its folder.
    pub fn recent_folders(&self) -> Vec<Folder> {
        let hidden: HashSet<String> = self
            .sessions
            .iter()
            .filter(|s| self.marks.folder_hidden(&folder_key(&s.cwd)))
            .map(|s| folder_key(&clean(&s.cwd)))
            .collect();
        let counts = |s: &&Session| {
            !s.cwd.as_os_str().is_empty()
                && !self.is_archived(s)
                && (self.config.show_archived || !s.archived)
                && !self.hidden_session(s)
        };
        let mut found: HashMap<String, (u64, PathBuf)> = HashMap::new();
        for s in self.sessions.iter().filter(counts) {
            let path = clean(&s.cwd);
            let key = folder_key(&path);
            if hidden.contains(&key) || self.marks.folder_hidden(&key) || self.folders.missing.contains(&key) {
                continue;
            }
            match found.get_mut(&key) {
                Some(newest) if newest.0 >= s.last_active_ms => {}
                Some(newest) => *newest = (s.last_active_ms, path),
                None => {
                    found.insert(key, (s.last_active_ms, path));
                }
            }
        }
        let mut folders: Vec<Folder> = found
            .into_iter()
            .map(|(key, (last_active_ms, path))| Folder {
                // A root has nothing above it and is named in full, separator and all.
                name: if path.parent().is_some() { folder_name(&path) } else { path.to_string_lossy().into_owned() },
                parent: path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                key,
                path,
                last_active_ms,
            })
            .collect();
        folders.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms).then_with(|| a.key.cmp(&b.key)));
        folders
    }

    /// Looks again, on a worker, at which of the sessions' folders exist. The demo's folders are
    /// made up and count as existing.
    pub fn check_folders(&mut self) {
        if !self.source.is_demo() {
            self.start_folder_check();
        }
    }

    fn start_folder_check(&mut self) {
        // A check still running (a slow network path) is not doubled.
        if self.folders.job.is_some() {
            return;
        }
        let folders: HashMap<String, PathBuf> = self
            .sessions
            .iter()
            .filter(|s| !s.cwd.as_os_str().is_empty())
            .map(|s| {
                let path = clean(&s.cwd);
                (folder_key(&path), path)
            })
            .collect();
        self.folders.job = Job::start(self.wake_fn(), move |_, _| missing_of(folders)).ok();
    }

    /// Takes a finished folder check. True when it changed what exists.
    pub fn take_folder_check(&mut self) -> bool {
        let Some(job) = self.folders.job.take_if(|job| !job.running()) else { return false };
        let Some(missing) = job.take() else { return false };
        if missing == self.folders.missing {
            return false;
        }
        self.folders.missing = missing;
        self.reload_new_session();
        true
    }
}

#[cfg(test)]
mod tests;
