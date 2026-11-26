//! Incremental, read-only views of Claude Code JSONL transcripts.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_PICTURE: usize = 32 * 1024 * 1024;
const SCAN_BYTES: u64 = 64 * 1024;

/// Stable across appends and history loads, but not across a reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ItemId {
    pub offset: u64,
    pub block: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub line_offset: u64,
    pub line_len: usize,
    pub media_type: String,
    pub base64_len: usize,
    generation: u64,
    block: usize,
    result_block: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Ok,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Compaction,
    ApiError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Prompt {
        text: String,
        pictures: Vec<Picture>,
    },
    Assistant {
        markdown: String,
    },
    Tool {
        id: String,
        name: String,
        summary: String,
        state: ToolState,
        pictures: Vec<Picture>,
        subagent: Option<String>,
    },
    Notice {
        kind: NoticeKind,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub timestamp: Option<String>,
    pub kind: ItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub content: String,
    pub status: TaskStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subagent {
    pub id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub model: Option<String>,
    /// Unknown when no usable sidecar has been written yet.
    pub finished: Option<bool>,
    pub spawn_depth: Option<u64>,
    pub tool_use_id: Option<String>,
    pub started_at: Option<String>,
    pub last_activity_at: Option<String>,
    pub tool_calls: usize,
    pub last_tool_summary: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Drop all previously displayed IDs and picture references before applying this update.
    pub reset: bool,
    pub added: BTreeSet<ItemId>,
    /// Previously visible items only; an item added and updated in this batch is just `added`.
    pub updated: BTreeSet<ItemId>,
    pub tasks_changed: bool,
    pub subagents_changed: bool,
}

pub struct Reader {
    tail: Tail,
    view: View,
    agents: BTreeMap<String, AgentFollow>,
    subagent_dir: Option<PathBuf>,
}

impl Reader {
    /// Open a session (or an agent transcript). The initial transcript read is bounded by `chunk_bytes`.
    /// A zero bound is allowed; historical loads still make progress one line at a time.
    pub fn open(path: impl AsRef<Path>, chunk_bytes: usize) -> io::Result<Self> {
        let path = path.as_ref();
        let is_agent = path.parent().and_then(Path::file_name).is_some_and(|name| name == "subagents");
        let subagent_dir = (!is_agent).then(|| path.with_extension("").join("subagents"));
        let mut reader = Self {
            tail: Tail::open(path, chunk_bytes)?,
            view: View { is_agent, ..View::default() },
            agents: BTreeMap::new(),
            subagent_dir,
        };
        reader.initial(&mut Changes::default())?;
        reader.refresh_agents(&mut Changes::default())?;
        Ok(reader)
    }

    pub fn items(&self) -> impl DoubleEndedIterator<Item = (ItemId, &Item)> + ExactSizeIterator {
        self.view.items.iter().map(|(&id, item)| (id, item))
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.view.items.get(&id)
    }

    pub fn tasks(&self) -> &[Task] {
        &self.view.tasks
    }

    pub fn subagents(&self) -> impl ExactSizeIterator<Item = &Subagent> {
        self.agents.values().map(|agent| &agent.info)
    }

    pub fn open_subagent(&self, id: &str, chunk_bytes: usize) -> io::Result<Self> {
        let agent = self.agents.get(id).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Self::open(&agent.tail.path, chunk_bytes)
    }

    pub fn has_earlier(&self) -> bool {
        self.tail.head != Some(0)
    }

    /// Prepend one historical chunk, extending backward if needed to fit at least one whole line.
    pub fn earlier(&mut self) -> io::Result<Changes> {
        let mut changes = Changes::default();
        if self.tail.replaced()? {
            return self.reset();
        }
        let view = &mut self.view;
        self.tail.earlier(|offset, line| view.line(offset, line, &mut changes))?;
        view.finish(&mut changes);
        Ok(changes)
    }

    /// Consume new complete lines, keeping a partial last line for the next call. If the initial
    /// byte window started inside that line, it belongs to `earlier()` once complete, unless a
    /// history load already recovered its prefix.
    pub fn poll(&mut self) -> io::Result<Changes> {
        if self.tail.replaced()? {
            return self.reset();
        }
        let mut changes = Changes::default();
        let view = &mut self.view;
        self.tail.append(|offset, line| view.line(offset, line, &mut changes))?;
        view.finish(&mut changes);
        self.refresh_agents(&mut changes)?;
        Ok(changes)
    }

    pub fn load_picture(&mut self, picture: &Picture) -> io::Result<Vec<u8>> {
        if picture.generation != self.view.generation || self.tail.replaced()? {
            return Err(invalid_picture("stale picture reference"));
        }
        if picture.base64_len > MAX_PICTURE.div_ceil(3) * 4 {
            return Err(invalid_picture("picture exceeds 32 MiB"));
        }
        let line = self.tail.read(picture.line_offset, picture.line_len as u64)?;
        let value: Value = serde_json::from_slice(&line).map_err(|_| invalid_picture("invalid picture line"))?;
        let block = &value["message"]["content"][picture.block];
        let block = match picture.result_block {
            Some(index) => &block["content"][index],
            None => block,
        };
        let (media, data) = image_source(block).ok_or_else(|| invalid_picture("missing picture block"))?;
        if media != picture.media_type || data.len() != picture.base64_len {
            return Err(invalid_picture("picture reference no longer matches"));
        }
        decode_base64(data.as_bytes(), MAX_PICTURE)
    }

    fn initial(&mut self, changes: &mut Changes) -> io::Result<()> {
        let view = &mut self.view;
        self.tail.initial(|offset, line| view.line(offset, line, changes))?;
        view.finish(changes);
        Ok(())
    }

    fn reset(&mut self) -> io::Result<Changes> {
        self.tail = Tail::open(&self.tail.path, self.tail.chunk)?;
        self.view = View { is_agent: self.view.is_agent, generation: self.view.generation + 1, ..View::default() };
        self.agents.clear();
        let mut changes = Changes { reset: true, ..Changes::default() };
        self.initial(&mut changes)?;
        self.refresh_agents(&mut changes)?;
        Ok(changes)
    }

    fn refresh_agents(&mut self, changes: &mut Changes) -> io::Result<()> {
        let Some(dir) = &self.subagent_dir else { return Ok(()) };
        let entries = match fs::read_dir(dir) {
            Ok(entries) => Some(entries),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        let mut seen = BTreeSet::new();
        for entry in entries.into_iter().flatten() {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|s| s.strip_prefix("agent-")).and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };
            if id.is_empty() || !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if !self.agents.contains_key(id) {
                let tail = match Tail::open(&path, 0) {
                    Ok(tail) => tail,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                    Err(err) => return Err(err),
                };
                self.agents.insert(
                    id.to_owned(),
                    AgentFollow { tail, info: Subagent { id: id.into(), ..Subagent::default() } },
                );
                changes.subagents_changed = true;
            }
            seen.insert(id.to_owned());
            let agent = self.agents.get_mut(id).expect("agent was inserted");
            let before = agent.info.clone();
            if agent.tail.replaced()? {
                *agent = AgentFollow {
                    tail: Tail::open(&path, 0)?,
                    info: Subagent { id: id.into(), ..Subagent::default() },
                };
            }
            let meta = match fs::read(path.with_extension("meta.json")) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Value::Null,
                Err(err) => return Err(err),
            };
            agent.info.agent_type = string(&meta["agentType"]);
            agent.info.description = string(&meta["description"]);
            agent.info.model = string(&meta["model"]);
            agent.info.finished = meta["finished"].as_bool();
            agent.info.spawn_depth = meta["spawnDepth"].as_u64();
            agent.info.tool_use_id = string(&meta["toolUseId"]);
            let info = &mut agent.info;
            agent.tail.append(|_, line| agent_line(info, line))?;
            changes.subagents_changed |= before != agent.info;
        }
        let count = self.agents.len();
        self.agents.retain(|id, _| seen.contains(id));
        changes.subagents_changed |= count != self.agents.len();
        if changes.subagents_changed {
            let links = self
                .agents
                .values()
                .filter_map(|agent| Some((agent.info.tool_use_id.clone()?, agent.info.id.clone())))
                .collect();
            self.view.set_links(links, changes);
        }
        Ok(())
    }
}

