//! The sessions on disk: transcripts joined with the desktop apps' records.

use crate::util::{read_json, squeeze, str_field};
use crate::{App, Config, Roots, Session, modified_ms};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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
            return Some(Lines { head: all, tail: Vec::new() });
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
        Some(Lines { head, tail })
    }

    fn iter(&self) -> impl DoubleEndedIterator<Item = &[u8]> + '_ {
        let mut all = split_lines(&self.head);
        all.extend(split_lines(&self.tail));
        all.into_iter()
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
fn prompt_of(v: &Value) -> Option<String> {
    if str_field(v, "type") != Some("user")
        || flag(v, "isSidechain")
        || flag(v, "isMeta")
        || flag(v, "isCompactSummary")
    {
        return None;
    }
    let text = texts(v).into_iter().map(str::trim_start).find(|t| !t.is_empty())?;
    if text.starts_with('<') {
        return None;
    }
    Some(squeeze(text, 80)).filter(|t| !t.is_empty())
}

fn read_inner(file: &Path) -> Option<(Session, TitleFrom)> {
    let id = file.file_stem()?.to_str()?.to_string();
    if file.extension().and_then(|e| e.to_str()) != Some("jsonl") || id.is_empty() {
        return None;
    }
    let lines = Lines::read(file, CHUNK)?;
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
                    prompt = prompt_of(&v);
                }
            }
            _ => {}
        }
    }
    let (mut cwd, mut branch, mut entrypoint) = (None::<String>, None::<String>, None::<String>);
    for line in lines.iter().rev() {
        if !contains(line, b"\"cwd\"") && !contains(line, b"\"entrypoint\"") && !contains(line, b"\"gitBranch\"") {
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
        if is_session && cwd.is_some() && entrypoint.is_some() {
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
        cwd: cwd.map(PathBuf::from).unwrap_or_default(),
        branch,
        last_active_ms: modified_ms(file),
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
            if folder.file_name() == "subagents" || !folder.path().is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
            for file in files.flatten() {
                if !file.file_type().is_ok_and(|t| t.is_file()) {
                    continue;
                }
                let Some((s, from)) = read_inner(&file.path()) else { continue };
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

    let mut sessions: Vec<Session> = found.into_values().map(|(s, _)| s).collect();
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
fn folder_key(cwd: &Path) -> String {
    let s = cwd.to_string_lossy().to_lowercase().replace('/', "\\");
    let trimmed = s.trim_end_matches('\\');
    if trimmed.is_empty() && !s.is_empty() { s } else { trimmed.to_string() }
}

/// A folder's last component, for a heading.
pub(crate) fn folder_name(cwd: &Path) -> String {
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
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};
    use serde_json::json;

    fn user(text: &str) -> String {
        json!({"type":"user","isSidechain":false,"cwd":"C:\\work\\kweb-mail","sessionId":"s","entrypoint":"cli",
               "gitBranch":"main","message":{"role":"user","content":text}})
        .to_string()
    }

    fn assistant(text: &str) -> String {
        json!({"type":"assistant","cwd":"C:\\work\\kweb-mail","entrypoint":"cli","message":{"role":"assistant",
               "content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":text},
                          {"type":"tool_use","id":"t","name":"Read","input":{}}]}})
        .to_string()
    }

    fn transcript(dir: &Path, id: &str, lines: &[String]) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        put(&path, &(lines.join("\n") + "\n"));
        path
    }

    #[test]
    fn the_custom_title_beats_the_ai_title_and_the_last_one_wins() {
        let t = TempDir::new("ses-title");
        let lines = [
            r#"{"type":"custom-title","customTitle":"First name","sessionId":"s"}"#.to_string(),
            user("hello there"),
            r#"{"type":"ai-title","aiTitle":"Generated","sessionId":"s"}"#.to_string(),
            r#"{"type":"custom-title","customTitle":"Second name","sessionId":"s"}"#.to_string(),
            r#"{"type":"ai-title","aiTitle":"Generated later","sessionId":"s"}"#.to_string(),
        ];
        let s = read_transcript(&transcript(&t.0, "a", &lines)).unwrap();
        assert_eq!(s.title, "Second name");
        let lines = [
            user("hello"),
            r#"{"type":"ai-title","aiTitle":"One"}"#.into(),
            r#"{"type":"ai-title","aiTitle":"Two"}"#.into(),
        ];
        assert_eq!(read_transcript(&transcript(&t.0, "b", &lines)).unwrap().title, "Two");
    }

    #[test]
    fn the_fallback_title_is_the_first_real_prompt() {
        let t = TempDir::new("ses-prompt");
        let meta = json!({"type":"user","isMeta":true,"message":{"role":"user","content":"meta text"}}).to_string();
        let command = user("<command-name>/clear</command-name>");
        let blocks = json!({"type":"user","cwd":"C:\\x","message":{"role":"user","content":[
            {"type":"text","text":"  Fix   the\nbug in the parser please"}]}})
        .to_string();
        let lines = ["not json at all".to_string(), meta, command, blocks, user("a later prompt")];
        let s = read_transcript(&transcript(&t.0, "0123456789abcdef", &lines)).unwrap();
        assert_eq!(s.title, "Fix the bug in the parser please");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.cwd, PathBuf::from("C:\\work\\kweb-mail"));
        assert_eq!(s.last_app, App::Cli);
        assert_eq!(s.id, "0123456789abcdef");
        let long = "abcdefghij ".repeat(20);
        let s = read_transcript(&transcript(&t.0, "long", &[user(&long)])).unwrap();
        assert_eq!(s.title.chars().count(), 80);
        let s = read_transcript(&transcript(&t.0, "0123456789", &[assistant("only an answer")])).unwrap();
        assert_eq!(s.title, "01234567");
    }

    #[test]
    fn a_file_without_session_lines_is_not_a_session() {
        let t = TempDir::new("ses-none");
        let lines = [r#"{"type":"file-history-snapshot","snapshot":{}}"#.to_string(), "garbage".into()];
        assert!(read_transcript(&transcript(&t.0, "x", &lines)).is_none());
        let side = json!({"type":"user","isSidechain":true,"cwd":"C:\\x","message":{"content":"hi"}}).to_string();
        assert!(read_transcript(&transcript(&t.0, "y", &[side])).is_none());
        put(&t.0.join("z.txt"), &user("hi"));
        assert!(read_transcript(&t.0.join("z.txt")).is_none());
    }

    #[test]
    fn the_newest_line_gives_the_folder_branch_and_app() {
        let t = TempDir::new("ses-newest");
        let desk = json!({"type":"assistant","cwd":"C:\\later","gitBranch":"feature","entrypoint":"claude-desktop",
                          "message":{"content":[{"type":"text","text":"ok"}]}})
        .to_string();
        let side = json!({"type":"assistant","isSidechain":true,"cwd":"C:\\side","entrypoint":"cli"}).to_string();
        let s = read_transcript(&transcript(&t.0, "n", &[user("hi"), desk, side])).unwrap();
        assert_eq!(s.cwd, PathBuf::from("C:\\later"));
        assert_eq!(s.branch.as_deref(), Some("feature"));
        assert_eq!(s.last_app, App::Desktop);
        assert!(!s.worker);
    }

    #[test]
    fn a_session_whose_newest_entrypoint_is_an_sdk_is_a_worker() {
        let t = TempDir::new("ses-worker");
        let line = |entry: &str| {
            json!({"type":"user","cwd":"C:\\w","entrypoint":entry,"message":{"role":"user","content":"go"}}).to_string()
        };
        let s = read_transcript(&transcript(&t.0, "w1", &[line("cli"), line("sdk-cli")])).unwrap();
        assert!(s.worker);
        assert_eq!(s.last_app, App::Cli);
        let s = read_transcript(&transcript(&t.0, "w2", &[line("sdk-ts")])).unwrap();
        assert!(s.worker);
        let s = read_transcript(&transcript(&t.0, "w3", &[line("sdk-ts"), line("cli")])).unwrap();
        assert!(!s.worker, "the newest entrypoint decides");
        let s = read_transcript(&transcript(&t.0, "w4", &[line("claude-desktop")])).unwrap();
        assert!(!s.worker);
        let s = read_transcript(&transcript(&t.0, "w5", &[assistant("no entrypoint here")])).unwrap();
        assert!(!s.worker);
    }

    #[test]
    fn a_big_transcript_is_read_at_both_ends_only() {
        let t = TempDir::new("ses-big");
        let mut lines =
            vec![r#"{"type":"custom-title","customTitle":"Early title"}"#.to_string(), user("first prompt")];
        let filler = assistant(&"x".repeat(1000));
        for _ in 0..1500 {
            lines.push(filler.clone());
        }
        // A title in the middle of the file is not seen: only the head and tail are read.
        lines.push(r#"{"type":"custom-title","customTitle":"Middle title"}"#.to_string());
        for _ in 0..1500 {
            lines.push(filler.clone());
        }
        lines.push(r#"{"type":"ai-title","aiTitle":"Late ai title"}"#.to_string());
        let file = transcript(&t.0, "big", &lines);
        assert!(std::fs::metadata(&file).unwrap().len() > 3 * CHUNK);
        let s = read_transcript(&file).unwrap();
        assert_eq!(s.title, "Early title");
        let p = preview(&file, 2);
        assert_eq!(p.len(), 2);
        assert!(!p[0].from_user && p[0].text.starts_with("xxx"));
    }

    #[test]
    fn a_line_cut_by_either_chunk_is_not_read() {
        let t = TempDir::new("ses-cut");
        let mut body = String::new();
        let big = r#"{"type":"custom-title","customTitle":"Hidden"}"#.to_string();
        body.push_str(&user("hi"));
        body.push('\n');
        // One huge line crossing the tail's start, whose end alone would parse as a title line.
        body.push_str(&format!(r#"{{"type":"assistant","pad":"{}","inner":{}}}"#, "y".repeat(700 * 1024), big));
        body.push('\n');
        body.push_str(&assistant("done"));
        body.push('\n');
        let file = t.0.join("cut.jsonl");
        put(&file, &body);
        let s = read_transcript(&file).unwrap();
        assert_eq!(s.title, "hi");
    }

    #[test]
    fn a_line_that_starts_where_the_tail_chunk_starts_is_read() {
        let t = TempDir::new("ses-edge");
        let title = r#"{"type":"custom-title","customTitle":"At the edge"}"#;
        // The last `CHUNK` bytes: the title line, then one line that fills the rest.
        let (open, close) = (r#"{"type":"assistant","pad":""#, "\"}\n");
        let pad = CHUNK as usize - title.len() - 1 - open.len() - close.len();
        let end = format!("{title}\n{open}{}{close}", "x".repeat(pad));
        assert_eq!(end.len() as u64, CHUNK);
        let start = format!("{}\n{}\n", user("hi"), assistant(&"y".repeat(2 * CHUNK as usize)));
        let file = t.0.join("edge.jsonl");
        put(&file, &(start + &end));
        assert_eq!(read_transcript(&file).unwrap().title, "At the edge");
    }

    fn setup_scan(t: &TempDir) -> (Roots, Config) {
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        (roots, config)
    }

    #[test]
    fn sidechains_and_subagent_folders_are_not_sessions() {
        let t = TempDir::new("ses-sub");
        let (roots, config) = setup_scan(&t);
        let folder = roots.primary_cli_dir().join("projects").join("C--work");
        transcript(&folder, "main-session", &[user("hi")]);
        transcript(&folder.join("main-session").join("subagents"), "agent-1", &[user("sub")]);
        transcript(&roots.primary_cli_dir().join("projects").join("subagents"), "agent-2", &[user("sub")]);
        let side = json!({"type":"user","isSidechain":true,"message":{"content":"x"}}).to_string();
        transcript(&folder, "side-only", &[side]);
        let ids: Vec<String> = scan(&roots, &config).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["main-session"]);
    }

    #[test]
    fn the_same_projects_folder_through_a_junction_counts_once() {
        let t = TempDir::new("ses-junction");
        let (roots, config) = setup_scan(&t);
        let folder = roots.primary_cli_dir().join("projects").join("C--work");
        transcript(&folder, "one", &[user("hi")]);
        transcript(&folder, "two", &[user("there")]);
        let work = &config.accounts[1];
        std::fs::create_dir_all(&work.cli_dir).unwrap();
        crate::share::apply(&roots, work).unwrap();
        assert!(work.cli_dir.join("projects").join("C--work").join("one.jsonl").is_file());
        let sessions = scan(&roots, &config);
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|s| s.file.starts_with(roots.primary_cli_dir())));
    }

    #[test]
    fn a_desktop_record_is_joined_by_its_cli_session_id() {
        let t = TempDir::new("ses-desktop");
        let (roots, config) = setup_scan(&t);
        let folder = roots.primary_cli_dir().join("projects").join("C--work");
        transcript(&folder, "cli-now", &[user("prompt text")]);
        transcript(&folder, "cli-before", &[user("older")]);
        transcript(&folder, "plain", &[r#"{"type":"custom-title","customTitle":"Mine"}"#.into(), user("x")]);
        let records = config.accounts[1].desktop_dir.join("claude-code-sessions").join("acc").join("org");
        put(
            &records.join("local_1.json"),
            &json!({"sessionId":"local_1","cliSessionId":"cli-now","cwd":"C:\\work","title":"Desktop title",
                    "isArchived":true,"lastActivityAt":4_000_000_000_000u64,"createdAt":1,
                    "priorCliSessionIds":["cli-before","cli-now"]})
            .to_string(),
        );
        put(
            &records.join("local_2.json"),
            &json!({"sessionId":"local_2","cliSessionId":"plain","title":"Ignored","lastActivityAt":1}).to_string(),
        );
        put(&records.join("tasks.json"), r#"{"scheduledTasks":[]}"#);
        put(&records.join("list.json"), "[1]");
        let sessions = scan(&roots, &config);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids[0], "cli-now", "the record's activity makes it the newest");
        assert!(!ids.contains(&"cli-before"), "an earlier transcript of a desktop session is left out");
        let s = &sessions[0];
        assert_eq!(s.title, "Desktop title");
        assert_eq!(s.desktop_id.as_deref(), Some("local_1"));
        assert_eq!(s.desktop_account.as_deref(), Some("work"));
        assert!(s.archived);
        assert_eq!(s.last_active_ms, 4_000_000_000_000);
        let plain = sessions.iter().find(|s| s.id == "plain").unwrap();
        assert_eq!(plain.title, "Mine", "a custom title beats the record's");
        assert_eq!(plain.desktop_id.as_deref(), Some("local_2"));
    }

    #[test]
    fn prior_ids_stay_when_the_current_transcript_is_missing() {
        let t = TempDir::new("ses-prior");
        let (roots, config) = setup_scan(&t);
        let folder = roots.primary_cli_dir().join("projects").join("C--work");
        transcript(&folder, "cli-before", &[user("older")]);
        let records = config.accounts[0].desktop_dir.join("claude-code-sessions").join("acc").join("org");
        put(
            &records.join("local_1.json"),
            &json!({"sessionId":"local_1","cliSessionId":"gone","priorCliSessionIds":["cli-before"]}).to_string(),
        );
        assert_eq!(scan(&roots, &config).len(), 1);
    }

    fn session(id: &str, cwd: &str, at: u64) -> Session {
        Session {
            id: id.into(),
            title: id.into(),
            cwd: PathBuf::from(cwd),
            branch: None,
            last_active_ms: at,
            bytes: 0,
            last_app: App::Cli,
            desktop_id: None,
            desktop_account: None,
            archived: false,
            worker: false,
            file: PathBuf::new(),
        }
    }

    #[test]
    fn groups_follow_their_newest_session() {
        let list = [
            session("a", "C:\\Work\\kweb-mail", 10),
            session("b", "c:\\work\\KWEB-MAIL\\", 30),
            session("c", "C:\\other", 20),
            session("d", "", 5),
            session("e", "C:\\", 1),
        ];
        let groups = group_by_folder(&list);
        let summary: Vec<(&str, Vec<usize>)> = groups.iter().map(|g| (g.name.as_str(), g.sessions.clone())).collect();
        assert_eq!(summary, [("KWEB-MAIL", vec![1, 0]), ("other", vec![2]), ("(no folder)", vec![3]), ("C:", vec![4])]);
        assert!(groups[2].cwd.as_os_str().is_empty());
    }

    #[test]
    fn the_preview_leaves_out_tools_meta_and_commands() {
        let t = TempDir::new("ses-preview");
        let tool_result = json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t","content":"file text"}]}})
        .to_string();
        let only_tool =
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}).to_string();
        let meta = json!({"type":"user","isMeta":true,"message":{"content":"meta"}}).to_string();
        let side = json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"s"}]}})
            .to_string();
        let lines = [
            user("first"),
            assistant("answer one"),
            user("<command-name>/model</command-name>"),
            tool_result,
            only_tool,
            meta,
            side,
            user("second\n\n  question"),
            assistant("answer two"),
        ];
        let file = transcript(&t.0, "p", &lines);
        let p = preview(&file, 3);
        let got: Vec<(bool, &str)> = p.iter().map(|l| (l.from_user, l.text.as_str())).collect();
        assert_eq!(got, [(false, "answer one"), (true, "second question"), (false, "answer two")]);
        assert!(preview(&file, 0).is_empty());
        assert!(preview(&t.0.join("missing.jsonl"), 5).is_empty());
        let long = transcript(&t.0, "q", &[user(&"z".repeat(1000))]);
        assert_eq!(preview(&long, 1)[0].text.len(), 400);
    }

    #[test]
    #[ignore = "reads the real folders of this computer"]
    fn the_real_sessions_are_listed() {
        let roots = Roots::from_env().expect("USERPROFILE and APPDATA");
        let start = std::time::Instant::now();
        let config = crate::config::load(&roots);
        let sessions = scan(&roots, &config);
        let scanned = start.elapsed().as_millis();
        let running = crate::running::scan(&roots, &config);
        let total = start.elapsed().as_millis();
        let groups = group_by_folder(&sessions);
        let desktop = sessions.iter().filter(|s| s.desktop_id.is_some()).count();
        let archived = sessions.iter().filter(|s| s.archived).count();
        let no_cwd = sessions.iter().filter(|s| s.cwd.as_os_str().is_empty()).count();
        let from_id = sessions.iter().filter(|s| s.title.len() == 8 && s.id.starts_with(&s.title)).count();
        println!(
            "accounts {} sessions {} groups {} with desktop record {} archived {} without folder {} \
             titled by id {} running {} desktop app found {} scan {} ms scan+running {} ms",
            config.accounts.len(),
            sessions.len(),
            groups.len(),
            desktop,
            archived,
            no_cwd,
            from_id,
            running.len(),
            crate::launch::desktop_exe().is_some(),
            scanned,
            total
        );
    }
}
