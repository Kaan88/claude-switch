//! The terminal tabs' side of the window: a tab opened for a launch, the loop's wake that feeds
//! every terminal, keys and text for the program, the mouse (reports, selection, the wheel),
//! the focus, closing with the safeguard, and the frame of the tab shown.

use std::io;
use std::time::Instant;

use claude_switch_core::Launch;
use mdshared_term::{MouseButton, MouseEvent, MouseKind, Selection, SelectionKind, input};
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::{Button, Cx, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;
use mdshared_ui::scroll::draw_scrollbar;
use mdshared_ui::termview::{Scheme, draw_screen_animated};

use crate::dialogs::{Dialog, Pending};
use crate::state::{AppState, Msg, Target};
use crate::tabs::{NewTab, SESSIONS};
use crate::term::{self, Term};
use crate::termkeys::{self, KeyAction, Wheel};

/// Space between the terminal's grid and the edges of its area.
pub const PAD: f32 = 6.0;
/// The animator's name for the cursor's blink.
const CURSOR: &str = "term-cursor";

/// What a wake found for the window to redraw.
#[derive(Default)]
pub struct Woken {
    /// Rectangles of the tab shown that changed; `all` when the whole terminal did.
    pub rects: Vec<Rect>,
    pub all: bool,
    pub strip: bool,
    pub output: bool,
}

fn button_of(b: Button) -> Option<MouseButton> {
    match b {
        Button::Left => Some(MouseButton::Left),
        Button::Middle => Some(MouseButton::Middle),
        Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

impl AppState {
    pub fn terminal_shown(&self) -> bool {
        self.tabs.shown().is_some()
    }

    /// No popup, menu, palette or dialog is over the window.
    pub fn nothing_over(&self) -> bool {
        self.dialog.is_none()
            && self.palette.is_none()
            && self.menu.is_none()
            && !self.accounts_dd.is_open()
            && !self.usage.popup.open
    }

    /// Whether the terminal shown has the keyboard: the window has it and nothing is over it.
    pub fn terminal_focused(&self) -> bool {
        self.window_focused && self.nothing_over() && self.terminal_shown()
    }

    fn term_wake(&self) -> impl Fn() + Send + Sync + 'static {
        let proxy = self.proxy.clone();
        move || {
            if let Some(p) = &proxy
                && let Ok(p) = p.lock()
            {
                p.send(Msg::Term);
            }
        }
    }

    /// Opens a tab that runs `l` and shows it.
    pub fn open_tab(&mut self, l: &Launch, session: Option<&str>, account: Option<&str>) -> io::Result<u64> {
        let pane = self.pane_for_launch(l, session, account);
        let id = self.open_command(&term::command(l), l.label.clone(), session, account)?;
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.pane = pane;
        }
        Ok(id)
    }

    /// Opens a tab that runs `command` and shows it.
    pub fn open_command(
        &mut self,
        command: &mdshared_pty::Command,
        label: String,
        session: Option<&str>,
        account: Option<&str>,
    ) -> io::Result<u64> {
        let t = Term::spawn(command, self.grid, self.term_wake())?;
        let color = account.map_or(self.pal().muted, |a| self.account_color(a));
        let new = NewTab { label, color, session: session.map(str::to_string), account: account.map(str::to_string) };
        let before = self.tabs.active;
        let id = self.tabs.open(new, t);
        self.report_switch(before);
        self.see_shown();
        Ok(id)
    }

    /// Tells the programs of the tab left and the tab shown that the keyboard moved, when they
    /// asked to be told.
    fn report_switch(&mut self, before: Option<u64>) {
        let now = self.tabs.active;
        if before == now {
            return;
        }
        self.clear_session_text_focus();
        if let Some(tab) = before.and_then(|id| self.tabs.get_mut(id)) {
            tab.term.stop_pointer();
        }
        if !self.window_focused {
            return;
        }
        for (id, on) in [(before, false), (now, true)] {
            if let Some(tab) = id.and_then(|id| self.tabs.get(id))
                && let Some(b) = input::encode_focus(on, &tab.term.screen.modes())
            {
                tab.term.write(&b);
            }
        }
    }

    /// Shows a tab ([`SESSIONS`] for the list).
    pub fn show_tab(&mut self, id: u64) {
        let before = self.tabs.active;
        if self.tabs.activate(id) {
            self.report_switch(before);
            self.see_shown();
            if !self.terminal_shown() {
                self.rebuild_view();
            }
        }
    }

    pub fn step_tab(&mut self, forward: bool) {
        let before = self.tabs.active;
        self.tabs.step(forward);
        self.report_switch(before);
        self.see_shown();
        if !self.terminal_shown() {
            self.rebuild_view();
        }
    }

    /// Closes a tab, asking first while its program runs.
    pub fn ask_close_tab(&mut self, id: u64) {
        let Some(tab) = self.tabs.get(id) else { return };
        if tab.term.running() {
            self.dialog = Some(Dialog::Confirm {
                title: "Close this tab?".into(),
                text: "The session ends.".into(),
                ok: "Close".into(),
                then: Pending::CloseTab(id),
            });
        } else {
            self.close_tab_now(id);
        }
    }

    /// Closes a tab; dropping its terminal ends the program.
    pub fn close_tab_now(&mut self, id: u64) {
        let before = self.tabs.active;
        if before == Some(id) {
            self.tabs.close(id);
            let now = self.tabs.active;
            if let Some(tab) = now.and_then(|n| self.tabs.get(n))
                && self.window_focused
                && let Some(b) = input::encode_focus(true, &tab.term.screen.modes())
            {
                tab.term.write(&b);
            }
        } else {
            self.tabs.close(id);
        }
        self.see_shown();
        self.rebuild_view();
    }

    /// The window is asked to close: true when it can at once; otherwise the question is open.
    pub fn ask_quit(&mut self) -> bool {
        let n = self.tabs.running();
        if n == 0 {
            return true;
        }
        let text = if n == 1 {
            "A terminal tab is still running. Its program ends.".to_string()
        } else {
            format!("{n} terminal tabs are still running. Their programs end.")
        };
        self.dialog =
            Some(Dialog::Confirm { title: "Close the window?".into(), text, ok: "Close".into(), then: Pending::Quit });
        false
    }

    /// Feeds every terminal what its program wrote, answers its queries, notes titles and
    /// ends. What the tab shown needs redrawn.
    pub fn pump_terms(&mut self) -> Woken {
        let mut w = Woken::default();
        let shown = self.tabs.active;
        let (area, metrics) = (self.term_area, self.metrics);
        for tab in &mut self.tabs.list {
            let had_selection = tab.term.selection.is_some();
            let r = tab.term.pump();
            w.strip |= r.ended;
            if let Some(title) = r.title {
                w.strip |= title != tab.program_title;
                tab.program_title = title;
            }
            let dirty = tab.term.screen.take_dirty();
            if shown != Some(tab.id) || !r.output {
                continue;
            }
            w.output = true;
            match metrics {
                Some(m) if had_selection == tab.term.selection.is_some() => {
                    let view = tab.term.view(true);
                    for rows in view.damage(&tab.term.screen, &dirty) {
                        for row in rows {
                            w.rects.push(m.row_rect(area, row));
                        }
                    }
                }
                _ => w.all = true,
            }
        }
        w
    }

    /// A key press while a terminal tab is shown and nothing is over it.
    pub fn term_key(&mut self, now: Instant, k: &KeyPress) {
        self.flush_drop_before_input();
        let Some(tab) = self.tabs.shown() else { return };
        let id = tab.id;
        let ended = tab.term.ended.is_some();
        if ended && k.name == Some(KeyName::Enter) {
            self.close_tab_now(id);
            return;
        }
        let modes = tab.term.screen.modes();
        let selected = tab.term.selected_text().is_some();
        let action = termkeys::key(k.name.as_ref(), k.mods, k.text.as_deref(), &modes, selected);
        match action {
            KeyAction::Command(c) => self.command(now, c, None),
            KeyAction::Copy => self.copy_selection(),
            KeyAction::Paste if !ended => self.paste(),
            KeyAction::Page(n) => {
                if let Some(tab) = self.tabs.shown_mut() {
                    let rows = tab.term.screen.rows() as i32;
                    tab.term.scroll_by(n * (rows - 1).max(1));
                }
            }
            KeyAction::Send(bytes) if !ended => {
                if let Some(tab) = self.tabs.shown_mut() {
                    tab.term.scroll = 0;
                    tab.term.selection = None;
                    tab.term.write(&bytes);
                }
            }
            _ => {}
        }
    }

    /// Text an input method committed.
    pub fn term_text(&mut self, text: &str) {
        self.flush_drop_before_input();
        if let Some(tab) = self.tabs.shown_mut() {
            tab.term.scroll = 0;
            tab.term.write(text.as_bytes());
        }
    }

    pub fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        if let Some(text) = tab.term.selected_text() {
            platform::set_clipboard_text(&text);
        }
        tab.term.selection = None;
    }

    pub fn paste(&mut self) {
        self.flush_drop_before_input();
        let text = platform::clipboard_text();
        if let Some(tab) = self.tabs.shown_mut() {
            let bytes = crate::view::attach::clipboard(text.as_deref(), &tab.term.screen.modes());
            tab.term.scroll = 0;
            tab.term.write(&bytes);
        }
    }

    /// A press on the terminal shown: a report to the program, or the start of a selection.
    pub fn term_press(&mut self, x: f32, y: f32, button: Button, clicks: u32, mods: Mods) {
        let (area, Some(m)) = (self.term_area, self.metrics) else { return };
        let Some(tab) = self.tabs.shown_mut() else { return };
        let t = &mut tab.term;
        let modes = t.screen.modes();
        let Some(b) = button_of(button) else { return };
        if termkeys::mouse_to_program(&modes, mods.shift) {
            if let Some((row, col)) = m.cell_at(area, x, y) {
                let ev =
                    MouseEvent { kind: MouseKind::Press, button: Some(b), col, row, mods: termkeys::term_mods(mods) };
                if let Some(bytes) = input::encode_mouse(ev, &modes) {
                    t.write(&bytes);
                }
                t.reported = Some(b);
                t.last_cell = Some((row, col));
            }
            return;
        }
        if b == MouseButton::Left {
            let at = t.view(true).point_at(&t.screen, &m, area, x, y);
            let kind = if mods.alt { SelectionKind::Block } else { SelectionKind::Stream };
            t.selection = Some(Selection::start(&t.screen, at, kind, clicks.clamp(1, 3) as u8));
            t.selecting = true;
        }
    }

    /// The pointer moved over (or, dragging, past) the terminal shown. True when it shows.
    pub fn term_move(&mut self, x: f32, y: f32, mods: Mods) -> bool {
        let (area, Some(m)) = (self.term_area, self.metrics) else { return false };
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let t = &mut tab.term;
        if t.selecting {
            let at = t.view(true).point_at(&t.screen, &m, area, x, y);
            return t.extend_selection(at);
        }
        let modes = t.screen.modes();
        if !termkeys::mouse_to_program(&modes, mods.shift) {
            return false;
        }
        let cell = match t.reported {
            Some(_) => Some(m.cell_clamped(area, x, y, t.screen.cols(), t.screen.rows())),
            None => m.cell_at(area, x, y),
        };
        if let Some((row, col)) = cell
            && t.last_cell != Some((row, col))
        {
            t.last_cell = Some((row, col));
            let ev =
                MouseEvent { kind: MouseKind::Move, button: t.reported, col, row, mods: termkeys::term_mods(mods) };
            if let Some(bytes) = input::encode_mouse(ev, &modes) {
                t.write(&bytes);
            }
        }
        false
    }

    /// A release after a press on the terminal shown. True when it was the terminal's.
    pub fn term_release(&mut self, x: f32, y: f32, button: Button, mods: Mods) -> bool {
        let (area, Some(m)) = (self.term_area, self.metrics) else { return false };
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let t = &mut tab.term;
        let b = button_of(button);
        if t.reported.is_some() && t.reported == b {
            let modes = t.screen.modes();
            let (row, col) = m.cell_clamped(area, x, y, t.screen.cols(), t.screen.rows());
            let ev = MouseEvent { kind: MouseKind::Release, button: b, col, row, mods: termkeys::term_mods(mods) };
            if let Some(bytes) = input::encode_mouse(ev, &modes) {
                t.write(&bytes);
            }
            t.reported = None;
            return true;
        }
        if t.selecting && b == Some(MouseButton::Left) {
            t.selecting = false;
            if t.selection.is_some_and(|s| s.is_empty()) {
                t.selection = None;
            }
            return true;
        }
        false
    }

    /// The wheel over the terminal shown.
    pub fn term_wheel(&mut self, dy: f32, mods: Mods) {
        let (area, metrics) = (self.term_area, self.metrics);
        let (x, y) = self.mouse;
        let Some(tab) = self.tabs.shown_mut() else { return };
        let t = &mut tab.term;
        let lines = termkeys::wheel_lines(dy, &mut t.wheel_rest);
        if lines == 0 {
            return;
        }
        let modes = t.screen.modes();
        match termkeys::wheel(&modes, mods.shift) {
            Wheel::Scroll => t.scroll_by(lines),
            Wheel::Arrows => {
                let key = if lines > 0 { mdshared_term::Key::Up } else { mdshared_term::Key::Down };
                let one = input::encode_key(key, mdshared_term::Mods::NONE, &modes);
                t.write(&one.repeat(lines.unsigned_abs() as usize));
            }
            Wheel::Report => {
                let Some(m) = metrics else { return };
                let (row, col) = m.cell_clamped(area, x, y, t.screen.cols(), t.screen.rows());
                let button = if lines > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
                let notches = ((lines.unsigned_abs() as f32) / termkeys::WHEEL_LINES).round().max(1.0) as usize;
                let ev = MouseEvent {
                    kind: MouseKind::Press,
                    button: Some(button),
                    col,
                    row,
                    mods: termkeys::term_mods(mods),
                };
                if let Some(bytes) = input::encode_mouse(ev, &modes) {
                    t.write(&bytes.repeat(notches));
                }
            }
        }
    }

    /// The window got or lost the keyboard.
    pub fn term_focus(&mut self, on: bool) {
        self.window_focused = on;
        if let Some(tab) = self.tabs.shown()
            && let Some(b) = input::encode_focus(on, &tab.term.screen.modes())
        {
            tab.term.write(&b);
        }
    }

    pub fn terminal_scheme(&self) -> Scheme {
        self.term_colours.0
    }

    /// Fits each tab to its own area, so switching to a shell doesn't reflow a hidden session.
    pub fn layout_terminals(&mut self, shown: Rect, full: Rect, scale: f32) {
        let inner = |area: Rect| {
            Rect::new(area.x + PAD, area.y + PAD, (area.w - 2.0 * PAD).max(0.0), (area.h - 2.0 * PAD).max(0.0))
        };
        let metrics = self.cell_font.metrics(f32::from(self.config.terminal_font_size), scale);
        let grid = |area| {
            let (cols, rows) = metrics.grid_size(inner(area));
            (cols.clamp(1, u16::MAX as usize), rows.clamp(1, u16::MAX as usize))
        };
        let open = self.session_width_enabled(full.w);
        let split = self.session_view.splitter.rects(full, mdshared_ui::splitter::Axis::Horizontal).0;
        let (full_grid, pane_grid) = (grid(full), grid(split));
        for tab in &mut self.tabs.list {
            let (cols, rows) = if open && tab.pane.is_some() { pane_grid } else { full_grid };
            tab.term.resize(cols, rows);
        }
        self.grid = (full_grid.0 as u16, full_grid.1 as u16);
        self.term_area = inner(shown);
        self.metrics = Some(metrics);
    }

    /// Draws the tab shown into the whole client area under the strip.
    pub fn draw_terminal(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect) {
        let pal = *self.pal();
        let scheme = self.terminal_scheme();
        p.fill(area, scheme.background);
        let (inner, Some(metrics)) = (self.term_area, self.metrics) else { return };
        let focused = self.terminal_focused();
        let Some(id) = self.tabs.active else { return };
        let Some(tab) = self.tabs.list.iter().find(|t| t.id == id) else { return };
        let t = &tab.term;
        let view = t.view(focused);
        let sel = t.selection.filter(|s| !s.is_empty());
        let blink = self.config.appearance.cursor_blink.then_some(self.blink).flatten();
        draw_screen_animated(
            p,
            &mut self.cell_font,
            &scheme,
            &metrics,
            inner,
            &t.screen,
            sel.as_ref(),
            view,
            cx.anim(),
            CURSOR,
            blink,
        );
        if blink.is_some()
            && cx.anim().animates()
            && focused
            && t.screen.cursor().visible
            && t.screen.cursor().blinking
            && let Some(row) = view.cursor_row(&t.screen)
        {
            cx.animating(metrics.row_rect(inner, row));
        }
        let back = view.scroll_in(&t.screen);
        if back > 0 {
            let total = (t.screen.scrollback_len() + t.screen.rows()) as f32;
            let track = Rect::new(area.right() - 8.0, inner.y, 6.0, inner.h);
            let h = (inner.h * t.screen.rows() as f32 / total).max(16.0);
            let top = (t.screen.scrollback_len() - back) as f32 / total * inner.h;
            let thumb = Rect::new(track.x, inner.y + top.min(inner.h - h), 6.0, h);
            draw_scrollbar(p, &pal, track, thumb, false);
        }
        self.hits.push((area, Target::Terminal));
    }

    /// A terminal's program wrote or ended: feeds them all and redraws what changed.
    pub fn term_woke(&mut self, cx: &mut Cx<'_, Msg>) {
        let w = self.pump_terms();
        if w.output {
            self.session_tick(cx, true);
        }
        if w.all || !w.rects.is_empty() {
            Self::cursor_moved(cx);
        }
        if w.all || self.metrics.is_none() {
            cx.request_redraw(self.term_area);
        } else {
            for r in w.rects {
                cx.request_redraw(r);
            }
        }
        if w.strip {
            cx.request_redraw(crate::tabs::strip_rect(TITLE_H, cx.size().0));
            if !self.terminal_shown() {
                self.rebuild_view();
                cx.request_redraw(self.list_area);
            }
        }
    }

    /// `--shell`, `--type` and `--sessions-tab` (see `main.rs`): a tab opened at the start.
    pub fn start_debug(&mut self, cx: &mut Cx<'_, Msg>) {
        if let Some(program) = self.debug_shell.take() {
            let command = mdshared_pty::Command::new(&program);
            let account = self.current().slug.clone();
            if let Err(e) = self.open_command(&command, program.clone(), None, Some(&account)) {
                self.error("Could not start it", format!("{program}: {e}"));
            }
            if self.debug_list {
                self.show_sessions();
            }
        }
        if let Some(at) = self.typed_due(cx.now()) {
            cx.wake_at(at);
        }
    }

    /// Restarts the cursor's blink, after a key or output.
    pub fn cursor_moved(cx: &mut Cx<'_, Msg>) {
        cx.anim().forget(CURSOR);
    }

    /// The text of the `--type` flag, sent to the first tab at its time.
    pub fn typed_due(&mut self, now: Instant) -> Option<Instant> {
        while let Some((at, _)) = self.debug_type.first() {
            if *at > now {
                return Some(*at);
            }
            let (_, bytes) = self.debug_type.remove(0);
            if let Some(tab) = self.tabs.list.first() {
                tab.term.write(&bytes);
            }
        }
        None
    }

    /// The tab a session is open in, as the safeguard sees it.
    pub fn tab_of_session(&self, session: &str) -> Option<u64> {
        self.tabs.by_session(session).map(|t| t.id)
    }

    /// The account of the tab a session is open in.
    pub fn tab_account(&self, session: &str) -> Option<String> {
        self.tabs.by_session(session).map(|t| t.account.clone().unwrap_or_else(|| self.current().slug.clone()))
    }

    /// Shows the list again (from the top bar's controls that work on it).
    pub fn show_sessions(&mut self) {
        self.show_tab(SESSIONS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ended_tabs_still_handle_local_scrollback() {
        let mut app = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
        let mut term = Term::empty((10, 3));
        term.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        term.ended = Some(mdshared_pty::Exit { code: Some(0), signal: None });
        app.tabs.open(NewTab { label: "ended".into(), color: [0; 3], account: None, session: None }, term);
        let key = KeyPress {
            name: Some(KeyName::PageUp),
            mods: Mods { shift: true, ..Mods::default() },
            text: None,
            repeat: false,
        };
        app.term_key(Instant::now(), &key);
        assert!(app.tabs.shown().unwrap().term.scroll > 0);
    }
}
