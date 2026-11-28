//! The read-only session pane. Worker updates change its content; they never give it keyboard focus.
pub(in crate::view) mod grid;
mod input;
mod markdown;
mod paint;
mod rows;
mod scroll;
mod select;
mod viewer;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use claude_switch_core::transcript::ItemId;
use mdshared_text::image::Image;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::theme::Palette;

use super::links::NamedFile;
use super::model::{Follow, Group, Model};
use super::outputs::{Destination, Output};
use super::working::{Footer, Turn};
pub use markdown::{Panel, Reading};
use rows::{HEAD, Id, Row, Rows, Style, Tone};
use scroll::{Drag, GUTTER, Scrollbar, Surface, Visibility};
use viewer::Overlay;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Conversation,
    Agents,
    Tasks,
    Details(super::chat::Side),
}

#[derive(Clone, PartialEq, Eq)]
enum Action {
    Page(Page),
    Attach,
    Earlier,
    Latest,
    Stop(Turn),
    Shell(ItemId),
    ShellList,
    Output(Output),
    Agent(String),
    Back,
    Group(ItemId),
    Tool(ItemId),
    Picture(ItemId, usize),
    File(NamedFile),
    Editor,
    Reading,
    CloseMarkdown,
    Open,
    Close,
    /// The picture viewer's side buttons: the conversation's picture before or after this one.
    Previous,
    Next,
    /// A code block's Copy button, with the code.
    CopyCode(String),
    /// Under a message: Copy (its text as shown), Rewind to here, Branch.
    CopyMessage(ItemId),
    Rewind(ItemId),
    Branch(ItemId),
}

#[derive(Default)]
pub struct Ui {
    page: Page,
    area: Rect,
    body: Rect,
    rows: Vec<Row>,
    expanded: BTreeSet<ItemId>,
    tool_expanded: BTreeSet<ItemId>,
    tool_jump: Option<(ItemId, bool)>,
    chat: bool,
    cache: Option<(u64, f32, f32, Palette)>,
    generation: Option<u64>,
    offset: f32,
    max: f32,
    follow: Follow,
    hits: Vec<(Rect, Action)>,
    hot: Option<Action>,
    pressed: Option<Action>,
    overlay: Option<Overlay>,
    markdown: Option<markdown::Markdown>,
    scrollbar: Option<Scrollbar>,
    scroll_drag: Option<Drag>,
    scroll_hot: Option<Surface>,
    conversation_visibility: Visibility,
    markdown_visibility: Visibility,
    chat_reading: Reading,
    selection: Option<select::Selection>,
    /// The message a right-click menu was opened on.
    menu_row: Option<Id>,
    /// The message with buttons the pointer is over (or over its buttons): they show.
    pointed: Option<ItemId>,
    /// The foot of the conversation kept for the round "jump to latest" button while it shows:
    /// no text is drawn (or taken by the pointer) under it.
    covered: f32,
    dragging: bool,
    /// What a double click (a word) or a triple click (a whole row) took: a drag from it goes
    /// by whole words (`false`) or whole rows (`true`).
    snap: Option<(select::Selection, bool)>,
    start: Option<(f32, f32, String)>,
    focused: bool,
    agent_origin: Option<Page>,
    busy_visible: bool,
    mouse: (f32, f32),
    working: Option<Footer>,
    /// The code block copied last, so its button can say so.
    copied: Option<String>,
    /// The docked Markdown panel's width and reading width, the window's, given by the host.
    panel: markdown::Panel,
    /// The area the panel and the conversation share while the panel is docked: where the
    /// divider is dragged.
    panel_area: Option<Rect>,
    /// The panel's width or reading width changed here, for the host to keep.
    panel_changed: bool,
    /// The left edge and width of the conversation's text in the last frame (Chat's reading
    /// column), which the lines above and below the conversation follow.
    column: (f32, f32),
    /// The newest message there was while the view last followed the latest.
    seen: Option<ItemId>,
    /// Chat shows a card that says what Claude waits for: the working line does not repeat it.
    working_hidden: bool,
    /// The running shell call the CLI offers to move to the background, and whether that key is
    /// on its way: its line in the conversation carries the button.
    offer: Option<(ItemId, bool)>,
    /// Where the last frame drew that button.
    offer_rect: Option<Rect>,
    /// The pending call a card asks about: its line says it waits, not that it runs.
    awaiting: Option<ItemId>,
    /// The colours of added and removed lines, from the terminal's theme.
    diff: Option<super::look::Diff>,
    /// The terminal.s font and colours, which command output is drawn in.
    grid: grid::Look,
    /// Chat's Details panel is closed: the working line shows the task list's progress, and a
    /// click on it opens the panel.
    progress: bool,
    /// Where the last frame drew that progress.
    progress_rect: Option<Rect>,
    /// What Chat says in place of an empty conversation: a quiet line, or nothing (a new
    /// session's greeting is the host's).
    empty_line: Option<&'static str>,
    /// The Sessions details' read-only conversation: no heading, no page tabs, no shell button.
    embedded: bool,
    /// Chat's messages waiting in Claude Code's queue, drawn after the last message.
    queued: super::chat::QueuedShown,
    /// Where the last frame drew their Send now.
    queued_rect: Option<Rect>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Release {
    Miss,
    Handled,
    Stop(Turn),
    Shell(ItemId),
    ShellList,
    /// Rewind to the prompt, as the right-click menu's "Rewind to here…".
    Rewind(ItemId),
    /// A new session from a copy of the conversation cut at the message.
    Branch(ItemId),
}

const PAD: f32 = 12.0;
/// Chat's side padding: the composer's box and the cards reach `look::INSET` past the reading
/// column on each side and still keep the window's margin.
const CHAT_PAD: f32 = PAD + super::look::INSET;
/// Under this conversation column Chat keeps only the window's own margin at its sides, as the
/// Session view does: the column keeps the room.
const CHAT_PAD_NARROW: f32 = 560.0;

/// Chat's side padding in a conversation area `width` wide.
pub(super) fn chat_pad(width: f32) -> f32 {
    if width - 2.0 * CHAT_PAD < CHAT_PAD_NARROW { PAD } else { CHAT_PAD }
}
/// The round "jump to latest" button.
const JUMP: f32 = 34.0;
/// A queued message: its bubble, the word under it and the gap after; then the row of Send now.
const QUEUED_BUBBLE: f32 = 34.0;
const QUEUED_ROW: f32 = QUEUED_BUBBLE + 26.0;
const QUEUED_FOOT: f32 = 34.0;

impl Ui {
    fn pad(&self) -> f32 {
        if self.chat { chat_pad(self.area.w) } else { PAD }
    }

