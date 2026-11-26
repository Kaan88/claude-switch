//! Stable row identities let a prepended history page move the scrollbar without moving the paragraph under the reader.
use std::collections::{BTreeMap, BTreeSet};

use claude_switch_core::transcript::{Item, ItemId, ItemKind, NoticeKind, Subagent, Task, TaskStatus, ToolState};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::theme::Palette;

use super::super::model::{Group, Model};
use super::super::preview::Preview;
use super::super::text::TextLayout;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Id {
    Message(ItemId),
    Group(ItemId),
    Agent(String),
    Task(usize),
    Empty,
}

pub(super) struct Row {
    pub id: Id,
    pub top: f32,
    pub h: f32,
    pub title: String,
    pub stamp: String,
    pub body: Option<Body>,
    pub pictures: Vec<(ItemId, usize)>,
    pub compact: bool,
    pub busy: bool,
    pub started: Option<i64>,
    pub agent: Option<String>,
    pub tone: Tone,
}

/// The parser and shaper are much more expensive than building headers. Keep unchanged paragraphs
/// across incremental transcript updates, even when earlier history shifts their positions.
pub(super) struct Body {
    raw: String,
    kind: BodyKind,
    width: f32,
    layout: Layout,
    visible_prefix: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Plain,
    Markdown,
    Compact,
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

