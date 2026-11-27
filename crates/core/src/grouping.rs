//! Local project, lineage, activity and prompt relationships.

use crate::sessions::{folder_key, folder_name};
use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
pub mod local;

/// Open a named regular file on a local fixed volume, refusing reparse points on Windows.
/// `read` is false for existence checks that do not need access to the file's contents.
pub fn open_local_file(path: &Path, read: bool) -> Option<std::fs::File> {
    local::Volume::at(path)?.file(path, read)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Recent,
    Projects,
    Threads,
    Topics,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Recent, Self::Projects, Self::Threads, Self::Topics];

    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Projects => "Projects",
            Self::Threads => "Threads",
            Self::Topics => "Topics",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    pub first_active: Option<u64>,
    pub first_prompt: Option<String>,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub key: String,
    pub name: String,
    pub branch: Option<String>,
    pub location: Option<String>,
}

impl Project {
    pub fn folder(folder: &Path) -> Self {
        Self { root: folder.into(), key: folder_key(folder), name: folder_name(folder), branch: None, location: None }
    }
}

fn small_text(volume: &local::Volume, path: &Path) -> Option<String> {
    let file = volume.open(path, true).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(8193).read_to_string(&mut text).ok()?;
    (text.len() <= 8192).then_some(text)
}

fn pointer(volume: &local::Volume, base: &Path, value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    volume.folder(&base.join(value))
}

/// No Git invocation. A malformed repository marker is a boundary, not permission to merge into an
/// outer repository. A submodule with its own gitdir but no commondir remains its own project.
pub fn project(folder: &Path) -> Project {
    let fallback = || Project::folder(folder);
    let Some(volume) = local::Volume::at(folder) else { return fallback() };
    let Some(cwd) = volume.folder(folder) else { return fallback() };
    for tree in cwd.ancestors() {
        let marker = tree.join(".git");
        let meta = match volume.metadata(&marker) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return fallback(),
        };
        let gitdir = if meta.is_dir() {
            marker
        } else if meta.is_file() {
            let Some(dir) =
                small_text(&volume, &marker).and_then(|s| pointer(&volume, tree, s.trim().strip_prefix("gitdir:")?))
            else {
                return fallback();
            };
            dir
        } else {
            return fallback();
        };
        let common_file = gitdir.join("commondir");
        let (root, common) = match volume.metadata(&common_file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (tree.to_owned(), gitdir),
            Ok(meta) if meta.is_file() => {
                let Some(common) = small_text(&volume, &common_file).and_then(|s| pointer(&volume, &gitdir, &s)) else {
                    return fallback();
                };
                // Ordinary worktree: <main>/.git. With a separate Git directory the worktree's
                // metadata cannot prove a main checkout, so don't merge it with an arbitrary folder.
                let Some(root) = common.parent().filter(|_| common.file_name().is_some_and(|s| s == ".git")) else {
                    return fallback();
                };
                (root.to_owned(), common)
            }
            _ => return fallback(),
        };
        let head = common.join("HEAD");
        let branch = match volume.open(&head, true) {
            Ok(file) if file.metadata().is_ok_and(|m| m.is_file()) => {
                let mut text = String::new();
                if file.take(8193).read_to_string(&mut text).is_err() || text.len() > 8192 {
                    return fallback();
                }
                text.trim().strip_prefix("ref: refs/heads/").filter(|s| !s.is_empty()).map(str::to_owned)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            _ => return fallback(),
        };
        let relative = cwd.strip_prefix(tree).ok().filter(|p| !p.as_os_str().is_empty());
        let location = match (tree == root, relative) {
            (true, None) => None,
            (true, Some(p)) => Some(p.display().to_string()),
            (false, None) => Some(format!("Worktree: {}", folder_name(tree))),
            (false, Some(p)) => Some(format!("Worktree: {} / {}", folder_name(tree), p.display())),
        };
        return Project { branch, location, ..Project::folder(&root) };
    }
    fallback()
}