struct AgentFollow {
    tail: Tail,
    info: Subagent,
}

/// Byte cursor shared by session views and the lightweight agent statistics reducer.
struct Tail {
    path: PathBuf,
    file: File,
    chunk: usize,
    cursor: u64,
    head: Option<u64>,
    pending_start: u64,
    pending: Vec<u8>,
    clipped: bool,
}

impl Tail {
    fn open(path: &Path, chunk: usize) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            path: path.to_owned(),
            file,
            chunk,
            cursor: 0,
            head: Some(0),
            pending_start: 0,
            pending: Vec::new(),
            clipped: false,
        })
    }

    fn read(&mut self, start: u64, len: u64) -> io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        self.file.by_ref().take(len).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn initial(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let size = self.file.metadata()?.len();
        let start = size.saturating_sub(self.chunk as u64);
        let bytes = self.read(start, size - start)?;
        self.cursor = start + bytes.len() as u64;
        let skip = if start == 0 {
            0
        } else if let Some(i) = newline(&bytes) {
            i + 1
        } else {
            self.head = None;
            self.clipped = true;
            self.pending_start = start;
            self.pending = bytes;
            return Ok(());
        };
        let mut offset = start + skip as u64;
        self.head = Some(offset);
        for line in bytes[skip..].split_inclusive(|&byte| byte == b'\n') {
            if line.ends_with(b"\n") {
                visit(offset, line);
                offset += line.len() as u64;
            } else {
                self.pending.extend_from_slice(line);
            }
        }
        self.pending_start = offset;
        Ok(())
    }

    fn replaced(&self) -> io::Result<bool> {
        let held = self.file.metadata()?;
        if held.len() < self.cursor {
            return Ok(true);
        }
        let path_meta = match fs::metadata(&self.path) {
            Ok(meta) => meta,
            // The writer may be between rename and recreate. Keep the old view until a new path exists.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err),
        };
        Ok(different_identity(&held, &path_meta)
            // On Windows stable std doesn't expose the file index. Comparing against the open handle
            // also catches replacements that inherit the former path's creation time.
            || ((path_meta.len(), path_meta.modified().ok()) != (held.len(), held.modified().ok())
                && self.file.metadata().is_ok_and(|now| {
                    (now.len(), now.modified().ok()) == (held.len(), held.modified().ok())
                })))
    }

    fn append(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let meta = self.file.metadata()?;
        if meta.len() <= self.cursor {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(self.cursor))?;
        let mut reader = BufReader::new(self.file.by_ref().take(meta.len() - self.cursor));
        loop {
            let count = reader.read_until(b'\n', &mut self.pending)?;
            self.cursor += count as u64;
            if count == 0 || !self.pending.ends_with(b"\n") {
                break;
            }
            if self.clipped {
                // The line began outside the initial budget. Leave it in history rather than
                // seeking backward in a poll or trying to parse a suffix as a whole line.
                self.clipped = false;
                self.head = Some(self.cursor);
            } else {
                visit(self.pending_start, &self.pending);
            }
            self.pending.clear();
            self.pending_start = self.cursor;
        }
        Ok(())
    }

    // Find the start of the line containing `at`, or return `at` if it is already a boundary.
    fn line_start(&mut self, mut at: u64) -> io::Result<u64> {
        while at > 0 {
            let start = at.saturating_sub(SCAN_BYTES);
            let bytes = self.read(start, at - start)?;
            if let Some(i) = bytes.iter().rposition(|&byte| byte == b'\n') {
                return Ok(start + i as u64 + 1);
            }
            at = start;
        }
        Ok(0)
    }

    fn earlier(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let end = match self.head {
            Some(head) => head,
            None => self.line_start(self.pending_start)?,
        };
        if self.clipped {
            // Keep the partial line's real start even as historical chunks move farther back.
            let mut prefix = self.read(end, self.pending_start - end)?;
            prefix.append(&mut self.pending);
            self.pending = prefix;
            self.pending_start = end;
            self.clipped = false;
        }
        if end == 0 {
            self.head = Some(0);
            return Ok(());
        }
        let probe = end.saturating_sub((self.chunk as u64).max(1));
        let candidate = if probe == 0 {
            0
        } else {
            let bytes = self.read(probe - 1, end - probe)?;
            newline(&bytes).map_or(end, |i| probe + i as u64)
        };
        let start = if candidate < end { candidate } else { self.line_start(end - 1)? };
        self.file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(self.file.by_ref().take(end - start));
        let mut line = Vec::new();
        let mut offset = start;
        while reader.read_until(b'\n', &mut line)? != 0 {
            if line.ends_with(b"\n") {
                visit(offset, &line);
            }
            offset += line.len() as u64;
            line.clear();
        }
        self.head = Some(start);
        Ok(())
    }
}

