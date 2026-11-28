use super::*;
use crate::dialogs::PeekInput;
use crate::tabs::SESSIONS;
use crate::windowlook;
use mdshared_ui::gfx::Painter;
use mdshared_ui::host::{App, Event};
use std::sync::{Arc, Mutex};

impl App for AppState {
    type Message = Msg;

    fn start(&mut self, cx: &mut Cx<'_, Msg>) {
        self.proxy = Some(Arc::new(Mutex::new(cx.proxy())));
        if let Some(place) = &self.win_file.place {
            winfile::apply(cx.window(), place);
        }
        self.load_tab_restore();
        windowlook::apply(self, cx);
        let now = cx.now();
        self.refresh(now);
        self.request_usage(now, Demand::Automatic, None);
        self.resume_polling(now);
        self.start_debug(cx);
        self.start_demo_session();
        self.start_usage_hover_demo(now);
        self.start_compact();
        self.serve_handovers();
        self.setup_notifications(cx);
        self.timers(cx);
    }

    fn event(&mut self, cx: &mut Cx<'_, Msg>, event: Event) {
        if matches!(event, Event::Key(_) | Event::Text(_)) {
            self.compact.jobs.typed = Some(cx.now());
        }
        match self.peek_event(&event) {
            PeekInput::Swallow => return,
            PeekInput::Redraw => {
                self.refresh_pointer(cx);
                cx.request_redraw_all();
                return;
            }
            PeekInput::Pass => {}
        }
        match event {
            Event::PointerMove { x, y } => {
                if self.moved(cx, x, y) {
                    cx.request_redraw_all();
                }
                self.timers(cx);
                return;
            }
            Event::PointerLeave => {
                self.tabs.drag = None;
                self.usage_hover(cx.now(), None);
                self.session_pointer_left();
                self.refresh_pointer(cx);
                cx.request_redraw_all();
            }
            Event::Press { x, y, button: Button::Left, clicks } => self.press(cx, x, y, clicks),
            Event::Release { x, y, button: Button::Left } => self.release(cx, x, y),
            Event::Press { x, y, button: Button::Right, .. } if self.field_menu(x, y, cx.now()) => {}
            Event::Press { x, y, button, clicks } => {
                self.field_mouse.cancel();
                if !self.nothing_over() || self.picture_overlay() {
                    return;
                }
                match self.hit(x, y) {
                    _ if button == Button::Right && self.conversation_menu(x, y) => {}
                    Some(Target::Tab(id) | Target::TabClose(id)) if button == Button::Middle && id != SESSIONS => {
                        self.ask_close_tab(id)
                    }
                    Some(Target::Tab(id) | Target::TabClose(id)) if button == Button::Right && id != SESSIONS => {
                        self.tab_menu(id, Rect::new(x, y, 0.0, 0.0));
                    }
                    Some(Target::Terminal) => self.term_press(x, y, button, clicks, cx.mods()),
                    Some(Target::Chat(crate::view::chat::Control::Editor)) if button == Button::Right => {
                        self.chat_editor_menu(x, y);
                    }
                    Some(Target::Entry(e) | Target::Pin(e) | Target::ArchivePick(e)) if button == Button::Right => {
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
            Event::Wheel { dy, .. } if self.nothing_over() && self.terminal_shown() && !self.usage.popup.open => {
                let (x, y) = self.mouse;
                if !(!self.picture_overlay() && self.chat_wheel(x, y, dy))
                    && !self.session_wheel(cx.size().0, x, y, dy)
                    && !self.wheel_other_pane(x, y, dy)
                    && self.term_area.contains(x, y)
                {
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
                } else if self.usage.popup.open {
                    self.usage.popup.scroll = (self.usage.popup.scroll - dy).max(0.0);
                } else if self.list_area.contains(x, y) {
                    self.sections.scroll = self.sections.clamp_scroll(self.sections.scroll - f64::from(dy));
                    self.moved(cx, x, y);
                } else if self.details_scroll.area.contains(x, y) {
                    self.details_scroll.wheel(dy);
                    self.moved(cx, x, y);
                }
            }
            Event::Key(k) => self.key(cx, k),
            Event::Text(t) => {
                if self.dialog.is_some() {
                    dialogs::text_input(self, &t);
                } else if self.find_focused() {
                    self.find_text(&t);
                } else if self.terminal_shown() && self.nothing_over() && !self.picture_overlay() {
                    if !self.chat_text(cx, &t) {
                        self.term_text(&t);
                    }
                } else if self.focus == Focus::Search && self.palette.is_none() && self.search.insert(&t) {
                    self.after_search();
                }
            }
            Event::Focus(false) => {
                self.field_mouse.cancel();
                self.folder_scroll_release();
                self.tabs.drag = None;
                self.chat_end_drag(None);
                self.usage.popup = Default::default();
                self.term_focus(false);
            }
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
                self.capture_window(cx);
                self.resume_polling(cx.now());
            }
            Event::Theme { .. } | Event::Contrast(_) => {
                self.retheme();
            }
            Event::Wake(now) => {
                if self.scroll_wake_due(now) {
                    cx.request_redraw_all();
                }
                if self.sync_usage_setting() {
                    cx.request_redraw_all();
                }
                self.attention_tick(cx);
                self.auto_handover_tick(cx);
                self.usage_warning_tick(now);
                if self.start_conversation_search(now) {
                    cx.request_redraw_all();
                }
                self.terminal_drop_due(cx, now);
                self.session_tick(cx, false);
                // A written handover held while the keyboard was in use.
                if self.open_finished_handover(now) {
                    cx.request_redraw_all();
                }
                if let Some(at) = self.typed_due(now) {
                    cx.wake_at(at);
                }
                if self.tip.tick(now) | self.toasts.tick(now) | self.field_tick(now) {
                    cx.request_redraw_all();
                }
                if self.usage_hover_due(now) {
                    cx.request_redraw_all();
                }
                if self.usage.repaint_at.take_if(|at| *at <= now).is_some() {
                    cx.request_redraw_all();
                }
                if self.config.show_usage && self.usage.cache_retry_at().is_some_and(|at| at <= now) {
                    self.take_usage_jobs(now);
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
                    // A new or forked tab still needs its verified transcript identity while minimized.
                    if cx.window().is_minimized() == Some(true)
                        && !self.tabs.list.iter().any(|tab| tab.pane.is_some() && tab.term.running())
                        && !self.pending_external_ownership()
                    {
                        self.next_poll = None;
                    } else {
                        self.poll_running();
                        if self.check_copy_deletes(now) {
                            cx.request_redraw_all();
                        }
                        self.next_poll = Some(now + POLL);
                    }
                }
                self.timers(cx);
                return;
            }
            Event::CloseRequested => self.request_window_close(cx),
            _ => {}
        }
        if self.exit_if_requested(cx) {
            return;
        }
        self.restore_tabs_if_always(cx.now());
        // A written handover waits for whatever modal thing this event may have closed.
        self.open_finished_handover(cx.now());
        self.attention_tick(cx);
        self.usage_warning_tick(cx.now());
        windowlook::apply(self, cx);
        cx.request_redraw_all();
        self.timers(cx);
    }

    fn raw(&mut self, cx: &mut Cx<'_, Msg>, event: &winit::event::WindowEvent) {
        // The host reduces only key presses; a held Space's peek ends on its release.
        if let winit::event::WindowEvent::KeyboardInput { event, .. } = event
            && event.state == winit::event::ElementState::Released
            && (event.physical_key == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Space)
                || event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space))
            && self.space_released()
        {
            self.refresh_pointer(cx);
            cx.request_redraw_all();
        }
        if matches!(event, winit::event::WindowEvent::Moved(_)) {
            self.capture_window(cx);
        }
        if let winit::event::WindowEvent::DroppedFile(file) = event {
            self.terminal_drop(cx, file.clone());
        }
    }

