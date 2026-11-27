//! The read-only session pane. Worker updates change its content; they never give it keyboard focus.
mod input;
mod markdown;
mod paint;
mod rows;
mod scroll;
mod select;

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
pub use markdown::Panel;
use rows::{HEAD, Id, Row, Rows, Style, Tone};
use scroll::{Drag, GUTTER, Scrollbar, Surface};

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
    /// A code block's Copy button, with the code.
    CopyCode(String),
}

struct Overlay {
    id: ItemId,
    index: usize,
    image: Option<Arc<Image>>,
    bounds: Rect,
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
    selection: Option<select::Selection>,
    /// The message a right-click menu was opened on.
    menu_row: Option<Id>,
    dragging: bool,
    start: Option<(f32, f32, String)>,
    focused: bool,
    agent_origin: Option<Page>,
    busy_visible: bool,
    epoch: Option<Instant>,
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
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Release {
    Miss,
    Handled,
    Stop(Turn),
    Shell(ItemId),
    ShellList,
}

const PAD: f32 = 12.0;
impl Ui {
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

    pub fn chat_conversation(&mut self, model: &mut Model) {
        self.set_chat(true);
        self.show(Page::Conversation);
        model.select_agent(None);
    }

    pub(super) fn set_working(&mut self, footer: Option<Footer>) -> bool {
        if self.working == footer {
            false
        } else {
            self.working = footer;
            true
        }
    }

    pub(super) fn footer_height(&self, width: f32, main: bool) -> f32 {
        if self.page == Page::Conversation && main && self.working.is_some() && width - 2.0 * PAD < 440.0 {
            49.0
        } else if self.chat {
            25.0
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
        let phase = now.duration_since(*self.epoch.get_or_insert(now)).as_secs_f32();
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
        let x = area.x + PAD;
        let width = (area.w - 2.0 * PAD).max(1.0);
        if !self.chat {
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
        if !self.chat {
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
        let mut top = if self.chat { area.y + 7.0 } else { tabs_y + 38.0 };
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
            p.text(Face::Ui, 12.0, x, top + 10.0, error, pal.warn, width);
            top += 24.0;
        }
        // The track must retain its length when paging or dragging toggles follow.
        let shell_count =
            if self.page == Page::Conversation && !self.chat && m.agent.is_none() { m.running_shells() } else { 0 };
        let shell_h = if shell_count > 0 { 26.0 } else { 0.0 };
        let footer = area.bottom() - self.footer_height(area.w, m.agent.is_none()) - shell_h;
        if self.page == Page::Conversation && m.has_earlier && !(self.markdown.is_some() && width < 640.0) {
            let h = if self.chat { 21.0 } else { 26.0 };
            self.button(p, pal, Rect::new(x, top, 106.0, h), "Load earlier", Action::Earlier, false);
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
            let rail = if self.chat { content.w.min(790.0) } else { content.w };
            let left = content.x + (content.w - rail) / 2.0;
            self.body = Rect::new(left, top, rail - GUTTER, content.h);
            self.layout(m, pal, p.scale);
            self.paint_rows(p, pal, m, phase, motion);
            self.scrollbar =
                Scrollbar::new(Rect::new(self.body.right() + 1.0, top, GUTTER - 1.0, self.body.h), self.max);
            if let Some(bar) = self.scrollbar {
                bar.draw(
                    p,
                    pal,
                    self.offset,
                    mouse,
                    self.scroll_drag.is_some_and(|d| d.surface == Surface::Conversation),
                );
            }
        }
        if let Some(dock) = dock {
            self.draw_markdown(p, pal, dock, m);
        }
        p.fill(Rect::new(x, footer, width, p.px()), pal.line);
        if self.page == Page::Conversation {
            self.conversation_footer(
                p,
                pal,
                Rect::new(x, footer, width, area.bottom() - footer - shell_h),
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
        if let Some(overlay) = &mut self.overlay
            && let Some(image) = m.picture(overlay.id, overlay.index, true)
        {
            overlay.image = Some(image);
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
            let mut r = Rows { previous, list: Vec::new(), end: 0.0, width: self.body.w, pal, scale, chat: self.chat };
            match self.page {
                Page::Conversation => {
                    r.conversation(m, &self.expanded, &self.tool_expanded);
                    if r.list.is_empty() {
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
        self.max = (self.rows.last().map_or(0.0, |r| r.top + r.h + 4.0) - self.body.h).max(0.0);
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

    pub(super) fn jump_to_tool(&mut self, model: &Model, id: ItemId) -> bool {
        let Some(first) = model.groups.iter().find_map(|group| match group {
            Group::Tools(ids) if ids.binary_search(&id).is_ok() => ids.first(),
            _ => None,
        }) else {
            return false;
        };
        if self.page != Page::Conversation {
            self.show(Page::Conversation);
        } else {
            self.close_markdown();
        }
        self.expanded.insert(*first);
        self.tool_expanded.insert(id);
        self.tool_jump = Some((id, true));
        self.cache = None;
        self.selection = None;
        self.follow.pause();
        true
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
    }

    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }
    pub fn animating(&self) -> bool {
        self.busy_visible && !self.overlay_open()
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