    /// Where the conversation's text goes in a Chat conversation area at `x`, `width` wide:
    /// `(left, width)`. The composer and the cards are laid out on it before the frame is drawn.
    pub(super) fn column_for(&self, x: f32, width: f32) -> (f32, f32) {
        // Chat's column whatever this pane last drew as: the layout is Chat's.
        let pad = chat_pad(width);
        self.reading_column(x + pad, (width - 2.0 * pad).max(1.0))
    }

    pub(super) fn detail_side(&self) -> Option<super::chat::Side> {
        if let Page::Details(side) = self.page { Some(side) } else { None }
    }

    pub(super) fn show_details(&mut self, side: super::chat::Side) {
        self.show(Page::Details(side));
    }
    pub fn set_chat(&mut self, chat: bool) {
        if self.chat != chat {
            self.chat = chat;
            self.cache = None;
            self.scroll_drag = None;
        }
    }

    #[cfg(test)]
    pub(super) fn offset(&self) -> f32 {
        self.offset
    }

    /// Draws the conversation alone, for the Sessions details: no heading, no Attach, no pages.
    pub fn set_embedded(&mut self) {
        self.embedded = true;
        self.cache = None;
    }

    pub fn set_chat_reading(&mut self, reading: Reading) {
        if self.chat_reading != reading {
            self.chat_reading = reading;
            self.cache = None;
        }
    }

    pub fn chat_conversation(&mut self, model: &mut Model) {
        self.set_chat(true);
        self.show(Page::Conversation);
        model.select_agent(None);
    }

    /// What the working line says, when Claude Code works or waits.
    pub(super) fn working(&self) -> Option<&Footer> {
        self.working.as_ref()
    }

    /// Chat hides the working line while a card says the same ("Waiting for your answer").
    pub(super) fn hide_working(&mut self, hidden: bool) {
        self.working_hidden = hidden;
    }

    /// The running shell call whose line offers "Run in background", and whether the key is on
    /// its way (the button is then dimmed).
    pub(super) fn set_offer(&mut self, offer: Option<(ItemId, bool)>) {
        if self.offer.map(|(id, _)| id) != offer.map(|(id, _)| id) {
            self.cache = None;
        }
        self.offer = offer;
    }

    /// The pending call a card asks about (it waits; it does not run yet).
    pub(super) fn set_awaiting(&mut self, item: Option<ItemId>) {
        if self.awaiting != item {
            self.awaiting = item;
            self.cache = None;
        }
    }

