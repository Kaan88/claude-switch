use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::theme::Palette;

use super::{Chat, Chip, Control, Drawer, Gate, Layout, Reason, Side};
use crate::state::{AppState, Msg, Target};

pub const STRIP_RESERVE: f32 = 143.0;

pub fn strip_button(
    p: &mut Painter,
    pal: &Palette,
    strip: Rect,
    chat: bool,
    hover: Option<Target>,
    hits: &mut Vec<(Rect, Target)>,
) {
    let x = strip.right() - STRIP_RESERVE;
    for (label, enabled, w, x) in [("Chat", true, 52.0, x), ("Terminal", false, 75.0, x + 53.0)] {
        let r = Rect::new(x, strip.y + 7.0, w, strip.h - 14.0);
        let target = Target::ChatMode(enabled);
        button(p, pal, r, label, hover == Some(target), enabled == chat);
        hits.push((r, target));
    }
}

impl AppState {
    pub fn layout_chat(&mut self, area: Rect) -> Rect {
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return Rect::default() };
        let chat = &mut pane.chat;
        chat.layout = Layout::new(
            area,
            !chat.side_hidden,
            chat.drawer != Drawer::Closed,
            chat.drawer_height,
            !chat.chips.is_empty(),
        );
        self.session_view.pane_area = chat.layout.conversation;
        chat.layout.drawer
    }

    pub fn draw_chat(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let Some(tab_id) = self.tabs.active else { return };
        let details_active = self.nothing_over() && !self.picture_overlay() && !self.usage.popup.open;
        let account = self.tabs.shown().and_then(|t| t.account.as_deref()).map(|slug| self.account_name(slug));
        let hover = self.hover;
        let focused = cx.focused() && self.chat_takes_input() && self.nothing_over() && !self.picture_overlay();
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return };
        let chat = &mut pane.chat;
        let l = chat.layout;
        let now = cx.now();
        let motion = cx.anim().animates();
        pane.ui.draw(p, pal, l.conversation, &mut pane.model, self.mouse, now, motion);
        if pane.ui.animating() && motion {
            cx.animating(l.conversation);
        }
        let mut hit = |p: &mut Painter, r, label: &str, control, selected| {
            let target = Target::Chat(control);
            button(p, pal, r, label, hover == Some(target), selected);
            self.hits.push((r, target));
        };
        let clip = p.clip(l.composer);
        let y = l.composer.bottom() - 48.0;
        let x = l.composer.x + 12.0;
        let right = l.composer.right() - 12.0;
        let e = l.editor;
        if !chat.chips.is_empty() {
            let base = chat.chip_page * 3;
            for (i, chip) in chat.chips.iter().enumerate().skip(base).take(3) {
                let label = match chip {
                    Chip::Clipboard(_) => "Clipboard picture",
                    Chip::Path(path) => path.file_name().and_then(|s| s.to_str()).unwrap_or("File"),
                };
                let chip_w = ((e.w - 68.0) / 3.0).clamp(54.0, 172.0);
                let r = Rect::new(x + (i - base) as f32 * chip_w, l.composer.y + 3.0, chip_w - 4.0, 23.0);
                hit(p, r, &format!("{}  ×", abbreviate(label, (chip_w / 7.0) as usize - 3)), Control::Remove(i), false);
            }
            if chat.chips.len() > 3 {
                let r = Rect::new(right - 62.0, l.composer.y + 3.0, 62.0, 23.0);
                hit(
                    p,
                    r,
                    &format!("{}/{} ›", chat.chip_page + 1, chat.chips.len().div_ceil(3)),
                    Control::MoreChips,
                    false,
                );
            }
        }
        chat.draw_editor(p, pal, focused);
        hit(p, Rect::new(x, y, 62.0, 25.0), "Attach…", Control::Attach, false);
        hit(p, Rect::new(x + 66.0, y, 60.0, 25.0), "/model", Control::SlashModel, false);
        hit(
            p,
            Rect::new(x + 130.0, y, 89.0, 25.0),
            if chat.drawer == Drawer::Closed { "Terminal ▴" } else { "Terminal ▾" },
            Control::Drawer,
            chat.drawer != Drawer::Closed,
        );
        if area.w >= 900.0 {
            hit(p, Rect::new(x + 223.0, y, 65.0, 25.0), "Details", Control::Panel, !chat.side_hidden);
        }
        let running = chat.last_gate == Some(Gate::Running) && chat.can_stop(&pane.model.chat);
        if running {
            hit(p, Rect::new(right - 108.0, y, 50.0, 25.0), "Stop", Control::Stop, false);
        }
        hit(p, Rect::new(right - 54.0, y, 54.0, 25.0), "Send", Control::Send, chat.last_gate == Some(Gate::Ready));
        let override_button = chat.error == Some("Open the terminal to check")
            && chat.last_gate == Some(Gate::Check)
            && chat.reason.is_none();
        if override_button {
            hit(p, Rect::new(right - 103.0, y + 27.0, 103.0, 19.0), "Send anyway", Control::Override, false);
        }
        self.hits.push((e, Target::Chat(Control::Editor)));
        let info = &pane.model.chat;
        let context = info.context.map(|n| format!("Context now: {n} tokens"));
        let detail = [account.as_deref(), info.model.as_deref(), context.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ");
        let hint = chat.error.map(str::to_owned).unwrap_or(detail);
        p.text(
            Face::Ui,
            11.5,
            x + 1.0,
            y + 36.0,
            &hint,
            pal.muted,
            (e.w - if override_button { 110.0 } else { 0.0 }).max(0.0),
        );
        p.restore(clip);

        let mut hit = |p: &mut Painter, r, label: &str, control, selected| {
            let target = Target::Chat(control);
            button(p, pal, r, label, hover == Some(target), selected);
            self.hits.push((r, target));
        };
        let mut detail_hits = Vec::new();
        if l.side.w > 0.0 {
            let clip = p.clip(l.side);
            p.fill(l.side, pal.pane);
            p.fill(Rect::new(l.side.x, l.side.y, p.px(), l.side.h), pal.line);
            let w = (l.side.w - 20.0) / 3.0;
            for (i, (side, name)) in
                [(Side::Tasks, "Tasks"), (Side::Agents, "Agents"), (Side::Files, "Files")].into_iter().enumerate()
            {
                hit(
                    p,
                    Rect::new(l.side.x + 10.0 + w * i as f32, l.side.y + 10.0, w - 3.0, 27.0),
                    name,
                    Control::Side(side),
                    chat.side == side,
                );
            }
            detail_hits = chat.details.draw(
                p,
                pal,
                super::side::Frame {
                    tab: tab_id,
                    area: l.side,
                    side: chat.side,
                    model: &pane.model,
                    cwd: &pane.cwd,
                    hover: hover.filter(|_| details_active),
                },
            );
            p.restore(clip);
        }
        if l.grip.h > 0.0 {
            // Opaque backing for the drawer label, resize handle and Focus terminal button.
            p.fill(l.grip, pal.bar);
            p.fill(Rect::new(l.grip.x, l.grip.y, l.grip.w, p.px()), pal.line);
            p.rbox(Rect::new(l.grip.cx() - 18.0, l.grip.y + 3.0, 36.0, 2.0), 1.0, pal.thumb);
            p.text(
                Face::Ui,
                11.5,
                l.grip.x + 14.0,
                l.grip.cy() + 2.0,
                chat.reason.map_or("Terminal drawer", Reason::label),
                pal.muted,
                (l.grip.w - 150.0).max(0.0),
            );
            self.hits.push((l.grip, Target::Chat(Control::Grip)));
            if chat.reason == Some(Reason::Tool) {
                let r = Rect::new(l.grip.right() - 128.0, l.grip.y + 2.0, 116.0, 20.0);
                let target = Target::Chat(Control::Open);
                button(p, pal, r, "Focus terminal", hover == Some(target), false);
                self.hits.push((r, target));
            }
        } else if chat.reason == Some(Reason::Tool) && l.conversation.h > 25.0 {
            let row = Rect::new(
                l.conversation.x + 12.0,
                l.conversation.bottom() - 26.0,
                (l.conversation.w - 24.0).max(0.0),
                24.0,
            );
            p.fill(row, pal.page);
            p.text(Face::Ui, 11.5, row.x, row.cy(), Reason::Tool.label(), pal.muted, (row.w - 122.0).max(0.0));
            hit(p, Rect::new(row.right() - 120.0, row.y, 120.0, 22.0), "Open terminal", Control::Open, false);
        }
        if details_active {
            self.hits.extend(detail_hits);
        }
    }
}

