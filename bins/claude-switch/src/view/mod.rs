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

use std::path::{Path, PathBuf};
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
const SESSION_CHANGED: &str = "Session changed in the terminal. Reopen the session to attach its transcript.";

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
    pub demo_permission: bool,
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
            demo_permission: false,
        }
    }
}

pub struct Pane {
    model: model::Model,
    pub chat: chat::Chat,
    pub(crate) transcript_source: model::Source,
    verified_path: Option<PathBuf>,
    ui: ui::Ui,
    cwd: PathBuf,
    next: Option<Instant>,
    _demo: Option<crate::demo::Transcript>,
}

impl Pane {
    pub(crate) fn note_terminal_input(&mut self) {
        let end = self.verified_path.as_ref().and_then(|p| std::fs::symlink_metadata(p).ok()).filter(|m| m.is_file());
        self.chat.invalidate_input(&self.model.chat, end.map(|m| m.len()));
    }
    pub fn restore_chat_layout(&mut self, chat: bool) {
        self.chat.enabled = chat;
        self.ui.set_chat(chat);
    }

    pub(crate) fn pinned_id(&self) -> Option<&str> {
        match &self.transcript_source {
            model::Source::File(path) => path.file_stem()?.to_str(),
            model::Source::Discover { verified, .. } => verified.get().map(String::as_str),
        }
    }

    fn identity_path(&self, id: &str) -> Option<PathBuf> {
        match &self.transcript_source {
            model::Source::File(path) => Some(path.clone()),
            model::Source::Discover { verified, .. } => {
                let path = self.transcript_source.candidate_path(verified.get().map_or(id, String::as_str))?;
                verified.get_or_init(|| id.to_owned());
                Some(path)
            }
        }
    }

    pub(crate) fn update_identity(&mut self, found: Option<&str>) -> bool {
        let mismatch = self.pinned_id().zip(found).is_some_and(|(pin, id)| !pin.eq_ignore_ascii_case(id));
        let path = found.filter(|_| !mismatch).and_then(|id| self.identity_path(id));
        let unverified = path.is_none();
        let changed = self.chat.identity_unverified != unverified
            || (mismatch && self.model.error.as_deref() != Some(SESSION_CHANGED));
        if self.chat.identity_unverified != unverified {
            self.chat.invalidate(&self.model.chat);
        }
        self.verified_path = path;
        self.chat.identity_unverified = unverified;
        if mismatch {
            self.model.error = Some(SESSION_CHANGED.into());
        } else if !unverified && self.model.error.as_deref() == Some(SESSION_CHANGED) {
            self.model.error = None;
        }
        changed
    }

    pub(crate) fn verified_transcript(&self) -> Option<&Path> {
        self.verified_path.as_deref()
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
                "  Chat keeps your draft and blocks both Send buttons while waiting.\r\n",
                "\x1b[?2004h",
                "\x1b[?25l",
            )
            .as_bytes(),
        );
        let preview = self.config.appearance.backdrop != "off";
        let permission = self.session_view.demo_permission;
        if permission {
            terminal.feed("\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No\r\nEsc to cancel".as_bytes());
        }
        if preview && !permission {
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
        pane.verified_path = pane.transcript_source.find_file();
        pane.chat = chat::Chat::demo();
        if permission {
            pane.chat.seed_permission_demo();
        } else if preview {
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

    pub(crate) fn new_session_pane(&self, source: model::Source, cwd: PathBuf) -> Pane {
        let proxy = self.proxy.clone();
        let model = model::Model::new(source.clone(), cwd.clone(), move || {
            if let Some(proxy) = &proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(Msg::SessionView);
            }
        });
        let mut chat = chat::Chat::default();
        chat.identity_unverified = true;
        chat.enabled = self.win_file.chat_default;
        let mut ui = ui::Ui::default();
        ui.set_chat(chat.enabled);
        Pane { model, transcript_source: source, verified_path: None, ui, cwd, next: None, _demo: None, chat }
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
        self.shown_pane(cx.size().0).is_some_and(|pane| pane.ui.release(x, y, cx.mods(), &mut pane.model))
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
        if let Some(pane) = self.shown_pane(area.w)
            && pane.ui.scroll_dragging()
        {
            let (cursor, changed) = pane.ui.moved(x, y);
            cx.set_cursor(cursor.unwrap_or(Cursor::Default));
            return Some(changed);
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
                // Clear transcript hover without claiming the pointer outside it:
                // Chat's Details rows still need the window's hover/cursor pass.
                cx.request_redraw_all();
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
            pane.ui.pointer_left();
        }
    }

    pub fn session_wheel(&mut self, width: f32, x: f32, y: f32, dy: f32) -> bool {
        (self.picture_overlay() || self.session_view.pane_area.contains(x, y))
            && self.shown_pane(width).is_some_and(|pane| {
                pane.ui.wheel(x, y, dy);
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