    /// The colours of added and removed lines, from the window's terminal theme.
    pub fn set_diff_colours(&mut self, diff: super::look::Diff) {
        self.diff = Some(diff);
    }

    /// The terminal theme.s colours, which command output is drawn in.
    pub fn set_terminal_colours(&mut self, scheme: mdshared_ui::termview::Scheme) {
        self.grid.scheme = Some(scheme);
    }

    /// What Chat shows for a conversation with nothing in it: one quiet line (a transcript still
    /// being read), or nothing at all. The Session view keeps its card.
    pub(super) fn set_empty_line(&mut self, line: Option<&'static str>) {
        self.empty_line = line;
    }

    /// Chat's Details panel is closed: the task list's progress ("2 of 3") stands at the right
    /// end of the working line.
    pub(super) fn show_progress(&mut self, shown: bool) {
        self.progress = shown;
    }

    /// Where the last frame drew the task list's progress, for the host to take its click.
    pub(super) fn progress_rect(&self) -> Option<Rect> {
        self.progress_rect
    }

    /// Where the last frame drew "Run in background", for the host to take its click.
    pub(super) fn offer_rect(&self) -> Option<Rect> {
        self.offer_rect.filter(|_| self.offer.is_some_and(|(_, busy)| !busy))
    }

    /// Chat's messages waiting in Claude Code's queue: each a quiet bubble after the last
    /// message, with Send now under them.
    pub(super) fn set_queued(&mut self, queued: super::chat::QueuedShown) {
        self.queued = queued;
    }

    /// Where the last frame drew Send now under the queued messages, for the host to take its
    /// click (none while its keys are on their way).
    pub(super) fn queued_rect(&self) -> Option<Rect> {
        self.queued_rect.filter(|_| !self.queued.sending)
    }

    /// The room the queued messages take after the last message.
    fn queued_room(&self, m: &Model) -> f32 {
        let shown = self.chat && self.page == Page::Conversation && m.agent.is_none();
        let n = self.queued.messages.len();
        if shown && n > 0 { n as f32 * QUEUED_ROW + QUEUED_FOOT } else { 0.0 }
    }

    /// The last frame drew "Run in background" on the command's line (dimmed or not): it is in
    /// view, so no card has to offer it.
    pub(super) fn offer_drawn(&self) -> bool {
        self.offer_rect.is_some()
    }

    /// The room Chat's working line takes after the last message.
    fn working_room(&self, m: &Model) -> f32 {
        let shown = self.chat
            && self.page == Page::Conversation
            && m.agent.is_none()
            && self.working.is_some()
            && !self.working_hidden;
        if shown { super::working::CHAT_LINE } else { 0.0 }
    }

    pub(super) fn set_working(&mut self, footer: Option<Footer>) -> bool {
        if self.working == footer {
            false
        } else {
            self.working = footer;
            true
        }
    }

    /// The left edge and width of the conversation's text between `x` and `x + width`: in Chat,
    /// the reading column beside the docked panel; elsewhere the whole width.
    fn text_column(&self, x: f32, width: f32) -> (f32, f32) {
        if self.chat { self.reading_column(x, width) } else { (x, width) }
    }

    /// Chat's reading column between `x` and `x + width`, beside the docked panel.
    fn reading_column(&self, x: f32, width: f32) -> (f32, f32) {
        let row = Rect::new(x, 0.0, width, 1.0);
        let content = if self.markdown.is_some() { self.panel.split(row).0 } else { row };
        if content.w <= GUTTER {
            return (x, width);
        }
        let column = self.chat_reading.place(Rect::new(content.x, 0.0, content.w - GUTTER, 1.0));
        (column.x, column.w)
    }

    /// Where the last frame put the conversation's text: `(left, width)`.
    pub(super) fn column(&self) -> (f32, f32) {
        self.column
    }

    pub(super) fn footer_height(&self, width: f32, main: bool) -> f32 {
        if self.chat {
            // Chat has no strip: the working line is the conversation's last row, Stop is the
            // composer's, and "jump to latest" floats over the conversation.
            return 0.0;
        }
        if self.page == Page::Conversation && main && self.working.is_some() && width - 2.0 * PAD < 440.0 {
            49.0
        } else {
            40.0
        }
    }