/// Whether `folder` is in a linked Git worktree: the first `.git` at or above it is a file that
/// points into a `worktrees` directory (a submodule's points into `modules`). Only that file is
/// read, so a worktree whose main repository has gone still counts. Paths [`project`] refuses
/// (network, redirected) are not probed and count as no worktree.
pub fn in_linked_worktree(folder: &Path) -> bool {
    let Some(volume) = local::Volume::at(folder) else { return false };
    let Some(cwd) = volume.folder(folder) else { return false };
    for tree in cwd.ancestors() {
        let marker = tree.join(".git");
        match volume.metadata(&marker) {
            Ok(meta) if meta.is_file() => {
                return small_text(&volume, &marker).is_some_and(|text| {
                    let gitdir = text.trim().strip_prefix("gitdir:").map(|dir| Path::new(dir.trim()));
                    let above = gitdir.and_then(Path::parent).and_then(Path::file_name);
                    above.is_some_and(|name| name.eq_ignore_ascii_case("worktrees"))
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return false,
        }
    }
    false
}

/// One root per node. Unknown parents are ignored. Discard every edge in a cycle, including
/// self-links, without losing acyclic descendants. Iterative and linear even for long chains.
pub fn thread_roots(parents: &[Option<usize>]) -> Vec<usize> {
    let mut roots = vec![None; parents.len()];
    let mut path_pos = vec![None; parents.len()];
    for i in 0..parents.len() {
        let mut path = Vec::new();
        let mut at = i;
        let root = loop {
            if let Some(root) = roots[at] {
                break root;
            }
            if let Some(start) = path_pos[at] {
                for &node in &path[start..] {
                    roots[node] = Some(node);
                }
                break at;
            }
            path_pos[at] = Some(path.len());
            path.push(at);
            let Some(parent) = parents[at].filter(|&p| p < parents.len()) else { break at };
            at = parent;
        };
        for node in path {
            roots[node].get_or_insert(root);
            path_pos[node] = None;
        }
    }
    roots.into_iter().enumerate().map(|(i, root)| root.unwrap_or(i)).collect()
}

#[derive(Clone, Copy)]
pub struct Activity<'a> {
    pub project: &'a str,
    pub first: Option<u64>,
    pub last: u64,
}

/// A cluster ID (an input row index) per activity. Clip overlapping ranges to the local day of
/// newest activity. Unknown starts count as point activity, and unknown end times never cluster.
pub fn sittings(activity: &[Activity<'_>], offset: i32) -> Vec<usize> {
    const DAY: i128 = 86_400_000;
    const GAP: u64 = 30 * 60_000;
    let shift = i128::from(offset) * 60_000;
    let mut rows = Vec::new();
    let mut result: Vec<_> = (0..activity.len()).collect();
    for (i, a) in activity.iter().enumerate().filter(|(_, a)| a.last != 0) {
        let day = (i128::from(a.last) + shift).div_euclid(DAY);
        let midnight = (day * DAY - shift).max(0) as u64;
        let first = a.first.unwrap_or(a.last).clamp(midnight, a.last);
        rows.push((a.project, day, first, a.last, i));
    }
    rows.sort_unstable();
    let mut current: Option<(&str, i128, u64, usize)> = None;
    for (project, day, first, last, i) in rows {
        if let Some((key, date, end, id)) = current
            && project == key
            && day == date
            && first <= end.saturating_add(GAP)
        {
            result[i] = id;
            current = Some((key, date, end.max(last), id));
        } else {
            current = Some((project, day, last, i));
        }
    }
    result
}

pub fn words(first: &str, last: &str) -> HashSet<String> {
    const COMMON: &[&str] = &[
        "about", "after", "all", "also", "and", "are", "but", "can", "could", "does", "for", "from", "has", "have",
        "how", "into", "its", "just", "like", "make", "more", "not", "now", "only", "our", "please", "should", "some",
        "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "those", "use", "using",
        "want", "was", "were", "what", "when", "where", "which", "will", "with", "would", "you", "your",
    ];
    [first, last]
        .into_iter()
        .flat_map(|s| s.split(|c: char| !c.is_alphanumeric()))
        .filter(|s| s.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|s| !COMMON.contains(&s.as_str()))
        .collect()
}

/// Jaccard overlap in thousandths; repeated words cannot inflate the score.
pub fn similarity(a: &HashSet<String>, b: &HashSet<String>) -> usize {
    let overlap = a.intersection(b).count();
    1000 * overlap / (a.len() + b.len() - overlap).max(1)
}

pub fn valid_topic(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()
        && text.chars().count() <= 60
        && text.split_whitespace().count() <= 3
        && text.chars().any(char::is_alphanumeric)
        && !text.chars().any(|c| c.is_control() || matches!(c, '"' | '`' | '“' | '”')))
    .then_some(text)
}

/// Resolve each distinct working folder once per refresh, including previous fallbacks.
pub fn projects_for_scan<'a>(folders: impl IntoIterator<Item = &'a Path>) -> HashMap<String, Project> {
    let mut projects = HashMap::new();
    for folder in folders {
        projects.entry(folder_key(folder)).or_insert_with(|| project(folder));
    }
    projects
}

#[cfg(test)]
mod tests;
