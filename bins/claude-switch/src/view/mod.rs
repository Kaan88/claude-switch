//! Session view integration with the window, terminal and splitter.

pub mod attach;
pub mod chat;
mod discovery;
mod links;
pub(crate) mod model;
mod pictures;
mod preview;
mod text;
mod ui;

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use claude_switch_core::Launch;
use mdshared_ui::cursor::Cursor;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::splitter::{self, Axis, Hold, Splitter};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Msg};
use crate::winfile::WindowFile;

const AXIS: Axis = Axis::Horizontal;
const POLL: Duration = Duration::from_millis(500);

fn enabled(preference: Option<bool>, width: f32) -> bool {
    preference.unwrap_or(width >= 1100.0)
}

pub struct Window {
    pub splitter: Splitter,
    area: Rect,
    pane_area: Rect,
    split_hover: bool,
    text_focus: bool,
    pub drops: attach::DropBatch,
}

impl Window {
    pub fn new(prefs: &WindowFile) -> Self {
        let mut splitter = Splitter::new(Hold::B(440.0)).min(270.0, 330.0);
        if !prefs.session_split.is_empty() {
            splitter.apply_setting(prefs.session_split.split(';').next().unwrap_or_default());
        }
        Self {
            splitter,
            area: Rect::default(),
            pane_area: Rect::default(),
            split_hover: false,
            text_focus: false,
            drops: Default::default(),
        }
    }
}

pub struct Pane {
    model: model::Model,
    pub chat: chat::Chat,
    pub(crate) transcript_source: model::Source,
    ui: ui::Ui,
    cwd: PathBuf,
    next: Option<Instant>,
    _demo: Option<crate::demo::Transcript>,
}

impl Pane {
    pub(crate) fn note_terminal_input(&mut self) {
        let end = self.transcript_source.find_file().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());
        self.chat.invalidate_input(&self.model.chat, end);
    }
    pub fn restore_chat_layout(&mut self, chat: bool) {
        self.chat.enabled = chat;
        self.ui.set_chat(chat);
    }

    pub(crate) fn verify_identity(&self, id: &str) -> bool {
        match &self.transcript_source {
            model::Source::File(path) => {
                path.file_stem().and_then(|s| s.to_str()).is_some_and(|stem| stem.eq_ignore_ascii_case(id))
            }
            model::Source::Discover { verified, .. } => {
                if let Some(previous) = verified.get() {
                    return previous == id && self.transcript_source.find_file().is_some();
                }
                self.transcript_source.verify_candidate(id) && verified.set(id.to_owned()).is_ok()
            }
        }
    }

    pub(crate) fn identity_unverified(&mut self, changed: bool) {
        self.chat.identity_unverified = true;
        if changed {
            self.model.error =
                Some("Session changed in the terminal. Reopen the session to attach its transcript.".into());
        }
    }
}

impl AppState {
    pub fn session_visible(&self, width: f32) -> bool {
        self.tabs.shown().is_some_and(|tab| tab.pane.is_some())
            && (self.chat_active() || self.session_width_enabled(width))
    }

    pub fn session_width_enabled(&self, width: f32) -> bool {
        enabled(self.win_file.session_view, width)
    }

    pub fn toggle_session_view(&mut self) {
        if !self.chat_active() && self.tabs.shown().is_some_and(|tab| tab.pane.is_some()) {
            self.win_file.session_view = Some(!self.session_visible(self.session_view.area.w));
            self.session_view.text_focus = false;
            self.save_window();
        }
    }

    fn shown_pane(&mut self, width: f32) -> Option<&mut Pane> {
        if !self.session_visible(width) {
            return None;
        }
        self.tabs.shown_mut()?.pane.as_mut()
    }

