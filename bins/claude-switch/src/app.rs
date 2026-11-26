//! The window loop's side of the state: the host's events routed to the open layer (palette,
//! drop-down, menu, dialog, then the window), timers, and the frame drawn from top to bottom.

use std::sync::{Arc, Mutex};

use mdshared_ui::chrome::{TITLE_H, WinButton};
use mdshared_ui::cursor::Cursor;
use mdshared_ui::dropdown::Outcome as DdOutcome;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::{App, Button, Cx, Event, KeyPress};
use mdshared_ui::keys::KeyName;
use mdshared_ui::palette::Outcome as PalOutcome;
use mdshared_ui::splitter::Axis;
use mdshared_ui::tooltip::Over;

use crate::actions;
use crate::dialogs::{self, Dialog};
use crate::state::{AppState, Focus, MenuKind, Msg, POLL, Target};
use crate::tabs::SESSIONS;
use crate::termkeys;
use crate::topbar::{self, Bar};
use crate::{list, windowlook, winfile};

impl AppState {
    fn resume_polling(&mut self, now: std::time::Instant) {
        if !matches!(self.source, crate::source::Source::Demo) {
            self.next_poll.get_or_insert(now + POLL);
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        let modal = self.dialog.is_some();
        self.hits
            .iter()
            .rev()
            .filter(|(_, t)| !modal || matches!(t, Target::Dialog(_)))
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| *t)
    }

    fn rect_of(&self, t: Target) -> Option<Rect> {
        self.hits.iter().rev().find(|(_, h)| *h == t).map(|(r, _)| *r)
    }

    /// The panes' area below the title row and the info bar, as the last frame laid it out.
    fn main_area(&self, cx: &Cx<'_, Msg>) -> Rect {
        let (w, h) = cx.size();
        let top = self.list_area.y.max(TITLE_H);
        Rect::new(0.0, top, w, h - top)
    }

    fn timers(&self, cx: &mut Cx<'_, Msg>) {
        for at in [self.next_poll, self.tip.deadline(), self.toasts.deadline()].into_iter().flatten() {
            cx.wake_at(at);
        }
    }

    pub(crate) fn save_window(&mut self) {
        self.win_file.split = self.splitter.to_setting();
        self.win_file.session_split = self.session_view.splitter.to_setting();
        winfile::save(self.source.window_file().as_deref(), &self.win_file);
    }

    fn activate(&mut self, cx: &mut Cx<'_, Msg>, t: Target, x: f32) {
        let now = cx.now();
        match t {
            Target::Caption(WinButton::Close) => {
                if self.ask_quit() {
                    self.quit = true;
                }
            }
            Target::TabClose(id) => self.ask_close_tab(id),
            Target::SessionToggle => self.toggle_session_view(),
            Target::Caption(WinButton::Max) => cx.toggle_maximize(),
            Target::Caption(WinButton::Min) => cx.minimize(),
            Target::SearchClear => {
                self.search.clear();
                self.after_search();
            }
            Target::NewSession => self.command(now, actions::NEW_SESSION, None),
            Target::Refresh => self.command(now, actions::REFRESH, None),
            Target::MenuButton => {
                if let Some(r) = self.rect_of(t) {
                    self.main_menu(r);
                }
            }
            Target::OpenCli => self.command(now, actions::OPEN_CLI, None),
            Target::OpenDesktop => self.command(now, actions::OPEN_DESKTOP, None),
            Target::OpenAs => self.command(now, actions::OPEN_AS, self.rect_of(t)),
            Target::OpenCopy => self.command(now, actions::OPEN_COPY, None),
            Target::InfoClose => self.info = None,
            Target::Dialog(id) => dialogs::click(self, now, id, x),
            _ => {}
        }
    }

    fn after_search(&mut self) {
        self.rebuild_view();
        let visible = self.selected.clone().and_then(|id| self.entry_of(&id)).is_some();
        if !visible {
            self.select_first();
        }
        self.sections.scroll = 0.0;
        self.reveal_selected();
    }

