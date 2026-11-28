//! Shared Chat/Session projection. Stable row identities preserve the reader's scroll anchor.
use std::collections::{BTreeMap, BTreeSet};

use claude_switch_core::transcript::{
    Item, ItemId, ItemKind, NoticeKind, Subagent, Task, TaskStatus, ToolCard, ToolState,
};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::theme::Palette;

use super::super::links::{Files, NamedFile};
use super::super::model::{Group, Model};
use super::super::outputs::{Destination, Output};
use super::super::preview::Preview;
use super::super::text::TextLayout;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Id {
    Message(ItemId),
    Tool(ItemId),
    Group(ItemId),
    Files(ItemId),
    Outputs(ItemId),
    Agent(String),
    Task(usize),
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Prompt,
    Answer,
    /// One call, or a notice: a line of its own, or a step of an open run.
    Tool,
    /// A run of calls: one line in words that opens into its steps.
    Group,
    /// A file edit: its path and counts, opening into the diff.
    Edit,
    Files,
    Card,
}

/// What a call was, for the words of a run and a step's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Command,
    Read,
    Search,
    Web,
    Agent,
    Tasks,
    Edit,
    Thought,
    Other,
}

impl Kind {
    pub fn icon(self) -> &'static str {
        use mdshared_ui::glyph;
        match self {
            Kind::Command => crate::view::look::TERMINAL,
            Kind::Read => glyph::DOC,
            Kind::Search => glyph::SEARCH,
            Kind::Web => glyph::GLOBE,
            Kind::Agent => crate::view::look::AGENT,
            Kind::Tasks => glyph::TASK,
            Kind::Edit => glyph::EDIT,
            Kind::Thought => glyph::INFO,
            Kind::Other => glyph::SLIDERS,
        }
    }
}

/// How a call reads: what it did, what it is doing, and what it waits to do.
struct Words {
    kind: Kind,
    done: String,
    doing: String,
    base: String,
    /// Its target is a command, a path or a pattern: set in the mono face on a step.
    mono: bool,
    /// The words say it all: no target after them.
    bare: bool,
}

fn words(name: &str) -> Words {
    let w = |kind, done: &str, doing: &str, base: &str, mono| Words {
        kind,
        done: done.into(),
        doing: doing.into(),
        base: base.into(),
        mono,
        bare: false,
    };
    match name {
        "Bash" | "PowerShell" => w(Kind::Command, "Ran", "Running", "run", true),
        "Read" | "NotebookRead" => w(Kind::Read, "Read", "Reading", "read", true),
        "Grep" => w(Kind::Search, "Searched for", "Searching for", "search for", true),
        "Glob" => w(Kind::Search, "Listed", "Listing", "list", true),
        "WebFetch" => w(Kind::Web, "Fetched", "Fetching", "fetch", false),
        "WebSearch" => w(Kind::Web, "Searched the web for", "Searching the web for", "search the web for", false),
        "Agent" | "Task" => w(Kind::Agent, "Agent", "Agent", "start agent", false),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" | "mcp__ccd_turn__update_plan" => Words {
            bare: true,
            ..w(Kind::Tasks, "Updated the task list", "Updating the task list", "update the task list", false)
        },
        "Edit" | "MultiEdit" | "NotebookEdit" => w(Kind::Edit, "Edited", "Editing", "edit", true),
        "Write" => w(Kind::Edit, "Wrote", "Writing", "write", true),
        "AskUserQuestion" => {
            Words { bare: true, ..w(Kind::Other, "Asked a question", "Asking a question", "ask a question", false) }
        }
        _ => {
            let name = one_line(name, 160);
            w(Kind::Other, &name, &name, &format!("use {name}"), false)
        }
    }
}

/// "Ran a command, read 3 files, searched the code": what a run of calls did, by kind, in the
/// order the kinds first came.
fn summary(kinds: &[Kind]) -> String {
    let only_tasks = kinds.iter().all(|k| *k == Kind::Tasks);
    let mut parts: Vec<String> = Vec::new();
    let mut seen: Vec<Kind> = Vec::new();
    for &kind in kinds {
        // The task list has the panel; beside other work it is not worth a word here. Nor is a
        // thought beside the calls it led to.
        let aside = matches!(kind, Kind::Tasks | Kind::Thought) && !only_tasks;
        if seen.contains(&kind) || aside && kinds.iter().any(|k| !matches!(k, Kind::Tasks | Kind::Thought)) {
            continue;
        }
        seen.push(kind);
        let n = kinds.iter().filter(|k| **k == kind).count();
        parts.push(match (kind, n) {
            (Kind::Command, 1) => "ran a command".into(),
            (Kind::Command, n) => format!("ran {n} commands"),
            (Kind::Read, 1) => "read a file".into(),
            (Kind::Read, n) => format!("read {n} files"),
            (Kind::Search, 1) => "searched the code".into(),
            (Kind::Search, n) => format!("searched the code {n} times"),
            (Kind::Web, 1) => "looked at a web page".into(),
            (Kind::Web, n) => format!("looked at {n} web pages"),
            (Kind::Agent, 1) => "ran an agent".into(),
            (Kind::Agent, n) => format!("ran {n} agents"),
            (Kind::Edit, 1) => "edited a file".into(),
            (Kind::Edit, n) => format!("edited {n} files"),
            (Kind::Tasks, _) => "updated the task list".into(),
            (Kind::Thought, _) => "thought".into(),
            (Kind::Other, 1) => "used a tool".into(),
            (Kind::Other, n) => format!("used {n} tools"),
        });
    }
    let mut text = parts.join(", ");
    if let Some(first) = text.chars().next() {
        text = first.to_uppercase().collect::<String>() + &text[first.len_utf8()..];
    }
    text
}

