//! The terminal tabs' side of the window: a tab opened for a launch, the loop's wake that feeds
//! every terminal, keys and text for the program, the mouse (reports, selection, the wheel),
//! the focus, closing with the safeguard, and the frame of the tab shown.

use std::io;
use std::time::Instant;

use claude_switch_core::Launch;
use mdshared_term::{MouseButton, MouseEvent, MouseKind, Selection, SelectionKind, input};
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::Rect;
use mdshared_ui::host::{Button, Cx, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;
use mdshared_ui::termview::Scheme;

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
    /// The shown tab's cursor moved or changed: its blink starts again, on.
    pub cursor: bool,
}

/// The variable that makes Claude Code draw with its classic main-screen renderer. Per the docs
/// (code.claude.com/docs/en/fullscreen), `1` wins over the saved `tui` setting and over
/// `CLAUDE_CODE_NO_FLICKER`, for this process only.
pub(crate) const CLASSIC_RENDERER: &str = "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN";

/// Asks a Claude tab for the classic renderer: the Chat view refuses to send to the alternate
/// screen, where the CLI's fullscreen renderer draws. Other programs are left as they are.
/// Handover tabs name Claude Code by its absolute path (`…\claude.exe`), so the file stem counts.
pub(crate) fn classic_renderer(launch: &mut Launch, enabled: bool) {
    if enabled && term::is_claude_program(&launch.program) {
        launch.env.insert(CLASSIC_RENDERER.into(), "1".into());
    }
}

/// What the terminal tabs' screen understands, as the toolkit's PTY names it on other systems.
pub(crate) const TERM_NAME: (&str, &str) = ("TERM", "xterm-256color");

/// Names the terminal for a Claude tab. On Windows nothing sets `TERM` for a program started from
/// the Start menu, and Claude Code then takes the terminal for one without Unicode: it draws its
/// input mark as `>`, check boxes as `[ ]` and so on (Claude Code 2.1.332), not the screens Chat
/// was recorded against. With `TERM=xterm-256color` it draws them as in any modern terminal. A
/// launch that sets `TERM` itself keeps its own; other programs are left as they are.
pub(crate) fn terminal_name(launch: &mut Launch) {
    if term::is_claude_program(&launch.program) {
        launch.env.entry(TERM_NAME.0.into()).or_insert_with(|| TERM_NAME.1.into());
    }
}