    fn palette_outcome(&mut self, cx: &mut Cx<'_, Msg>, o: PalOutcome) {
        match o {
            PalOutcome::Chosen(id) => {
                self.palette = None;
                self.command(cx.now(), id, None);
            }
            PalOutcome::Closed => self.palette = None,
            _ => {}
        }
    }

    fn dropdown_outcome(&mut self, o: DdOutcome) {
        if let DdOutcome::Changed(i) = o
            && let Some(slug) = self.accounts_dd.value(i).cloned()
        {
            self.switch_account(&slug);
        }
    }

    fn menu_chosen(&mut self, cx: &mut Cx<'_, Msg>) {
        if let Some((kind, menu)) = self.menu.take()
            && let Some(id) = menu.chosen()
        {
            let anchor = self.rect_of(if kind == MenuKind::OpenAs { Target::OpenAs } else { Target::MenuButton });
            self.command(cx.now(), id, anchor);
        }
    }

    fn key(&mut self, cx: &mut Cx<'_, Msg>, k: KeyPress) {
        let now = cx.now();
        let Some(name) = k.name.clone() else {
            // A key the toolkit has no name for may still have typed text for a program.
            if self.terminal_shown() && self.nothing_over() && !self.picture_overlay() {
                self.term_key(now, &k);
            }
            return;
        };
        let (mods, text) = (k.mods, k.text.as_deref());
        let bar = Bar::layout(cx.size().0);
        if let Some(p) = &mut self.palette {
            let o = p.key(&name, mods, text);
            self.palette_outcome(cx, o);
            return;
        }
        if self.accounts_dd.is_open() {
            let o = self.accounts_dd.key(bar.account, &name, mods);
            self.dropdown_outcome(o);
            return;
        }
        if let Some((_, menu)) = &mut self.menu {
            match name {
                KeyName::Up => menu.step(false),
                KeyName::Down => menu.step(true),
                KeyName::Enter | KeyName::Space => self.menu_chosen(cx),
                KeyName::Escape | KeyName::Tab => self.menu = None,
                _ => {}
            }
            return;
        }
        if self.dialog.is_some() {
            dialogs::key(self, now, &name, mods, text);
            return;
        }
        if self.session_key(cx.size().0, &name, mods) {
            return;
        }
        // The shortcuts that stay with the window in every tab, then a terminal's own keys.
        if let Some(id) = termkeys::shortcut(&name, mods)
            && (id != actions::SESSION_VIEW || self.tabs.shown().is_some_and(|t| t.pane.is_some()))
        {
            self.command(now, id, None);
            return;
        }
        if self.terminal_shown() {
            self.term_key(now, &k);
            Self::cursor_moved(cx);
            return;
        }
        if let Some(id) = self.keymap.lookup(&name, mods) {
            self.command(now, id, None);
            return;
        }
        if name == KeyName::Tab {
            self.key_used = true;
            let order = [Focus::List, Focus::Search, Focus::Account];
            let at = order.iter().position(|f| *f == self.focus).unwrap_or(0);
            self.focus = order[(at + if mods.shift { 2 } else { 1 }) % 3];
            return;
        }
        match self.focus {
            Focus::Search => match name {
                KeyName::Escape if !self.search.is_empty() => {
                    self.search.clear();
                    self.after_search();
                }
                KeyName::Escape => self.focus = Focus::List,
                KeyName::Enter => self.command(now, actions::OPEN_LAST, None),
                KeyName::Up | KeyName::Down | KeyName::PageUp | KeyName::PageDown => {
                    list::key(self, &name);
                }
                _ => {
                    if self.search.handle_key(&name, mods, text, true) == Some(true) {
                        self.after_search();
                    }
                }
            },
            Focus::Account => {
                let o = self.accounts_dd.key(bar.account, &name, mods);
                self.dropdown_outcome(o);
            }
            Focus::List => {
                if list::key(self, &name) {
                    self.key_used = true;
                } else if name == KeyName::Enter {
                    self.command(now, actions::OPEN_LAST, None);
                } else if name == KeyName::Escape && !self.search.is_empty() {
                    self.search.clear();
                    self.after_search();
                } else if let Some(t) = text.filter(|t| !t.chars().any(char::is_control) && !mods.ctrl && !mods.alt)
                    && !t.trim().is_empty()
                {
                    self.focus = Focus::Search;
                    self.search.insert(t);
                    self.after_search();
                }
            }
        }
    }