    #[allow(clippy::too_many_arguments)] // Frame inputs from the host; the pane does not hold the host context.
    pub fn draw(
        &mut self,
        p: &mut Painter,
        pal: &Palette,
        area: Rect,
        m: &mut Model,
        mouse: (f32, f32),
        now: Instant,
        motion: bool,
    ) {
        self.mouse = mouse;
        self.busy_visible = false;
        if self.generation.replace(m.generation).is_some_and(|g| g != m.generation) {
            self.expanded.clear();
            self.tool_expanded.clear();
            self.tool_jump = None;
            self.selection = None;
            self.overlay = None;
            self.markdown = None;
            self.scroll_drag = None;
            self.cache = None;
        }
        self.area = area;
        self.hits.clear();
        let clip = p.clip(area);
        p.fill(area, pal.page);
        let pad = self.pad();
        let x = area.x + pad;
        let width = (area.w - 2.0 * pad).max(1.0);
        if !self.chat && !self.embedded {
            p.text(Face::Semibold, 17.0, x, area.y + 22.0, "Session view", pal.text, (width - 116.0).max(1.0));
            self.button(
                p,
                pal,
                Rect::new(area.right() - PAD - 110.0, area.y + 7.0, 110.0, 28.0),
                "Attach picture…",
                Action::Attach,
                false,
            );
        }
        let tabs_y = area.y + 44.0;
        if !self.chat && !self.embedded {
            let mut bx = x;
            use super::chat::Side;
            let tabs = [
                (Page::Conversation, "Conversation", None),
                (Page::Tasks, "Tasks", (width >= 365.0).then_some(m.tasks.len())),
                (Page::Agents, "Agents", (width >= 365.0).then_some(m.agents.len())),
                (Page::Details(Side::Shells), "Shells", Some(m.shells.len() + m.mcp_tasks.len())),
                (Page::Details(Side::Files), "Files", None),
                (Page::Details(Side::Outputs), "Outputs", None),
            ];
            let needs = tabs.map(|(_, label, count)| {
                p.measure(Face::Semibold, 12.0, label) + if count.is_some() { 29.0 } else { 4.0 }
            });
            let need = needs.iter().sum::<f32>();
            for ((page, label, count), want) in tabs.into_iter().zip(needs) {
                let w = width * want / need;
                let label = (label, count.filter(|_| width >= need - 5.0));
                self.segment(p, pal, Rect::new(bx, tabs_y, (w - 2.0).max(1.0), 29.0), label, page);
                bx += w;
            }
        }
        if self.detail_side().is_some() {
            self.scrollbar = None;
            self.body = Rect::default();
            p.restore(clip);
            return;
        }
        let mut top = if self.chat {
            area.y + 7.0
        } else if self.embedded {
            area.y + 4.0
        } else {
            tabs_y + 38.0
        };
        // Lines above and below the conversation start where its text does.
        self.column = self.text_column(x, width);
        let (cx, cw) = self.column;
        if self.page == Page::Conversation
            && let Some(agent) = &m.agent
        {
            self.button(p, pal, Rect::new(x, top, 64.0, 26.0), "‹  Back", Action::Back, false);
            let title =
                m.agents.iter().find(|a| a.id == *agent).and_then(|a| a.description.as_deref()).unwrap_or(agent);
            p.text(Face::Semibold, 13.0, x + 72.0, top + 13.0, title, pal.text, (width - 72.0).max(1.0));
            top += 34.0;
        }
        if let Some(error) = &m.error {
            p.text(Face::Ui, 12.0, cx, top + 10.0, error, pal.warn, cw);
            top += 24.0;
        }
        // The track must retain its length when paging or dragging toggles follow.
        let shell_count = if self.page == Page::Conversation && !self.chat && !self.embedded && m.agent.is_none() {
            m.running_shells()
        } else {
            0
        };
        let shell_h = if shell_count > 0 { 26.0 } else { 0.0 };
        let footer = area.bottom() - self.footer_height(area.w, m.agent.is_none()) - shell_h;
        if self.page == Page::Conversation && m.has_earlier && !(self.markdown.is_some() && width < 640.0) {
            let h = if self.chat { 21.0 } else { 26.0 };
            self.button(p, pal, Rect::new(cx, top, 106.0, h), "Load earlier", Action::Earlier, false);
            top += h + 6.0;
        }
        let full = Rect::new(x, top, width, (footer - top).max(0.0));
        let (content, dock) = if self.markdown.is_some() {
            let (content, grip, dock) = self.panel.split(full);
            self.panel_area = grip.map(|_| full);
            if let Some(grip) = grip {
                self.draw_divider(p, pal, grip);
            }
            (content, Some(dock))
        } else {
            self.panel_area = None;
            (full, None)
        };
        let visible = content.w > GUTTER;
        self.scrollbar = None;
        self.body = Rect::default();
        if visible {
            let body = Rect::new(content.x, top, (content.w - GUTTER).max(0.0), content.h);
            self.body = if self.chat { self.chat_reading.place(body) } else { body };
            self.layout(m, pal, p.scale);
            self.covered = if self.jump_shown() { JUMP + 20.0 } else { 0.0 };
            self.offer_rect = None;
            self.progress_rect = None;
            self.queued_rect = None;
            if self.chat
                && self.rows.is_empty()
                && let Some(line) = self.empty_line
            {
                p.text(Face::Ui, 13.0, self.body.x, self.body.y + 14.0, line, pal.muted, self.body.w);
            }
            if self.embedded && self.rows.is_empty() {
                let line = if m.loaded { "No messages to show." } else { "Reading the conversation\u{2026}" };
                p.text(Face::Ui, 13.0, self.body.x, self.body.y + 14.0, line, pal.muted, self.body.w);
            }
            self.paint_rows(p, pal, m);
            self.paint_queued(p, pal, m);
            self.paint_working(p, pal, m);
            self.scrollbar =
                Scrollbar::new(Rect::new(self.body.right() + 1.0, top, GUTTER - 1.0, self.body.h), self.max);
            if let Some(bar) = self.scrollbar {
                let dragging = self.scroll_drag.is_some_and(|d| d.surface == Surface::Conversation);
                let hot = dragging || bar.track.contains(mouse.0, mouse.1);
                let alpha = self.conversation_visibility.observe(self.offset, hot, now, motion);
                bar.draw(p, pal, self.offset, mouse, dragging, alpha);
            } else {
                self.conversation_visibility = Visibility::default();
            }
        } else {
            self.conversation_visibility = Visibility::default();
        }
        if let Some(dock) = dock {
            self.draw_markdown(p, pal, dock, m);
            if let Some(pane) = &self.markdown
                && let Some(bar) = pane.scroll
            {
                let dragging = self.scroll_drag.is_some_and(|d| d.surface == Surface::Markdown);
                let hot = dragging || bar.track.contains(mouse.0, mouse.1);
                let alpha = self.markdown_visibility.observe(pane.offset(), hot, now, motion);
                bar.draw(p, pal, pane.offset(), mouse, dragging, alpha);
            } else {
                self.markdown_visibility = Visibility::default();
            }
        } else {
            self.markdown_visibility = Visibility::default();
        }
        if !self.chat {
            p.fill(Rect::new(x, footer, width, p.px()), pal.line);
        }
        if self.page == Page::Conversation && self.chat {
            self.jump_button(p, pal, m, visible);
        }
        if self.page == Page::Conversation && !self.chat {
            self.conversation_footer(
                p,
                pal,
                Rect::new(cx, footer, cw, area.bottom() - footer - shell_h),
                visible,
                m.agent.is_none(),
            );
            if shell_count > 0 {
                self.button(
                    p,
                    pal,
                    Rect::new(x, area.bottom() - 25.0, 136.0, 22.0),
                    &super::shells::running_label(shell_count),
                    Action::ShellList,
                    false,
                );
            }
        } else {
            let count = if self.page == Page::Agents { m.agents.len() } else { m.tasks.len() };
            p.text(
                Face::Ui,
                12.0,
                x,
                footer + 20.0,
                &format!("{count} {}", if self.page == Page::Agents { "agents" } else { "tasks" }),
                pal.muted,
                width,
            );
        }
        if let Some(overlay) = &mut self.overlay {
            overlay.refresh(m);
        }
        p.restore(clip);
    }

