//! A job's output: its `<id>.jsonl` event stream read from the tail, then incrementally as it
//! grows, as the transcript reader does. Reads are bounded: the first read starts at most
//! [`START`] bytes before the end, each later read takes at most [`STEP`] bytes, a line longer
//! than [`MAX_LINE`] is skipped, and at most [`KEEP`] entries are kept.
//!
//! Two event shapes are read. An OpenCode worker writes `opencode run --format json` events:
//! `{type: "text"|"reasoning"|"tool_use"|"step_start"|"step_finish"|"error", part: {...}}`, a tool
//! call's `part` carrying `tool` and `state: {status, title, input}`. A Claude Code worker writes
//! `claude -p --output-format stream-json` messages: `system` (`init`, `api_retry`), `assistant`
//! and `user` with `message.content` blocks, and the closing `result`. Anything else is skipped.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::util::{squeeze, str_field};

/// How far before the end the first read starts.
pub const START: u64 = 256 * 1024;
/// The most one read takes.
pub const STEP: usize = 1024 * 1024;
/// A longer line is skipped.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// The most entries kept; older ones go.
pub const KEEP: usize = 2_000;
/// The longest text an entry keeps.
const TEXT: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The worker's answer text.
    Text,
    /// Its reasoning, when the stream carries it.
    Thought,
    /// A tool call: `title` is the tool, `text` what it was called on.
    Tool,
    /// A failed tool call or an error event.
    Error,
    /// The end of the run.
    Result,
    /// A note: the run started, a retry.
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub title: String,
    pub text: String,
}

impl Entry {
    fn new(kind: Kind, title: impl Into<String>, text: &str) -> Self {
        let text = if text.len() > TEXT {
            let mut end = TEXT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &text[..end])
        } else {
            text.to_owned()
        };
        Self { kind, title: title.into(), text: text.trim().to_owned() }
    }
}

/// The tail of one job's event stream.
pub struct Tail {
    path: PathBuf,
    /// Where the next read starts.
    offset: u64,
    /// The start of a line not yet ended.
    carry: Vec<u8>,
    /// Inside a line too long to keep: skip to its end.
    skipping: bool,
    /// The file's creation time, to tell a replaced file from a grown one.
    created: Option<std::time::SystemTime>,
    pub entries: VecDeque<Entry>,
    /// Entries before these were not read (the first read started inside the file) or were let go.
    pub cut: bool,
    /// Bumped whenever `entries` change.
    pub revision: u64,
    /// Entries pushed since the last reset: `entries` are the last of them, so a reader that
    /// measured some can tell which are new.
    pub pushed: u64,
    /// Bumped when the stream is read again from its start (it shrank or was replaced).
    pub resets: u64,
}

