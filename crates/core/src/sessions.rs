//! The sessions on disk: transcripts joined with the desktop apps' records.

use crate::util::{read_json, squeeze, str_field};
use crate::{App, Config, Roots, Session, modified_ms};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

mod archive_excerpts;
mod grouping;
mod prompts;
pub use archive_excerpts::archive_excerpts;
pub use prompts::{TurnActivity, naming_prompts, row_activity, turn_activity};

/// How much of a transcript is read at its start and at its end.
const CHUNK: u64 = 256 * 1024;

/// Where a session's title came from, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TitleFrom {
    Id,
    Prompt,
    Desktop,
    Ai,
    Custom,
}

/// The lines of the head and the tail of a file, in file order, partial lines at the cut dropped.
struct Lines {
    head: Vec<u8>,
    tail: Vec<u8>,
    whole: bool,
}

impl Lines {
    /// Reads at most `head_len` bytes of the start and `CHUNK` bytes of the end. With `head_len` 0
    /// only the tail is read.
    fn read(file: &Path, head_len: u64) -> Option<Lines> {
        let mut f = std::fs::File::open(file).ok()?;
        let len = f.metadata().ok()?.len();
        if len <= head_len + CHUNK {
            let mut all = Vec::with_capacity(len as usize);
            f.read_to_end(&mut all).ok()?;
            return Some(Lines { head: all, tail: Vec::new(), whole: true });
        }
        let mut head = vec![0; head_len as usize];
        f.read_exact(&mut head).ok()?;
        if let Some(end) = head.iter().rposition(|&b| b == b'\n') {
            head.truncate(end + 1);
        } else {
            head.clear();
        }
        // One byte more than the chunk: when it is the end of a line, only it is dropped, and the line
        // that starts at the chunk's first byte is kept.
        f.seek(SeekFrom::Start(len - CHUNK - 1)).ok()?;
        let mut tail = Vec::with_capacity(CHUNK as usize + 1);
        f.take(CHUNK + 1).read_to_end(&mut tail).ok()?;
        match tail.iter().position(|&b| b == b'\n') {
            Some(start) => {
                tail.drain(..=start);
            }
            None => tail.clear(),
        }
        Some(Lines { head, tail, whole: false })
    }

    fn iter(&self) -> impl DoubleEndedIterator<Item = &[u8]> + '_ {
        let mut all = split_lines(&self.head);
        all.extend(split_lines(&self.tail));
        all.into_iter()
    }

    fn recent(&self) -> impl Iterator<Item = &[u8]> {
        let buf = if self.whole {
            let start = self.head.len().saturating_sub(CHUNK as usize);
            let tail = &self.head[start..];
            if start == 0 || self.head[start - 1] == b'\n' {
                tail
            } else {
                tail.iter().position(|&b| b == b'\n').map_or(&[][..], |i| &tail[i + 1..])
            }
        } else {
            &self.tail
        };
        buf.rsplit(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).filter(|l| !l.is_empty())
    }
}

/// The non-blank lines of a buffer, without their line ends.
fn split_lines(buf: &[u8]) -> Vec<&[u8]> {
    buf.split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
        .collect()
}

/// Whether a line holds `needle`: a cheap test before a line is parsed. A line that is not UTF-8
/// would not parse either.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    match (std::str::from_utf8(hay), std::str::from_utf8(needle)) {
        (Ok(h), Ok(n)) => h.contains(n),
        _ => false,
    }
}