    fn layout(&mut self, m: &Model, pal: &Palette, scale: f32) {
        let key = (m.revision, self.body.w, scale, *pal);
        if self.cache.as_ref() != Some(&key) {
            let anchor =
                self.rows.iter().find(|r| r.top + r.h > self.offset).map(|r| (r.id.clone(), self.offset - r.top));
            let previous =
                if self.cache.is_some_and(|(_, w, s, colors)| w == self.body.w && s == scale && colors == *pal) {
                    self.rows.drain(..).filter_map(|r| Some((r.id, r.body?))).collect()
                } else {
                    Default::default()
                };
            let cells = self.grid.font.metrics(grid::FONT_SIZE, scale);
            let mut r = Rows {
                previous,
                list: Vec::new(),
                end: 0.0,
                width: self.body.w,
                pal,
                scale,
                chat: self.chat,
                offer: self.offer.map(|(id, _)| id),
                awaiting: self.awaiting,
                actions: rows::MessageActions::of(m, self.chat),
                cells,
            };
            match self.page {
                Page::Conversation => {
                    r.conversation(m, &self.expanded, &self.tool_expanded);
                    if r.list.is_empty() && !self.chat && !self.embedded {
                        r.empty(
                            "Waiting for messages",
                            "The conversation appears here when its transcript is available.",
                        );
                    }
                }
                Page::Agents => r.agents(&m.agents),
                Page::Tasks => r.tasks(&m.tasks),
                Page::Details(_) => unreachable!("shared Details draws outside transcript rows"),
            }
            self.rows = r.list;
            self.cache = Some(key);
            if let Some((id, delta)) = anchor
                && let Some(row) = self.rows.iter().find(|r| r.id == id)
            {
                self.offset = row.top + delta;
            }
        }
        self.max = (self.rows_end() + self.queued_room(m) + self.working_room(m) - self.body.h).max(0.0);
        if let Some((id, pending)) = &mut self.tool_jump
            && *pending
            && self.body.h > 0.0
            && let Some(row) = self.rows.iter().find(|r| r.id == Id::Tool(*id))
        {
            self.offset = (row.top - 12.0).max(0.0);
            *pending = false;
        }
        self.offset = if self.page == Page::Conversation && self.follow.active() {
            self.max
        } else {
            self.offset.clamp(0.0, self.max)
        };
    }

