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
mod active;
mod background;
mod chat;
pub use background::{BackgroundShell, McpStatus, McpTask, ResultDetails, ShellStatus};
mod decode;
mod denial;
pub use denial::{Denial, DenialKind};
mod local;
pub use local::escape_len;
mod native_tasks;
mod notices;
pub use chat::{ChatInfo, Hunk, Patch, Phase as ChatPhase, Question, QuestionOption, ToolCard};
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
    /// Claude's readable thinking.
    Thought,
    HookError,
    /// A message a hook asked Claude Code to show (`systemMessage`).
    HookMessage,
    /// A hook refused the prompt.
    PromptBlocked,
    /// A Stop or SubagentStop hook kept Claude working.
    StopHook,
    Memory,
    /// The session recap shown on returning to the terminal.
    Recap,
    /// Another banner Claude Code marked as a warning.
    Info,
    /// Claude Code's own record of an Esc: "[Request interrupted by user]" (or "… for tool use").
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Prompt {
        text: String,
        pictures: Vec<Picture>,
        synthetic: bool,
        /// Sent while a turn ran (a `queued_command` record): Claude Code hands it to the running
        /// turn, so it starts no turn (its rewind list shows it all the same).
        queued: bool,
    },
    Assistant {
        markdown: String,
    },
    Tool {
        id: String,
        name: String,
        summary: String,
        file_path: Option<String>,
        state: ToolState,
        duration_ms: Option<u64>,
        pictures: Vec<Picture>,
        subagent: Option<String>,
        card: Option<ToolCard>,
        /// Claude Code refused the call (the user's No, a rule, auto mode): it never ran. Boxed:
        /// few calls are denied, and every item would carry the room.
        denial: Option<Box<Denial>>,
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

pub fn timestamp_ms(timestamp: &str) -> Option<i64> {
    chat::clock_ms(timestamp)
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
    /// Claude Code's colour for it (`red`, `blue`, `green`, `yellow`, `purple`, `orange`, `pink`,
    /// `cyan`), when its sidecar names one.
    pub color: Option<String>,
    /// How it ended and where that was read; `None` while nothing in the files says it ended
    /// (it may run, or have ended without a record: see [`AgentEnd`]).
    pub end: Option<(AgentEnd, EndSource)>,
    pub spawn_depth: Option<u64>,
    pub tool_use_id: Option<String>,
    pub started_at: Option<String>,
    pub last_activity_at: Option<String>,
    pub tool_calls: usize,
    pub last_tool_summary: Option<String>,
}

/// How a subagent's work ended. Claude Code 2.1.332 writes `finished: true` into the agent's
/// sidecar when it ends (also when it was stopped) and never `false`, so a running agent and one
/// whose end went unrecorded (a closed or crashed session) read the same here: no end. What the
/// terminal's agent list shows tells them apart while the session runs in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentEnd {
    Finished,
    /// Stopped by the user (the agent list's `x`, TaskStop) or by Claude Code.
    Stopped,
    /// Esc during its turn: its transcript, or its foreground call's result, says so.
    Interrupted,
    Failed,
}

