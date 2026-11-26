use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::theme::Palette;

use super::{Control, Drawer, Layout, Reason, Side, composer};
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
        chat.shell_count = pane.model.running_shells();
        let running = chat.shell_count != 0;
        let reveal = matches!(chat.side, Side::Shells | Side::Outputs);
        let (panel, drawer, chips, notice) = (
            !chat.side_hidden,
            chat.drawer != Drawer::Closed,
            !chat.chips.is_empty(),
            chat.notice(&pane.model.chat).is_some(),
        );
        chat.layout = if running || reveal {
            Layout::with_shells(area, panel, drawer, chat.drawer_height, chips, notice, running, reveal)
        } else {
            Layout::new(area, panel, drawer, chat.drawer_height, chips, notice)
        };
        super::question::place(chat, &pane.model);
        self.session_view.pane_area = chat.layout.conversation;
        chat.layout.drawer
    }

    pub fn draw_chat(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let Some(tab_id) = self.tabs.active else { return };
        let details_active = self.nothing_over() && !self.picture_overlay() && !self.usage.popup.open;
        let hover = self.hover;
        let mouse = self.background_pointer();
        let focused = cx.focused() && self.chat_takes_input() && self.nothing_over() && !self.picture_overlay();
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return };
        let chat = &mut pane.chat;
        let l = chat.layout;
        let suggestions = chat.suggestion_plan(&self.win_file.chat_pins);
        let transcript_mouse = suggestions.as_ref().map_or(mouse, |popup| popup.pointer_behind(mouse));
        let now = cx.now();
        let motion = cx.anim().animates();
        pane.ui.draw(p, pal, l.conversation, &mut pane.model, transcript_mouse, now, motion);
        if pane.ui.animating() && motion {
            cx.animating(l.conversation);
        }
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
            let w = (l.side.w - 12.0) / 5.0;
            let shells = format!("Shells {}", pane.model.shells.len());
            for (i, (side, name)) in [
                (Side::Tasks, "Tasks"),
                (Side::Agents, "Agents"),
                (Side::Shells, shells.as_str()),
                (Side::Files, "Files"),
                (Side::Outputs, "Outputs"),
            ]
            .into_iter()
            .enumerate()
            {
                hit(
                    p,
                    Rect::new(l.side.x + 6.0 + w * i as f32, l.side.y + 10.0, w - 2.0, 27.0),
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
        } else {
            chat.details.visible_shells.clear();
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
        } else if chat.reason == Some(Reason::Tool)
            && super::question::pending(&pane.model).is_none()
            && l.conversation.h > 25.0
        {
            let row = Rect::new(
                l.conversation.x + 12.0,
                l.conversation.bottom() - pane.ui.footer_height(l.conversation.w, pane.model.agent.is_none()) - 26.0,
                (l.conversation.w - 24.0).max(0.0),
                24.0,
            );
            p.fill(row, pal.page);
            p.text(Face::Ui, 11.5, row.x, row.cy(), Reason::Tool.label(), pal.muted, (row.w - 122.0).max(0.0));
            hit(p, Rect::new(row.right() - 120.0, row.y, 120.0, 22.0), "Open terminal", Control::Open, false);
        }
        super::question::Draw { pal, hover, focused }.card(p, chat, &pane.model, &mut self.hits);
        if details_active {
            self.hits.extend(detail_hits);
        }
        // The slash picker sits above the transcript, including its footer and the closed-drawer hint.
        composer::Draw {
            pal,
            focused,
            wide: area.w >= 900.0 || l.side.w > 0.0,
            hover,
            hits: &mut self.hits,
            pins: &self.win_file.chat_pins,
        }
        .composer(p, chat, &pane.model.chat, suggestions);
    }
}

pub(super) fn button(p: &mut Painter, pal: &Palette, r: Rect, label: &str, hot: bool, selected: bool) {
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