    /// Where the conversation's rows end, with a little air under the last.
    fn rows_end(&self) -> f32 {
        self.rows.last().map_or(0.0, |r| r.top + r.h + 4.0)
    }

    /// Chat's working line after the last message, where the next text will appear.
    fn paint_working(&mut self, p: &mut Painter, pal: &Palette, m: &Model) {
        if self.working_room(m) == 0.0 {
            return;
        }
        let Some(work) = &self.working else { return };
        let body = self.shown_body();
        let y = body.y + self.rows_end() + self.queued_room(m) - self.offset;
        let clip = p.clip(body);
        let cy = y + super::working::CHAT_LINE / 2.0 + 2.0;
        let mut room = body.w;
        if self.progress && !m.tasks.is_empty() && body.w >= 320.0 {
            use claude_switch_core::transcript::TaskStatus;
            let done = m.tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
            let label = format!("{done} of {}", m.tasks.len());
            let w = p.measure(Face::Ui, 12.0, &label) + 34.0;
            let r = Rect::new(body.right() - w, cy - 12.0, w, 24.0);
            let hot = r.contains(self.mouse.0, self.mouse.1);
            p.frame(r, 12.0, if hot { pal.hover } else { pal.page }, super::look::edge(pal));
            p.icon(mdshared_ui::glyph::TASK, 12.0, r.x + 14.0, cy, pal.muted);
            p.text(Face::Ui, 12.0, r.x + 24.0, cy, &label, if hot { pal.text } else { pal.muted }, w);
            self.progress_rect = intersection(r, body);
            room -= w + 12.0;
        }
        super::working::draw_chat_line(p, pal, body.x, cy, &work.label, room);
        p.restore(clip);
    }

    /// Chat's messages waiting in Claude Code's queue, after the last message and before the
    /// working line: each a quiet bubble at the right, as your messages stand, with "Queued —
    /// waiting for its turn" under it, then Send now (Claude Code's own key for its queue).
    fn paint_queued(&mut self, p: &mut Painter, pal: &Palette, m: &Model) {
        if self.queued_room(m) == 0.0 {
            return;
        }
        let body = self.shown_body();
        let clip = p.clip(body);
        let edge = super::look::edge(pal);
        let mut y = body.y + self.rows_end() - self.offset + 4.0;
        let max_w = (body.w * 0.78).floor();
        for (text, picture) in &self.queued.messages {
            let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let line = match (*picture, line.is_empty()) {
                (true, true) => "[Picture]".to_owned(),
                (true, false) => format!("[Picture] {line}"),
                _ => line,
            };
            let w = (p.measure(Face::Ui, 13.5, &line) + 32.0).clamp(80.0, max_w);
            let r = Rect::new(body.right() - w, y, w, QUEUED_BUBBLE);
            p.frame(r, (QUEUED_BUBBLE / 2.0).min(16.0), pal.page, edge);
            p.text(Face::Ui, 13.5, r.x + 16.0, r.cy(), &line, pal.muted, (r.w - 32.0).max(0.0));
            let note = "Queued — waiting for its turn";
            let nw = p.measure(Face::Ui, 11.5, note);
            p.icon(mdshared_ui::glyph::RECENT, 11.0, body.right() - nw - 14.0, r.bottom() + 11.0, pal.faint);
            p.text(Face::Ui, 11.5, body.right() - nw - 4.0, r.bottom() + 11.0, note, pal.faint, nw + 2.0);
            y += QUEUED_ROW;
        }
        // Claude Code hands every queued message in at once: one Send now for them all.
        let label = match (self.queued.sending, self.queued.messages.len()) {
            (true, _) => "Sending now…",
            (false, 1) => "Send now",
            _ => "Send all now",
        };
        let w = p.measure(Face::Semibold, 12.0, label) + 24.0;
        let r = Rect::new(body.right() - w, y - 4.0, w, 26.0);
        let hot = !self.queued.sending && r.contains(self.mouse.0, self.mouse.1);
        p.frame(r, 13.0, if hot { pal.hover } else { pal.page }, edge);
        let ink = if self.queued.sending { pal.muted } else { pal.text };
        p.text_center(Face::Semibold, 12.0, r, label, ink);
        self.queued_rect = intersection(r, body);
        p.restore(clip);
    }