/// Where an agent's end was read, the most direct first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndSource {
    /// The `<task-notification>` a background agent reports through.
    Notification,
    /// The result of the Agent call that ran it in the foreground.
    Result,
    /// Claude Code's `agent-<id>.meta.json` (`finished`, `stoppedByUser`, `sweptByStop`).
    Sidecar,
    /// The agent's own transcript ends with its final answer or an interruption.
    Transcript,
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
    pub background_changed: bool,
    /// Items left or joined the active branch (a prompt after a rewind): show `items()` afresh.
    pub branch_changed: bool,
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

    /// The items on the conversation's active branch, in transcript order.
    pub fn items(&self) -> impl DoubleEndedIterator<Item = (ItemId, &Item)> {
        self.view.items.iter().filter(|(id, _)| !self.view.active.hides(id.offset)).map(|(&id, item)| (id, item))
    }

    /// The session's background shells. One started in a turn a rewind replaced is left out once it
    /// is known to have ended; a rewind stops no process, so a running one stays (with Stop).
    pub fn background_shells(&self) -> Vec<BackgroundShell> {
        let mut shells = self.view.background.shells();
        shells.retain(|s| s.status == ShellStatus::Running || !self.view.active.hides(s.tool.offset));
        shells
    }

    /// The session's background MCP tasks, as far as the transcript records them; as for shells,
    /// one from a replaced turn only while it runs.
    pub fn mcp_tasks(&self) -> Vec<McpTask> {
        let mut tasks = self.view.background.mcp_tasks();
        tasks.retain(|t| t.status == McpStatus::Running || !self.view.active.hides(t.tool.offset));
        tasks
    }

    pub fn result_details(&self, id: ItemId) -> Option<&ResultDetails> {
        let ItemKind::Tool { id: tool, .. } = &self.item(id)?.kind else { return None };
        self.view.results.get(tool).map(|(_, r)| &r.details)
    }

    pub fn chat(&self) -> ChatInfo {
        let mut info = self.view.chat.clone();
        info.generation = self.view.generation;
        let newest = self.view.active.newest();
        info.continued_in =
            self.view.continued.as_ref().filter(|(at, _)| newest.is_none_or(|n| n < *at)).map(|(_, id)| id.clone());
        info.pending = self
            .view
            .items
            .iter()
            .rev()
            .take_while(|(id, _)| info.prompt.is_none_or(|prompt| id.offset > prompt))
            .find_map(|(id, item)| {
                matches!(item.kind, ItemKind::Tool { state: ToolState::Running, .. }).then_some(id.offset)
            });
        info
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.view.items.get(&id).filter(|_| !self.view.active.hides(id.offset))
    }

    pub fn tasks(&self) -> &[Task] {
        &self.view.tasks
    }

    /// The session's subagents. One whose call is in a turn a rewind replaced is left out once its
    /// end is known ([`Subagent::end`], from a notification, its result, its sidecar or its own
    /// transcript); a rewind does not stop it, so one nothing says has ended (it may run) stays.
    pub fn subagents(&self) -> impl Iterator<Item = &Subagent> {
        let replaced = |info: &Subagent| {
            info.tool_use_id
                .as_ref()
                .and_then(|tool| self.view.tools.get(tool))
                .is_some_and(|at| self.view.active.hides(at.offset))
        };
        self.agents.values().map(|agent| &agent.info).filter(move |info| info.end.is_none() || !replaced(info))
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
        self.settle_agents(&mut changes);
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
        // A message sent while a turn ran keeps its blocks in its queued_command attachment.
        let content =
            if value["type"] == "attachment" { &value["attachment"]["prompt"] } else { &value["message"]["content"] };
        let block = &content[picture.block];
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
                self.agents.insert(id.to_owned(), AgentFollow::new(tail, id));
                changes.subagents_changed = true;
            }
            seen.insert(id.to_owned());
            let agent = self.agents.get_mut(id).expect("agent was inserted");
            let before = agent.info.clone();
            if agent.tail.replaced()? {
                *agent = AgentFollow::new(Tail::open(&path, 0)?, id);
            }
            let meta = read_sidecar(&path.with_extension("meta.json"))?;
            agent.info.agent_type = string(&meta["agentType"]);
            agent.info.description = string(&meta["description"]);
            agent.info.model = string(&meta["model"]);
            agent.info.color = string(&meta["color"]);
            agent.sidecar_end = sidecar_end(&meta);
            agent.info.spawn_depth = meta["spawnDepth"].as_u64();
            agent.info.tool_use_id = string(&meta["toolUseId"]);
            let (info, end) = (&mut agent.info, &mut agent.transcript_end);
            agent.tail.append(|_, line| agent_line(info, end, line))?;
            agent.info.end = agent.end(&self.view);
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
        self.settle_agents(changes);
        Ok(())
    }

    /// The agents' ends again, after the session's own records changed (a notification, a result).
    fn settle_agents(&mut self, changes: &mut Changes) {
        for agent in self.agents.values_mut() {
            let end = agent.end(&self.view);
            changes.subagents_changed |= agent.info.end != end;
            agent.info.end = end;
        }
    }
}

