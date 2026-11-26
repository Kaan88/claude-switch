//! Incremental, read-only views of Claude Code JSONL transcripts.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub(crate) mod handover;

const MAX_PICTURE: usize = 32 * 1024 * 1024;
pub(crate) const MAX_LINE: usize = 48 * 1024 * 1024;
const MAX_SIDECAR: u64 = 1024 * 1024;
mod decode;
mod tail;
use decode::*;
pub(crate) use decode::{LineKind, line_kind};
use tail::Tail;
use tail::bounded_line;

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
        if picture.line_len > MAX_LINE {
            return Err(invalid_picture("containing line exceeds the transcript line limit"));
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
            let meta = read_sidecar(&path.with_extension("meta.json"))?;
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

fn read_sidecar(path: &Path) -> io::Result<Value> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR + 1).read_to_end(&mut bytes)?;
    Ok(if bytes.len() as u64 > MAX_SIDECAR {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    })
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

pub(crate) fn prompt_text(text: &str) -> Option<&str> {
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

#[cfg(test)]
mod tests;
