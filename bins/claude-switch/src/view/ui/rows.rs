//! Shared Chat/Session projection. Stable row identities preserve the reader's scroll anchor.
use std::collections::{BTreeMap, BTreeSet};

use claude_switch_core::transcript::{
    Item, ItemId, ItemKind, NoticeKind, Subagent, Task, TaskStatus, ToolCard, ToolState,
};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::theme::Palette;

use super::super::links::{Files, NamedFile};
use super::super::model::{Group, Model};
use super::super::preview::Preview;
use super::super::text::TextLayout;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Id {
    Message(ItemId),
    Tool(ItemId),
    Group(ItemId),
    Files(ItemId),
    Agent(String),
    Task(usize),
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Prompt,
    Answer,
    Tool,
    Group,
    Files,
    Card,
}

pub(super) const HEAD: f32 = 31.0;
pub(super) const TOOL_H: f32 = 25.0;
pub(super) const PIC: f32 = 104.0;
pub(super) const CHIP_H: f32 = 26.0;
const INSET: f32 = 12.0;
const TOOL_INDENT: f32 = 15.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct Geometry {
    pub x: f32,
    pub w: f32,
    pub tx: f32,
    pub tw: f32,
    pub ty: f32,
}

impl Geometry {
    fn new(style: Style, w: f32, prompt_w: f32) -> Self {
        let answer = 25.0_f32.min(w * 0.14);
        let tucked = answer + TOOL_INDENT.min(w * 0.08);
        let (x, width, pad, ty) = match style {
            Style::Prompt => {
                let width = prompt_w.clamp(1.0, Self::prompt_max(w));
                (w - width, width, INSET, 8.0)
            }
            Style::Answer => (answer, w - answer, 0.0, 0.0),
            Style::Tool => (tucked, w - tucked, 8.0, TOOL_H + 6.0),
            Style::Group => (tucked, w - tucked, 8.0, TOOL_H),
            Style::Files => (answer, w - answer, 0.0, 0.0),
            Style::Card => (0.0, w, INSET, HEAD),
        };
        let pad = pad.min(width * 0.20);
        Self { x, w: width, tx: x + pad, tw: (width - 2.0 * pad).max(1.0), ty }
    }

    fn prompt_max(w: f32) -> f32 {
        (w * 0.82).max(1.0)
    }
}

pub(super) fn chip_rect(geo: Geometry, index: usize) -> Rect {
    let columns = (geo.w / 158.0).floor().max(1.0) as usize;
    let slot = (geo.w / columns as f32).min(178.0);
    Rect::new(
        geo.x + (index % columns) as f32 * slot,
        (index / columns) as f32 * CHIP_H,
        (slot - 5.0).max(1.0),
        CHIP_H - 4.0,
    )
}

pub(super) struct Row {
    pub id: Id,
    pub style: Style,
    pub geo: Geometry,
    pub top: f32,
    pub h: f32,
    pub title: String,
    pub stamp: String,
    pub body: Option<Body>,
    pub pictures: Vec<(ItemId, usize)>,
    pub files: Vec<NamedFile>,
    pub target: String,
    pub duration: String,
    pub open: bool,
    pub busy: bool,
    pub started: Option<i64>,
    pub agent: Option<String>,
    pub tone: Tone,
}

/// Reuse unchanged shaping across appends and prepended history.
pub(super) struct Body {
    raw: String,
    kind: BodyKind,
    width: f32,
    layout: Layout,
    pub files: Files,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Plain,
    Markdown,
    Mono,
    Shell,
}

enum Layout {
    Plain(Box<TextLayout>),
    Markdown(Box<Preview>),
}

impl Body {
    pub fn text(&self) -> &str {
        match &self.layout {
            Layout::Plain(_) => &self.raw,
            Layout::Markdown(t) => t.text(),
        }
    }

    pub fn height(&self) -> f32 {
        match &self.layout {
            Layout::Plain(t) => t.height(),
            Layout::Markdown(t) => t.height(),
        }
    }

    fn used_width(&self) -> f32 {
        match &self.layout {
            Layout::Plain(t) => t.width(),
            Layout::Markdown(_) => self.width,
        }
    }

    pub fn point(&self, x: f32, y: f32) -> (usize, Option<&str>) {
        match &self.layout {
            Layout::Plain(t) => (t.at(x, y), t.link(x, y)),
            Layout::Markdown(t) => t.point(x, y),
        }
    }

    pub fn draw(
        &self,
        p: &mut Painter,
        area: Rect,
        clip: Rect,
        selection: Option<std::ops::Range<usize>>,
        pal: &Palette,
    ) {
        match &self.layout {
            Layout::Plain(t) => t.draw(p, area.x, area.y, selection, pal),
            Layout::Markdown(t) => t.draw(p, area, clip, selection, pal),
        }
    }
}