impl Chat {
    fn draw_editor(&mut self, p: &mut Painter, pal: &Palette, focused: bool) {
        let e = self.layout.editor;
        p.rbox(e, 7.0, if focused { pal.accent } else { pal.line });
        p.rbox(Rect::new(e.x + 1.0, e.y + 1.0, (e.w - 2.0).max(0.0), (e.h - 2.0).max(0.0)), 6.0, pal.field);
        let c = p.clip(e);
        let x = e.x + 8.0;
        self.cell_width = p.measure(Face::Mono, 13.0, "M").max(1.0);
        self.editor.visible = (((e.h - 7.0) / 17.0) as usize).max(1);
        self.editor.wrap(((e.w - 16.0) / self.cell_width) as usize);
        let selection = self.editor.selection();
        let caret_row = self.editor.row_at(self.editor.caret);
        for (i, row) in self.editor.rows.iter().enumerate().skip(self.editor.scroll).take(self.editor.visible) {
            let y = e.y + 12.0 + (i - self.editor.scroll) as f32 * 17.0;
            let span = &self.editor.text[row.clone()];
            let mut before = |at| p.measure(Face::Mono, 13.0, &self.editor.text[row.start..at]);
            let (start, end) = (selection.start.max(row.start), selection.end.min(row.end));
            if focused && end > start {
                let (a, b) = (before(start), before(end));
                p.fill(Rect::new(x + a, y - 8.0, b - a, 16.0), pal.on);
            }
            let caret = (focused && i == caret_row)
                .then(|| p.measure(Face::Mono, 13.0, &self.editor.text[row.start..self.editor.caret.min(row.end)]));
            if i == 0 && self.editor.text.is_empty() {
                p.text(
                    Face::Ui,
                    13.0,
                    x + 4.0,
                    y,
                    "Message Claude Code…  Shift+Enter for a new line",
                    pal.muted,
                    (e.w - 20.0).max(0.0),
                );
            } else {
                p.text(Face::Mono, 13.0, x, y, span, pal.text, (e.w - 16.0).max(0.0));
            }
            if let Some(at) = caret {
                p.fill(Rect::new(x + at, y - 7.0, p.px(), 15.0), pal.accent);
            }
        }
        p.restore(c);
    }
}

fn button(p: &mut Painter, pal: &Palette, r: Rect, label: &str, hot: bool, selected: bool) {
    p.rbox(
        r,
        5.0,
        if hot {
            pal.hover
        } else if selected {
            pal.on
        } else {
            pal.bar
        },
    );
    p.text_center(Face::Semibold, 11.5, r, label, if selected { pal.accent } else { pal.text });
}

fn abbreviate(label: &str, len: usize) -> String {
    let mut chars = label.chars();
    let mut out: String = chars.by_ref().take(len).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
