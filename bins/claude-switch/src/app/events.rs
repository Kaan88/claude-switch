use super::*;
use crate::tabs::SESSIONS;
use crate::windowlook;
use mdshared_ui::gfx::Painter;
use mdshared_ui::host::{App, Event};
use std::sync::{Arc, Mutex};

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
                    Some(Target::Entry(e) | Target::Pin(e)) if button == Button::Right => {
                        self.mouse = (x, y);
                        self.focus = Focus::List;
                        self.row_menu(e, Rect::new(x, y, 0.0, 0.0));
                    }
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
                self.see_shown();
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
                if self.undo_hide.take_if(|undo| undo.until <= now).is_some() {
                    cx.request_redraw_all();
                }
                if self.marks_flush_at.is_some_and(|at| at <= now) {
                    self.save_marks();
                    if self.info.is_some() {
                        cx.request_redraw_all();
                    }
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
            if let Some(job) = self.naming_job.as_ref() {
                job.cancel();
                if job.running() {
                    return;
                }
            }
            self.finish_exit(cx);
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
                let changed = self.take_jobs();
                if self.quit && self.naming_job.is_none() {
                    self.finish_exit(cx);
                } else if changed {
                    cx.request_redraw_all();
                }
                self.timers(cx);
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

impl AppState {
    fn finish_exit(&mut self, cx: &mut Cx<'_, Msg>) {
        if self.marks_flush_at.is_some() {
            self.save_marks();
        }
        self.save_window();
        cx.exit();
    }
}
