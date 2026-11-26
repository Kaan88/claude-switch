//! The read-only session pane. Worker updates change its content; they never give it keyboard focus.
mod input;
mod rows;

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use claude_switch_core::transcript::ItemId;
use mdshared_text::image::Image;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::theme::Palette;

use super::model::{Follow, Model};
use rows::{CompactHead, HEAD, Id, PIC, Row, Rows, Tone};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Conversation,
    Agents,
    Tasks,
}

#[derive(Clone, PartialEq, Eq)]
enum Action {
    Page(Page),
    Attach,
    Earlier,
    Latest,
    Agent(String),
    Back,
    Group(ItemId),
    Tool(ItemId),
    Picture(ItemId, usize),
    Open,
    Close,
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
    selection: Option<(Id, usize, usize)>,
    dragging: bool,
    start: Option<(f32, f32, String)>,
    focused: bool,
    agent_origin: Option<Page>,
    busy_visible: bool,
    epoch: Option<Instant>,
    mouse: (f32, f32),
}

const PAD: f32 = 12.0;
const INSET: f32 = 12.0;

impl Ui {
    pub fn set_chat(&mut self, chat: bool) {
        self.chat = chat;
        self.cache = None;
    }

    pub fn chat_conversation(&mut self, model: &mut Model) {
        self.set_chat(true);
        self.show(Page::Conversation);
        model.select_agent(None);
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
            self.selection = None;
            self.overlay = None;
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
            for (page, label, frac) in [
                (Page::Conversation, "Conversation", 0.45),
                (Page::Agents, "Agents", 0.285),
                (Page::Tasks, "Tasks", 0.265),
            ] {
                let w = width * frac;
                let count = match page {
                    Page::Conversation => None,
                    Page::Agents => Some(m.agents.len()),
                    Page::Tasks => Some(m.tasks.len()),
                };
                self.segment(p, pal, Rect::new(bx, tabs_y, (w - 3.0).max(1.0), 29.0), (label, count), page);
                bx += w;
            }
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
        let compact_footer = self.chat && area.h < 125.0 && self.follow.active();
        let footer = area.bottom()
            - if compact_footer {
                4.0
            } else if self.chat {
                25.0
            } else {
                40.0
            };
        if self.page == Page::Conversation && m.has_earlier {
            let h = if self.chat { 21.0 } else { 26.0 };
            self.button(p, pal, Rect::new(x, top, 106.0, h), "Load earlier", Action::Earlier, false);
            top += h + 6.0;
        }
        self.body = Rect::new(x, top, width, (footer - top).max(0.0));
        self.layout(m, pal, p.scale);
        let body_clip = p.clip(self.body);
        let compact = CompactHead::new(width);
        let start = self.rows.partition_point(|r| r.top + r.h < self.offset);
        for row in &self.rows[start..] {
            let y = self.body.y + row.top - self.offset;
            if y > self.body.bottom() {
                break;
            }
            let r = Rect::new(x, y, width, row.h);
            p.rbox(r, 7.0, pal.pane);
            let title = if matches!(row.id, Id::Agent(_)) { r } else { Rect::new(x, y, width, HEAD) };
            let act = match &row.id {
                Id::Group(id) => Some(Action::Group(*id)),
                Id::Tool(id) => Some(Action::Tool(*id)),
                _ => row.agent.clone().map(Action::Agent),
            };
            if let Some(act) = act {
                if title.contains(mouse.0, mouse.1) {
                    p.rbox(title, 6.0, pal.hover);
                }
                if let Some(hit) = intersection(title, self.body) {
                    self.hits.push((hit, act));
                }
            }
            let color = match row.tone {
                Tone::Accent => pal.accent,
                Tone::Normal => pal.text,
                Tone::Muted => pal.muted,
                Tone::Warning => pal.warn,
            };
            let stamp = row.status();
            let sw = if row.compact {
                compact.stamp_w
            } else if stamp.is_empty() {
                0.0
            } else {
                (p.measure(Face::Ui, 11.5, &stamp) + 10.0).min(width * 0.44)
            };
            let tx = x + INSET;
            let label = if row.agent.is_some() { format!("{}  ›", row.title) } else { row.title.clone() };
            if !row.compact {
                p.text(Face::Semibold, 13.0, tx, y + HEAD / 2.0, &label, color, (width - 2.0 * INSET - sw).max(1.0));
            }
            let stamp_x = x + if row.compact { compact.stamp_x } else { width - INSET - sw };
            p.text(Face::Ui, 11.5, stamp_x, y + HEAD / 2.0, &stamp, if row.compact { color } else { pal.muted }, sw);
            if row.busy && y + HEAD > self.body.y && !self.overlay_open() {
                self.busy_visible = true;
                let a = if motion { 0.70 + 0.30 * (phase * 4.0).sin() } else { 1.0 };
                if stamp_x - 7.0 >= x {
                    p.rbox_a(Rect::new(stamp_x - 7.0, y + HEAD / 2.0 - 2.0, 4.0, 4.0), 2.0, pal.accent, a);
                }
            }
            let mut py = y + HEAD;
            if let Some(body) = &row.body {
                let range = selection(&self.selection, &row.id, body.text());
                if row.compact {
                    if compact.text_w > 0.0 {
                        let bounds = Rect::new(x + compact.text_x, y, compact.text_w, HEAD);
                        let c = p.clip(bounds);
                        body.draw(p, Rect::new(bounds.x, y + 7.0, bounds.w, body.height()), bounds, range, pal);
                        p.restore(c);
                    }
                } else {
                    body.draw(
                        p,
                        Rect::new(tx, py, (width - 2.0 * INSET).max(1.0), body.height()),
                        self.body,
                        range,
                        pal,
                    );
                    py += body.height();
                }
            }
            {
                for &(id, i) in &row.pictures {
                    let frame = Rect::new(tx, py + 4.0, (width - 2.0 * INSET).clamp(1.0, 240.0), PIC - 8.0);
                    py += PIC;
                    let Some(hit) = intersection(frame, self.body) else { continue };
                    self.hits.push((hit, Action::Picture(id, i)));
                    p.rbox(frame, 5.0, pal.field);
                    if let Some(image) = m.picture(id, i, false) {
                        paint_image(p, &image, frame, 4.0, false);
                    } else {
                        let hint =
                            if m.picture_failed(id, i, false) { "Preview unavailable" } else { "Loading preview…" };
                        p.text_center(Face::Ui, 12.0, frame, hint, pal.muted);
                    }
                }
            }
        }
        p.restore(body_clip);
        if self.max > 0.0 && self.body.h > 0.0 {
            let h = (self.body.h.powi(2) / (self.max + self.body.h)).clamp(8.0_f32.min(self.body.h), self.body.h);
            p.rbox(
                Rect::new(area.right() - 5.0, top + (self.body.h - h) * self.offset / self.max, 3.0, h),
                1.5,
                pal.thumb,
            );
        }
        p.fill(Rect::new(x, footer, width, p.px()), pal.line);
        if self.page == Page::Conversation {
            let r = Rect::new(
                area.right() - PAD - 110.0,
                footer + if self.chat { 1.0 } else { 7.0 },
                110.0,
                if self.chat { 23.0 } else { 26.0 },
            );
            if compact_footer {
                // Preserve the bottom of the current card in a short Chat viewport.
            } else if self.follow.active() {
                p.text_center(Face::Ui, 12.0, r, "Following latest", pal.muted);
            } else {
                self.button(p, pal, r, "Jump to latest", Action::Latest, false);
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
            let mut r = Rows { previous, list: Vec::new(), end: 0.0, width: self.body.w, pal, scale };
            match self.page {
                Page::Conversation => {
                    r.conversation(m, &self.expanded, self.chat.then_some(&self.tool_expanded));
                    if r.list.is_empty() {
                        r.empty(
                            "Waiting for messages",
                            "The conversation appears here when its transcript is available.",
                        );
                    }
                }
                Page::Agents => r.agents(&m.agents),
                Page::Tasks => r.tasks(&m.tasks),
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
        self.offset = if self.page == Page::Conversation && self.follow.active() {
            self.max
        } else {
            self.offset.clamp(0.0, self.max)
        };
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

    pub fn draw_overlay(&mut self, p: &mut Painter, pal: &Palette, window: Rect, m: &Model, mouse: (f32, f32)) {
        let Some(overlay) = &mut self.overlay else { return };
        self.mouse = mouse;
        self.hits.clear();
        p.fill_a(window, pal.title, 0.75);
        let bounds =
            Rect::new(window.x + window.w * 0.065, window.y + window.h * 0.07, window.w * 0.87, window.h * 0.86);
        overlay.bounds = bounds;
        p.rbox(bounds, 10.0, pal.popup);
        let frame = Rect::new(bounds.x + 14.0, bounds.y + 13.0, (bounds.w - 28.0).max(1.0), (bounds.h - 66.0).max(1.0));
        if let Some(image) = &overlay.image {
            paint_image(p, image, frame, 4.0, true);
        } else {
            let hint = if m.picture_failed(overlay.id, overlay.index, true) {
                "This picture could not be decoded."
            } else {
                "Loading picture…"
            };
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

fn selection(sel: &Option<(Id, usize, usize)>, id: &Id, text: &str) -> Option<Range<usize>> {
    let (row, a, b) = sel.as_ref()?;
    let r = *a.min(b)..*a.max(b);
    (row == id && text.get(r.clone()).is_some()).then_some(r)
}