struct AgentFollow {
    tail: Tail,
    info: Subagent,
    sidecar_end: Option<AgentEnd>,
    /// What the newest record of the agent's own transcript says: its final answer, an Esc, or
    /// nothing (it was mid-step).
    transcript_end: Option<AgentEnd>,
}

impl AgentFollow {
    fn new(tail: Tail, id: &str) -> Self {
        let info = Subagent { id: id.into(), ..Subagent::default() };
        Self { tail, info, sidecar_end: None, transcript_end: None }
    }

    /// The session's notification first (a background agent's own report), then its foreground
    /// call's result, then the sidecar, then the end of its own transcript.
    fn end(&self, view: &View) -> Option<(AgentEnd, EndSource)> {
        let result = || view.agent_results.get(self.info.tool_use_id.as_deref()?).map(|(_, end)| *end);
        view.background
            .agent_end(&self.info.id)
            .map(|end| (end, EndSource::Notification))
            .or_else(|| result().map(|end| (end, EndSource::Result)))
            .or(self.sidecar_end.map(|end| (end, EndSource::Sidecar)))
            .or(self.transcript_end.map(|end| (end, EndSource::Transcript)))
    }
}

/// Claude Code writes `stoppedByUser`, `sweptByStop` and `finished` only as `true`.
fn sidecar_end(meta: &Value) -> Option<AgentEnd> {
    if meta["stoppedByUser"] == true || meta["sweptByStop"] == true {
        Some(AgentEnd::Stopped)
    } else {
        (meta["finished"] == true).then_some(AgentEnd::Finished)
    }
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
    /// Native TaskCreate/TaskUpdate calls by their row, and every tool result's outcome by tool
    /// use id, replayed in transcript order (history can be paged in after newer records).
    native_calls: BTreeMap<ItemId, (String, native_tasks::Call)>,
    native_results: HashMap<String, (ItemId, native_tasks::Outcome)>,
    chat: ChatInfo,
    /// The active branch, so a rewind's replaced turns are not shown.
    active: active::Active,
    /// The newest `continued-in` record: its offset and the session it names.
    continued: Option<(u64, String)>,
    background: background::Background,
    /// How foreground Agent calls ended, by tool use id: from their results.
    agent_results: HashMap<String, (ItemId, AgentEnd)>,
    /// The newest slash command, for the output record that follows it.
    last_command: Option<String>,
    /// The newest shell-mode row, which its output record fills.
    last_shell: Option<ItemId>,
}

struct ToolResult {
    state: ToolState,
    denial: Option<Denial>,
    ended_ms: Option<i64>,
    pictures: Vec<Picture>,
    lines: Vec<String>,
    /// An edit's recorded patch, when the record holds this one result.
    patch: Vec<chat::Hunk>,
    details: ResultDetails,
}