    pub fn start_demo_session(&mut self) {
        if !self.source.is_demo() || !self.tabs.is_empty() {
            return;
        }
        let demo = match crate::demo::Transcript::new_chat() {
            Ok(demo) => demo,
            Err(e) => {
                self.error("Could not start the demo transcript", e.to_string());
                return;
            }
        };
        let mut terminal = crate::term::Term::empty(self.grid);
        terminal.feed(
            concat!(
                "\x1b[1mClaude Code · demo\x1b[0m\r\n",
                "\x1b[2mSynthetic session · no CLI process\x1b[0m\r\n\r\n",
                "> Make a north ridge pocket guide.\r\n\r\n",
                "  The demo is waiting for a decision about field-notes.md.\r\n",
                "  No command will run here. Try Send anyway in Chat.\r\n",
                "\x1b[?2004h",
                "\x1b[?25l",
            )
            .as_bytes(),
        );
        let preview = self.config.appearance.backdrop != "off";
        if preview {
            terminal.feed(b"\x1b[44m Explicit background \x1b[0m\r\nSelected text\r\nCursor: \x1b[?25h");
            let line = terminal.screen.screen_start() + 8;
            let mut selection = mdshared_term::Selection::start(
                &terminal.screen,
                mdshared_term::Point::new(line, 0),
                mdshared_term::SelectionKind::Stream,
                1,
            );
            selection.extend(&terminal.screen, mdshared_term::Point::new(line, 12));
            terminal.selection = Some(selection);
        }
        let account = self.current().slug.clone();
        let id = self.tabs.open(
            crate::tabs::NewTab {
                label: "Field guide · Demo".into(),
                color: self.account_color(&account),
                session: Some("field-guide".into()),
                account: Some(account),
            },
            terminal,
        );
        let mut pane = self.new_session_pane(model::Source::File(demo.path().to_owned()), demo.cwd().to_owned());
        pane._demo = Some(demo);
        pane.chat = chat::Chat::demo();
        if preview {
            pane.chat.open_drawer();
        }
        pane.ui.chat_conversation(&mut pane.model);
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.pane = Some(pane);
        }
        if self.debug_list {
            self.show_sessions();
        }
    }

    pub fn pane_for_launch(&self, launch: &Launch, session: Option<&str>, account: Option<&str>) -> Option<Pane> {
        if launch.args.first().is_some_and(|arg| arg == "--session-id") {
            let (id, account) = (session?, account?);
            let path = crate::compact_open::transcript_path(&self.config, launch, account, id)?;
            return Some(self.new_session_pane(model::Source::File(path), launch.cwd.clone()?));
        }
        if !session_launch(launch) {
            return None;
        }
        let cwd = launch.cwd.clone()?;
        let source = if let Some(id) = session {
            model::Source::File(self.sessions.get(*self.by_id.get(id)?)?.file.clone())
        } else {
            let account = claude_switch_core::account(&self.config, account?).ok()?;
            model::Source::Discover {
                config: account.cli_dir.clone(),
                cwd: cwd.clone(),
                started: SystemTime::now(),
                verified: Default::default(),
            }
        };
        Some(self.new_session_pane(source, cwd))
    }

    fn new_session_pane(&self, source: model::Source, cwd: PathBuf) -> Pane {
        let proxy = self.proxy.clone();
        let model = model::Model::new(source.clone(), move || {
            if let Some(proxy) = &proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(Msg::SessionView);
            }
        });
        let mut chat = chat::Chat::default();
        chat.enabled = self.win_file.chat_default;
        let mut ui = ui::Ui::default();
        ui.set_chat(chat.enabled);
        Pane { model, transcript_source: source, ui, cwd, next: None, _demo: None, chat }
    }

    pub fn layout_session_view(&mut self, cx: &mut Cx<'_, Msg>, area: Rect) -> Rect {
        self.session_view.area = area;
        if self.chat_active() {
            self.session_view.split_hover = false;
            self.session_tick(cx, false);
            return self.layout_chat(area);
        }
        if self.session_visible(area.w) {
            let (terminal, _, pane) = self.session_view.splitter.rects(area, AXIS);
            self.session_view.pane_area = pane;
            self.session_tick(cx, false);
            terminal
        } else {
            self.session_view.pane_area = Rect::default();
            self.session_view.split_hover = false;
            area
        }
    }

    pub fn session_tick(&mut self, cx: &mut Cx<'_, Msg>, output: bool) {
        if cx.window().is_minimized() == Some(true) {
            return;
        }
        let width = cx.size().0;
        let now = cx.now();
        if !self.session_visible(width) {
            return;
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        if output || pane.next.is_none_or(|at| at <= now) {
            pane.model.poll();
            pane.next = Some(now + POLL);
        }
        if let Some(next) = pane.next {
            cx.wake_at(next);
        }
        self.session_ready(cx);
    }

    pub fn session_ready(&mut self, cx: &mut Cx<'_, Msg>) {
        if !self.session_visible(cx.size().0) || cx.window().is_minimized() == Some(true) {
            return;
        }
        let area = self.session_view.pane_area;
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        if pane.model.collect() {
            if pane.chat.enabled || pane.ui.overlay_open() || area.w <= 0.0 {
                cx.request_redraw_all();
            } else {
                cx.request_redraw(area);
            }
        }
        self.chat_input_tick(cx);
    }

    pub fn draw_session_view(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let rects = self.session_view.splitter.rects(area, AXIS);
        let mouse = self.mouse;
        let hot = self.nothing_over() && !self.picture_overlay() && self.session_view.split_hover;
        let motion = cx.anim().animates();
        if let Some(pane) = self.shown_pane(area.w) {
            pane.ui.draw(p, pal, rects.2, &mut pane.model, mouse, cx.now(), motion);
            if pane.ui.animating() && motion {
                cx.animating(rects.2);
            }
            splitter::draw(p, pal, rects, AXIS, hot);
        }
    }

    pub fn draw_session_overlay(&mut self, p: &mut Painter, pal: &Palette, window: Rect) {
        if let Some(pane) = self.shown_pane(window.w) {
            pane.ui.draw_overlay(p, pal, window, &pane.model);
        }
    }

    pub fn picture_overlay(&self) -> bool {
        self.session_visible(self.session_view.area.w)
            && self.tabs.shown().and_then(|tab| tab.pane.as_ref()).is_some_and(|pane| pane.ui.overlay_open())
    }

    pub fn session_key(&mut self, width: f32, name: &KeyName, mods: Mods) -> bool {
        (self.session_view.text_focus || self.picture_overlay())
            && self.shown_pane(width).is_some_and(|pane| pane.ui.key(name, mods))
    }

    pub fn clear_session_text_focus(&mut self) {
        self.session_view.text_focus = false;
        if let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) {
            pane.ui.blur();
        }
    }

    pub fn session_press(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32, clicks: u32) -> bool {
        let width = cx.size().0;
        let overlay = self.picture_overlay();
        if !overlay && !self.chat_active() && self.session_visible(width) {
            let split = &mut self.session_view.splitter;
            let area = self.session_view.area;
            if split.hit(area, AXIS, x, y) {
                if clicks == 2 {
                    split.reset();
                    self.save_window();
                } else {
                    split.press(area, AXIS, x, y);
                }
                return true;
            }
        }
        let over = overlay || self.session_view.pane_area.contains(x, y);
        if over {
            self.pressed = None;
        }
        if over {
            self.session_view.text_focus = true;
        } else {
            self.clear_session_text_focus();
        }
        if over && let Some(pane) = self.shown_pane(width) {
            if pane.ui.press(x, y, clicks, cx.mods(), &mut pane.model) {
                let cwd = pane.cwd.clone();
                self.attach_picker(Some(&cwd), false);
            }
            return true;
        }
        false
    }

    pub fn session_release(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) -> bool {
        if self.session_view.splitter.release() {
            self.save_window();
            return true;
        }
        if !self.nothing_over() {
            return false;
        }
        self.shown_pane(cx.size().0).is_some_and(|pane| pane.ui.release(x, y, cx.mods(), &mut pane.model, &pane.cwd))
    }

    pub fn session_move(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) -> Option<bool> {
        if !self.nothing_over() {
            return None;
        }
        let area = self.session_view.area;
        if self.session_view.splitter.dragging() {
            cx.set_cursor(Cursor::ResizeEw);
            return Some(self.session_view.splitter.drag(area, AXIS, x, y));
        }
        let hot = !self.chat_active()
            && self.session_visible(area.w)
            && !self.picture_overlay()
            && self.session_view.splitter.hit(area, AXIS, x, y);
        let split_changed = std::mem::replace(&mut self.session_view.split_hover, hot) != hot;
        if hot {
            cx.set_cursor(Cursor::ResizeEw);
            return Some(split_changed);
        }
        if let Some(pane) = self.shown_pane(area.w) {
            let (cursor, changed) = pane.ui.moved(x, y);
            if let Some(cursor) = cursor {
                cx.set_cursor(cursor);
                return Some(changed || split_changed);
            }
            if changed {
                return Some(true);
            }
        }
        if split_changed {
            cx.request_redraw_all();
        }
        None
    }

    pub fn session_pointer_left(&mut self) {
        self.session_view.split_hover = false;
        self.mouse = (-1.0, -1.0);
        let width = self.session_view.area.w;
        if let Some(pane) = self.shown_pane(width) {
            // Re-evaluate hover on re-entry rather than leaving a hot button behind.
            pane.ui.moved(-1.0, -1.0);
        }
    }

    pub fn session_wheel(&mut self, width: f32, x: f32, y: f32, dy: f32) -> bool {
        (self.picture_overlay() || self.session_view.pane_area.contains(x, y))
            && self.shown_pane(width).is_some_and(|pane| {
                pane.ui.wheel(dy);
                true
            })
    }
}