    pub(super) fn jump_to_tool(&mut self, model: &Model, id: ItemId) -> bool {
        let Some(ids) = model.groups.iter().find_map(|group| match group {
            Group::Tools(ids) if ids.binary_search(&id).is_ok() => Some(ids),
            _ => None,
        }) else {
            return false;
        };
        // The run the call is a step of opens; a call that is a line of its own has none.
        let run = rows::parts(&model.items, ids).into_iter().find_map(|part| match part {
            rows::Part::Run(run) if run.len() > 1 && run.contains(&id) => run.first().copied(),
            _ => None,
        });
        if self.page != Page::Conversation {
            self.show(Page::Conversation);
        } else {
            self.close_markdown();
        }
        self.expanded.extend(run);
        // Command output starts open, and a mark in the set folds it: leave such a row as it is.
        let starts_open = model.items.get(&id).is_some_and(rows::starts_open);
        if starts_open {
            self.tool_expanded.remove(&id);
        } else {
            self.tool_expanded.insert(id);
        }
        self.tool_jump = Some((id, true));
        self.cache = None;
        self.selection = None;
        self.follow.pause();
        true
    }

    /// Chat's "jump to latest": a round button over the foot of the conversation, only while the
    /// view does not follow; a dot on it once newer messages arrived.
    /// The round "jump to latest" button shows: Chat's conversation, scrolled back, tall enough.
    fn jump_shown(&self) -> bool {
        self.page == Page::Conversation && self.chat && !self.follow.active() && self.body.h >= JUMP + 24.0
    }

    /// The conversation's area where rows are drawn: the body without the foot the jump button
    /// keeps while it shows.
    pub(super) fn shown_body(&self) -> Rect {
        let body = self.body;
        Rect::new(body.x, body.y, body.w, (body.h - self.covered).max(0.0))
    }

    fn jump_button(&mut self, p: &mut Painter, pal: &Palette, m: &Model, visible: bool) {
        let newest = m.items.keys().next_back().copied();
        if self.follow.active() || !visible {
            self.seen = newest;
            return;
        }
        let body = self.body;
        if !self.jump_shown() {
            return;
        }
        let r = Rect::new((body.cx() - JUMP / 2.0).round(), body.bottom() - JUMP - 10.0, JUMP, JUMP);
        let hot = r.contains(self.mouse.0, self.mouse.1);
        let pressed = hot && self.pressed == Some(Action::Latest);
        let fill = if pressed {
            pal.press
        } else if hot {
            pal.hover
        } else {
            super::look::raised(pal)
        };
        p.shadow_with(r, JUMP / 2.0, pal.dark, 0.9);
        p.frame(r, JUMP / 2.0, fill, super::look::edge(pal));
        p.icon(mdshared_ui::glyph::DOWN, 15.0, r.cx(), r.cy() + 0.5, pal.text);
        // Before the view ever followed, nothing counts as new.
        if self.seen.is_some_and(|seen| newest > Some(seen)) {
            let dot = Rect::new(r.right() - 10.0, r.y + 1.0, 9.0, 9.0);
            p.rbox(dot, 4.5, pal.page);
            p.rbox(dot.inset(1.5), 3.0, pal.accent);
        }
        self.hits.push((r, Action::Latest));
    }

    fn button(&mut self, p: &mut Painter, pal: &Palette, r: Rect, label: &str, act: Action, selected: bool) {
        self.button_base(p, pal, r, act, selected);
        p.text_center(Face::Semibold, 12.0, r, label, if selected { pal.accent } else { pal.text });
    }

