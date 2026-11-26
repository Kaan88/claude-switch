use std::path::Path;

use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::theme::Palette;

use super::{Chat, Control, Drawer, Layout, Reason, Side, composer};
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
            chat.notice(&pane.model.chat).is_some(),
        );
        self.session_view.pane_area = chat.layout.conversation;
        chat.layout.drawer
    }

    pub fn draw_chat(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let hover = self.hover;
        let mouse = self.background_pointer();
        let focused = cx.focused() && self.chat_takes_input() && self.nothing_over() && !self.picture_overlay();
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return };
        let chat = &mut pane.chat;
        let l = chat.layout;
        let now = cx.now();
        let motion = cx.anim().animates();
        pane.ui.draw(p, pal, l.conversation, &mut pane.model, mouse, now, motion);
        if pane.ui.animating() && motion {
            cx.animating(l.conversation);
        }
        composer::Draw {
            pal,
            focused,
            wide: area.w >= 900.0,
            hover,
            hits: &mut self.hits,
            pins: &self.win_file.chat_pins,
        }
        .composer(p, chat, &pane.model.chat);

        let mut hit = |p: &mut Painter, r, label: &str, control, selected| {
            let target = Target::Chat(control);
            button(p, pal, r, label, hover == Some(target), selected);
            self.hits.push((r, target));
        };
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
            chat.draw_side(p, pal, &pane.model);
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
    }
}

impl Chat {
    fn draw_side(&mut self, p: &mut Painter, pal: &Palette, model: &super::super::model::Model) {
        use claude_switch_core::transcript::TaskStatus;
        let r = self.layout.side;
        let heading = match self.side {
            Side::Tasks => "Tasks",
            Side::Agents => "Agents",
            Side::Files => "Changed files · from tool results",
        };
        p.text(Face::Semibold, 12.0, r.x + 12.0, r.y + 53.0, heading, pal.muted, r.w - 24.0);
        let rows: Vec<_> = match self.side {
            Side::Tasks => model
                .tasks
                .iter()
                .map(|task| {
                    let mark = match task.status {
                        TaskStatus::Completed => "✓",
                        TaskStatus::InProgress => "◉",
                        TaskStatus::Pending => "○",
                    };
                    (format!("{mark}  {}", task.content), task.status == TaskStatus::InProgress)
                })
                .collect(),
            Side::Agents => model
                .agents
                .iter()
                .map(|a| {
                    (
                        format!(
                            "{}  {}",
                            match a.finished {
                                Some(true) => "✓",
                                Some(false) => "◉",
                                None => "?",
                            },
                            a.description.as_deref().unwrap_or(&a.id)
                        ),
                        a.finished == Some(false),
                    )
                })
                .collect(),
            Side::Files => model
                .changed_files
                .iter()
                .map(|path| {
                    let name = Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or(path);
                    (format!("▹  {name}"), false)
                })
                .collect(),
        };
        let visible = ((r.h - 66.0) / 26.0) as usize;
        self.side_max = rows.len().saturating_sub(visible);
        self.side_scroll = self.side_scroll.min(self.side_max);
        if rows.is_empty() {
            p.text(Face::Ui, 12.0, r.x + 12.0, r.y + 80.0, "Nothing to show yet", pal.muted, r.w - 24.0);
        }
        for (i, (label, active)) in rows.iter().skip(self.side_scroll).take(visible).enumerate() {
            p.text(
                Face::Ui,
                12.0,
                r.x + 12.0,
                r.y + 79.0 + i as f32 * 26.0,
                label,
                if *active { pal.accent } else { pal.text },
                r.w - 24.0,
            );
        }
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