#[cfg(unix)]
fn different_identity(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) != (b.dev(), b.ino())
}

#[cfg(not(unix))]
fn different_identity(a: &Metadata, b: &Metadata) -> bool {
    a.created().ok() != b.created().ok()
}

fn newline(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&byte| byte == b'\n')
}

#[derive(Default)]
struct View {
    is_agent: bool,
    generation: u64,
    items: BTreeMap<ItemId, Item>,
    tools: HashMap<String, ItemId>,
    results: HashMap<String, (ItemId, ToolResult)>,
    links: HashMap<String, String>,
    plans: BTreeMap<ItemId, Plan>,
    tasks: Vec<Task>,
    plans_dirty: bool,
}

struct ToolResult {
    state: ToolState,
    pictures: Vec<Picture>,
}

impl View {
    fn line(&mut self, offset: u64, line: &[u8], changes: &mut Changes) {
        let Some((kind, value)) = message_line(line) else { return };
        if flag(&value, "isSidechain") && !self.is_agent {
            return;
        }
        let timestamp = value["timestamp"].as_str();
        let id = |block| ItemId { offset, block };
        let content = &value["message"]["content"];
        if flag(&value, "isCompactSummary") || flag(&value, "isApiErrorMessage") {
            let kind = if flag(&value, "isCompactSummary") { NoticeKind::Compaction } else { NoticeKind::ApiError };
            let text = [content, &value["content"], &value["message"], &value["error"], &value["error"]["message"]]
                .into_iter()
                .map(message_text)
                .find(|text| !text.is_empty());
            if let Some(text) = text {
                self.add(id(0), timestamp, ItemKind::Notice { kind, text }, changes);
            }
            return;
        }
        let picture = |block, result_block, value| {
            let (media, data) = image_source(value)?;
            Some(Picture {
                line_offset: offset,
                line_len: line.len(),
                media_type: media.into(),
                base64_len: data.len(),
                generation: self.generation,
                block,
                result_block,
            })
        };
        match kind {
            LineKind::User => {
                let mut texts = Vec::new();
                let mut pictures = Vec::new();
                let mut results = Vec::new();
                if let Some(text) = content.as_str().and_then(prompt_text) {
                    texts.push(text);
                }
                for (i, block) in blocks(content).enumerate() {
                    match block["type"].as_str() {
                        Some("text") => texts.extend(block["text"].as_str().and_then(prompt_text)),
                        Some("image") => pictures.extend(picture(i, None, block)),
                        Some("tool_result") => {
                            if let Some(tool) = block["tool_use_id"].as_str() {
                                let state = if flag(block, "is_error") { ToolState::Failed } else { ToolState::Ok };
                                let pictures = blocks(&block["content"])
                                    .enumerate()
                                    .filter_map(|(j, block)| picture(i, Some(j), block))
                                    .collect();
                                results.push((tool, id(i), ToolResult { state, pictures }));
                            }
                        }
                        _ => (),
                    }
                }
                if !texts.is_empty() || !pictures.is_empty() {
                    self.add(id(0), timestamp, ItemKind::Prompt { text: texts.join("\n"), pictures }, changes);
                }
                for (tool, at, result) in results {
                    if self.results.get(tool).is_none_or(|(previous, _)| *previous < at) {
                        self.results.insert(tool.into(), (at, result));
                        self.update_tool(tool, changes);
                    }
                }
            }
            LineKind::Assistant => {
                if let Some(markdown) = content.as_str().filter(|s| !s.is_empty()) {
                    self.add(id(0), timestamp, ItemKind::Assistant { markdown: markdown.into() }, changes);
                }
                for (i, block) in blocks(content).enumerate() {
                    if block["type"] == "text" {
                        if let Some(text) = block["text"].as_str().filter(|s| !s.is_empty()) {
                            self.add(id(i), timestamp, ItemKind::Assistant { markdown: text.into() }, changes);
                        }
                    } else if let Some((tool, name, input)) = tool_use(block) {
                        let item = ItemKind::Tool {
                            id: tool.into(),
                            name: name.into(),
                            summary: tool_summary(name, input),
                            state: ToolState::Running,
                            pictures: Vec::new(),
                            subagent: None,
                        };
                        self.add(id(i), timestamp, item, changes);
                        self.tools.insert(tool.into(), id(i));
                        self.update_tool(tool, changes);
                        if let Some(plan) = Plan::parse(name, input) {
                            self.plans.insert(id(i), plan);
                            self.plans_dirty = true;
                        }
                    }
                }
            }
            LineKind::System => (),
        }
    }

    fn add(&mut self, id: ItemId, timestamp: Option<&str>, kind: ItemKind, changes: &mut Changes) {
        self.items.insert(id, Item { timestamp: timestamp.map(str::to_owned), kind });
        changes.added.insert(id);
    }