pub fn handover_transcript(config: &std::path::Path, cwd: &std::path::Path, id: &str) -> PathBuf {
    model::project(config, cwd).join(format!("{id}.jsonl"))
}

fn session_launch(launch: &Launch) -> bool {
    launch.place == claude_switch_core::Place::Terminal
        && launch.program == std::path::Path::new("claude")
        && (launch.args.is_empty() || launch.args.first().is_some_and(|arg| arg == "--resume"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_width_default_is_inclusive_and_explicit_choices_win() {
        assert!(!enabled(None, 1099.0));
        assert!(enabled(None, 1100.0));
        assert!(enabled(Some(true), 760.0));
        assert!(!enabled(Some(false), 1800.0));
    }

    #[test]
    fn only_interactive_claude_launches_have_a_session_view() {
        use claude_switch_core::Place;
        let mut launch = Launch {
            program: "claude".into(),
            args: vec![],
            cwd: None,
            env: Default::default(),
            env_remove: vec![],
            place: Place::Terminal,
            label: String::new(),
        };
        assert!(session_launch(&launch));
        launch.args = ["--resume", "session"].map(str::to_owned).into();
        assert!(session_launch(&launch));
        launch.args.push("--fork-session".into());
        assert!(session_launch(&launch));
        launch.args = ["auth", "login"].map(str::to_owned).into();
        assert!(!session_launch(&launch));
        launch.args.clear();
        launch.place = Place::Detached;
        assert!(!session_launch(&launch));
        launch.place = Place::Terminal;
        launch.program = "cmd.exe".into();
        assert!(!session_launch(&launch));
    }
}