impl Row {
    pub fn status(&self) -> std::borrow::Cow<'_, str> {
        if let Some(start) = self.started
            && let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            && let Some(elapsed) = (now.as_secs() as i64).checked_sub(start).filter(|&s| s >= 0)
        {
            format!("{} · {}", self.stamp, mdshared_app::fmt::duration(elapsed as u64)).into()
        } else {
            self.stamp.as_str().into()
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Normal,
    Accent,
    Muted,
    Warning,
}

pub(super) struct Rows<'a> {
    pub previous: BTreeMap<Id, Body>,
    pub list: Vec<Row>,
    pub end: f32,
    pub width: f32,
    pub scale: f32,
    pub pal: &'a Palette,
    pub chat: bool,
}

impl Rows<'_> {
    fn shape(&mut self, id: &Id, raw: &str, width: f32, kind: BodyKind, files: Option<&Files>) -> Body {
        if let Some(body) = self.previous.remove(id)
            && body.kind == kind
            && body.width == width
            && body.raw == raw
            && files.map_or_else(|| body.files.is_empty(), |f| *f == body.files)
        {
            return body;
        }
        let size = if self.chat { 16.0 } else { 14.0 };
        let empty = Files::new();
        let layout = if kind == BodyKind::Markdown {
            Layout::Markdown(Box::new(Preview::new(
                raw,
                self.pal,
                self.scale,
                width,
                size,
                Some(files.unwrap_or(&empty)),
            )))
        } else {
            let mono = matches!(kind, BodyKind::Mono | BodyKind::Shell);
            Layout::Plain(Box::new(TextLayout::new(
                raw,
                self.pal,
                self.scale,
                width,
                if mono { 12.0 } else { size },
                if kind == BodyKind::Shell { None } else { Some(files.unwrap_or(&empty)) },
                mono,
            )))
        };
        Body { raw: raw.to_owned(), kind, width, layout, files: files.cloned().unwrap_or_default() }
    }

    fn row(&self, id: Id, style: Style) -> Row {
        Row {
            id,
            style,
            geo: Geometry::new(style, self.width, Geometry::prompt_max(self.width)),
            top: self.end,
            h: 0.0,
            title: String::new(),
            stamp: String::new(),
            body: None,
            pictures: Vec::new(),
            files: Vec::new(),
            target: String::new(),
            duration: String::new(),
            open: false,
            busy: false,
            started: None,
            agent: None,
            tone: Tone::Normal,
        }
    }

    fn put(&mut self, mut row: Row) {
        let content = row.body.as_ref().map_or(0.0, Body::height) + PIC * row.pictures.len() as f32;
        let (h, gap) = match row.style {
            Style::Prompt => {
                let used = row.body.as_ref().map_or(0.0, Body::used_width).max(if row.pictures.is_empty() {
                    0.0
                } else {
                    240.0
                });
                row.geo = Geometry::new(row.style, self.width, (used + 2.0 * INSET + 1.0).max(34.0));
                (row.geo.ty + content + 8.0, 15.0)
            }
            Style::Answer => (content.max(19.0), 7.0),
            Style::Tool if row.open => {
                (row.geo.ty + content + 6.0 + if row.agent.is_some() { CHIP_H } else { 0.0 }, 4.0)
            }
            Style::Tool | Style::Group => (TOOL_H, 4.0),
            Style::Files => (chip_rect(row.geo, row.files.len() - 1).bottom() + 3.0, 12.0),
            Style::Card => (HEAD + content + 12.0, 7.0),
        };
        row.h = h;
        self.end += h + gap;
        self.list.push(row);
    }

    fn card(&mut self, id: Id, title: &str, stamp: &str, text: &str, tone: Tone, files: Option<&Files>) -> Row {
        let mut row = self.row(id, Style::Card);
        row.title = title.into();
        row.stamp = stamp.into();
        row.tone = tone;
        row.body = (!text.is_empty()).then(|| self.shape(&row.id, text, row.geo.tw, BodyKind::Plain, files));
        row
    }

    pub fn empty(&mut self, title: &str, hint: &str) {
        let row = self.card(Id::Empty, title, "", hint, Tone::Muted, None);
        self.put(row);
    }

    pub fn conversation(&mut self, m: &Model, expanded: &BTreeSet<ItemId>, tool_expanded: &BTreeSet<ItemId>) {
        let mut writes = TurnFiles::default();
        for group in &m.groups {
            let ids = match group {
                Group::One(id) => std::slice::from_ref(id),
                Group::Tools(ids) => ids.as_slice(),
            };
            if let [first, ..] = ids
                && collapsible(ids.len())
            {
                let mut row = self.row(Id::Group(*first), Style::Group);
                row.open = expanded.contains(first);
                row.title = format!("{} tool calls", ids.len());
                let mut failed = 0;
                let mut running = 0;
                for id in ids {
                    if let Some(Item { kind: ItemKind::Tool { state, .. }, .. }) = m.items.get(id) {
                        failed += usize::from(*state == ToolState::Failed);
                        running += usize::from(*state == ToolState::Running);
                    }
                }
                row.stamp = group_status(failed, running);
                row.tone = if failed > 0 { Tone::Warning } else { Tone::Muted };
                row.busy = running > 0;
                self.put(row);
            }
            for &id in ids {
                let Some(item) = m.items.get(&id) else { continue };
                let files = m.files.get(&id);
                if matches!(item.kind, ItemKind::Prompt { synthetic: false, .. }) {
                    self.put_files(&mut writes);
                }
                writes.observe(id, item, files);
                if !collapsible(ids.len()) || expanded.contains(&ids[0]) {
                    self.item(id, item, tool_expanded.contains(&id), files);
                }
            }
        }
        self.put_files(&mut writes);
    }

    fn put_files(&mut self, turn: &mut TurnFiles) {
        if let Some((id, files)) = turn.take() {
            let mut row = self.row(Id::Files(id), Style::Files);
            row.files = files;
            self.put(row);
        }
    }

    fn item(&mut self, id: ItemId, item: &Item, open: bool, files: Option<&Files>) {
        let row = match &item.kind {
            ItemKind::Prompt { text, pictures, .. } => {
                let mut row = self.row(Id::Message(id), Style::Prompt);
                row.body = (!text.is_empty()).then(|| self.shape(&row.id, text, row.geo.tw, BodyKind::Plain, files));
                row.pictures.extend((0..pictures.len()).map(|i| (id, i)));
                row
            }
            ItemKind::Assistant { markdown } => {
                let mut row = self.row(Id::Message(id), Style::Answer);
                row.body = Some(self.shape(&row.id, markdown, row.geo.tw, BodyKind::Markdown, files));
                row
            }
            ItemKind::Notice { kind, text } => {
                let (title, tone) = match kind {
                    NoticeKind::Compaction => ("Context compacted", Tone::Muted),
                    NoticeKind::ApiError => ("API error", Tone::Warning),
                };
                let stamp = item.timestamp.as_deref().and_then(|s| s.get(11..19)).unwrap_or("");
                self.card(Id::Message(id), title, stamp, text, tone, files)
            }
            ItemKind::Tool { name, summary, file_path, state, pictures, subagent, card, duration_ms, .. } => {
                let mut row = self.row(Id::Tool(id), Style::Tool);
                row.title = one_line(name, 160);
                let target = file_path.as_deref().unwrap_or(summary);
                row.target = one_line(target, 320);
                if row.target == row.title {
                    row.target.clear();
                }
                let (label, tone) = match state {
                    ToolState::Running => ("◌ running", Tone::Accent),
                    ToolState::Ok => ("✓ done", Tone::Muted),
                    ToolState::Failed => ("✗ failed", Tone::Warning),
                };
                row.stamp = label.into();
                row.tone = tone;
                row.busy = *state == ToolState::Running;
                row.duration = duration_ms.map(duration).unwrap_or_default();
                row.open = open;
                if open {
                    let (detail, kind) = tool_detail(card.as_ref(), target);
                    row.body = (!detail.is_empty()).then(|| self.shape(&row.id, &detail, row.geo.tw, kind, files));
                    row.agent.clone_from(subagent);
                    row.pictures.extend((0..pictures.len()).map(|i| (id, i)));
                }
                row
            }
        };
        self.put(row);
    }

    pub fn agents(&mut self, agents: &[Subagent]) {
        for a in agents {
            let (state, tone) = match a.finished {
                Some(true) => ("Finished", Tone::Muted),
                Some(false) => ("Running", Tone::Accent),
                None => ("Unknown", Tone::Normal),
            };
            let start = a.started_at.as_deref().and_then(seconds);
            let end = if a.finished != Some(false) { a.last_activity_at.as_deref().and_then(seconds) } else { None };
            let mut info: Vec<String> =
                [a.agent_type.as_deref(), a.model.as_deref()].into_iter().flatten().map(str::to_owned).collect();
            if let Some(depth) = a.spawn_depth {
                info.push(format!("depth {depth}"));
            }
            info.push(format!("{} tool call{}", a.tool_calls, if a.tool_calls == 1 { "" } else { "s" }));
            if let (Some(start), Some(end)) = (start, end)
                && end >= start
            {
                info.push(mdshared_app::fmt::duration((end - start) as u64));
            }
            let mut detail = info.join(" · ");
            if let Some(tool) = &a.last_tool_summary {
                detail.push('\n');
                detail.push_str(tool);
            }
            let mut row = self.card(
                Id::Agent(a.id.clone()),
                a.description.as_deref().unwrap_or(&a.id),
                state,
                &detail,
                tone,
                None,
            );
            row.agent = Some(a.id.clone());
            row.busy = a.finished == Some(false);
            row.started = if row.busy { start } else { None };
            self.put(row);
        }
        if agents.is_empty() {
            self.empty("No agents yet", "Delegated work will appear here.");
        }
    }

    pub fn tasks(&mut self, tasks: &[Task]) {
        for (i, task) in tasks.iter().enumerate() {
            let (title, tone) = match task.status {
                TaskStatus::Pending => ("○  Pending", Tone::Muted),
                TaskStatus::InProgress => ("◉  In progress", Tone::Accent),
                TaskStatus::Completed => ("✓  Completed", Tone::Normal),
            };
            let row = self.card(Id::Task(i), title, "", &task.content, tone, None);
            self.put(row);
        }
        if tasks.is_empty() {
            self.empty("No tasks yet", "The session’s current plan will appear here.");
        }
    }
}