    fn press(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32, clicks: u32) {
        let hover = self.hit(x, y);
        self.tip.press(hover);
        let bar = Bar::layout(cx.size().0);
        if let Some(p) = &mut self.palette {
            let o = p.press(x, y, cx.mods(), clicks.min(3) as u8);
            self.palette_outcome(cx, o);
            return;
        }
        if self.accounts_dd.is_open() {
            let o = self.accounts_dd.press(bar.account, x, y);
            self.dropdown_outcome(o);
            return;
        }
        if let Some((_, menu)) = &self.menu {
            if !menu.contains(x, y) {
                self.menu = None;
            }
            return;
        }
        if let Some(Dialog::Settings(form)) = &mut self.dialog {
            let o = form.press(x, y, cx.mods(), clicks.min(3) as u8);
            dialogs::settings_outcome(self, o);
            return;
        }
        if self.dialog.is_some() {
            self.pressed = hover;
            if hover == Some(Target::Dialog(dialogs::FIELD)) {
                dialogs::click(self, cx.now(), dialogs::FIELD, x);
            }
            return;
        }
        if self.session_press(cx, x, y, clicks) {
            return;
        }
        match hover {
            Some(Target::Tab(id)) => {
                self.show_tab(id);
                return;
            }
            Some(Target::Terminal) => {
                self.term_press(x, y, Button::Left, clicks, cx.mods());
                return;
            }
            Some(Target::Search | Target::Entry(_)) if self.terminal_shown() => self.show_sessions(),
            _ => {}
        }
        let area = self.main_area(cx);
        if !self.terminal_shown() && self.splitter.press(area, Axis::Horizontal, x, y) {
            return;
        }
        self.key_used = false;
        match hover {
            Some(Target::Title) if clicks == 2 => cx.title_double_click(),
            Some(Target::Title) => cx.drag_window(),
            Some(Target::Account) => {
                self.focus = Focus::Account;
                let o = self.accounts_dd.press(bar.account, x, y);
                self.dropdown_outcome(o);
            }
            Some(Target::Search) => {
                self.focus = Focus::Search;
                let inner = bar.search_inner();
                if clicks >= 2 {
                    self.search.select_all();
                } else {
                    self.search.press(x - inner.x, &self.search_stops, cx.mods().shift);
                }
            }
            Some(Target::Entry(e)) => {
                self.focus = Focus::List;
                list::click(self, e);
                if clicks == 2 && self.session_at(e).is_some() {
                    self.command(cx.now(), actions::OPEN_LAST, None);
                }
            }
            Some(t) => self.pressed = Some(t),
            None => {}
        }
    }

    fn release(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) {
        if self.term_release(x, y, Button::Left, cx.mods()) {
            return;
        }
        if self.session_release(cx, x, y) {
            return;
        }
        if self.splitter.release() {
            self.save_window();
            return;
        }
        if let Some(p) = &mut self.palette {
            let o = p.release(x, y);
            self.palette_outcome(cx, o);
            return;
        }
        if self.accounts_dd.is_open() || self.accounts_dd.pressed() {
            let bar = Bar::layout(cx.size().0);
            let o = self.accounts_dd.release(bar.account, x, y);
            self.dropdown_outcome(o);
            return;
        }
        if let Some((_, menu)) = &mut self.menu {
            if menu.contains(x, y) {
                menu.pointer(x, y);
                self.menu_chosen(cx);
            }
            return;
        }
        if let Some(Dialog::Settings(form)) = &mut self.dialog {
            let o = form.release(x, y);
            dialogs::settings_outcome(self, o);
            return;
        }
        if let Some(t) = self.pressed.take()
            && self.hit(x, y) == Some(t)
        {
            self.activate(cx, t, x);
        }
    }