    fn message(&mut self, cx: &mut Cx<'_, Msg>, m: Msg) {
        match m {
            Msg::Woke => {
                let changed = self.take_jobs()
                    | self.take_usage_jobs(cx.now())
                    | self.take_desktop_job(cx.now())
                    | self.take_compact_jobs()
                    | self.take_background_handovers()
                    | self.take_opencode_job(cx.now());
                let changed = changed | self.open_finished_handover(cx.now());
                self.handover_attention(cx);
                let changed = changed | self.restore_tabs_if_always(cx.now());
                self.attention_tick(cx);
                self.auto_handover_tick(cx);
                self.usage_warning_tick(cx.now());
                if self.exit_if_requested(cx) {
                    return;
                }
                if changed {
                    cx.request_redraw_all();
                }
                self.timers(cx);
            }
            Msg::Term => self.term_woke(cx),
            Msg::Tray(event) => {
                self.sync_usage_setting();
                self.tray_event(cx, event);
                if !self.exit_if_requested(cx) {
                    self.timers(cx);
                }
            }
            Msg::SessionView => self.session_ready(cx),
            Msg::ThemeFile(result) => {
                self.theme_file(result);
                cx.request_redraw_all();
            }
            // `take` claims the request; false when the pipe thread already told the caller it
            // was not opened, so it is not acted on.
            Msg::Handover(_, _, ticket) if !ticket.take() => {}
            Msg::Handover(request, reply, _) => {
                let result = self.handover_requested(*request);
                if result.is_ok() {
                    crate::tray::show(cx.window());
                }
                let _ = reply.send(result);
                cx.request_redraw_all();
                self.timers(cx);
            }
            Msg::HandoverNotice(notice) => {
                match notice {
                    crate::compact_cli::Notice::Failed(text) => self.error("Could not open handover", text),
                    crate::compact_cli::Notice::Note(text) => self.note("Handover opened", text),
                }
                crate::tray::show(cx.window());
                cx.request_redraw_all();
                self.timers(cx);
            }
            Msg::ArchiveReportSaved(result) => {
                self.report_saved(result);
                cx.request_redraw_all();
            }
            Msg::FolderPicked(folder) => {
                self.new_session_folder_picked(&folder);
                cx.request_redraw_all();
            }
            Msg::SessionFolderPicked(id, folder) => {
                self.continue_in_folder(cx.now(), &id, &folder);
                cx.request_redraw_all();
            }
            Msg::BranchWritten(job, written) => {
                self.branch_written(cx.now(), *job, written);
                cx.request_redraw_all();
            }
            Msg::AttachPictures(id, files, chat) => {
                if chat {
                    self.chat_attach(id, &files);
                    cx.request_redraw_all();
                } else if self.paste_paths(id, &files) {
                    cx.request_redraw_all();
                }
            }
        }
    }