    fn update_tool(&mut self, tool: &str, changes: &mut Changes) {
        let Some(&id) = self.tools.get(tool) else { return };
        let Some(Item { kind: ItemKind::Tool { state, pictures, subagent, .. }, .. }) = self.items.get_mut(&id) else {
            return;
        };
        let result = self.results.get(tool).map(|(_, result)| result);
        let new_state = result.map_or(ToolState::Running, |r| r.state);
        let new_pictures = result.map_or(&[][..], |r| r.pictures.as_slice());
        let new_agent = self.links.get(tool);
        if *state != new_state || *pictures != new_pictures || subagent.as_ref() != new_agent {
            *state = new_state;
            *pictures = new_pictures.to_vec();
            *subagent = new_agent.cloned();
            if !changes.added.contains(&id) {
                changes.updated.insert(id);
            }
        }
    }

    fn set_links(&mut self, links: HashMap<String, String>, changes: &mut Changes) {
        let old = std::mem::replace(&mut self.links, links);
        let changed: Vec<_> =
            old.keys().chain(self.links.keys()).filter(|key| old.get(*key) != self.links.get(*key)).cloned().collect();
        for tool in changed {
            self.update_tool(&tool, changes);
        }
    }

    fn finish(&mut self, changes: &mut Changes) {
        if !std::mem::take(&mut self.plans_dirty) {
            return;
        }
        // A later snapshot supersedes all earlier snapshots and actions, including paged-in history.
        let mut plans = self.plans.values().rev();
        let mut actions = Vec::new();
        let mut tasks = loop {
            match plans.next() {
                Some(Plan::Snapshot(tasks)) => break tasks.clone(),
                Some(action) => actions.push(action),
                None => break Vec::new(),
            }
        };
        for action in actions.into_iter().rev() {
            action.apply(&mut tasks);
        }
        if self.tasks != tasks {
            self.tasks = tasks;
            changes.tasks_changed = true;
        }
    }
}

fn flag(value: &Value, name: &str) -> bool {
    value[name].as_bool().unwrap_or(false)
}

fn string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn blocks(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn message_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.into();
    }
    blocks(value)
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn prompt_text(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    if let Some(command) = trimmed.strip_prefix("<command-name>").and_then(|s| s.split_once("</command-name>")) {
        return command.0.trim().starts_with('/').then(|| command.0.trim());
    }
    (!trimmed.is_empty() && !trimmed.starts_with('<')).then_some(text)
}

fn tool_use(block: &Value) -> Option<(&str, &str, &Value)> {
    (block["type"] == "tool_use").then_some(())?;
    Some((block["id"].as_str()?, block["name"].as_str()?, block.get("input").filter(|input| input.is_object())?))
}

fn tool_summary(name: &str, input: &Value) -> String {
    if let Some((server, tool)) = name.strip_prefix("mcp__").and_then(|s| s.split_once("__")) {
        return one_line(&format!("{server} / {tool}"));
    }
    let keys: &[&str] = match name {
        "Bash" | "PowerShell" => &["command"],
        "Read" | "Write" | "Edit" => &["file_path", "path"],
        "Grep" | "Glob" => &["pattern"],
        "Agent" | "Task" => &["description"],
        "WebSearch" => &["query"],
        "WebFetch" => &["url"],
        "Skill" => &["skill"],
        _ => &[],
    };
    one_line(keys.iter().find_map(|key| input[*key].as_str()).unwrap_or(name))
}