const fn collapsible(count: usize) -> bool {
    count >= 4
}

fn group_status(failed: usize, running: usize) -> String {
    match (failed, running) {
        (0, 0) => String::new(),
        (0, r) => format!("{r} running"),
        (f, 0) => format!("{f} failed"),
        (f, r) => format!("{f} failed · {r} running"),
    }
}

fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{}.{:01}s", ms / 1000, ms / 100 % 10)
    } else {
        mdshared_app::fmt::duration(ms / 1000)
    }
}

fn one_line(s: &str, cap: usize) -> String {
    let mut chars = s.chars();
    let mut out: String =
        chars.by_ref().take(cap).map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c }).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn tool_detail(card: Option<&ToolCard>, summary: &str) -> (String, BodyKind) {
    match card {
        Some(ToolCard::Edit { path, added, removed, lines }) => (
            format!(
                "{path}\n+{added} −{removed}\n{}",
                lines.iter().take(20).map(String::as_str).collect::<Vec<_>>().join("\n")
            ),
            BodyKind::Mono,
        ),
        Some(ToolCard::Shell { command, result }) => (
            format!("{command}\n\n{}", if result.is_empty() { "No text result.".into() } else { result.join("\n") }),
            BodyKind::Shell,
        ),
        None => (summary.into(), BodyKind::Mono),
    }
}