impl Tail {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            offset: 0,
            carry: Vec::new(),
            skipping: false,
            created: None,
            entries: VecDeque::new(),
            cut: false,
            revision: 0,
            pushed: 0,
            resets: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads what was appended since the last read (at most [`STEP`] bytes). True when entries
    /// changed. A file that shrank or was replaced is read again from its tail.
    pub fn read(&mut self) -> io::Result<bool> {
        let mut file = File::open(&self.path)?;
        let meta = file.metadata()?;
        let created = meta.created().ok();
        let len = meta.len();
        let mut changed = false;
        if len < self.offset || (self.created.is_some() && created != self.created) {
            self.reset();
            changed = true;
        }
        self.created = created;
        if self.offset == 0 && len > START {
            self.offset = len - START;
            self.skipping = true;
            self.cut = true;
        }
        if len == self.offset {
            return Ok(changed);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let want = (len - self.offset).min(STEP as u64);
        let mut bytes = Vec::with_capacity(want as usize);
        file.take(want).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        Ok(self.feed(&bytes) || changed)
    }

    /// More than [`STEP`] bytes are waiting: read again soon.
    pub fn behind(&self) -> bool {
        std::fs::metadata(&self.path).is_ok_and(|m| m.len() > self.offset)
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.carry.clear();
        self.skipping = false;
        self.entries.clear();
        self.cut = false;
        self.revision += 1;
        self.pushed = 0;
        self.resets += 1;
    }

    fn feed(&mut self, mut bytes: &[u8]) -> bool {
        let before = self.revision;
        while !bytes.is_empty() {
            let Some(end) = bytes.iter().position(|&b| b == b'\n') else {
                if !self.skipping {
                    self.carry.extend_from_slice(bytes);
                    if self.carry.len() > MAX_LINE {
                        self.carry.clear();
                        self.skipping = true;
                    }
                }
                break;
            };
            if self.skipping {
                self.skipping = false;
            } else {
                self.carry.extend_from_slice(&bytes[..end]);
                if self.carry.len() <= MAX_LINE {
                    let line = std::mem::take(&mut self.carry);
                    for entry in parse_line(&line) {
                        self.push(entry);
                    }
                }
                self.carry.clear();
            }
            bytes = &bytes[end + 1..];
        }
        self.revision != before
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push_back(entry);
        self.pushed += 1;
        while self.entries.len() > KEEP {
            self.entries.pop_front();
            self.cut = true;
        }
        self.revision += 1;
    }
}

/// The entries of one line of the stream; none for a line that is not an event shown here.
pub fn parse_line(line: &[u8]) -> Vec<Entry> {
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return Vec::new() };
    let mut out = Vec::new();
    let kind = str_field(&v, "type").unwrap_or("");
    let part = v.get("part");
    match kind {
        // OpenCode.
        "text" | "reasoning" if part.is_some() => {
            let text = part.and_then(|p| str_field(p, "text")).unwrap_or("");
            if !text.trim().is_empty() {
                let kind = if kind == "text" { Kind::Text } else { Kind::Thought };
                out.push(Entry::new(kind, "", text));
            }
        }
        "tool_use" if part.is_some() => {
            let part = part.unwrap();
            let tool = str_field(part, "tool").unwrap_or("tool");
            let state = part.get("state");
            let input = state.and_then(|s| s.get("input"));
            let target = state
                .and_then(|s| str_field(s, "title"))
                .map(str::to_owned)
                .or_else(|| input.map(summary))
                .unwrap_or_default();
            let failed = state.and_then(|s| str_field(s, "status")) == Some("error");
            if failed {
                let why = state.and_then(|s| str_field(s, "error")).unwrap_or("");
                let text = if why.is_empty() { target } else { format!("{target} — {}", squeeze(why, 300)) };
                out.push(Entry::new(Kind::Error, tool, &text));
            } else {
                out.push(Entry::new(Kind::Tool, tool, &target));
            }
        }
        "error" => {
            let e = v.get("error").unwrap_or(&Value::Null);
            let text = str_field(e, "message")
                .or_else(|| e.get("data").and_then(|d| str_field(d, "message")))
                .or_else(|| e.as_str())
                .map_or_else(|| squeeze(&e.to_string(), 400), |s| squeeze(s, 400));
            out.push(Entry::new(Kind::Error, "Error", &text));
        }
        // Claude Code.
        "system" => match str_field(&v, "subtype") {
            Some("init") => {
                let model = str_field(&v, "model").unwrap_or("Claude Code");
                out.push(Entry::new(Kind::Note, "Started", model));
            }
            Some("api_retry") => {
                let attempt = v.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                out.push(Entry::new(Kind::Note, "Retrying", &format!("attempt {attempt}")));
            }
            _ => {}
        },
        "assistant" => {
            for block in blocks(&v) {
                match str_field(block, "type") {
                    Some("text") => {
                        let text = str_field(block, "text").unwrap_or("");
                        if !text.trim().is_empty() {
                            out.push(Entry::new(Kind::Text, "", text));
                        }
                    }
                    Some("thinking") => {
                        let text = str_field(block, "thinking").unwrap_or("");
                        if !text.trim().is_empty() {
                            out.push(Entry::new(Kind::Thought, "", text));
                        }
                    }
                    Some("tool_use") => {
                        let tool = str_field(block, "name").unwrap_or("tool");
                        let target = block.get("input").map(summary).unwrap_or_default();
                        out.push(Entry::new(Kind::Tool, tool, &target));
                    }
                    _ => {}
                }
            }
        }
        "user" => {
            for block in blocks(&v) {
                if str_field(block, "type") == Some("tool_result")
                    && block.get("is_error").and_then(Value::as_bool) == Some(true)
                {
                    let text = match block.get("content") {
                        Some(Value::String(s)) => squeeze(s, 300),
                        Some(Value::Array(items)) => squeeze(
                            &items.iter().filter_map(|i| str_field(i, "text")).collect::<Vec<_>>().join(" "),
                            300,
                        ),
                        _ => String::new(),
                    };
                    out.push(Entry::new(Kind::Error, "Tool failed", &text));
                }
            }
        }
        "result" => {
            let failed = v.get("is_error").and_then(Value::as_bool) == Some(true);
            let secs = v.get("duration_ms").and_then(Value::as_u64).map(|ms| ms / 1000);
            let title = if failed { "Ended with an error" } else { "Finished" };
            let text = secs.map(|s| format!("after {}", duration(s))).unwrap_or_default();
            out.push(Entry::new(if failed { Kind::Error } else { Kind::Result }, title, &text));
        }
        _ => {}
    }
    out
}

fn blocks(v: &Value) -> impl Iterator<Item = &Value> {
    v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array).into_iter().flatten()
}

/// What a tool was called on, in one line: the first of its usual arguments.
fn summary(input: &Value) -> String {
    const KEYS: [&str; 10] = [
        "file_path",
        "filePath",
        "path",
        "command",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
        "notebook_path",
    ];
    KEYS.iter().find_map(|k| str_field(input, k)).map(|s| squeeze(s, 200)).unwrap_or_default()
}

/// Seconds as "42s", "3m 05s" or "1h 02m".
pub fn duration(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {:02}s", s / 60, s % 60),
        s => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}