/// A part of a group of tool calls as the conversation shows it.
pub(super) enum Part<'a> {
    /// Calls (and thoughts) said in one line; a single call reads as itself.
    Run(&'a [ItemId]),
    /// A row of its own: a file edit, a plan, what a slash or shell command printed.
    Solo(&'a [ItemId]),
}

/// Splits a group of tool calls into runs and the rows that stand alone between them.
pub(super) fn parts<'a>(items: &BTreeMap<ItemId, Item>, ids: &'a [ItemId]) -> Vec<Part<'a>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, id) in ids.iter().enumerate() {
        // A call that is not loaded (it cannot be, in a group) would only be skipped when drawn.
        if items.get(id).is_some_and(solo) {
            if start < i {
                out.push(Part::Run(&ids[start..i]));
            }
            out.push(Part::Solo(&ids[i..=i]));
            start = i + 1;
        }
    }
    if start < ids.len() {
        out.push(Part::Run(&ids[start..]));
    }
    out
}

/// The call is a row of its own: an edit that went through (or waits), a plan, and what a slash
/// command or a shell-mode command printed, which keep their open output.
fn solo(item: &Item) -> bool {
    match &item.kind {
        ItemKind::Tool { card: Some(ToolCard::Edit { .. }), state, denial, .. } => {
            *state != ToolState::Failed && denial.is_none()
        }
        ItemKind::Tool { card: Some(ToolCard::Plan(_) | ToolCard::Output(_)), .. } => true,
        _ => false,
    }
}

/// The row starts open and a click folds it: longer output of a slash or shell-mode command.
pub(super) fn starts_open(item: &Item) -> bool {
    matches!(&item.kind, ItemKind::Tool { card: Some(ToolCard::Output(text)), summary, file_path, .. }
        if !text.trim().is_empty() && !(file_path.is_none() && summary.is_empty() && one_liner(text)))
}

/// One line of a diff: its number in the file (the old file's for a removed line), its sign and
/// its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiffLine {
    pub number: Option<usize>,
    pub sign: char,
}

/// A hunk's first line in the diff is marked so a gap can be drawn before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Diff {
    pub lines: Vec<DiffLine>,
    /// Where hunks after the first begin.
    pub breaks: Vec<usize>,
    /// Lines of the patch that are not shown.
    pub more: usize,
}

/// The most lines an open diff shows.
const DIFF_LINES: usize = 60;

/// An edit's diff: from the patch Claude Code recorded with the result when there is one (line
/// numbers, lines around the change), else the changed lines alone. Returns the lines' text too.
fn diff_of(card: &ToolCard) -> (Diff, Vec<String>) {
    let ToolCard::Edit { added, removed, lines, .. } = card else { return Default::default() };
    let patch = card.hunks();
    let mut diff = Diff::default();
    let mut texts = Vec::new();
    if patch.is_empty() {
        // The reader's preview: "− old" and "+ new", at most ten of each.
        for line in lines.iter().take(DIFF_LINES) {
            let mut chars = line.chars();
            let sign = if chars.next() == Some('+') { '+' } else { '-' };
            let text = chars.as_str();
            texts.push(text.strip_prefix(' ').unwrap_or(text).to_owned());
            diff.lines.push(DiffLine { number: None, sign });
        }
        diff.more = (added + removed).saturating_sub(diff.lines.len());
        return (diff, texts);
    }
    for hunk in patch {
        if !diff.lines.is_empty() {
            diff.breaks.push(diff.lines.len());
        }
        let (mut old, mut new) = (hunk.old_start, hunk.new_start);
        for line in &hunk.lines {
            let sign = line.chars().next().unwrap_or(' ');
            let number = if sign == '-' { old } else { new };
            old += usize::from(sign != '+');
            new += usize::from(sign != '-');
            if diff.lines.len() < DIFF_LINES {
                texts.push(line[sign.len_utf8()..].to_owned());
                diff.lines.push(DiffLine { number: Some(number), sign });
            } else {
                diff.more += 1;
            }
        }
        diff.more += hunk.more;
    }
    (diff, texts)
}