    /// The pointer moved. True when something shows a change.
    fn moved(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) -> bool {
        self.mouse = (x, y);
        let terminal_drag = self.tabs.shown().is_some_and(|tab| tab.term.selecting || tab.term.reported.is_some());
        if !terminal_drag && let Some(changed) = self.session_move(cx, x, y) {
            return changed;
        }
        let area = self.main_area(cx);
        if self.splitter.dragging() {
            cx.set_cursor(Cursor::ResizeEw);
            return self.splitter.drag(area, Axis::Horizontal, x, y);
        }
        let mut changed = false;
        if self.terminal_shown() && self.nothing_over() {
            changed |= self.term_move(x, y, cx.mods());
        }
        if let Some(p) = &mut self.palette {
            changed |= p.pointer_move(x, y);
        }
        if self.accounts_dd.is_open() {
            changed |= self.accounts_dd.pointer(x, y);
        }
        if let Some((_, menu)) = &mut self.menu {
            changed |= menu.pointer(x, y);
        }
        if let Some(Dialog::Settings(form)) = &mut self.dialog {
            changed |= form.pointer_move(x, y);
        }
        let hover = self.hit(x, y);
        if hover != self.hover {
            self.hover = hover;
            changed = true;
        }
        let over = hover
            .and_then(|t| Some((t, topbar::tip_of(t)?, self.rect_of(t)?)))
            .filter(|_| self.menu.is_none() && self.dialog.is_none() && self.palette.is_none())
            .map(|(key, text, anchor)| Over { key, text: text.to_string(), anchor });
        changed |= self.tip.hover_at(cx.now(), over, (x, y));
        let on_split =
            self.dialog.is_none() && !self.terminal_shown() && self.splitter.hit(area, Axis::Horizontal, x, y);
        let program_mouse =
            self.tabs.shown().is_some_and(|t| termkeys::mouse_to_program(&t.term.screen.modes(), cx.mods().shift));
        cx.set_cursor(match hover {
            _ if on_split => Cursor::ResizeEw,
            Some(Target::Terminal) if !program_mouse => Cursor::Text,
            Some(Target::Search) if self.dialog.is_none() => Cursor::Text,
            Some(Target::Dialog(dialogs::FIELD)) => Cursor::Text,
            _ => Cursor::Default,
        });
        changed
    }
}

impl App for AppState {
    type Message = Msg;

    fn start(&mut self, cx: &mut Cx<'_, Msg>) {
        self.proxy = Some(Arc::new(Mutex::new(cx.proxy())));
        windowlook::apply(self, cx);
        let now = cx.now();
        self.refresh(now);
        self.resume_polling(now);
        self.start_debug(cx);
        self.start_demo_session();
        self.timers(cx);
    }