fn one_line(text: &str) -> String {
    text.split(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn agent_line(info: &mut Subagent, bytes: &[u8]) {
    let Some((kind, line)) = message_line(bytes) else { return };
    if let Some(time) = line["timestamp"].as_str() {
        info.started_at.get_or_insert_with(|| time.into());
        info.last_activity_at = Some(time.into());
    }
    if kind == LineKind::Assistant {
        for (_, name, input) in blocks(&line["message"]["content"]).filter_map(tool_use) {
            info.tool_calls += 1;
            info.last_tool_summary = Some(tool_summary(name, input));
        }
    }
}

enum Plan {
    Snapshot(Vec<Task>),
    Start,
    Advance,
    Done,
}

impl Plan {
    fn parse(name: &str, input: &Value) -> Option<Self> {
        let key = match name {
            "TodoWrite" => "todos",
            "mcp__ccd_turn__update_plan" => "items",
            _ => return None,
        };
        if let Some(items) = input[key].as_array() {
            let tasks: Vec<_> = items
                .iter()
                .filter_map(|item| {
                    let status = match item["status"].as_str()? {
                        "pending" => TaskStatus::Pending,
                        "in_progress" => TaskStatus::InProgress,
                        "completed" => TaskStatus::Completed,
                        _ => return None,
                    };
                    Some(Task { content: item["content"].as_str()?.into(), status })
                })
                .collect();
            return (items.is_empty() || !tasks.is_empty()).then_some(Self::Snapshot(tasks));
        }
        if key == "items" {
            for (key, action) in [("start", Self::Start), ("advance", Self::Advance), ("done", Self::Done)] {
                if input[key].as_bool() == Some(true) || input["action"].as_str() == Some(key) {
                    return Some(action);
                }
            }
        }
        None
    }

    fn apply(&self, tasks: &mut [Task]) {
        if matches!(self, Self::Advance | Self::Done)
            && let Some(active) = tasks.iter_mut().find(|task| task.status == TaskStatus::InProgress)
        {
            active.status = TaskStatus::Completed;
        }
        if matches!(self, Self::Start | Self::Advance)
            && !tasks.iter().any(|task| task.status == TaskStatus::InProgress)
            && let Some(next) = tasks.iter_mut().find(|task| task.status == TaskStatus::Pending)
        {
            next.status = TaskStatus::InProgress;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    User,
    Assistant,
    System,
}

fn message_line(line: &[u8]) -> Option<(LineKind, Value)> {
    let kind = line_kind(line)?;
    let value: Value = serde_json::from_slice(line).ok()?;
    // In case of duplicate JSON keys, do not expose content classified by a different type.
    let expected = match kind {
        LineKind::User => "user",
        LineKind::Assistant => "assistant",
        LineKind::System => "system",
    };
    (value["type"] == expected && !flag(&value, "isMeta")).then_some((kind, value))
}

/// Scan top-level keys without building a JSON tree or allocating for skipped values.
fn line_kind(mut bytes: &[u8]) -> Option<LineKind> {
    bytes = bytes.trim_ascii_start().strip_prefix(b"{")?;
    loop {
        bytes = bytes.trim_ascii_start();
        let key_end = string_end(bytes)?;
        let is_type = json_string_eq(&bytes[..key_end], "type");
        bytes = bytes[key_end..].trim_ascii_start().strip_prefix(b":")?.trim_ascii_start();
        if is_type {
            let value = &bytes[..string_end(bytes)?];
            return [("user", LineKind::User), ("assistant", LineKind::Assistant), ("system", LineKind::System)]
                .into_iter()
                .find_map(|(text, kind)| json_string_eq(value, text).then_some(kind));
        }
        let mut i = 0;
        let mut depth = 0usize;
        loop {
            match *bytes.get(i)? {
                b'"' => {
                    i += string_end(&bytes[i..])?;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth = depth.checked_sub(1)?,
                b',' if depth == 0 => break,
                _ => (),
            }
            i += 1;
        }
        bytes = &bytes[i + 1..];
    }
}

fn json_string_eq(bytes: &[u8], expected: &str) -> bool {
    let inner = &bytes[1..bytes.len() - 1];
    inner == expected.as_bytes()
        || (inner.contains(&b'\\') && serde_json::from_slice::<String>(bytes).is_ok_and(|s| s == expected))
}

fn string_end(bytes: &[u8]) -> Option<usize> {
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    loop {
        match *bytes.get(i)? {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
}

fn image_source(block: &Value) -> Option<(&str, &str)> {
    let source = &block["source"];
    (block["type"] == "image" && source["type"] == "base64").then_some(())?;
    Some((source["media_type"].as_str()?, source["data"].as_str()?))
}

fn invalid_picture(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn decode_base64(bytes: &[u8], limit: usize) -> io::Result<Vec<u8>> {
    let invalid = || invalid_picture("invalid base64");
    let digits = bytes.strip_suffix(b"==").or_else(|| bytes.strip_suffix(b"=")).unwrap_or(bytes);
    let tail = digits.len() % 4;
    let padding = bytes.len() - digits.len();
    if tail == 1 || (padding > 0 && (tail == 0 || tail + padding != 4)) {
        return Err(invalid());
    }
    let len = digits.len() / 4 * 3 + tail.saturating_sub(1);
    if len > limit {
        return Err(invalid_picture("picture exceeds 32 MiB"));
    }
    let mut out = Vec::with_capacity(len);
    let mut bits = 0u8;
    let mut acc = 0u16;
    for &digit in digits {
        let digit = match digit {
            b'A'..=b'Z' => digit - b'A',
            b'a'..=b'z' => digit - b'a' + 26,
            b'0'..=b'9' => digit - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(invalid()),
        };
        acc = (acc << 6) | u16::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if acc != 0 {
        return Err(invalid());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::time::{Instant, SystemTime};

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let nonce = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("cs-transcript-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn session(&self) -> PathBuf {
            self.0.join("session.jsonl")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn record(value: Value) -> String {
        format!("{value}\n")
    }

    fn append(path: &Path, bytes: impl AsRef<[u8]>) {
        OpenOptions::new().append(true).open(path).unwrap().write_all(bytes.as_ref()).unwrap();
    }

    fn prompt(text: &str) -> String {
        record(json!({"type":"user","message":{"content":text}}))
    }

    fn tool(id: &str, name: &str, input: Value) -> String {
        record(
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}),
        )
    }

    fn image(data: &str) -> Value {
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":data}})
    }

    #[test]
    fn filters_hidden_invalid_and_repeated_content_and_keeps_order() {
        let folder = Folder::new();
        let path = folder.session();
        let lines = [
            r#"{"message":{"content":"ignore"},"type":"api-request-blob"}"#,
            r#"{"type":"attachment","message":{"content":"ignore"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"hide"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"hide"}}"#,
            r#"{"type":"user","message":{"content":"  <system-reminder>hide</system-reminder>"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/x</command-name><command-args>ignore</command-args>"}}"#,
            r#"{"type":"user","message":{"content":"bad"#,
            r#"{"type":"user","message":{}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hide"},{"type":"text","text":"**hi**\n  there"},{"type":"text"},{"type":"tool_use","name":"Bash"}]},"timestamp":"2026-11-26T01:00:00Z"}"#,
            r#"{"t\u0079pe":"u\u0073er","message":{"content":"works"}}"#,
            r#"{"type":"user","message":{"content":"hide"},"type":"api-request-blob"}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}}"#,
            r#"{"type":"system","isApiErrorMessage":true,"error":{"message":"unavailable"}}"#,
            r#"{"type":"system","content":"ordinary system record"}"#,
        ];
        fs::write(&path, lines.join("\r\n") + "\r\n").unwrap();
        let reader = Reader::open(&path, 4096).unwrap();
        let items: Vec<_> = reader.items().map(|(_, item)| item).collect();
        assert_eq!(items.len(), 5);
        assert!(matches!(&items[0].kind, ItemKind::Prompt { text, .. } if text == "/x"));
        assert_eq!(items[1].timestamp.as_deref(), Some("2026-11-26T01:00:00Z"));
        assert!(matches!(&items[1].kind, ItemKind::Assistant { markdown } if markdown == "**hi**\n  there"));
        assert!(matches!(&items[2].kind, ItemKind::Prompt { text, .. } if text == "works"));
        assert_eq!(items[3].kind, ItemKind::Notice { kind: NoticeKind::Compaction, text: "one\ntwo".into() });
        assert_eq!(items[4].kind, ItemKind::Notice { kind: NoticeKind::ApiError, text: "unavailable".into() });
        assert!(!reader.has_earlier());
    }

    #[test]
    fn split_result_updates_only_its_call_and_result_picture_uses_real_offsets() {
        let folder = Folder::new();
        let path = folder.session();
        let calls = tool("good", "Bash", json!({"command":"echo first\n echo second"}))
            + &tool("bad", "Read", json!({"file_path":"example.rs"}));
        fs::write(&path, &calls).unwrap();
        let mut reader = Reader::open(&path, 4096).unwrap();
        let ids: Vec<_> = reader.items().map(|(id, _)| id).collect();
        assert!(matches!(&reader.item(ids[0]).unwrap().kind, ItemKind::Tool { summary, state: ToolState::Running, .. }
            if summary == "echo first echo second"));
        let result = record(json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"good","content":[{"type":"tool_reference"},{"type":"text","text":"ok"},image("AAH+/w==")]},
            {"type":"tool_result","tool_use_id":"bad","is_error":true,"content":"error"}
        ]}}));
        let split = result.len() / 2;
        append(&path, &result[..split]);
        assert_eq!(reader.poll().unwrap(), Changes::default());
        append(&path, &result[split..result.len() - 1]);
        assert_eq!(reader.poll().unwrap(), Changes::default());
        append(&path, "\n");
        let change = reader.poll().unwrap();
        assert_eq!(change.updated, BTreeSet::from([ids[0], ids[1]]));
        assert!(change.added.is_empty());
        let ItemKind::Tool { pictures, state: ToolState::Ok, .. } = &reader.item(ids[0]).unwrap().kind else {
            panic!()
        };
        let pic = pictures[0].clone();
        assert_eq!(
            (pic.line_offset, pic.line_len, pic.media_type.as_str(), pic.base64_len),
            (calls.len() as u64, result.len(), "image/png", 8)
        );
        assert_eq!(reader.load_picture(&pic).unwrap(), [0, 1, 254, 255]);
        assert!(matches!(&reader.item(ids[1]).unwrap().kind, ItemKind::Tool { state: ToolState::Failed, .. }));
        assert_eq!(reader.poll().unwrap(), Changes::default());
    }

    #[test]
    fn prompt_pictures_are_lazy_and_bad_data_is_reported_only_when_loaded() {
        let folder = Folder::new();
        let path = folder.session();
        let prefix = prompt("first");
        let line = record(json!({"type":"user","message":{"content":[
            {"type":"text","text":"look"},
            {"type":"text","text":"<hidden>"},
            image("aGk="),
            image("!!!!")
        ]}}));
        fs::write(&path, prefix.clone() + &line).unwrap();
        let mut reader = Reader::open(&path, 4096).unwrap();
        let items: Vec<_> = reader.items().map(|(_, item)| item).collect();
        let ItemKind::Prompt { text, pictures } = &items[1].kind else { panic!() };
        assert_eq!(text, "look");
        let pictures = pictures.clone();
        assert_eq!(pictures[0].line_offset, prefix.len() as u64);
        assert_eq!(pictures[0].line_len, line.len());
        assert_eq!(reader.load_picture(&pictures[0]).unwrap(), b"hi");
        assert_eq!(reader.load_picture(&pictures[1]).unwrap_err().kind(), io::ErrorKind::InvalidData);
        let mut oversized = pictures[0].clone();
        oversized.base64_len = MAX_PICTURE.div_ceil(3) * 4 + 4;
        assert!(reader.load_picture(&oversized).unwrap_err().to_string().contains("32 MiB"));
    }

    #[test]
    fn base64_validates_padding_bits_alphabet_and_size_before_decoding() {
        for (encoded, decoded) in [
            (b"".as_slice(), b"".as_slice()),
            (b"Zg==", b"f"),
            (b"Zm8=", b"fo"),
            (b"Zm9v", b"foo"),
            (b"Zm9vYg", b"foob"),
            (b"/+/9", &[255, 239, 253]),
        ] {
            assert_eq!(decode_base64(encoded, decoded.len()).unwrap(), decoded);
        }
        for bad in [b"Z".as_slice(), b"Zg=", b"Zm9v=", b"Z===", b"Zh==", b"Zm9=", b"Z g=", b"__8=", b"Z=g="] {
            assert!(decode_base64(bad, MAX_PICTURE).is_err(), "{bad:?}");
        }
        // This has the same encoded length as an image at the limit but decodes one byte larger.
        let big = vec![b'A'; MAX_PICTURE.div_ceil(3) * 4];
        assert!(decode_base64(&big, MAX_PICTURE).unwrap_err().to_string().contains("32 MiB"));
    }

    #[test]
    fn bounded_tail_and_history_keep_ids_and_apply_results_that_were_read_first() {
        let folder = Folder::new();
        let path = folder.session();
        let first = prompt(&"long".repeat(50_000));
        let call = tool("old", "Grep", json!({"pattern":"old\nvalue"}));
        let result = record(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"old"}]}}));
        let last = prompt("recent");
        fs::write(&path, first.clone() + &call + &result + &last).unwrap();
        let mut reader = Reader::open(&path, result.len() + last.len() + 5).unwrap();
        assert!(reader.has_earlier());
        assert_eq!(reader.items().len(), 1);
        let recent = reader.items().next().unwrap().0;
        let change = reader.earlier().unwrap();
        assert_eq!(change.added, BTreeSet::from([ItemId { offset: first.len() as u64, block: 0 }]));
        assert!(matches!(&reader.items().next().unwrap().1.kind, ItemKind::Tool { state: ToolState::Ok, summary, .. }
            if summary == "old value"));
        assert!(reader.has_earlier());
        assert_eq!(reader.earlier().unwrap().added.len(), 1);
        assert!(!reader.has_earlier());
        assert_eq!(reader.items().next_back().unwrap().0, recent);
        assert_eq!(reader.earlier().unwrap(), Changes::default());
    }

    #[test]
    fn clipped_partial_line_survives_poll_or_interleaved_history_load() {
        for load_history_first in [false, true] {
            let folder = Folder::new();
            let path = folder.session();
            let text = "long".repeat(4000);
            let line = prompt(&text);
            fs::write(&path, prompt("previous") + &line[..line.len() - 4]).unwrap();
            let mut reader = Reader::open(&path, 64).unwrap();
            assert_eq!(reader.items().len(), 0);
            assert!(reader.has_earlier());
            if load_history_first {
                assert_eq!(reader.earlier().unwrap().added.len(), 1);
            }
            append(&path, &line[line.len() - 4..line.len() - 1]);
            assert_eq!(reader.poll().unwrap(), Changes::default());
            append(&path, "\n".to_owned() + &prompt("new"));
            assert_eq!(reader.poll().unwrap().added.len(), if load_history_first { 2 } else { 1 });
            if !load_history_first {
                assert_eq!(reader.earlier().unwrap().added.len(), 1);
                assert_eq!(reader.earlier().unwrap().added.len(), 1);
            }
            assert!(!reader.has_earlier());
            assert!(
                matches!(&reader.items().nth(1).unwrap().1.kind, ItemKind::Prompt { text: got, .. } if *got == text)
            );
        }
    }

    #[test]
    fn initial_partial_first_line_is_kept_until_its_newline() {
        let folder = Folder::new();
        let path = folder.session();
        let line = prompt("a  b");
        fs::write(&path, &line[..line.len() - 1]).unwrap();
        let mut reader = Reader::open(&path, 4096).unwrap();
        assert_eq!(reader.items().len(), 0);
        assert!(!reader.has_earlier());
        assert_eq!(reader.poll().unwrap(), Changes::default());
        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(reader.poll().unwrap().added, BTreeSet::from([ItemId { offset: 0, block: 0 }]));
        drop(writer);
        assert_eq!(reader.poll().unwrap(), Changes::default());
        assert!(matches!(&reader.items().next().unwrap().1.kind, ItemKind::Prompt { text, .. } if text == "a  b"));
    }

    #[test]
    fn summaries_use_tool_specific_fields_and_preserve_inline_spaces() {
        for (name, input, expected) in [
            (
                "PowerShell",
                json!({"command":"Write-Output 'two  spaces'\r\nGet-Date"}),
                "Write-Output 'two  spaces' Get-Date",
            ),
            ("Write", json!({"file_path":"a  b.txt","content":"hidden"}), "a  b.txt"),
            ("Edit", json!({"path":"a.txt","new_string":"hidden"}), "a.txt"),
            ("Glob", json!({"pattern":"src/**/*.rs"}), "src/**/*.rs"),
            ("Task", json!({"description":"Describe work","prompt":"hidden"}), "Describe work"),
            ("WebSearch", json!({"query":"search phrase"}), "search phrase"),
            ("WebFetch", json!({"url":"https://example.com"}), "https://example.com"),
            ("Skill", json!({"skill":"sample"}), "sample"),
            ("mcp__server_name__tool__suffix", json!({"private":"hidden"}), "server_name / tool__suffix"),
        ] {
            assert_eq!(tool_summary(name, &input), expected);
        }
    }

    #[test]
    fn latest_task_snapshot_wins_and_actions_replay_in_file_order() {
        let folder = Folder::new();
        let path = folder.session();
        let plan = "mcp__ccd_turn__update_plan";
        let old = tool("old", "TodoWrite", json!({"todos":[{"content":"old","status":"completed"}]}));
        let snapshot = tool(
            "p",
            plan,
            json!({"items":[{"content":"first","status":"pending"},
            {"content":"second","status":"pending"},{"content":"invalid","status":"unknown"}]}),
        );
        fs::write(&path, old + &snapshot).unwrap();
        let mut reader = Reader::open(&path, snapshot.len() + 10).unwrap();
        assert_eq!(reader.tasks().len(), 2);
        assert!(reader.has_earlier());
        assert!(!reader.earlier().unwrap().tasks_changed);
        append(&path, tool("a", plan, json!({"start":true})));
        assert!(reader.poll().unwrap().tasks_changed);
        assert_eq!(reader.tasks()[0].status, TaskStatus::InProgress);
        append(&path, tool("b", plan, json!({"action":"advance"})));
        assert!(reader.poll().unwrap().tasks_changed);
        assert_eq!(reader.tasks()[0].status, TaskStatus::Completed);
        assert_eq!(reader.tasks()[1].status, TaskStatus::InProgress);
        append(&path, tool("c", plan, json!({"done":true})));
        assert!(reader.poll().unwrap().tasks_changed);
        assert!(reader.tasks().iter().all(|task| task.status == TaskStatus::Completed));
        append(&path, tool("d", "TodoWrite", json!({"todos":[]})));
        assert!(reader.poll().unwrap().tasks_changed);
        assert!(reader.tasks().is_empty());
    }

    #[test]
    fn subagent_without_meta_follows_activity_and_links_when_meta_appears() {
        let folder = Folder::new();
        let path = folder.session();
        let dir = path.with_extension("").join("subagents");
        fs::create_dir_all(&dir).unwrap();
        let agent_path = dir.join("agent-abc.jsonl");
        fs::write(&path, tool("spawn", "Agent", json!({"description":"Find\nthis","prompt":"private input"}))).unwrap();
        fs::write(
            &agent_path,
            record(json!({"type":"user","isSidechain":true,"timestamp":"2026-11-26T01:00:00Z",
            "message":{"content":"agent prompt"}})),
        )
        .unwrap();
        let mut reader = Reader::open(&path, 4096).unwrap();
        let spawn = reader.items().next().unwrap().0;
        assert!(
            matches!(&reader.item(spawn).unwrap().kind, ItemKind::Tool { summary, subagent: None, .. } if summary == "Find this")
        );
        let info = reader.subagents().next().unwrap();
        assert_eq!(info.id, "abc");
        assert_eq!(info.finished, None);
        assert_eq!(info.started_at.as_deref(), Some("2026-11-26T01:00:00Z"));
        assert_eq!(info.tool_calls, 0);
        let mut child = reader.open_subagent("abc", 4096).unwrap();
        assert_eq!(child.items().len(), 1);
        append(
            &agent_path,
            record(json!({"type":"assistant","isSidechain":true,"timestamp":"2026-11-26T01:02:00Z",
            "message":{"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"file.rs"}},
            {"type":"tool_use","id":"mcp","name":"mcp__server__tool","input":{}}]}})),
        );
        fs::write(
            dir.join("agent-abc.meta.json"),
            record(json!({"agentType":"explore","description":"Find this",
            "model":"model-x","finished":true,"toolUseId":"spawn","spawnDepth":1})),
        )
        .unwrap();
        let change = reader.poll().unwrap();
        assert!(change.subagents_changed);
        assert_eq!(change.updated, BTreeSet::from([spawn]));
        assert!(matches!(&reader.item(spawn).unwrap().kind, ItemKind::Tool { subagent: Some(id), .. } if id == "abc"));
        let info = reader.subagents().next().unwrap();
        assert_eq!(info.finished, Some(true));
        assert_eq!(info.spawn_depth, Some(1));
        assert_eq!(info.model.as_deref(), Some("model-x"));
        assert_eq!(info.agent_type.as_deref(), Some("explore"));
        assert_eq!(info.description.as_deref(), Some("Find this"));
        assert_eq!(info.last_activity_at.as_deref(), Some("2026-11-26T01:02:00Z"));
        assert_eq!(info.tool_calls, 2);
        assert_eq!(info.last_tool_summary.as_deref(), Some("server / tool"));
        assert_eq!(reader.poll().unwrap(), Changes::default());
        assert_eq!(child.poll().unwrap().added.len(), 2);
        drop(child);
        fs::remove_file(agent_path).unwrap();
        let change = reader.poll().unwrap();
        assert!(change.subagents_changed);
        assert_eq!(change.updated, BTreeSet::from([spawn]));
        assert_eq!(reader.subagents().len(), 0);
    }

    #[test]
    fn shrink_and_larger_replacement_reset_history() {
        let folder = Folder::new();
        let path = folder.session();
        let line = record(json!({"type":"user","message":{"content":[image("aGk=")]}}));
        fs::write(&path, line + &prompt("old")).unwrap();
        let mut reader = Reader::open(&path, 4096).unwrap();
        let ItemKind::Prompt { pictures, .. } = &reader.items().next().unwrap().1.kind else { panic!() };
        let stale_picture = pictures[0].clone();
        fs::write(&path, prompt("new")).unwrap();
        let change = reader.poll().unwrap();
        assert!(change.reset);
        assert_eq!(change.added.len(), 1);
        assert_eq!(reader.items().len(), 1);
        assert!(reader.load_picture(&stale_picture).is_err());
        fs::rename(&path, folder.0.join("old.jsonl")).unwrap();
        assert_eq!(reader.poll().unwrap(), Changes::default());
        fs::write(&path, prompt("a larger replacement")).unwrap();
        let change = reader.poll().unwrap();
        assert!(change.reset);
        assert_eq!(change.added.len(), 1);
        assert_eq!(reader.items().len(), 1);
        assert!(
            matches!(&reader.items().next().unwrap().1.kind, ItemKind::Prompt { text, .. } if text == "a larger replacement")
        );
    }

    #[test]
    fn ignores_large_blobs_with_type_before_or_after_the_payload() {
        let blob = "A".repeat(3 * 1024 * 1024);
        let first = format!(r#"{{"type":"api-request-blob","message":{{"content":"{blob}"}}}}"#);
        let last = format!(r#"{{"message":{{"type":"user","content":"{blob}"}},"type":"api-request-blob"}}"#);
        assert!(line_kind(first.as_bytes()).is_none());
        assert!(line_kind(last.as_bytes()).is_none());
        let folder = Folder::new();
        let path = folder.session();
        fs::write(&path, first + "\n" + &last + "\n" + &prompt("visible")).unwrap();
        let reader = Reader::open(&path, 8 * 1024 * 1024).unwrap();
        assert_eq!(reader.items().len(), 1);
    }

    #[test]
    #[ignore = "manual transcript timing on generated data"]
    fn timing_generated_20_mb() {
        let folder = Folder::new();
        let path = folder.session();
        let mut file = File::create(&path).unwrap();
        let user = prompt(&"a".repeat(1024));
        let assistant = record(json!({"type":"assistant","message":{"content":"**markdown**"}}));
        let picture = record(json!({"type":"user","message":{"content":[image(&"A".repeat(128 * 1024))]}}));
        let ignored = format!("{{\"type\":\"api-request-blob\",\"payload\":\"{}\"}}\n", "A".repeat(256 * 1024));
        let mut written = 0;
        let mut i = 0;
        while written < 20_000_000 {
            let line = match i % 128 {
                0 => &ignored,
                1 => &picture,
                n if n % 2 == 0 => &user,
                _ => &assistant,
            };
            file.write_all(line.as_bytes()).unwrap();
            written += line.len();
            i += 1;
        }
        drop(file);
        let start = Instant::now();
        let mut reader = Reader::open(&path, 2 * 1024 * 1024).unwrap();
        let open = start.elapsed();
        assert!(reader.has_earlier());
        let items = reader.items().len();
        assert!(items > 0);
        let start = Instant::now();
        let added = reader.earlier().unwrap().added.len();
        let earlier = start.elapsed();
        assert!(added > 0);
        append(&path, prompt("after timing"));
        let start = Instant::now();
        assert_eq!(reader.poll().unwrap().added.len(), 1);
        eprintln!(
            "{written} bytes: open {open:?} ({items} items), earlier {earlier:?} ({added} items), poll {:?}",
            start.elapsed()
        );
    }
}