    fn segment(&mut self, p: &mut Painter, pal: &Palette, r: Rect, label: (&str, Option<usize>), page: Page) {
        let (name, count) = label;
        let selected = page == self.page;
        self.button_base(p, pal, r, Action::Page(page), selected);
        let color = if selected { pal.accent } else { pal.text };
        let Some(count) = count else {
            p.text_center(Face::Semibold, 12.0, r, name, color);
            return;
        };
        let count = if count > 99 { "99+".to_owned() } else { count.to_string() };
        let name_width = p.measure(Face::Semibold, 12.0, name);
        let badge_width = (p.measure(Face::Ui, 11.0, &count) + 10.0).max(17.0);
        let x = r.cx() - (name_width + 6.0 + badge_width) * 0.5;
        p.text(Face::Semibold, 12.0, x, r.cy(), name, color, name_width + 1.0);
        let badge = Rect::new(x + name_width + 6.0, r.cy() - 8.0, badge_width, 16.0);
        p.rbox(badge, 8.0, if selected { pal.pane } else { pal.on });
        p.text_center(Face::Ui, 11.0, badge, &count, color);
    }

    fn button_base(&mut self, p: &mut Painter, pal: &Palette, r: Rect, act: Action, selected: bool) {
        let mouse = self.mouse;
        let hot = r.contains(mouse.0, mouse.1);
        let bg = if self.pressed.as_ref() == Some(&act) && hot {
            pal.press
        } else if hot {
            pal.hover
        } else if selected {
            pal.on
        } else {
            pal.bar
        };
        p.rbox(r, 5.0, bg);
        if selected {
            p.rbox(Rect::new(r.x + r.w * 0.32, r.bottom() - 3.0, r.w * 0.36, 2.0), 1.0, pal.accent);
        }
        self.hits.push((r, act));
    }

    pub fn draw_overlay(
        &mut self,
        p: &mut Painter,
        pal: &Palette,
        window: Rect,
        m: &Model,
        mouse: (f32, f32),
        radius: f32,
    ) {
        let Some(overlay) = &mut self.overlay else { return };
        self.mouse = mouse;
        self.hits.clear();
        let bounds =
            Rect::new(window.x + window.w * 0.065, window.y + window.h * 0.07, window.w * 0.87, window.h * 0.86);
        overlay.bounds = bounds;
        p.frame(bounds, radius, pal.popup, pal.popup_line);
        let frame = Rect::new(bounds.x + 14.0, bounds.y + 13.0, (bounds.w - 28.0).max(1.0), (bounds.h - 66.0).max(1.0));
        if let Some(image) = &overlay.image {
            paint_image(p, image, frame, 4.0, true);
        } else {
            let hint = m.picture_failed(overlay.id, overlay.index, true).unwrap_or("Loading picture…");
            p.text_center(Face::Ui, 14.0, frame, m.error.as_deref().unwrap_or(hint), pal.muted);
        }
        let y = bounds.bottom() - 41.0;
        let open = Rect::new(bounds.x + 14.0, y, 164.0, 29.0);
        let close = Rect::new(bounds.right() - 86.0, y, 72.0, 29.0);
        self.hits.push((frame, Action::Close));
        self.button(p, pal, open, "Open in system viewer", Action::Open, false);
        self.button(p, pal, close, "Close", Action::Close, false);
        self.draw_steps(p, pal, frame, open, close);
    }

    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }
    pub fn animating(&self) -> bool {
        (self.busy_visible || self.scroll_deadline().is_some()) && !self.overlay_open()
    }

    pub fn scroll_deadline(&self) -> Option<Instant> {
        [self.conversation_visibility.deadline(), self.markdown_visibility.deadline()].into_iter().flatten().min()
    }
}

fn paint_image(p: &mut Painter, im: &Image, frame: Rect, radius: f32, enlarge: bool) {
    let scale = (frame.w / im.width as f32).min(frame.h / im.height as f32);
    let scale = if enlarge { scale } else { scale.min(1.0 / p.scale) };
    let (w, h) = (im.width as f32 * scale, im.height as f32 * scale);
    p.image(
        &im.rgba,
        im.width,
        im.height,
        Rect::new(frame.x + (frame.w - w) * 0.5, frame.y + (frame.h - h) * 0.5, w, h),
        radius,
        1.0,
    );
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (w, h) = (a.right().min(b.right()) - x, a.bottom().min(b.bottom()) - y);
    (w > 0.0 && h > 0.0).then(|| Rect::new(x, y, w, h))
}