pub(super) const HEAD: f32 = 31.0;
/// A tool line in the Session view: low, so a run of calls reads as a quiet note.
pub(super) const TOOL_H: f32 = 22.0;
/// A tool line in Chat.
pub(super) const CHAT_TOOL_H: f32 = 26.0;
/// An edit's row: its path and counts.
pub(super) const EDIT_H: f32 = 34.0;
/// A step's text and an opened call's detail start this far in: past the timeline's icons.
pub(super) const STEP_INDENT: f32 = 26.0;
/// The padding of the box around an opened call's output.
pub(super) const BOX_PAD: (f32, f32) = (12.0, 8.0);
/// A diff's rows, as a multiple of its text's size.
const DIFF_SPACING: f32 = 1.6;
/// Where a diff's text starts in its row, after the line number and the sign.
pub(super) const DIFF_TEXT: f32 = 66.0;

/// The height of a tool line.
pub(super) fn tool_h(chat: bool) -> f32 {
    if chat { CHAT_TOOL_H } else { TOOL_H }
}
/// A thumbnail's slot: a picture keeps its aspect within it, about 240 px on the long side.
pub(super) const THUMB_W: f32 = 240.0;
pub(super) const THUMB_H: f32 = 180.0;
const THUMB_GAP: f32 = 8.0;
/// One line of thumbnails: the slot, its caption and the gap below.
pub(super) const PIC: f32 = THUMB_H + 30.0;
pub(super) const CHIP_H: f32 = 26.0;
const INSET: f32 = 12.0;
/// A prompt bubble's padding: its text sits this far inside it.
const BUBBLE_PAD: f32 = 14.0;
const BUBBLE_TOP: f32 = 10.0;
const AFTER_TOOLS: f32 = 8.0;
/// A little more air between an answer and the tool lines under it.
const UNDER_ANSWER: f32 = 3.0;
/// The row of buttons under a message in Chat (Copy, Rewind, Branch): its buttons' height, and
/// the room kept for it under the message, shown or not, so pointing moves nothing.
pub(super) const ACTION_H: f32 = 24.0;
pub(super) const ACTIONS_ROOM: f32 = ACTION_H + 8.0;

/// The buttons under a message in Chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Actions {
    /// "Rewind to here", as the right-click menu offers it: a prompt the CLI's rewind list shows.
    pub rewind: bool,
    /// The newest answer: its buttons always show; the others' while the pointer is over them.
    pub newest: bool,
}

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
        // One column: answers, tool lines and chips start at its left edge; only the user's
        // bubble is set apart, on the right.
        let (x, width, pad, ty) = match style {
            Style::Prompt => {
                let width = prompt_w.clamp(1.0, Self::prompt_max(w));
                (w - width, width, BUBBLE_PAD, BUBBLE_TOP)
            }
            Style::Answer => (0.0, w, 0.0, 0.0),
            // A tool row's text is placed when its row is made (`Rows::place_detail`).
            Style::Tool | Style::Group | Style::Edit => (0.0, w, 0.0, 0.0),
            Style::Files => (0.0, w, 0.0, 0.0),
            Style::Card => (0.0, w, INSET, HEAD),
        };
        let pad = pad.min(width * 0.20);
        Self { x, w: width, tx: x + pad, tw: (width - 2.0 * pad).max(1.0), ty }
    }

    fn prompt_max(w: f32) -> f32 {
        (w * 0.80).max(1.0)
    }
}

/// The height of a Files row's chips, nothing when it holds only pictures.
pub(super) fn chips_height(row: &Row) -> f32 {
    (row.files.len() + row.outputs.len()).checked_sub(1).map_or(0.0, |last| chip_rect(row.geo, last).bottom())
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

/// Thumbnails flow left to right in lines; `width` is the room for them.
pub(super) fn thumb_rect(width: f32, index: usize) -> Rect {
    let columns = ((width + THUMB_GAP) / (THUMB_W + THUMB_GAP)).floor().max(1.0) as usize;
    Rect::new(
        (index % columns) as f32 * (THUMB_W + THUMB_GAP),
        4.0 + (index / columns) as f32 * PIC,
        width.clamp(1.0, THUMB_W),
        THUMB_H,
    )
}

pub(super) fn thumbs_height(width: f32, count: usize) -> f32 {
    count.checked_sub(1).map_or(0.0, |last| thumb_rect(width, last).y - 4.0 + PIC)
}

/// The width a line of `count` thumbnails needs, at most `room`.
fn thumbs_width(room: f32, count: usize) -> f32 {
    let columns = ((room + THUMB_GAP) / (THUMB_W + THUMB_GAP)).floor().max(1.0);
    let n = (count as f32).min(columns);
    if count == 0 { 0.0 } else { (n * (THUMB_W + THUMB_GAP) - THUMB_GAP).min(room) }
}

/// A picture in the conversation, with its caption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Thumb {
    pub id: ItemId,
    pub index: usize,
    pub caption: String,
}