fn parse(line: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(line).ok().filter(Value::is_object)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The text blocks of a message's content (a string, or blocks of which the `text` ones count).
fn texts(v: &Value) -> Vec<&str> {
    match v.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

/// The prompt a user line holds, when it is one a person typed.
fn prompt_of(v: &Value, max: usize) -> Option<String> {
    Some(squeeze(&prompt_text(v)?, max)).filter(|t| !t.is_empty())
}

fn prompt_text(v: &Value) -> Option<String> {
    if str_field(v, "type") != Some("user")
        || flag(v, "isSidechain")
        || flag(v, "isMeta")
        || flag(v, "isCompactSummary")
    {
        return None;
    }
    let parts = texts(v);
    if parts.iter().any(|s| s.trim_start().starts_with('<')) {
        return None;
    }
    Some(parts.join(" ")).filter(|t| !t.trim().is_empty())
}

fn read_inner(file: &Path) -> Option<(Session, TitleFrom)> {
    let id = file.file_stem()?.to_str()?.to_string();
    if file.extension().and_then(|e| e.to_str()) != Some("jsonl") || id.is_empty() {
        return None;
    }
    let lines = Lines::read(file, CHUNK)?;
    let transcript_ms = modified_ms(file);
    let (last_prompt, turn_ended) = prompts::activity(&lines);
    let grouping = grouping::head(&lines.head, &id);
    let (mut custom, mut ai, mut prompt) = (None::<String>, None::<String>, None::<String>);
    let mut is_session = false;
    for line in lines.iter() {
        let titled = contains(line, b"-title\"");
        let user = prompt.is_none() && contains(line, b"\"user\"");
        if !titled && !user {
            continue;
        }
        let Some(v) = parse(line) else { continue };
        match str_field(&v, "type") {
            Some("custom-title") => custom = str_field(&v, "customTitle").map(|s| squeeze(s, 200)).or(custom),
            Some("ai-title") => ai = str_field(&v, "aiTitle").map(|s| squeeze(s, 200)).or(ai),
            Some("user") if !flag(&v, "isSidechain") => {
                is_session = true;
                if prompt.is_none() {
                    prompt = prompt_of(&v, 80);
                }
            }
            _ => {}
        }
    }
    let (mut cwd, mut branch, mut entrypoint) = (None::<String>, None::<String>, None::<String>);
    for line in lines.iter().rev() {
        let needed = (!is_session || cwd.is_none()) && contains(line, b"\"cwd\"")
            || (!is_session || entrypoint.is_none()) && contains(line, b"\"entrypoint\"")
            || (!is_session || branch.is_none()) && contains(line, b"\"gitBranch\"");
        if !needed {
            continue;
        }
        let Some(v) = parse(line) else { continue };
        if flag(&v, "isSidechain") {
            continue;
        }
        if matches!(str_field(&v, "type"), Some("user" | "assistant")) {
            is_session = true;
        }
        let take = |slot: &mut Option<String>, key: &str| {
            if slot.is_none() {
                *slot = str_field(&v, key).map(str::to_string);
            }
        };
        take(&mut cwd, "cwd");
        take(&mut branch, "gitBranch");
        take(&mut entrypoint, "entrypoint");
        if is_session && cwd.is_some() && entrypoint.is_some() && branch.is_some() {
            break;
        }
    }
    if !is_session {
        return None;
    }
    let (title, from) = match (custom, ai, prompt) {
        (Some(t), _, _) => (t, TitleFrom::Custom),
        (None, Some(t), _) => (t, TitleFrom::Ai),
        (None, None, Some(t)) => (t, TitleFrom::Prompt),
        _ => (id.chars().take(8).collect(), TitleFrom::Id),
    };
    let session = Session {
        title,
        last_prompt,
        grouping,
        turn_ended,
        transcript_ms,
        cwd: cwd.map(PathBuf::from).unwrap_or_default(),
        branch,
        last_active_ms: transcript_ms,
        bytes: std::fs::metadata(file).map_or(0, |m| m.len()),
        last_app: if entrypoint.as_deref() == Some("claude-desktop") { App::Desktop } else { App::Cli },
        desktop_id: None,
        desktop_account: None,
        archived: false,
        worker: entrypoint.as_deref().is_some_and(|e| e.starts_with("sdk")),
        file: file.to_path_buf(),
        id,
    };
    Some((session, from))
}

/// The JSON files two folder levels under `<desktop_dir>/claude-code-sessions` (account, organisation).
pub(crate) fn desktop_record_files(desktop_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    for account in subdirs(&desktop_dir.join("claude-code-sessions")) {
        for org in subdirs(&account) {
            let Ok(rd) = std::fs::read_dir(&org) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("json") && e.file_type().is_ok_and(|t| t.is_file()) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Adds `dir` to `seen` by its canonical path; false when it was there already or does not exist.
pub(crate) fn first_visit(seen: &mut HashSet<PathBuf>, dir: &Path) -> bool {
    match std::fs::canonicalize(dir) {
        Ok(c) => seen.insert(c),
        Err(_) => false,
    }
}

/// Every session of every account, newest first. Reads each account's `<cli_dir>/projects` (a folder
/// reached twice through a link counts once: sessions are unique by id, and paths are compared after
/// `canonicalize`) and each account's desktop records. Skips `subagents` folders and files that hold
/// no session lines. A desktop record's `priorCliSessionIds` are not sessions of their own: they are
/// left out when the record's current `cliSessionId` has a transcript. Never fails.
pub fn scan(roots: &Roots, config: &Config) -> Vec<Session> {
    let _ = roots;
    let mut found: HashMap<String, (Session, TitleFrom)> = HashMap::new();
    let mut seen = HashSet::new();
    for account in &config.accounts {
        let projects = account.cli_dir.join("projects");
        if !first_visit(&mut seen, &projects) {
            continue;
        }
        let Ok(folders) = std::fs::read_dir(&projects) else { continue };
        for folder in folders.flatten() {
            if folder.file_name() == "subagents"
                || crate::naming::is_private_project(&folder.file_name())
                || !folder.path().is_dir()
            {
                continue;
            }
            let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
            for file in files.flatten() {
                if !file.file_type().is_ok_and(|t| t.is_file()) {
                    continue;
                }
                let Some((s, from)) = read_inner(&file.path()) else { continue };
                if crate::naming::is_private_folder(&s.cwd) {
                    continue;
                }
                match found.get(&s.id) {
                    Some((old, _)) if old.last_active_ms >= s.last_active_ms => {}
                    _ => {
                        found.insert(s.id.clone(), (s, from));
                    }
                }
            }
        }
    }

    // The desktop records, oldest activity first so that a newer record of the same transcript wins.
    let mut records: Vec<(String, Value)> = Vec::new();
    let mut seen_desktop = HashSet::new();
    for account in &config.accounts {
        if !first_visit(&mut seen_desktop, &account.desktop_dir) {
            continue;
        }
        for path in desktop_record_files(&account.desktop_dir) {
            if let Some(v) = read_json(&path).filter(|v| str_field(v, "cliSessionId").is_some()) {
                records.push((account.slug.clone(), v));
            }
        }
    }
    records.sort_by_key(|(_, v)| v.get("lastActivityAt").and_then(Value::as_u64).unwrap_or(0));
    let current: HashSet<String> =
        records.iter().filter_map(|(_, v)| str_field(v, "cliSessionId")).map(str::to_string).collect();
    let mut prior_ids: HashSet<String> = HashSet::new();
    for (slug, v) in &records {
        let cli_id = str_field(v, "cliSessionId").unwrap_or_default();
        let Some((s, from)) = found.get_mut(cli_id) else { continue };
        s.desktop_id = str_field(v, "sessionId").map(str::to_string);
        s.desktop_account = Some(slug.clone());
        s.archived = flag(v, "isArchived");
        if let Some(at) = v.get("lastActivityAt").and_then(Value::as_u64) {
            s.last_active_ms = s.last_active_ms.max(at);
        }
        if let Some(title) = str_field(v, "title").map(|t| squeeze(t, 200)).filter(|t| !t.is_empty())
            && *from <= TitleFrom::Desktop
        {
            s.title = title;
            *from = TitleFrom::Desktop;
        }
        if s.cwd.as_os_str().is_empty()
            && let Some(cwd) = str_field(v, "cwd")
        {
            s.cwd = PathBuf::from(cwd);
        }
        if let Some(prior) = v.get("priorCliSessionIds").and_then(Value::as_array) {
            prior_ids.extend(prior.iter().filter_map(Value::as_str).filter(|p| *p != cli_id).map(str::to_string));
        }
    }
    for id in prior_ids.difference(&current) {
        found.remove(id);
    }

    let mut sessions: Vec<Session> =
        found.into_values().map(|(s, _)| s).filter(|s| !crate::naming::is_private_folder(&s.cwd)).collect();
    sessions.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms).then_with(|| a.id.cmp(&b.id)));
    sessions
}

/// One transcript read alone (head and tail only, see `docs/DESIGN.md`), without desktop records.
/// `None` when the file is not a session's transcript.
pub fn read_transcript(file: &Path) -> Option<Session> {
    read_inner(file).map(|(s, _)| s)
}

/// The sessions of one working folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The folder. Empty for sessions that named none.
    pub cwd: PathBuf,
    /// The folder's last component, for a heading; `"(no folder)"` when empty.
    pub name: String,
    /// Indices into the slice that was grouped, newest first.
    pub sessions: Vec<usize>,
}

/// The key two folders are compared by: lower case, no trailing separator.
pub fn folder_key(cwd: &Path) -> String {
    let s = cwd.to_string_lossy().to_lowercase().replace('/', "\\");
    let trimmed = s.trim_end_matches('\\');
    if trimmed.is_empty() && !s.is_empty() { s } else { trimmed.to_string() }
}

/// A folder's last component, for a heading.
pub fn folder_name(cwd: &Path) -> String {
    if cwd.as_os_str().is_empty() {
        return "(no folder)".into();
    }
    let s = cwd.to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    match trimmed.rsplit(['\\', '/']).next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => s.into_owned(),
    }
}

/// Groups sessions by working folder (compared without regard to case or a trailing separator).
/// Groups are ordered by their newest session, newest first.
pub fn group_by_folder(sessions: &[Session]) -> Vec<Group> {
    let mut order: Vec<usize> = (0..sessions.len()).collect();
    order.sort_by(|&a, &b| sessions[b].last_active_ms.cmp(&sessions[a].last_active_ms).then(a.cmp(&b)));
    let mut groups: Vec<Group> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for i in order {
        let cwd = &sessions[i].cwd;
        let g = *by_key.entry(folder_key(cwd)).or_insert_with(|| {
            groups.push(Group { cwd: cwd.clone(), name: folder_name(cwd), sessions: Vec::new() });
            groups.len() - 1
        });
        groups[g].sessions.push(i);
    }
    groups
}

/// One piece of a conversation, for a preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewLine {
    /// True for what the user wrote, false for the assistant's text.
    pub from_user: bool,
    /// The text, with runs of white space as one space, cut to 400 characters.
    pub text: String,
}

