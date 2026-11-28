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
        let reading = crate::view::Reading::from_key(&self.win_file.chat_reading);
        // The display scale, as the terminal's cells were last measured for it.
        let scale = self.metrics.map_or(1.0, |m| m.scale());
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return Rect::default() };
        let chat = &mut pane.chat;
        chat.shell_count = pane.model.running_shells();
        // The panel opens when it has a first section to show and closes when none is left,
        // unless the user opened or closed it. Its lists are brought up to date only while it
        // shows.
        let (details, model, cwd) = (&mut chat.details, &pane.model, &pane.cwd);
        chat.side_hidden = !chat.side_choice.unwrap_or_else(|| details.wants_panel(model, cwd));
        if !chat.side_hidden {
            chat.details.refresh_all(&pane.model, &pane.cwd);
        }
        let running = chat.shell_count != 0;
        let reveal = matches!(chat.side, Side::Shells | Side::Outputs);
        let (panel, drawer, chips, notice) = (
            !chat.side_hidden,
            chat.drawer != Drawer::Closed,
            !chat.chips.is_empty(),
            chat.notice(&pane.model.chat).is_some(),
        );
        // The draft's wrapped rows as last flowed; `draw_chat` flows again first and re-lays.
        let lines = chat.editor.rows.len().max(1);
        let left = composer::left_group(&composer::chips_of(&self.config.chat));
        // A session with no message yet has its box in the middle, under a greeting.
        let welcome = chat.welcome(&pane.model);
        // The composer and the cards go on the conversation's reading column.
        pane.ui.set_chat_reading(reading);
        let main_w = (area.w - Layout::panel_width(area, panel, reveal)).max(0.0);
        let column = Some(pane.ui.column_for(area.x, main_w));
        chat.layout = Layout::with_shells(
            area,
            panel,
            drawer,
            chat.drawer_height,
            chips,
            notice,
            running,
            reveal,
            lines,
            left,
            column,
            welcome,
        );
        super::question::place(chat, &pane.model);
        super::terminal_card::place(chat, scale);
        self.session_view.pane_area = chat.layout.conversation;
        chat.layout.drawer
    }

    pub fn draw_chat(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let (now, motion, focused) = (cx.now(), cx.anim().animates(), cx.focused());
        let (deadline, moving) = self.draw_chat_with(p, area, pal, now, motion, focused);
        if let Some(deadline) = deadline {
            cx.wake_at(deadline);
        }
        if let Some(r) = moving {
            cx.animating(r);
        }
    }

    /// [`draw_chat`](Self::draw_chat) without a window, at `now`, in a window that has the
    /// keyboard when `window_focused`: the transcript's next scroll deadline, and what moves.
    pub fn draw_chat_with(
        &mut self,
        p: &mut Painter,
        area: Rect,
        pal: &Palette,
        now: std::time::Instant,
        motion: bool,
        window_focused: bool,
    ) -> (Option<std::time::Instant>, Option<Rect>) {
        let mut frame = (None, None);
        let Some(tab_id) = self.tabs.active else { return frame };
        let details_active = self.nothing_over() && !self.picture_overlay() && !self.usage.popup.open;
        let hover = self.hover;
        let mouse = self.background_pointer();
        let diff = crate::view::look::Diff::from_ansi(&self.term_colours.0.ansi);
        let scheme = self.term_colours.0;
        let focused = window_focused && self.chat_takes_input() && self.nothing_over() && !self.picture_overlay();
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return frame };
        let chat = &mut pane.chat;
        // The box grows with the draft: wrap it at the box's width, then lay out for its rows.
        let before = chat.editor.rows.len();
        let width = composer::editor_text_width(chat.layout.editor, chat.shell_mode);
        chat.editor.flow(p, width, chat.layout.editor.h);
        if chat.editor.rows.len() != before {
            self.layout_chat(area);
        }
        // A new session's greeting says what Claude Code's banner on the screen says.
        let greets = |t: &&crate::tabs::Tab| t.pane.as_ref().is_some_and(|p| p.chat.layout.greeting.h > 0.0);
        let banner = self.tabs.shown().filter(greets).and_then(|t| super::welcome::banner(&t.term.screen));
        let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) else { return frame };
        let chat = &mut pane.chat;
        let l = chat.layout;
        // A card that waits for an answer stands where the composer is: the composer is not
        // drawn then, and its draft is kept.
        let docked = chat.question.docked();
        let suggestions = if docked { None } else { chat.suggestion_plan(&self.win_file.chat_pins) };
        let popup = if suggestions.is_none() && !docked { chat.popup_plan(&pane.model) } else { None };
        let transcript_mouse = suggestions.as_ref().map_or(mouse, |popup| popup.pointer_behind(mouse));
        let transcript_mouse = popup.as_ref().map_or(transcript_mouse, |popup| popup.pointer_behind(transcript_mouse));
        pane.ui.set_chat_reading(crate::view::Reading::from_key(&self.win_file.chat_reading));
        // A card that asks says what Claude waits for; the working line does not repeat it.
        let asking = chat.question.visible() && !chat.question.only_denials();
        let waiting = pane.ui.working().is_some_and(|w| super::question::waits_for_user(&w.label));
        pane.ui.hide_working(asking && waiting);
        // Stop is the composer's: beside Send while a turn runs, dimmed while its checks fail.
        let info = &pane.model.chat;
        let running = chat.last_gate == Some(super::Gate::Running) || pane.ui.working().is_some() && !waiting;
        let stop_ready = pane.ui.working().is_some_and(|w| w.stop.is_some())
            || chat.last_gate == Some(super::Gate::Running) && chat.can_stop(info);
        let stop = running.then_some(stop_ready);
        // The turn this Stop is drawn for: a click stops that turn only, never one that started
        // since (the working line's Stop carries its turn the same way).
        chat.stop_turn = if stop_ready && running {
            pane.ui
                .working()
                .and_then(|w| w.stop)
                .or_else(|| info.prompt.map(|prompt| crate::view::working::Turn(info.generation, prompt)))
        } else {
            None
        };
        pane.ui.set_offer(chat.question.background_inline());
        pane.ui.set_queued(chat.queued_shown());
        // An empty conversation: the greeting of a new session (drawn below), or a quiet line
        // while there is nothing to read yet.
        let welcome = l.greeting.h > 0.0 && !docked;
        pane.ui.set_empty_line(match () {
            _ if welcome || pane.model.loaded || chat.awaiting_transcript => None,
            _ if chat.identity_unverified => Some("The conversation appears here when its transcript is available."),
            _ => Some("Reading the conversation…"),
        });
        // With the panel closed the working line shows the task list's progress, which opens it;
        // not where the window has no room for the panel (the click could not show it).
        pane.ui.show_progress(l.side.w <= 0.0 && l.panel_room);
        // The pending call the card asks about has not started: its line says it waits.
        let own = asking && chat.question.answerable.is_some() && chat.question.answerable == info.pending;
        pane.ui.set_awaiting(super::question::pending(&pane.model).filter(|_| own).map(|p| p.item));
        pane.ui.set_diff_colours(diff);
        pane.ui.set_terminal_colours(scheme);
        pane.ui.draw(p, pal, l.conversation, &mut pane.model, transcript_mouse, now, motion);
        if welcome {
            super::welcome::Welcome::read(chat, &pane.cwd, banner.as_ref(), &self.win_file.chat_pins).draw(
                p,
                pal,
                l.greeting,
                l.starts,
                hover,
                &mut self.hits,
            );
        }
        if let Some(r) = pane.ui.progress_rect() {
            // "2 of 3" on the working line opens the panel, as its own button does.
            self.hits.push((r, Target::Chat(Control::Panel)));
        }
        if let Some(r) = pane.ui.queued_rect() {
            // Send now under the messages waiting in Claude Code's queue.
            self.hits.push((r, Target::Chat(Control::SendQueued)));
        }
        if let Some(r) = pane.ui.offer_rect() {
            // "Run in background" on the command's line: the same click as the card's button.
            self.hits.push((r, Target::Chat(Control::Question(super::question::Hit::Background))));
        }
        frame.0 = pane.ui.scroll_deadline();
        if pane.ui.animating() && motion {
            frame.1 = Some(l.conversation);
        }
        // Where the command's line could not show "Run in background", the card offers it: laid
        // out in the next frame, which is asked for at once.
        let inline = pane.ui.offer_drawn();
        if chat.question.offer_inline != inline {
            chat.question.offer_inline = inline;
            frame.1 = Some(area);
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
            detail_hits = chat.details.draw(
                p,
                pal,
                super::side::Frame {
                    tab: tab_id,
                    area: l.side,
                    side: chat.side,
                    sections: true,
                    diff,
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
            && !chat.terminal_card.shown()
            && (!chat.question.visible() || chat.question.only_denials())
            && l.conversation.h > 25.0
        {
            // In the conversation's reading column, like the working line under it.
            let (x, w) = pane.ui.column();
            let row = Rect::new(
                x,
                l.conversation.bottom() - pane.ui.footer_height(l.conversation.w, pane.model.agent.is_none()) - 26.0,
                w.max(0.0),
                24.0,
            );
            p.fill(row, pal.page);
            p.text(Face::Ui, 11.5, row.x, row.cy(), Reason::Tool.label(), pal.muted, (row.w - 122.0).max(0.0));
            hit(p, Rect::new(row.right() - 120.0, row.y, 120.0, 22.0), "Open terminal", Control::Open, false);
        }
        chat.question.colours = Some(diff);
        super::question::Draw { pal, hover, focused, now }.card(p, chat, &pane.model, &mut self.hits);
        super::terminal_card::draw(chat, p, pal, scheme, hover, &mut self.hits);
        if std::mem::take(&mut chat.question.relayout) {
            // The card was measured at another scale: the next frame lays it out at this one.
            frame.1 = Some(area);
        }
        if details_active {
            self.hits.extend(detail_hits);
        }
        if docked {
            chat.layout.suggestions = Rect::default();
            return frame;
        }
        // The slash picker sits above the transcript, including its footer and the closed-drawer hint.
        composer::Draw {
            pal,
            focused,
            wide: area.w >= 900.0 || l.side.w > 0.0,
            hover,
            hits: &mut self.hits,
            pins: &self.win_file.chat_pins,
            chips: &composer::chips_of(&self.config.chat),
            own: &self.config.chat.commands,
            stop,
            welcome,
        }
        .composer(p, chat, &pane.model.chat, suggestions, popup);
        frame
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