fn thumbs(id: ItemId, count: usize, caption: impl Fn(usize) -> String) -> impl Iterator<Item = Thumb> {
    (0..count).map(move |index| Thumb { id, index, caption: caption(index) })
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
    pub pictures: Vec<Thumb>,
    pub files: Vec<NamedFile>,
    pub outputs: Vec<Output>,
    pub background: bool,
    pub target: String,
    pub duration: String,
    pub open: bool,
    pub busy: bool,
    pub started: Option<i64>,
    pub agent: Option<String>,
    pub tone: Tone,
    /// A folded group that holds the call the CLI offers to run in the background.
    pub offer: bool,
    /// What the call was: a step's icon.
    pub kind: Kind,
    /// A step of an open run, on its timeline: whether it is the first and the last.
    pub step: Option<(bool, bool)>,
    /// The target is a command, a path or a pattern.
    pub mono: bool,
    /// The call runs (or waits to): its line is in the text colour.
    pub running: bool,
    /// The detail is command output or the like: drawn in a box.
    pub boxed: bool,
    /// The height of the row's first line.
    pub head: f32,
    /// An edit's lines added and removed, and its diff when open.
    pub counts: (usize, usize),
    pub diff: Diff,
    /// The buttons under a message (Chat only).
    pub actions: Option<Actions>,
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
    /// The user's message, in its bubble.
    Prompt,
    Markdown,
    Mono,
    Shell,
    /// A diff's lines: mono, in the text colour, never wrapped (its row clips them).
    Diff,
}

enum Layout {
    Plain(Box<TextLayout>),
    Markdown(Box<Preview>),
}

impl Body {
    /// The source: Markdown for an answer, the literal text otherwise.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn text(&self) -> &str {
        match &self.layout {
            Layout::Plain(_) => &self.raw,
            Layout::Markdown(t) => t.text(),
        }
    }

    /// An answer's code blocks, relative to the text's top.
    pub fn code_blocks(&self) -> Vec<crate::view::preview::CodeBlock<'_>> {
        match &self.layout {
            Layout::Plain(_) => Vec::new(),
            Layout::Markdown(t) => t.code_blocks(),
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
    /// The running shell call the CLI offers to move to the background.
    pub offer: Option<ItemId>,
    /// The pending call a card asks about: it has not started, it waits for the answer.
    pub awaiting: Option<ItemId>,
    /// The answers with buttons under them (each turn's last), and the newest of them.
    pub answers: (BTreeSet<ItemId>, Option<ItemId>),
}