    fn event(&mut self, cx: &mut Cx<'_, Msg>, event: Event) {
        match event {
            Event::PointerMove { x, y } => {
                if self.moved(cx, x, y) {
                    cx.request_redraw_all();
                }
                self.timers(cx);
                return;
            }
            Event::PointerLeave => {
                self.hover = None;
                self.session_pointer_left();
                self.tip.hover(cx.now(), None);
                cx.request_redraw_all();
            }
            Event::Press { x, y, button: Button::Left, clicks } => self.press(cx, x, y, clicks),
            Event::Release { x, y, button: Button::Left } => self.release(cx, x, y),
            Event::Press { x, y, button, clicks } => {
                if !self.nothing_over() || self.picture_overlay() {
                    return;
                }
                match self.hit(x, y) {
                    Some(Target::Tab(id) | Target::TabClose(id)) if button == Button::Middle && id != SESSIONS => {
                        self.ask_close_tab(id)
                    }
                    Some(Target::Terminal) => self.term_press(x, y, button, clicks, cx.mods()),
                    _ => return,
                }
            }
            Event::Release { x, y, button } => {
                if self.picture_overlay() || !self.term_release(x, y, button, cx.mods()) {
                    return;
                }
            }
            Event::Wheel { dy, .. } if self.nothing_over() && self.terminal_shown() => {
                let (x, y) = self.mouse;
                if !self.session_wheel(cx.size().0, x, y, dy) && self.term_area.contains(x, y) {
                    self.term_wheel(dy, cx.mods());
                }
            }
            Event::Wheel { dy, .. } => {
                let (x, y) = self.mouse;
                if let Some(p) = &mut self.palette {
                    p.wheel(dy);
                } else if self.accounts_dd.is_open() {
                    self.accounts_dd.wheel(x, y, dy);
                } else if let Some((_, menu)) = &mut self.menu {
                    menu.scroll_by(-dy);
                } else if self.dialog.is_some() {
                    dialogs::wheel(self, x, y, dy);
                } else if self.list_area.contains(x, y) {
                    self.sections.scroll = self.sections.clamp_scroll(self.sections.scroll - f64::from(dy));
                    self.moved(cx, x, y);
                }
            }
            Event::Key(k) => self.key(cx, k),
            Event::Text(t) => {
                if self.dialog.is_some() {
                    dialogs::text_input(self, &t);
                } else if self.terminal_shown() && self.nothing_over() && !self.picture_overlay() {
                    self.term_text(&t);
                } else if self.focus == Focus::Search && self.palette.is_none() && self.search.insert(&t) {
                    self.after_search();
                }
            }
            Event::Focus(false) => self.term_focus(false),
            Event::Focus(true) => {
                self.term_focus(true);
                self.retheme();
                let now = cx.now();
                if self.last_scan.is_none_or(|t| now.duration_since(t).as_secs() >= 3) {
                    self.refresh(now);
                }
                self.resume_polling(now);
            }
            Event::Resize { w, h } => {
                if !cx.maximized() && w > 0.0 && h > 0.0 {
                    self.win_file.w = w;
                    self.win_file.h = h;
                }
                self.resume_polling(cx.now());
            }
            Event::Theme { .. } | Event::Contrast(_) => {
                self.retheme();
            }
            Event::Wake(now) => {
                self.terminal_drop_due(cx, now);
                self.session_tick(cx, false);
                if let Some(at) = self.typed_due(now) {
                    cx.wake_at(at);
                }
                if self.tip.tick(now) | self.toasts.tick(now) {
                    cx.request_redraw_all();
                }
                if self.next_poll.is_some_and(|t| t <= now) {
                    // An idle window that cannot be seen stops polling until it is shown again.
                    if cx.window().is_minimized() == Some(true) {
                        self.next_poll = None;
                    } else {
                        self.poll_running();
                        self.next_poll = Some(now + POLL);
                    }
                }
                self.timers(cx);
                return;
            }
            Event::CloseRequested if self.ask_quit() => self.quit = true,
            _ => {}
        }
        if self.quit {
            self.save_window();
            cx.exit();
            return;
        }
        windowlook::apply(self, cx);
        cx.request_redraw_all();
        self.timers(cx);
    }

    fn raw(&mut self, cx: &mut Cx<'_, Msg>, event: &winit::event::WindowEvent) {
        if let winit::event::WindowEvent::DroppedFile(file) = event {
            self.terminal_drop(cx, file.clone());
        }
    }

    fn message(&mut self, cx: &mut Cx<'_, Msg>, m: Msg) {
        match m {
            Msg::Woke => {
                if self.take_jobs() {
                    cx.request_redraw_all();
                }
            }
            Msg::Term => self.term_woke(cx),
            Msg::SessionView => self.session_ready(cx),
            Msg::AttachPictures(id, files) => {
                if self.paste_paths(id, &files) {
                    cx.request_redraw(self.term_area);
                }
            }
        }
    }

    fn draw(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, _damage: Rect) {
        crate::app_draw::draw(self, cx, p);
    }
}