    pub fn point(&self, x: f32, y: f32) -> (usize, Option<&str>) {
        match &self.layout {
            Layout::Plain(t) => (t.at(x, y).min(self.visible_prefix.unwrap_or(self.raw.len())), t.link(x, y)),
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
            Layout::Plain(t) => {
                let end = self.visible_prefix.unwrap_or(self.raw.len());
                let shown = selection.map(|r| r.start.min(end)..r.end.min(end));
                t.draw(p, area.x, area.y, shown, pal);
            }
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

pub(super) const HEAD: f32 = 31.0;
pub(super) const PIC: f32 = 104.0;

/// Coordinates are relative to the row's left edge; drawing, shaping and pointer hits agree.
pub(super) struct CompactHead {
    pub text_x: f32,
    pub text_w: f32,
    pub stamp_x: f32,
    pub stamp_w: f32,
}

impl CompactHead {
    pub fn new(width: f32) -> Self {
        let inset = super::INSET.min(width * 0.125);
        let inner = (width - 2.0 * inset).max(0.0);
        let stamp_w = inner.min(58.0);
        let stamp_x = inset + inner - stamp_w;
        let text_w = (inner - stamp_w - 10.0).max(0.0); // Also leave room for the activity dot.
        Self { text_x: inset, text_w, stamp_x, stamp_w }
    }
}

pub(super) struct Rows<'a> {
    pub previous: BTreeMap<Id, Body>,
    pub list: Vec<Row>,
    pub end: f32,
    pub width: f32,
    pub scale: f32,
    pub pal: &'a Palette,
}

impl Rows<'_> {
    fn shape(&mut self, id: &Id, raw: &str, width: f32, kind: BodyKind) -> Body {
        if let Some(body) = self.previous.remove(id)
            && body.kind == kind
            && body.width == width
            && body.raw == raw
        {
            return body;
        }
        let (layout, visible_prefix) = match kind {
            BodyKind::Markdown => (Layout::Markdown(Box::new(Preview::new(raw, self.pal, self.scale, width))), None),
            BodyKind::Plain => (Layout::Plain(Box::new(TextLayout::new(raw, self.pal, self.scale, width))), None),
            BodyKind::Compact => {
                let (line, prefix) = TextLayout::compact(raw, self.pal, self.scale, width);
                (Layout::Plain(Box::new(line)), Some(prefix))
            }
        };
        Body { raw: raw.to_owned(), kind, width, layout, visible_prefix }
    }

    fn add(&mut self, id: Id, title: &str, stamp: &str, text: &str, rich: bool, tone: Tone) -> &mut Row {
        let kind = if rich { BodyKind::Markdown } else { BodyKind::Plain };
        let width = (self.width - 2.0 * super::INSET).max(1.0);
        let body = (!text.is_empty()).then(|| self.shape(&id, text, width, kind));
        let h = HEAD + body.as_ref().map_or(0.0, |t| t.height()) + 12.0;
        self.list.push(Row {
            id,
            top: self.end,
            h,
            title: title.into(),
            stamp: stamp.into(),
            body,
            tone,
            pictures: Vec::new(),
            compact: false,
            agent: None,
            busy: false,
            started: None,
        });
        self.end += h + 6.0;
        self.list.last_mut().unwrap()
    }

    fn pictures(&mut self, id: ItemId, count: usize) {
        if let Some(row) = self.list.last_mut() {
            let extra = PIC * count as f32;
            row.h += extra;
            self.end += extra;
            row.pictures.extend((0..count).map(|index| (id, index)));
        }
    }

    pub fn empty(&mut self, title: &str, hint: &str) {
        self.add(Id::Empty, title, "", hint, false, Tone::Muted);
    }

    pub fn conversation(&mut self, m: &Model, expanded: &BTreeSet<ItemId>) {
        for group in &m.groups {
            let ids = match group {
                Group::One(id) => std::slice::from_ref(id),
                Group::Tools(ids) => ids.as_slice(),
            };
            if let [first, _, ..] = ids {
                let opened = expanded.contains(first);
                let mut failed = 0;
                let mut busy = 0;
                for id in ids {
                    if let Some(Item { kind: ItemKind::Tool { state, .. }, .. }) = m.items.get(id) {
                        failed += usize::from(*state == ToolState::Failed);
                        busy += usize::from(*state == ToolState::Running);
                    }
                }
                let mark = if opened { "▾" } else { "▸" };
                let mut state = Vec::new();
                if failed > 0 {
                    state.push(format!("{failed} failed"));
                }
                if busy > 0 {
                    state.push(format!("{busy} running"));
                }
                self.add(
                    Id::Group(*first),
                    &format!("{mark}  {} tool calls", ids.len()),
                    &state.join(" · "),
                    "",
                    false,
                    if failed > 0 { Tone::Warning } else { Tone::Muted },
                )
                .busy = busy > 0;
                if !opened {
                    for id in ids {
                        if let Some(Item { kind: ItemKind::Tool { pictures, .. }, .. }) = m.items.get(id) {
                            self.pictures(*id, pictures.len());
                        }
                    }
                    continue;
                }
            }
            for &id in ids {
                if let Some(item) = m.items.get(&id) {
                    self.item(id, item, &m.agents);
                }
            }
        }
    }

    fn item(&mut self, id: ItemId, item: &Item, agents: &[Subagent]) {
        let stamp = item.timestamp.as_deref().and_then(|s| s.get(11..19)).unwrap_or("");
        match &item.kind {
            ItemKind::Prompt { text, pictures } => {
                self.add(Id::Message(id), "You", stamp, text, false, Tone::Accent);
                self.pictures(id, pictures.len());
            }
            ItemKind::Assistant { markdown } => {
                self.add(Id::Message(id), "Claude", stamp, markdown, true, Tone::Normal);
            }
            ItemKind::Notice { kind, text } => {
                let (title, tone) = match kind {
                    NoticeKind::Compaction => ("Context compacted", Tone::Muted),
                    NoticeKind::ApiError => ("API error", Tone::Warning),
                };
                self.add(Id::Message(id), title, stamp, text, false, tone);
            }
            ItemKind::Tool { name, summary, state, pictures, subagent, .. } => {
                let (status, tone) = if let Some(child) = subagent {
                    agent_status(agents.iter().find(|a| &a.id == child).and_then(|a| a.finished))
                } else {
                    match state {
                        ToolState::Running => ("Running", Tone::Accent),
                        ToolState::Ok => ("Done", Tone::Muted),
                        ToolState::Failed => ("Failed", Tone::Warning),
                    }
                };
                let label = if subagent.is_some() { format!("{name} ›") } else { name.to_owned() };
                let text = format!("{label}   {}", summary.replace(['\n', '\r', '\u{2028}', '\u{2029}'], " "));
                let width = CompactHead::new(self.width).text_w;
                let body = self.shape(&Id::Message(id), &text, width, BodyKind::Compact);
                let row = self.add(Id::Message(id), "", status, "", false, tone);
                row.compact = true;
                row.busy = status == "Running";
                row.body = Some(body);
                row.agent.clone_from(subagent);
                self.pictures(id, pictures.len());
            }
        }
    }

    pub fn agents(&mut self, agents: &[Subagent]) {
        for a in agents {
            let (state, tone) = agent_status(a.finished);
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
            let row = self.add(
                Id::Agent(a.id.clone()),
                a.description.as_deref().unwrap_or(&a.id),
                state,
                &detail,
                false,
                tone,
            );
            row.agent = Some(a.id.clone());
            row.busy = a.finished == Some(false);
            if row.busy {
                row.started = start;
            }
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
            self.add(Id::Task(i), title, "", &task.content, false, tone);
        }
        if tasks.is_empty() {
            self.empty("No tasks yet", "The session’s current plan will appear here.");
        }
    }
}

fn agent_status(finished: Option<bool>) -> (&'static str, Tone) {
    match finished {
        Some(true) => ("Finished", Tone::Muted),
        Some(false) => ("Running", Tone::Accent),
        None => ("Unknown", Tone::Normal),
    }
}

/// Transcript timestamps are ISO 8601; elapsed time spans midnight and may include an explicit offset.
fn seconds(s: &str) -> Option<i64> {
    let n = |a, b| s.get(a..b)?.parse::<i64>().ok();
    let (mut year, month, day) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let y = year.rem_euclid(400);
    let m = month + if month > 2 { -3 } else { 9 };
    let days = era * 146097 + 365 * y + y / 4 - y / 100 + (153 * m + 2) / 5 + day - 1 - 719468;
    let suffix = s.get(19..)?;
    let offset = if let Some(i) = suffix.find(['+', '-']) {
        let i = i + 19;
        let sign = if s.as_bytes()[i] == b'+' { 1 } else { -1 };
        sign * (n(i + 1, i + 3)? * 3600 + n(i + 4, i + 6)? * 60)
    } else {
        0
    };
    Some(86400 * days + n(11, 13)? * 3600 + n(14, 16)? * 60 + n(17, 19)? - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_stops_before_the_activity_dot_and_stamp_at_every_width() {
        for width in (0..=2400).map(|n| n as f32 / 2.0) {
            let r = CompactHead::new(width);
            assert!(r.text_x >= 0.0 && r.text_w >= 0.0 && r.stamp_w >= 0.0);
            assert!(r.stamp_x >= r.text_x && r.stamp_x + r.stamp_w <= width);
            if r.text_w > 0.0 {
                assert!(r.text_x + r.text_w <= r.stamp_x - 10.0, "width {width}");
            }
        }
        let r = CompactHead::new(300.0);
        assert_eq!((r.text_x, r.text_w, r.stamp_x, r.stamp_w), (12.0, 208.0, 230.0, 58.0));
    }
}