/// The answers that carry the buttons in Chat: the last answer before each prompt and the last of
/// all, which is the newest.
pub(super) fn answer_actions(items: &BTreeMap<ItemId, Item>) -> (BTreeSet<ItemId>, Option<ItemId>) {
    let mut set = BTreeSet::new();
    let mut last = None;
    for (&id, item) in items {
        match item.kind {
            ItemKind::Prompt { synthetic: false, .. } => set.extend(last.take()),
            ItemKind::Assistant { .. } => last = Some(id),
            _ => {}
        }
    }
    set.extend(last);
    (set, last)
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
        let size = match kind {
            BodyKind::Prompt if self.chat => 15.0,
            _ if self.chat => 16.0,
            _ => 14.0,
        };
        let empty = Files::new();
        // Tool details (diffs, command output) are read on demand: muted, like their row.
        let quiet = Palette { text: self.pal.muted, ..*self.pal };
        let layout = if kind == BodyKind::Markdown {
            // In the conversation a fenced block has a header: the language and Copy.
            Layout::Markdown(Box::new(Preview::with_code_headers(
                raw,
                self.pal,
                self.scale,
                width,
                size,
                Some(files.unwrap_or(&empty)),
            )))
        } else {
            let mono = matches!(kind, BodyKind::Mono | BodyKind::Shell | BodyKind::Diff);
            let literal = matches!(kind, BodyKind::Shell | BodyKind::Diff);
            if kind == BodyKind::Diff {
                // Never wrapped (its row cuts a long line), and spaced as rows.
                let layout = TextLayout::spaced(raw, self.pal, self.scale, 100_000.0, 12.0, DIFF_SPACING);
                let files = files.cloned().unwrap_or_default();
                return Body { raw: raw.to_owned(), kind, width, layout: Layout::Plain(Box::new(layout)), files };
            }
            Layout::Plain(Box::new(TextLayout::new(
                raw,
                if mono { &quiet } else { self.pal },
                self.scale,
                width,
                if mono { 12.0 } else { size },
                if literal { None } else { Some(files.unwrap_or(&empty)) },
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
            outputs: Vec::new(),
            background: false,
            target: String::new(),
            duration: String::new(),
            open: false,
            busy: false,
            started: None,
            agent: None,
            tone: Tone::Normal,
            offer: false,
            kind: Kind::Other,
            step: None,
            mono: false,
            running: false,
            boxed: false,
            head: 0.0,
            counts: (0, 0),
            diff: Diff::default(),
            actions: None,
        }
    }

    fn put(&mut self, mut row: Row) {
        // Tool lines sit close together; an answer before them and whatever follows them get a
        // little air.
        let tool = |style| matches!(style, Style::Tool | Style::Group | Style::Edit);
        match (self.list.last().map(|last| last.style), tool(row.style)) {
            (Some(Style::Answer), true) => self.end += UNDER_ANSWER,
            (Some(last), false) if tool(last) => self.end += AFTER_TOOLS,
            _ => {}
        }
        row.top = self.end;
        let pictures = row.pictures.len();
        if row.style == Style::Prompt {
            let room = Geometry::prompt_max(self.width) - 2.0 * BUBBLE_PAD;
            let used = row.body.as_ref().map_or(0.0, Body::used_width).max(thumbs_width(room, pictures));
            row.geo = Geometry::new(row.style, self.width, (used + 2.0 * BUBBLE_PAD + 1.0).max(40.0));
        }
        let content = row.body.as_ref().map_or(0.0, Body::height) + thumbs_height(row.geo.tw, pictures);
        let (h, gap) = match row.style {
            Style::Prompt => (row.geo.ty + content + BUBBLE_TOP, 18.0),
            Style::Answer => (content.max(19.0), 7.0),
            Style::Tool | Style::Edit if row.open && (row.body.is_some() || pictures > 0 || row.agent.is_some()) => {
                let pad = if row.boxed || row.style == Style::Edit { BOX_PAD.1 } else { 2.0 };
                let more = if row.diff.more > 0 { 20.0 } else { 0.0 };
                let chip = if row.agent.is_some() { CHIP_H } else { 0.0 };
                (row.geo.ty + content + pad + more + chip + 4.0, if row.step.is_some() { 0.0 } else { 4.0 })
            }
            // Steps of a run follow each other on its timeline without a gap.
            Style::Tool if row.step.is_some() => (row.head, 0.0),
            Style::Tool | Style::Group | Style::Edit => (row.head, 4.0),
            Style::Files => (chips_height(&row) + thumbs_height(row.geo.w, pictures) + 3.0, 12.0),
            Style::Card => (HEAD + content + 12.0, 7.0),
        };
        row.h = h;
        // The buttons under a message keep their room, so that pointing at it moves nothing.
        let gap = if row.actions.is_some() { f32::min(gap, 10.0) + ACTIONS_ROOM } else { gap };
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
        let backgrounds = m.shells.iter().map(|shell| shell.tool).collect::<BTreeSet<_>>();
        let mut cards = output_cards(&m.items, &m.outputs);
        for group in &m.groups {
            let ids = match group {
                Group::One(id) => std::slice::from_ref(id),
                Group::Tools(ids) => ids.as_slice(),
            };
            for &id in ids {
                let Some(item) = m.items.get(&id) else { continue };
                if matches!(item.kind, ItemKind::Prompt { synthetic: false, queued: false, .. }) {
                    self.put_files(&mut writes);
                }
                writes.observe(id, item, m.files.get(&id));
            }
            let line = Line { step: None, open: false, background: false };
            match group {
                Group::One(id) => {
                    if let Some(item) = m.items.get(id) {
                        let line = Line { open: tool_expanded.contains(id), ..line };
                        self.item(*id, item, line, m.files.get(id));
                    }
                }
                Group::Tools(ids) => {
                    for part in parts(&m.items, ids) {
                        let (run, alone) = match part {
                            Part::Solo(one) => (one, true),
                            Part::Run(run) => (run, run.len() == 1),
                        };
                        let shown = alone || self.run(m, run, expanded);
                        let last = run.len() - 1;
                        for (n, &id) in run.iter().enumerate().filter(|_| shown) {
                            let Some(item) = m.items.get(&id) else { continue };
                            let line = Line {
                                step: (!alone).then_some((n == 0, n == last)),
                                open: tool_expanded.contains(&id),
                                background: backgrounds.contains(&id),
                            };
                            self.item(id, item, line, m.files.get(&id));
                        }
                    }
                }
            }
            if let Some(first) = ids.first() {
                let outputs = ids.iter().filter_map(|id| cards.remove(id)).flatten().collect::<Vec<_>>();
                if !outputs.is_empty() {
                    let mut row = self.row(Id::Outputs(*first), Style::Files);
                    // A picture shows as itself, captioned; everything else as a labelled chip.
                    for output in outputs {
                        match output.to {
                            Destination::Picture(id, index) => {
                                row.pictures.push(Thumb { id, index, caption: output.title });
                            }
                            _ => row.outputs.push(output),
                        }
                    }
                    self.put(row);
                }
            }
        }
        self.put_files(&mut writes);
    }

    /// The line of a run of calls: what they did, in words; what is running, while one is; how
    /// many failed or were refused. Returns whether the run is open.
    fn run(&mut self, m: &Model, run: &[ItemId], expanded: &BTreeSet<ItemId>) -> bool {
        let first = run[0];
        let mut row = self.row(Id::Group(first), Style::Group);
        row.head = tool_h(self.chat);
        row.open = expanded.contains(&first);
        let (mut failed, mut denied) = (0, 0);
        let mut kinds = Vec::new();
        let mut active = None;
        for id in run {
            match m.items.get(id).map(|item| &item.kind) {
                Some(ItemKind::Tool { name, state, denial, .. }) => {
                    // A denied call did not fail: it never ran.
                    denied += usize::from(denial.is_some());
                    failed += usize::from(*state == ToolState::Failed && denial.is_none());
                    kinds.push(words(name).kind);
                    if *state == ToolState::Running && active.is_none() {
                        active = Some(*id);
                    }
                }
                Some(ItemKind::Notice { .. }) => kinds.push(Kind::Thought),
                _ => {}
            }
        }
        row.title = summary(&kinds);
        if let Some(id) = active
            && let Some(ItemKind::Tool { name, summary, file_path, .. }) = m.items.get(&id).map(|item| &item.kind)
        {
            let target = one_line(file_path.as_deref().unwrap_or(summary), 320);
            let (title, target) = self.active_words(id, name, &target);
            row.title = title;
            row.target = target;
            row.running = true;
        }
        row.stamp = group_status(denied, failed);
        row.tone = if failed + denied > 0 { Tone::Warning } else { Tone::Muted };
        // Folded, the run's line stands for the call the CLI offers to run in the background.
        row.offer = !row.open && self.offer.is_some_and(|offer| run.contains(&offer));
        let open = row.open;
        self.put(row);
        open
    }

    /// A running call's words: "Running cargo bench", or "Waiting to run cargo bench" while a
    /// card asks about it.
    fn active_words(&self, id: ItemId, name: &str, target: &str) -> (String, String) {
        let words = words(name);
        let target = if words.bare || target == name { String::new() } else { target.to_owned() };
        if self.awaiting == Some(id) && !words.bare {
            (format!("Waiting to {}", words.base), target)
        } else {
            (words.doing, target)
        }
    }

    fn put_files(&mut self, turn: &mut TurnFiles) {
        if let Some((id, files)) = turn.take() {
            let mut row = self.row(Id::Files(id), Style::Files);
            row.files = files;
            self.put(row);
        }
    }

    /// Where an opened row's detail goes: under its first line, past the timeline when it is a
    /// step, inside a box when it is output.
    fn place_detail(&self, row: &mut Row) {
        let indent = if row.step.is_some() { STEP_INDENT.min(self.width * 0.1) } else { 0.0 };
        let (px, py) = if row.boxed { BOX_PAD } else { (0.0, 2.0) };
        row.geo.tx = row.geo.x + indent + px;
        row.geo.tw = (row.geo.w - indent - 2.0 * px).max(1.0);
        row.geo.ty = row.head + 2.0 + py;
    }

    fn item(&mut self, id: ItemId, item: &Item, line: Line, files: Option<&Files>) {
        let open = line.open;
        let row = match &item.kind {
            ItemKind::Prompt { text, pictures, synthetic, .. } => {
                let mut row = self.row(Id::Message(id), Style::Prompt);
                let rewind = crate::view::chat::question::rewind::listed(text);
                row.actions = (self.chat && !synthetic).then_some(Actions { rewind, newest: false });
                row.body = (!text.is_empty()).then(|| self.shape(&row.id, text, row.geo.tw, BodyKind::Prompt, files));
                let count = pictures.len();
                row.pictures.extend(thumbs(id, count, move |i| {
                    if count > 1 { format!("Pasted picture {}", i + 1) } else { "Pasted picture".into() }
                }));
                row
            }
            ItemKind::Assistant { markdown } => {
                let mut row = self.row(Id::Message(id), Style::Answer);
                let newest = self.answers.1 == Some(id);
                row.actions = (self.chat && self.answers.0.contains(&id)).then_some(Actions { rewind: false, newest });
                row.body = Some(self.shape(&row.id, markdown, row.geo.tw, BodyKind::Markdown, files));
                row
            }
            ItemKind::Notice { kind: kind @ (NoticeKind::Compaction | NoticeKind::ApiError), text } => {
                let (title, tone) = if *kind == NoticeKind::Compaction {
                    ("Context compacted", Tone::Muted)
                } else {
                    ("API error", Tone::Warning)
                };
                let stamp = item.timestamp.as_deref().and_then(|s| s.get(11..19)).unwrap_or("");
                self.card(Id::Message(id), title, stamp, text, tone, files)
            }
            ItemKind::Notice { kind, text } => self.notice(id, *kind, text, line, files),
            ItemKind::Tool { card: Some(card @ ToolCard::Edit { path, added, removed, .. }), state, .. }
                if solo(item) =>
            {
                let mut row = self.row(Id::Tool(id), Style::Edit);
                row.head = EDIT_H;
                row.kind = Kind::Edit;
                row.target = one_line(path, 320);
                row.counts = (*added, *removed);
                row.running = *state == ToolState::Running;
                if row.running {
                    row.stamp = if self.awaiting == Some(id) { "waiting" } else { "running" }.into();
                }
                row.open = open;
                if open {
                    let (diff, texts) = diff_of(card);
                    // An empty line keeps its row.
                    let text = texts.iter().map(|t| if t.is_empty() { " " } else { t }).collect::<Vec<_>>().join("\n");
                    row.diff = diff;
                    // Without line numbers the gutter holds only the sign.
                    let numbered = row.diff.lines.iter().any(|line| line.number.is_some());
                    row.geo.tx = row.geo.x + if numbered { DIFF_TEXT } else { 30.0 };
                    row.geo.tw = (row.geo.w - (row.geo.tx - row.geo.x) - 8.0).max(1.0);
                    row.geo.ty = EDIT_H + 5.0;
                    row.body = (!text.is_empty()).then(|| self.shape(&row.id, &text, row.geo.tw, BodyKind::Diff, None));
                }
                row
            }
            ItemKind::Tool {
                name, summary, file_path, state, pictures, subagent, card, duration_ms, denial, ..
            } => {
                let mut row = self.row(Id::Tool(id), Style::Tool);
                row.head = tool_h(self.chat);
                row.step = line.step;
                let words = words(name);
                row.kind = words.kind;
                row.mono = words.mono;
                let target = file_path.as_deref().unwrap_or(summary);
                row.target = one_line(target, 320);
                // A command's one-line reply ("Set model to …", "Settings dialog dismissed") reads on
                // its row like a note; longer output is a mono card under it.
                let reply = match card {
                    Some(ToolCard::Output(text)) if target.is_empty() && one_liner(text) => Some(text),
                    _ => None,
                };
                if let Some(text) = reply {
                    row.target = one_line(text, 320);
                    row.mono = false;
                }
                if words.bare || row.target == *name {
                    row.target.clear();
                }
                row.title = words.done;
                let (label, tone) = match (state, denial) {
                    // Who refused it, instead of "failed": the call never ran.
                    (_, Some(denial)) => (crate::view::chat::question::denials::label(denial.kind), Tone::Warning),
                    (ToolState::Running, _) => ("", Tone::Normal),
                    (ToolState::Ok, _) => ("", Tone::Muted),
                    (ToolState::Failed, _) => ("failed", Tone::Warning),
                };
                row.stamp = label.trim_start_matches(['✗', ' ']).into();
                row.tone = tone;
                row.running = *state == ToolState::Running;
                if row.running {
                    let (title, target) = self.active_words(id, name, &row.target);
                    row.title = title;
                    row.target = target;
                }
                row.duration = duration_ms.map(duration).unwrap_or_default();
                // A plan is read before it is approved: always open. Command output starts open and a
                // click folds it; everything else starts folded.
                row.open = match card {
                    Some(ToolCard::Plan(_)) => true,
                    Some(ToolCard::Output(text)) if reply.is_none() && !text.trim().is_empty() => !open,
                    _ => open,
                };
                row.background = line.background;
                if row.open {
                    // The row itself shows a one-line command: its box holds what it printed.
                    let shown = row.target == one_line(target, 320) && !target.contains('\n');
                    let (mut detail, kind) = tool_detail(card.as_ref(), target, shown);
                    if let Some(denial) = denial {
                        // Why it did not run, under what was asked for.
                        let why = crate::view::chat::question::denials::why(denial);
                        detail = format!("{}\n\n{why}", detail.trim_end()).trim_start().to_owned();
                    }
                    row.boxed = matches!(kind, BodyKind::Mono | BodyKind::Shell);
                    self.place_detail(&mut row);
                    row.body = (!detail.is_empty()).then(|| self.shape(&row.id, &detail, row.geo.tw, kind, files));
                    row.agent.clone_from(subagent);
                    let count = pictures.len();
                    let title = one_line(name, 160);
                    row.pictures.extend(thumbs(id, count, move |i| {
                        if count > 1 { format!("{title} · picture {}", i + 1) } else { format!("{title} · picture") }
                    }));
                }
                row
            }
        };
        self.put(row);
    }

    /// A small notice: one line like a tool row (its kind and the text's first line) that
    /// expands to the whole text. A thought in a run is one of its steps.
    fn notice(&mut self, id: ItemId, kind: NoticeKind, text: &str, line: Line, files: Option<&Files>) -> Row {
        let (title, tone, body) = notice_look(kind);
        let mut row = self.row(Id::Tool(id), Style::Tool);
        row.head = tool_h(self.chat);
        row.step = line.step;
        row.kind = if kind == NoticeKind::Thought { Kind::Thought } else { Kind::Other };
        row.title = title.into();
        if kind != NoticeKind::Thought {
            row.target = one_line(text.lines().find(|l| !l.trim().is_empty()).unwrap_or(""), 320);
        }
        row.tone = tone;
        row.open = line.open;
        if line.open {
            self.place_detail(&mut row);
            row.body = Some(self.shape(&row.id, text, row.geo.tw, body, files));
        }
        row
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

fn output_cards(
    items: &BTreeMap<ItemId, Item>,
    outputs: &BTreeMap<ItemId, Vec<Output>>,
) -> BTreeMap<ItemId, Vec<Output>> {
    let mut next_answer = None;
    let mut current_turn = None;
    let mut layer = Vec::new();
    let mut preferred = BTreeSet::new();
    // Tool-only results sit under the next answer in the same turn, or their tool group
    // if the turn has not produced another answer. Prefer the assistant's link label.
    for (&id, item) in items.iter().rev() {
        if matches!(item.kind, ItemKind::Prompt { synthetic: false, queued: false, .. }) {
            next_answer = None;
            current_turn = Some(id);
        }
        let answer = matches!(item.kind, ItemKind::Assistant { .. });
        if answer {
            next_answer = Some(id);
        }
        for output in outputs.get(&id).into_iter().flatten().rev().filter(|o| !o.has_markdown_chip) {
            let key = (current_turn, output.key());
            if answer {
                preferred.insert(key.clone());
            }
            layer.push((next_answer.unwrap_or(id), answer, key, output));
        }
    }
    let mut by_row = BTreeMap::<ItemId, Vec<Output>>::new();
    let mut seen = BTreeSet::new();
    for (id, answer, key, output) in layer.into_iter().rev() {
        if (answer || !preferred.contains(&key)) && seen.insert(key) {
            by_row.entry(id).or_default().push(output.clone());
        }
    }
    by_row
}

fn notice_look(kind: NoticeKind) -> (&'static str, Tone, BodyKind) {
    match kind {
        NoticeKind::Thought => ("Thought", Tone::Muted, BodyKind::Markdown),
        NoticeKind::HookError => ("Hook error", Tone::Warning, BodyKind::Plain),
        NoticeKind::HookMessage => ("Hook", Tone::Muted, BodyKind::Plain),
        NoticeKind::PromptBlocked => ("Prompt blocked by a hook", Tone::Warning, BodyKind::Plain),
        NoticeKind::StopHook => ("Stop hook feedback", Tone::Accent, BodyKind::Plain),
        NoticeKind::Memory => ("Memory", Tone::Muted, BodyKind::Plain),
        NoticeKind::Recap => ("Recap", Tone::Muted, BodyKind::Markdown),
        NoticeKind::Info => ("Notice", Tone::Warning, BodyKind::Plain),
        NoticeKind::Compaction => ("Context compacted", Tone::Muted, BodyKind::Plain),
        NoticeKind::ApiError => ("API error", Tone::Warning, BodyKind::Plain),
    }
}

/// How a tool row is placed: a line of its own or a step of an open run, opened or not.
#[derive(Clone, Copy)]
struct Line {
    step: Option<(bool, bool)>,
    open: bool,
    background: bool,
}

fn group_status(denied: usize, failed: usize) -> String {
    let parts = [(denied, "denied"), (failed, "failed")];
    let parts: Vec<String> = parts.iter().filter(|(n, _)| *n > 0).map(|(n, what)| format!("{n} {what}")).collect();
    parts.join(" · ")
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

/// Short enough to read whole on a tool row.
fn one_liner(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !text.contains('\n') && text.chars().count() <= 140
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

/// An opened call's detail. `shown` says the row already shows the whole command.
fn tool_detail(card: Option<&ToolCard>, summary: &str, shown: bool) -> (String, BodyKind) {
    match card {
        Some(ToolCard::Edit { path, added, removed, lines, .. }) => (
            format!(
                "{path}\n+{added} −{removed}\n{}",
                lines.iter().take(20).map(String::as_str).collect::<Vec<_>>().join("\n")
            ),
            BodyKind::Mono,
        ),
        Some(ToolCard::Shell { command, result }) => {
            let result = if result.is_empty() { "No text result.".into() } else { result.join("\n") };
            (if shown { result } else { format!("{command}\n\n{result}") }, BodyKind::Shell)
        }
        Some(ToolCard::Plan(plan)) => (plan.clone(), BodyKind::Markdown),
        Some(ToolCard::Output(text)) => (text.clone(), BodyKind::Mono),
        Some(ToolCard::Questions(_)) | None => (summary.into(), BodyKind::Mono),
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