impl View {
    fn line(&mut self, offset: u64, line: &[u8], changes: &mut Changes) {
        let line_kind = decode::line_kind(line);
        // Most attachments are large context records; parse only the kinds that are shown. Each
        // line is parsed once: those not parsed further give the active branch their links alone.
        if line_kind.is_none() || line_kind == Some(LineKind::Attachment) && !notices::shown_attachment(line) {
            // A `continued-in` record has no links; anything else may (progress, attachments).
            if !self.is_agent && !(line_kind.is_none() && self.continued_in(offset, line)) {
                self.active.line(offset, line);
            }
            return;
        }
        let Some((kind, value)) = message_line_with_meta(line) else { return };
        if !self.is_agent {
            self.active.value(offset, &value);
        }
        if flag(&value, "isSidechain") && !self.is_agent {
            return;
        }
        let timestamp = value["timestamp"].as_str();
        let id = |block| ItemId { offset, block };
        if matches!(kind, LineKind::User | LineKind::System | LineKind::Attachment)
            && self.background.notification(id(0), &value)
        {
            changes.background_changed = true;
            return;
        }
        if matches!(kind, LineKind::User | LineKind::System) {
            // A slash command's reply, whether Chat or the terminal sent the command.
            let content = &value["message"]["content"];
            let texts = [content, &value["content"]].into_iter().flat_map(|c| {
                c.as_str()
                    .into_iter()
                    .chain(blocks(c).filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()))
            });
            let texts: Vec<&str> = texts.collect();
            let mut output = false;
            for text in texts.iter().filter(|t| t.contains("<local-command-stdout>")) {
                self.chat.local_output(offset, text);
                output = true;
            }
            if output {
                self.chat.command_done(offset);
            } else if kind == LineKind::User
                && !flag(&value, "isMeta")
                && texts.iter().any(|t| t.trim_start().starts_with("<command-name>"))
            {
                self.chat.command_started(offset);
            }
            if self.local_record(offset, timestamp, &texts, changes) {
                return;
            }
        }
        if kind == LineKind::System && value["subtype"] == "compact_boundary" {
            self.chat.compaction(offset, Some(&value["compactMetadata"]));
        } else if flag(&value, "isCompactSummary") {
            self.chat.compaction(offset, None);
        }
        if flag(&value, "isMeta") {
            if kind == LineKind::User
                && let Some((kind, text)) = notices::meta(&value)
            {
                self.add(id(0), timestamp, ItemKind::Notice { kind, text }, changes);
            }
            return;
        }
        let content = &value["message"]["content"];
        if flag(&value, "isCompactSummary") || flag(&value, "isApiErrorMessage") {
            self.chat.observe(offset, ChatPhase::Unknown);
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
                if let Some(text) = content.as_str().and_then(prompt_value) {
                    texts.push(text);
                }
                for (i, block) in blocks(content).enumerate() {
                    match block["type"].as_str() {
                        Some("text") => texts.extend(block["text"].as_str().and_then(prompt_value)),
                        Some("image") => pictures.extend(picture(i, None, block)),
                        Some("tool_result") => {
                            if let Some(tool) = block["tool_use_id"].as_str() {
                                let state = if flag(block, "is_error") { ToolState::Failed } else { ToolState::Ok };
                                let pictures = blocks(&block["content"])
                                    .enumerate()
                                    .filter_map(|(j, block)| picture(i, Some(j), block))
                                    .collect();
                                let lines = chat::result_lines(&block["content"]);
                                let ended_ms = timestamp.and_then(chat::clock_ms);
                                let details = ResultDetails {
                                    text: background::bounded_text(&block["content"]),
                                    timestamp: timestamp.filter(|s| s.len() <= 64).map(str::to_owned),
                                };
                                let denial = denial::from_record(&value, block);
                                let patch = Vec::new();
                                let result = ToolResult { state, denial, ended_ms, pictures, lines, patch, details };
                                results.push((tool, id(i), result));
                            }
                        }
                        _ => (),
                    }
                }
                for outcome in native_tasks::outcomes(content, &value["toolUseResult"]) {
                    let known = self.tools.get(&outcome.tool).and_then(|at| self.items.get(at));
                    let task =
                        |item: &Item| matches!(&item.kind, ItemKind::Tool { name, .. } if name.starts_with("Task"));
                    if known.is_some_and(|item| !task(item)) {
                        continue;
                    }
                    self.plans_dirty |= known.is_some();
                    self.native_results.insert(outcome.tool.clone(), (id(0), outcome));
                }
                // Claude Code's own words for an Esc, written as a user text: not the user's prompt,
                // and no new turn (Claude Code 2.1.332, `recorder compat`'s `queued-esc`).
                if let Some(at) = texts.iter().position(|t| interrupted(t)) {
                    let text = texts.remove(at).trim().trim_matches(['[', ']']).to_owned();
                    self.add(id(0), timestamp, ItemKind::Notice { kind: NoticeKind::Interrupted, text }, changes);
                    if texts.is_empty() && pictures.is_empty() {
                        self.chat.observe(offset, ChatPhase::Active);
                    }
                }
                if !texts.is_empty() || !pictures.is_empty() {
                    let synthetic = flag(&value, "isMeta");
                    if !synthetic {
                        self.chat.prompt = Some(self.chat.prompt.map_or(offset, |old| old.max(offset)));
                        self.chat.observe(offset, ChatPhase::Active);
                    }
                    self.add(
                        id(0),
                        timestamp,
                        ItemKind::Prompt { text: texts.join("\n"), pictures, synthetic, queued: false },
                        changes,
                    );
                }
                if !results.is_empty() {
                    self.chat.observe(offset, ChatPhase::Active);
                }
                let structured = if results.len() == 1 { &value["toolUseResult"] } else { &Value::Null };
                // The record's denial is about its one result: with several, none can be named.
                let lone = results.len() == 1;
                for (tool, at, mut result) in results {
                    if !lone {
                        result.denial = None;
                    } else if result.state == ToolState::Ok {
                        // The record's structured result is about its one result too.
                        result.patch = chat::patch(structured);
                    }
                    if self.results.get(tool).is_none_or(|(previous, _)| *previous < at) {
                        self.background.result(tool, at, &result.details, structured, result.state == ToolState::Ok);
                        if let Some(end) = self.background.agent_result(tool, &result.lines, structured, result.state) {
                            self.agent_results.insert(tool.into(), (at, end));
                        }
                        changes.background_changed = true;
                        self.results.insert(tool.into(), (at, result));
                        self.update_tool(tool, changes);
                        if let Some(&item) = self.tools.get(tool).filter(|id| !changes.added.contains(id)) {
                            changes.updated.insert(item);
                        }
                    }
                }
            }
            LineKind::Assistant => {
                self.chat.usage(offset, &value["message"]);
                let phase = match value["message"]["stop_reason"].as_str() {
                    Some("end_turn") if !blocks(content).any(|b| b["type"] == "tool_use") => ChatPhase::EndTurn,
                    Some("tool_use") => ChatPhase::Active,
                    _ => ChatPhase::Unknown,
                };
                self.chat.observe(offset, phase);
                if let Some(markdown) = content.as_str().filter(|s| !s.is_empty()) {
                    self.add(id(0), timestamp, ItemKind::Assistant { markdown: markdown.into() }, changes);
                }
                // With thinking shown as updates, Claude Code shows a thinking block's text as an answer
                // line ("· summary"): it is what Claude says to the user, often the only answer to a
                // message sent while a turn ran.
                let updates = value["thinkingDisplay"] == "updates";
                for (i, block) in blocks(content).enumerate() {
                    if let Some(text) = notices::thought(block) {
                        let kind = if updates {
                            ItemKind::Assistant { markdown: text }
                        } else {
                            ItemKind::Notice { kind: NoticeKind::Thought, text }
                        };
                        self.add(id(i), timestamp, kind, changes);
                    } else if block["type"] == "text" {
                        if let Some(text) = block["text"].as_str().filter(|s| !s.is_empty()) {
                            self.add(id(i), timestamp, ItemKind::Assistant { markdown: text.into() }, changes);
                        }
                    } else if let Some((tool, name, input)) = tool_use(block) {
                        let item = ItemKind::Tool {
                            id: tool.into(),
                            name: name.into(),
                            summary: tool_summary(name, input),
                            file_path: tool_path(name, input).filter(|p| p.len() <= 4096).map(str::to_owned),
                            state: ToolState::Running,
                            duration_ms: None,
                            pictures: Vec::new(),
                            subagent: None,
                            card: ToolCard::from_input(name, input),
                            denial: None,
                        };
                        self.add(id(i), timestamp, item, changes);
                        self.tools.insert(tool.into(), id(i));
                        self.background.tool(tool, id(i), name, input, timestamp);
                        self.update_tool(tool, changes);
                        if let Some(plan) = Plan::parse(name, input) {
                            self.plans.insert(id(i), plan);
                            self.plans_dirty = true;
                        }
                        if let Some(call) = native_tasks::Call::parse(name, input) {
                            self.native_calls.insert(id(i), (tool.into(), call));
                            self.plans_dirty = true;
                        }
                    }
                }
            }
            LineKind::System => {
                if let Some((kind, text)) = notices::system(&value) {
                    self.add(id(0), timestamp, ItemKind::Notice { kind, text }, changes);
                }
            }
            LineKind::Attachment => {
                let attachment = &value["attachment"];
                if let Some(prompt) = queued_prompt(attachment, self.is_agent) {
                    // A message the user sent while a turn ran: Claude Code records it only here and
                    // hands it to Claude inside the running turn, so it starts no turn of its own.
                    let mut texts = Vec::new();
                    let mut pictures = Vec::new();
                    texts.extend(prompt.as_str().and_then(prompt_value));
                    for (i, block) in blocks(prompt).enumerate() {
                        match block["type"].as_str() {
                            Some("text") => texts.extend(block["text"].as_str().and_then(prompt_value)),
                            Some("image") => pictures.extend(picture(i, None, block)),
                            _ => (),
                        }
                    }
                    if !texts.is_empty() || !pictures.is_empty() {
                        self.chat.observe(offset, ChatPhase::Active);
                        let item =
                            ItemKind::Prompt { text: texts.join("\n"), pictures, synthetic: false, queued: true };
                        self.add(id(0), timestamp, item, changes);
                    }
                } else if let Some((kind, text)) = notices::attachment(attachment) {
                    self.add(id(0), timestamp, ItemKind::Notice { kind, text }, changes);
                }
            }
        }
    }

    /// Claude Code's record that the conversation goes on in another session (written last in the
    /// transcript a session left, seen right after a `/fork` from 2.1.325 and once from 2.1.332).
    /// True when the line is such a record, whatever it names.
    fn continued_in(&mut self, offset: u64, line: &[u8]) -> bool {
        if !line.windows(14).any(|w| w == b"\"continued-in\"") {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else { return false };
        if value["type"] != "continued-in" {
            return false;
        }
        if let Some(id) = value["continuedInSessionId"].as_str()
            && crate::launch::validate_session_id(id).is_ok()
            && self.continued.as_ref().is_none_or(|(at, _)| *at < offset)
        {
            self.continued = Some((offset, id.to_owned()));
        }
        true
    }

    /// Command output and shell mode as finished rows; true when the record was one of them. The CLI
    /// may write a shell command and its output as two text blocks of one record, so every block is read.
    fn local_record(&mut self, offset: u64, timestamp: Option<&str>, texts: &[&str], changes: &mut Changes) -> bool {
        let mut found = false;
        for (block, text) in texts.iter().enumerate() {
            let id = ItemId { offset, block };
            if let Some(command) =
                text.trim_start().strip_prefix("<command-name>").and_then(|s| s.split_once("</command-name>"))
            {
                let args = text.split_once("<command-args>").and_then(|(_, s)| s.split_once("</command-args>"));
                let args = args.map_or("", |(a, _)| a.trim());
                let line = [command.0.trim(), args].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                self.last_command = Some(one_line(&line));
                return false;
            }
            let Some(record) = local::parse(text) else {
                // A command that printed nothing (a menu closed without a choice): its name must not
                // label the next command's output.
                if text.trim_start().starts_with("<local-command-std") {
                    self.last_command = None;
                }
                continue;
            };
            found = true;
            let key = format!("local:{offset}:{block}");
            match record {
                local::Local::Output { text, ansi, failed } => {
                    let name = self.last_command.take().unwrap_or_else(|| "Command output".to_owned());
                    let item = local::item(key, &name, String::new(), text, ansi, failed);
                    self.add(id, timestamp, item, changes);
                }
                local::Local::Shell(input) => {
                    let item = local::item(key, "Shell", format!("! {input}"), String::new(), None, false);
                    self.add(id, timestamp, item, changes);
                    self.last_shell = Some(id);
                }
                local::Local::ShellOutput { text, ansi, failed } => match self.last_shell.take() {
                    Some(shell) if self.items.contains_key(&shell) => {
                        if let Some(Item { kind: ItemKind::Tool { card, state, .. }, .. }) = self.items.get_mut(&shell)
                        {
                            *card = Some(ToolCard::Output { text, ansi });
                            *state = if failed { ToolState::Failed } else { ToolState::Ok };
                        }
                        if !changes.added.contains(&shell) {
                            changes.updated.insert(shell);
                        }
                    }
                    _ => {
                        let item = local::item(key, "Shell", String::new(), text, ansi, failed);
                        self.add(id, timestamp, item, changes);
                    }
                },
            }
        }
        found
    }

    fn add(&mut self, id: ItemId, timestamp: Option<&str>, kind: ItemKind, changes: &mut Changes) {
        self.items.insert(id, Item { timestamp: timestamp.map(str::to_owned), kind });
        changes.added.insert(id);
    }

    fn update_tool(&mut self, tool: &str, changes: &mut Changes) {
        let Some(&id) = self.tools.get(tool) else { return };
        let Some(Item { kind: ItemKind::Tool { state, duration_ms, pictures, subagent, card, denial, .. }, timestamp }) =
            self.items.get_mut(&id)
        else {
            return;
        };
        let result = self.results.get(tool).map(|(_, result)| result);
        let new_state = result.map_or(ToolState::Running, |r| r.state);
        let new_duration = timestamp
            .as_deref()
            .and_then(chat::clock_ms)
            .zip(result.and_then(|r| r.ended_ms))
            .and_then(|(start, end)| u64::try_from(end - start).ok());
        let new_pictures = result.map_or(&[][..], |r| r.pictures.as_slice());
        let new_agent = self.links.get(tool);
        let new_denial = result.and_then(|r| r.denial.as_ref());
        let changed_result = if let Some(ToolCard::Shell { result: lines, .. }) = card {
            let new = result.map_or(&[][..], |r| r.lines.as_slice());
            if *lines != new {
                *lines = new.to_vec();
                true
            } else {
                false
            }
        } else if let Some(ToolCard::Edit { patch, .. }) = card {
            let new = result.map_or(&[][..], |r| r.patch.as_slice());
            if patch.as_deref().map_or(&[][..], |p| p.hunks.as_slice()) != new {
                *patch = (!new.is_empty()).then(|| Box::new(chat::Patch { hunks: new.to_vec() }));
                true
            } else {
                false
            }
        } else {
            false
        };
        if *state != new_state
            || *duration_ms != new_duration
            || *pictures != new_pictures
            || subagent.as_ref() != new_agent
            || denial.as_deref() != new_denial
            || changed_result
        {
            *state = new_state;
            *denial = new_denial.cloned().map(Box::new);
            *duration_ms = new_duration;
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
        if self.active.finish() {
            changes.branch_changed = true;
            changes.added.retain(|id| !self.active.hides(id.offset));
            changes.updated.retain(|id| !self.active.hides(id.offset));
            // The pages read the branch too: the plan again, the agents and shells afresh.
            self.plans_dirty = true;
            changes.subagents_changed = true;
            changes.background_changed = true;
        }
        if !std::mem::take(&mut self.plans_dirty) {
            return;
        }
        // A later snapshot supersedes all earlier snapshots and actions, including paged-in history.
        // Only the active branch's plan updates and task calls count (a rewind's replaced turns do not).
        let active = &self.active;
        let mut plans = self.plans.iter().filter(|(id, _)| !active.hides(id.offset)).map(|(_, plan)| plan).rev();
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
        // Native tasks replace the plan's list when they changed after its last update, as in a
        // handover.
        if let Some((native, at)) = self.native_tasks()
            && self.plans.keys().rfind(|id| !self.active.hides(id.offset)).is_none_or(|plan| *plan < at)
        {
            tasks = native;
        }
        if self.tasks != tasks {
            self.tasks = tasks;
            changes.tasks_changed = true;
        }
    }
}

impl View {
    /// The native task list and where it last changed, when it ever did.
    fn native_tasks(&self) -> Option<(Vec<Task>, ItemId)> {
        enum Event<'a> {
            Call(&'a str, &'a native_tasks::Call),
            Outcome(&'a native_tasks::Outcome),
        }
        let mut events = Vec::new();
        for (at, (tool, call)) in self.native_calls.iter().filter(|(at, _)| !self.active.hides(at.offset)) {
            events.push((*at, Event::Call(tool, call)));
            if let Some((when, outcome)) =
                self.native_results.get(tool).filter(|(when, _)| !self.active.hides(when.offset))
            {
                events.push((*when, Event::Outcome(outcome)));
            }
        }
        events.sort_by_key(|(at, event)| (*at, matches!(event, Event::Outcome(_))));
        let mut tasks = native_tasks::Tasks::default();
        let mut latest = None;
        for (at, event) in events {
            match event {
                Event::Call(tool, call) => tasks.pend(tool, call.clone()),
                Event::Outcome(outcome) => {
                    if tasks.outcome(outcome.clone()) {
                        latest = Some(at);
                    }
                }
            }
        }
        latest.map(|at| (tasks.finish(), at))
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

/// The content of a message the user typed while a turn ran (`queued_command`, `commandMode:
/// "prompt"`, `origin.kind: "human"`; recorded from Claude Code 2.1.332). Background task
/// notifications and messages from other sessions use the same record and are not prompts.
///
/// A message the user sent to a running subagent (its conversation shown, the "· 1 queued" on its
/// row) waits in Claude Code's memory and is written into the agent's own transcript when it is
/// handed in at the agent's next step, as a `queued_command` without `commandMode` (2.1.332's
/// bundle; `recorder compat`'s `agent-message`): in an agent's transcript that is a prompt too.
fn queued_prompt(attachment: &Value, agent: bool) -> Option<&Value> {
    let mode = &attachment["commandMode"];
    let human = attachment["origin"]["kind"] == "human" || agent && attachment["origin"].is_null();
    (attachment["type"] == "queued_command" && (mode == "prompt" || agent && mode.is_null()) && human)
        .then(|| &attachment["prompt"])
}

/// A prompt's text as Claude Code showed it: a long paste is recorded wrapped as
/// `<pasted_content id="N">\n…\n</pasted_content id="N">` (2.1.332, `recorder compat`'s
/// `queued-text`); the tags go, the text stays. `None` for what is no prompt (see `prompt_text`).
fn prompt_value(text: &str) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    match unwrap_pastes(text) {
        Cow::Borrowed(text) => prompt_text(text).map(Cow::Borrowed),
        Cow::Owned(text) => prompt_text(&text).is_some().then_some(Cow::Owned(text)),
    }
}

/// The text with every `<pasted_content …>` tag pair taken out around what it holds.
fn unwrap_pastes(text: &str) -> std::borrow::Cow<'_, str> {
    const OPEN: &str = "<pasted_content";
    const CLOSE: &str = "</pasted_content";
    if !text.contains(OPEN) {
        return text.into();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(OPEN) {
        let after = &rest[at..];
        let Some(open_end) = after.find('>') else { break };
        let inner = &after[open_end + 1..];
        let Some(close) = inner.find(CLOSE) else { break };
        let Some(close_end) = inner[close..].find('>') else { break };
        out.push_str(&rest[..at]);
        let body = inner[..close].strip_prefix('\n').unwrap_or(&inner[..close]);
        out.push_str(body.strip_suffix('\n').unwrap_or(body));
        rest = &inner[close + close_end + 1..];
    }
    out.push_str(rest);
    out.into()
}

/// Claude Code's record of an Esc: `[Request interrupted by user]`, `… for tool use]`.
fn interrupted(text: &str) -> bool {
    matches!(text.trim(), "[Request interrupted by user]" | "[Request interrupted by user for tool use]")
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

fn tool_path<'a>(name: &str, input: &'a Value) -> Option<&'a str> {
    match name {
        "Read" | "Write" | "Edit" => input["file_path"].as_str().or_else(|| input["path"].as_str()),
        "NotebookEdit" => input["notebook_path"].as_str(),
        _ => None,
    }
}

fn one_line(text: &str) -> String {
    text.split(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod active_pages_tests;
#[cfg(test)]
mod active_tests;
#[cfg(test)]
mod tests;
