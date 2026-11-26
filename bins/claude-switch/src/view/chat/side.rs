//! Details rows are a read-only projection of the current pane's transcript.
mod extra;
mod paint;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use claude_switch_core::transcript::{ItemId, ItemKind, TaskStatus, ToolCard, ToolState};

use super::super::Pane;
use super::super::links::{self, NamedFile};
use super::super::model::Model;
use super::Side;

pub(in crate::view) use paint::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    tab: u64,
    revision: u64,
    side: Side,
    index: usize,
    part: Part,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Expand,
    Open,
    Edit(usize),
    AgentTranscript,
    Full,
    Follow,
    Stop,
    /// The tool call a background item began with, in the conversation.
    Call,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Task(usize, String),
    Agent(String),
    File(FileKey),
    Shell(ItemId),
    Mcp(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FileKey {
    Local(PathBuf),
    Unavailable(String),
}

struct Change {
    id: ItemId,
    label: String,
}

struct ChangedFile {
    key: FileKey,
    path: String,
    file: Option<NamedFile>,
    edits: Vec<Change>,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Expand(Key),
    Preview(NamedFile),
    DefaultApp(NamedFile),
    Jump(ItemId),
    AgentTranscript(String),
    Output(super::super::outputs::Destination),
    ShellFollow(ItemId),
    ShellFull(ItemId),
    ShellStop(ItemId),
    McpResult(String),
}

#[derive(Default)]
pub(in crate::view) struct Details {
    generation: Option<u64>,
    files_revision: Option<u64>,
    files: Vec<ChangedFile>,
    expanded: BTreeSet<Key>,
    offset: f32,
    max: f32,
    reveal: Option<Key>,
    message: Option<&'static str>,
    frozen: BTreeMap<ItemId, Option<Vec<String>>>,
    pub(in crate::view) visible_shells: Vec<ItemId>,
}

impl Details {
    pub fn reset_scroll(&mut self) {
        self.offset = 0.0;
        self.reveal = None;
        self.message = None;
        self.visible_shells.clear();
    }

    pub fn wheel(&mut self, dy: f32) {
        self.offset = (self.offset - dy).clamp(0.0, self.max);
    }

    fn refresh(&mut self, model: &Model, cwd: &Path, side: Side) {
        if self.generation != Some(model.generation) {
            *self = Self { generation: Some(model.generation), ..Self::default() };
        }
        if side == Side::Files && self.files_revision != Some(model.revision) {
            self.files = changed_files(model, cwd);
            self.files_revision = Some(model.revision);
        }
    }

    fn action(&self, model: &Model, tab: u64, side: Side, hit: Hit) -> Option<Action> {
        if hit.tab != tab
            || hit.side != side
            || hit.revision != model.revision
            || self.generation != Some(model.generation)
        {
            return None;
        }
        let i = hit.index;
        match (side, hit.part) {
            (Side::Tasks, Part::Expand) => Some(Action::Expand(task_key(i, &model.tasks.get(i)?.content))),
            (Side::Tasks, Part::Open) => model.tasks.get(i).and(plan_update(model)).map(Action::Jump),
            (Side::Agents, Part::Expand) => Some(Action::Expand(Key::Agent(model.agents.get(i)?.id.clone()))),
            (Side::Agents, Part::Open) => {
                let agent = model.agents.get(i)?;
                (model.agent.as_deref() != Some(agent.id.as_str())).then(|| Action::AgentTranscript(agent.id.clone()))
            }
            (Side::Agents, Part::AgentTranscript) => {
                let agent = model.agents.get(i)?;
                self.expanded.contains(&Key::Agent(agent.id.clone())).then(|| Action::AgentTranscript(agent.id.clone()))
            }
            (Side::Files, part) if self.files_revision == Some(model.revision) => {
                let file = self.files.get(i)?;
                match part {
                    Part::Expand => Some(Action::Expand(Key::File(file.key.clone()))),
                    Part::Open => open_action(file.file.as_ref()?),
                    Part::Edit(j) if self.expanded.contains(&Key::File(file.key.clone())) => {
                        Some(Action::Jump(file.edits.get(j)?.id))
                    }
                    _ => None,
                }
            }
            (Side::Shells | Side::Outputs, _) => self.extra_action(model, side, i, hit.part),
            _ => None,
        }
    }

    fn toggle(&mut self, key: Key) {
        self.reveal = if self.expanded.remove(&key) {
            None
        } else {
            self.expanded.insert(key.clone());
            Some(key)
        };
    }
}

pub(in crate::view) enum Activated {
    None,
    Focus,
    Stop(ItemId),
}

/// Shared for the Chat rail and the Session view's Details pages.
pub(in crate::view) fn activate(pane: &mut Pane, tab: u64, side: Side, hit: Hit) -> Activated {
    let chat = &mut pane.chat;
    if pane.ui.overlay_open() || (chat.enabled && (chat.side_hidden || chat.layout.side.w <= 0.0)) {
        return Activated::None;
    }
    let Some(action) = chat.details.action(&pane.model, tab, side, hit) else { return Activated::None };
    chat.details.message = None;
    match action {
        Action::Expand(key) => chat.details.toggle(key),
        Action::Preview(file) => {
            if file.available() {
                pane.ui.open_markdown(file, &mut pane.model);
                return Activated::Focus;
            }
            chat.details.message = Some("The file is no longer available.");
        }
        Action::DefaultApp(file) => {
            if !file.open(false) {
                chat.details.message = Some("Could not open the file in its default app.");
            }
        }
        Action::Jump(id) => {
            if !pane.ui.jump_to_tool(&pane.model, id) {
                chat.details.message = Some("This edit is no longer in the loaded conversation.");
            }
        }
        Action::AgentTranscript(id) => pane.ui.open_agent(&id, &pane.model),
        Action::Output(output) => {
            pane.ui.open_output(output, &mut pane.model);
            return Activated::Focus;
        }
        Action::ShellFollow(id) => {
            if chat.details.frozen.remove(&id).is_none() {
                chat.details.frozen.insert(id, pane.model.shell_tails.get(&id).cloned().flatten());
            }
        }
        Action::ShellFull(id) => {
            if extra::open_full(pane, id) {
                return Activated::Focus;
            }
        }
        Action::ShellStop(id) => return Activated::Stop(id),
        Action::McpResult(id) => {
            if extra::open_result(pane, &id) {
                return Activated::Focus;
            }
        }
    }
    Activated::None
}

/// The newest plan or native task update in the loaded conversation: where the task list was
/// last set.
fn plan_update(model: &Model) -> Option<ItemId> {
    model.items.iter().rev().find_map(|(&id, item)| {
        matches!(&item.kind, ItemKind::Tool { name, .. } if matches!(name.as_str(), "TodoWrite" | "mcp__ccd_turn__update_plan" | "TaskCreate" | "TaskUpdate"))
            .then_some(id)
    })
}

fn open_action(file: &NamedFile) -> Option<Action> {
    if file.markdown() {
        Some(Action::Preview(file.clone()))
    } else if file.supports_default_app() {
        Some(Action::DefaultApp(file.clone()))
    } else {
        None
    }
}

fn task_key(index: usize, content: &str) -> Key {
    Key::Task(index, content.to_owned())
}

fn task_status(status: TaskStatus) -> (&'static str, &'static str) {
    match status {
        TaskStatus::Completed => ("✓", "Completed"),
        TaskStatus::InProgress => ("◉", "In progress"),
        TaskStatus::Pending => ("○", "Pending"),
    }
}

fn changed_files(model: &Model, cwd: &Path) -> Vec<ChangedFile> {
    let mut indices = BTreeMap::new();
    let mut files: Vec<ChangedFile> = Vec::new();
    for (&id, item) in model.items.iter().rev() {
        let ItemKind::Tool { name, file_path: Some(path), state: ToolState::Ok, card, .. } = &item.kind else {
            continue;
        };
        if !matches!(name.as_str(), "Write" | "Edit") {
            continue;
        }
        let key = links::transcript_path(path, cwd).map_or_else(|| FileKey::Unavailable(path.clone()), FileKey::Local);
        let index = *indices.entry(key.clone()).or_insert_with(|| {
            files.push(ChangedFile { key, path: path.clone(), file: None, edits: vec![] });
            files.len() - 1
        });
        let entry = &mut files[index];
        if entry.file.is_none() {
            entry.file = model.files.get(&id).and_then(|paths| paths.get(path)).cloned();
        }
        let mut label = name.to_string();
        if let Some(time) = item.timestamp.as_deref().and_then(|s| s.get(11..19)) {
            label.push_str(&format!(" · {time}"));
        }
        if let Some(ToolCard::Edit { added, removed, .. }) = card {
            label.push_str(&format!("  +{added} −{removed}"));
        }
        entry.edits.push(Change { id, label });
    }
    files
}

#[cfg(test)]
mod tests;