fn button_of(b: Button) -> Option<MouseButton> {
    match b {
        Button::Left => Some(MouseButton::Left),
        Button::Middle => Some(MouseButton::Middle),
        Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MouseClipboard {
    Copy(String),
    Paste,
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
            && !self.usage.popup.takes_keyboard()
    }

    /// Whether the terminal shown has the keyboard: the window has it and nothing is over it.
    pub fn terminal_focused(&self) -> bool {
        self.window_focused && self.nothing_over() && self.terminal_shown() && !self.chat_takes_input()
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
        let mut pane = self.pane_for_launch(l, session, account);
        let signing_in = claude_switch_core::launch::is_login(l);
        let mut l = l.clone();
        classic_renderer(&mut l, self.config.classic_terminal_view);
        terminal_name(&mut l);
        if let Some(pane) = &mut pane {
            pane.chat.classic_renderer = l.env.contains_key(CLASSIC_RENDERER);
            if !self.source.is_demo() {
                pane.chat.set_starting_effort(crate::view::chat::effort::starting(&l, &self.source.home()));
            }
        }
        l.program = crate::run::native_program(&l.program)?;
        let managed = pane.is_some() && !signing_in && self.auto_handover_env(&mut l);
        let id = self.open_command(&term::command(&l), l.label.clone(), session, account)?;
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.auto.managed = managed;
            tab.cwd = pane.as_ref().and(l.cwd.clone());
            tab.pane = pane;
            tab.signing_in = signing_in;
        }
        self.see_shown();
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
            self.poll_workers();
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
        self.poll_workers();
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
        let handovers = self.handover_close_note();
        if n == 0 && handovers.is_none() {
            return true;
        }
        // The confirmation replaces the dialog: an open review waits in the strip if you stay.
        self.park_open_review(std::time::Instant::now());
        let mut text = match n {
            0 => String::new(),
            1 => "A terminal tab is still running. Its program ends.".to_string(),
            n => format!("{n} terminal tabs are still running. Their programs end."),
        };
        if let Some(note) = handovers {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&note);
        }
        self.dialog =
            Some(Dialog::Confirm { title: "Close the window?".into(), text, ok: "Close".into(), then: Pending::Quit });
        false
    }

    /// Feeds every terminal what its program wrote, answers its queries, notes titles and
    /// ends. What the tab shown needs redrawn.
    pub fn pump_terms(&mut self) -> Woken {
        let mut w = Woken::default();
        let mut sign_ins = Vec::new();
        let shown = self.tabs.active;
        let focused = self.window_focused && !self.notifications.hidden;
        let (area, metrics) = (self.term_area, self.metrics);
        for tab in &mut self.tabs.list {
            let had_selection = tab.term.selection.is_some();
            let r = tab.term.pump();
            if r.ended
                && tab.signing_in
                && let Some(slug) = &tab.account
            {
                sign_ins.push(slug.clone());
            }
            if let Some(pane) = &mut tab.pane {
                let bell = tab.attention.terminal(&mut tab.term.screen, r.output, focused && shown == Some(tab.id));
                pane.chat.output(Instant::now(), r.output, bell);
            }
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
            w.cursor |= r.cursor_moved;
            if tab.term.scroll > 0 {
                // Scrollback growth can move the overlaid thumb outside the dirty grid rows.
                w.all = true;
                continue;
            }
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
        if !sign_ins.is_empty() {
            let now = Instant::now();
            self.refresh(now);
            for slug in sign_ins {
                self.request_usage(now, claude_switch_core::usage::Demand::SignIn, Some(&slug));
            }
            w.all = true;
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
        let keyboard = termkeys::keyboard_now(tab.term.keyboard, &tab.term.screen);
        let action = termkeys::key(k.name.as_ref(), k.mods, k.text.as_deref(), &modes, selected, keyboard);
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
                self.note_chat_terminal_input();
                let demo_echo = self.source.is_demo() && self.usage.demo_echo;
                if let Some(tab) = self.tabs.shown_mut() {
                    tab.term.scroll = 0;
                    tab.term.selection = None;
                    if demo_echo && tab.term.pty.is_none() {
                        #[cfg(test)]
                        self.usage.demo_keys.extend_from_slice(&bytes);
                        tab.term.feed(if k.name == Some(KeyName::Enter) { b"\r\n" } else { &bytes });
                    } else {
                        tab.term.write(&bytes);
                    }
                }
            }
            _ => {}
        }
    }

    /// Text an input method committed.
    pub fn term_text(&mut self, text: &str) {
        self.flush_drop_before_input();
        self.note_chat_terminal_input();
        let demo_echo = self.source.is_demo() && self.usage.demo_echo;
        if let Some(tab) = self.tabs.shown_mut() {
            tab.term.scroll = 0;
            if demo_echo && tab.term.pty.is_none() {
                tab.term.feed(text.as_bytes());
            } else {
                tab.term.write(text.as_bytes());
            }
        }
    }

    fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        if let Some(text) = tab.term.take_selected_text(false).filter(|t| !t.is_empty()) {
            platform::set_clipboard_text(&text);
        }
    }

    pub fn paste(&mut self) {
        if !self.chat_paste() {
            self.paste_terminal(platform::clipboard_text);
        }
    }

    fn paste_terminal(&mut self, clipboard_text: impl FnOnce() -> Option<String>) {
        if self.tabs.shown().is_none_or(|tab| tab.term.ended.is_some()) {
            return;
        }
        self.flush_drop_before_input();
        self.note_chat_terminal_input();
        let text = clipboard_text();
        if let Some(tab) = self.tabs.shown_mut() {
            if !tab.term.screen.modes().bracketed_paste && text.as_ref().is_some_and(|s| s.contains(['\r', '\n'])) {
                self.warn(
                    "Paste not sent",
                    "Use the Chat composer for multiline input when bracketed paste is off.".into(),
                );
                return;
            }
            let bytes = crate::view::attach::clipboard(text.as_deref(), &tab.term.screen.modes());
            tab.term.scroll = 0;
            tab.term.write(&bytes);
        }
    }

    /// A press on the terminal shown, including clipboard actions targeted at the drawer itself.
    pub fn term_press(&mut self, x: f32, y: f32, button: Button, clicks: u32, mods: Mods) {
        match self.term_pointer_press(x, y, button, clicks, mods) {
            Some(MouseClipboard::Copy(text)) => {
                platform::set_clipboard_text(&text);
            }
            Some(MouseClipboard::Paste) => self.paste_terminal(platform::clipboard_text),
            None => {}
        }
    }

    fn term_pointer_press(
        &mut self,
        x: f32,
        y: f32,
        button: Button,
        clicks: u32,
        mods: Mods,
    ) -> Option<MouseClipboard> {
        let (area, m) = (self.term_area, self.metrics?);
        let tab = self.tabs.shown_mut()?;
        let t = &mut tab.term;
        let modes = t.screen.modes();
        let b = button_of(button)?;
        // Windows Terminal passes mouse input to a tracking program before trying local
        // selection or clipboard actions. Shift suppresses those reports.
        if t.ended.is_none() && termkeys::mouse_to_program(&modes, mods.shift) {
            if let Some((row, col)) = m.cell_at(area, x, y) {
                let ev =
                    MouseEvent { kind: MouseKind::Press, button: Some(b), col, row, mods: termkeys::term_mods(mods) };
                if let Some(bytes) = input::encode_mouse(ev, &modes) {
                    if let Some(pane) = &mut tab.pane {
                        pane.note_terminal_input();
                    }
                    t.write(&bytes);
                }
                t.reported = Some(b);
                t.last_cell = Some((row, col));
            }
            return None;
        }
        match b {
            MouseButton::Left => {
                let at = t.view(true).point_at(&t.screen, &m, area, x, y);
                let kind = if mods.alt { SelectionKind::Block } else { SelectionKind::Stream };
                t.selection = Some(Selection::start(&t.screen, at, kind, clicks.clamp(1, 3) as u8));
                t.selecting = true;
                None
            }
            MouseButton::Right => match t.take_selected_text(mods.shift) {
                Some(text) if !text.is_empty() => Some(MouseClipboard::Copy(text)),
                Some(_) => None,
                None => t.ended.is_none().then_some(MouseClipboard::Paste),
            },
            _ => None,
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
        if t.ended.is_some() || !termkeys::mouse_to_program(&modes, mods.shift) {
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
                // A drag belongs to its press, which counted as input; a hover is no input at
                // all: it must not stop a send or a card's keys halfway.
                if t.reported.is_some()
                    && let Some(pane) = &mut tab.pane
                {
                    pane.note_terminal_input();
                }
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
                if let Some(pane) = &mut tab.pane {
                    pane.note_terminal_input();
                }
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
        if t.ended.is_some() {
            t.scroll_by(lines);
            return;
        }
        let modes = t.screen.modes();
        match termkeys::wheel(&modes, mods.shift) {
            Wheel::Scroll => t.scroll_by(lines),
            Wheel::Arrows => {
                if let Some(pane) = &mut tab.pane {
                    pane.note_terminal_input();
                }
                let key = if lines > 0 { mdshared_term::Key::Up } else { mdshared_term::Key::Down };
                let one = input::encode_key(key, mdshared_term::Mods::NONE, &modes);
                t.write(&one.repeat(lines.unsigned_abs() as usize));
            }
            Wheel::Report => {
                let Some(m) = metrics else { return };
                // The program scrolls; card keys go on, each checking the screen first.
                if let Some(pane) = &mut tab.pane {
                    pane.note_terminal_wheel();
                }
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
            let (cols, rows) = if tab.pane.as_ref().is_some_and(|p| p.chat.enabled) {
                if self.tabs.active != Some(tab.id) || shown.h <= 0.0 {
                    continue;
                }
                grid(shown)
            } else if open && tab.pane.is_some() {
                pane_grid
            } else {
                full_grid
            };
            tab.term.resize(cols, rows);
        }
        self.grid = (full_grid.0 as u16, full_grid.1 as u16);
        self.term_area = inner(shown);
        self.metrics = Some(metrics);
    }

    /// A terminal's program wrote or ended: feeds them all and redraws what changed.
    pub fn term_woke(&mut self, cx: &mut Cx<'_, Msg>) {
        let w = self.pump_terms();
        self.attention_tick(cx);
        if w.output {
            self.session_tick(cx, true);
        }
        // Only a cursor that changed starts its blink again. A program that repaints around a
        // parked cursor (Claude Code's counter while it works) would otherwise cut every off
        // phase short, about once a second.
        if w.cursor {
            Self::cursor_moved(cx);
        }
        if w.all || self.metrics.is_none() {
            cx.request_redraw(self.term_area.inset(-PAD));
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
            let account = self.current().slug.clone();
            let demo = self.source.is_demo();
            let cwd = if demo { std::env::temp_dir() } else { self.source.home() };
            let mut launch = claude_switch_core::launch::shell(&cwd);
            launch.program = program.clone().into();
            launch.label = program.clone();
            if demo
                && launch
                    .program
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
            {
                launch.args.push("/d".into()); // No user-defined cmd AutoRun commands in the demo.
            }
            match self.open_tab(&launch, None, Some(&account)) {
                Ok(tab) => self.debug_tab = Some(tab),
                Err(e) => self.error("Could not start it", format!("{program}: {e}")),
            }
            if self.debug_list {
                self.show_sessions();
            }
        }
        if let Some(at) = self.typed_due(cx.now()) {
            cx.wake_at(at);
        }
    }

    /// Restarts the cursor's blink, after a key or output that moved or changed the cursor.
    pub fn cursor_moved(cx: &mut Cx<'_, Msg>) {
        cx.anim().forget(CURSOR);
    }

    /// The text of the `--type` flag, sent only to the tab opened by `--shell`.
    pub fn typed_due(&mut self, now: Instant) -> Option<Instant> {
        while let Some((at, _)) = self.debug_type.first() {
            if *at > now {
                return Some(*at);
            }
            let (_, bytes) = self.debug_type.remove(0);
            if let Some(tab) = self.debug_input_tab().and_then(|id| self.tabs.list.iter().find(|tab| tab.id == id)) {
                tab.term.write(&bytes);
            }
        }
        None
    }

    fn debug_input_tab(&self) -> Option<u64> {
        self.debug_tab.filter(|id| self.tabs.list.iter().any(|tab| tab.id == *id))
    }

    /// The tab a session is open in, as the safeguard sees it.
    pub fn tab_of_session(&self, session: &str) -> Option<u64> {
        self.tabs
            .by_session(session, |t| t.term.running() || self.source.is_demo() && t.term.ended.is_none())
            .map(|t| t.id)
    }

    /// The account of the tab a session is open in.
    pub fn tab_account(&self, session: &str) -> Option<String> {
        let tab = self.tab_of_session(session)?;
        self.tabs
            .list
            .iter()
            .find(|t| t.id == tab)
            .map(|t| t.account.clone().unwrap_or_else(|| self.current().slug.clone()))
    }

    /// Shows the list again (from the top bar's controls that work on it).
    pub fn show_sessions(&mut self) {
        self.show_tab(SESSIONS);
    }
}

mod paint;

#[cfg(test)]
mod tests;