    fn draw(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, _damage: Rect) {
        crate::app_draw::draw(self, cx, p);
        // The frame just rebuilt the hit map. A tab or overlay can move under a stationary pointer.
        if self.refresh_pointer(cx) {
            cx.request_redraw_all();
        }
        self.timers(cx);
    }
}

impl AppState {
    fn exit_if_requested(&mut self, cx: &mut Cx<'_, Msg>) -> bool {
        if !self.quit {
            return false;
        }
        if self.opencode.restart_pending() {
            return true;
        }
        if self.cancel_compact_work() {
            return true;
        }
        if let Some(job) = &self.desktop_job {
            job.cancel();
            if job.running() {
                return true;
            }
        }
        if let Some(job) = &self.naming_job {
            job.cancel();
            if job.running() {
                return true;
            }
        }
        self.finish_exit(cx);
        true
    }

    fn capture_window(&mut self, cx: &Cx<'_, Msg>) {
        if !self.source.is_demo()
            && let Some(place) = winfile::capture(cx.window())
        {
            self.win_file.place = Some(place);
        }
    }

    fn finish_exit(&mut self, cx: &mut Cx<'_, Msg>) {
        if self.marks_flush_at.is_some() {
            self.save_marks();
        }
        self.capture_window(cx);
        if !self.restore.window_save_failed
            && let Err(error) = self.write_window()
        {
            self.restore.window_save_failed = true;
            self.quit = false;
            self.error("Could not save window settings", format!("{error} Close again to exit without saving them."));
            cx.request_redraw_all();
            return;
        }
        // The first window has gone if its channel can be claimed: this one is then the last to
        // close and its tabs are the ones to restore. Should the exit stop below, it serves the
        // channel like a first window.
        if self.restore.second_window
            && !self.source.is_demo()
            && let crate::handover_pipe::Claim::Mine(listener) = crate::handover_pipe::claim()
        {
            self.compact.listener = Some(listener);
            self.serve_handovers();
            self.inherit_tab_restore();
        }
        if !self.restore.save_failed
            && let Err(error) = self.save_tab_restore()
        {
            self.restore.save_failed = true;
            self.quit = false;
            self.error(
                "Could not save terminal tabs",
                format!("{error} Close again to leave the saved snapshot unchanged."),
            );
            cx.request_redraw_all();
            return;
        }
        cx.exit();
    }
}