#[derive(Default)]
struct TurnFiles {
    first: Option<ItemId>,
    files: Vec<NamedFile>,
    seen: BTreeSet<String>,
}

impl TurnFiles {
    fn observe(&mut self, id: ItemId, item: &Item, files: Option<&Files>) {
        if let ItemKind::Tool { name, state: ToolState::Ok, file_path: Some(path), .. } = &item.kind
            && matches!(name.as_str(), "Edit" | "Write")
            && let Some(file) = files.and_then(|files| files.get(path)).filter(|f| f.markdown())
            && self.seen.insert(file.path().to_string_lossy().to_ascii_lowercase())
        {
            self.first.get_or_insert(id);
            self.files.push(file.clone());
        }
    }

    fn take(&mut self) -> Option<(ItemId, Vec<NamedFile>)> {
        self.seen.clear();
        Some((self.first.take()?, std::mem::take(&mut self.files)))
    }
}

fn seconds(s: &str) -> Option<i64> {
    let n = |a, b| s.get(a..b)?.parse::<i64>().ok();
    let (mut year, month, day) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    year -= i64::from(month <= 2);
    let y = year.rem_euclid(400);
    let m = month + if month > 2 { -3 } else { 9 };
    let days = year.div_euclid(400) * 146097 + 365 * y + y / 4 - y / 100 + (153 * m + 2) / 5 + day - 1 - 719468;
    let offset = if let Some(i) = s.get(19..)?.find(['+', '-']) {
        let i = i + 19;
        let sign = if s.as_bytes()[i] == b'+' { 1 } else { -1 };
        sign * (n(i + 1, i + 3)? * 3600 + n(i + 4, i + 6)? * 60)
    } else {
        0
    };
    Some(86400 * days + n(11, 13)? * 3600 + n(14, 16)? * 60 + n(17, 19)? - offset)
}

#[cfg(test)]
mod tests;