/// The last `max` user prompts and assistant texts of a transcript, oldest first, read from the
/// file's tail only. Tool calls, tool results, meta lines, sidechains and prompts that start with
/// `<` are left out. Empty when nothing can be read.
pub fn preview(file: &Path, max: usize) -> Vec<PreviewLine> {
    if max == 0 {
        return Vec::new();
    }
    let Some(lines) = Lines::read(file, 0) else { return Vec::new() };
    let mut out = Vec::new();
    for line in lines.iter().rev() {
        if out.len() >= max {
            break;
        }
        if !contains(line, b"\"user\"") && !contains(line, b"\"assistant\"") {
            continue;
        }
        let Some(v) = parse(line) else { continue };
        if flag(&v, "isSidechain") || flag(&v, "isMeta") || flag(&v, "isCompactSummary") {
            continue;
        }
        let from_user = match str_field(&v, "type") {
            Some("user") => true,
            Some("assistant") => false,
            _ => continue,
        };
        let parts = texts(&v);
        if from_user && parts.iter().any(|t| t.trim_start().starts_with('<')) {
            continue;
        }
        let text = squeeze(&parts.join(" "), 400);
        if !text.is_empty() {
            out.push(PreviewLine { from_user, text });
        }
    }
    out.reverse();
    out
}

#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
