//! Session view integration with the window, terminal and splitter.

pub mod attach;
pub mod chat;
mod discovery;
mod documents;
mod links;
pub(crate) mod look;
pub(crate) mod model;
mod outputs;
mod pictures;
mod preview;
pub mod reader;
mod shells;
mod text;
mod ui;
pub(crate) use ui::Reading;
mod working;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use claude_switch_core::Launch;
use claude_switch_core::transcript::ItemId;
use mdshared_ui::cursor::Cursor;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::splitter::{self, Axis, Hold, Splitter};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Msg, Target};
use crate::winfile::WindowFile;

const AXIS: Axis = Axis::Horizontal;
const POLL: Duration = Duration::from_millis(500);
pub(crate) const SESSION_CHANGED: &str =
    "Session changed in the terminal. Reopen the session to attach its transcript.";

fn enabled(preference: Option<bool>, width: f32) -> bool {
    preference.unwrap_or(width >= 1100.0)
}

pub struct Window {
    pub splitter: Splitter,
    /// The docked Markdown panel's width and reading width, shared by every pane.
    pub markdown: ui::Panel,
    area: Rect,
    pane_area: Rect,
    text_focus: bool,
    pub drops: attach::DropBatch,
    pub demo_permission: bool,
    pub demo_working: bool,
}

impl Window {
    pub fn new(prefs: &WindowFile) -> Self {
        let mut splitter = Splitter::new(Hold::B(440.0)).min(270.0, 330.0);
        if !prefs.session_split.is_empty() {
            splitter.apply_setting(prefs.session_split.split(';').next().unwrap_or_default());
        }
        Self {
            splitter,
            markdown: ui::Panel::from_settings(&prefs.markdown_split, &prefs.markdown_reading),
            area: Rect::default(),
            pane_area: Rect::default(),
            text_focus: false,
            drops: Default::default(),
            demo_permission: false,
            demo_working: false,
        }
    }
}

/// What a pane knows about its session, from the live record and the transcript folder.
enum Identity {
    Attached(PathBuf),
    /// The record names a new session whose transcript the CLI has not written yet.
    Awaiting,
    Unverified,
}

type Wake = std::sync::Arc<dyn Fn() + Send + Sync>;

pub struct Pane {
    model: model::Model,
    wake: Wake,
    /// A new session this pane followed and the one it continues, for the Threads link.
    pub(crate) continued: Option<(String, String)>,
    pub chat: chat::Chat,
    pub(crate) transcript_source: model::Source,
    verified_path: Option<PathBuf>,
    ui: ui::Ui,
    working: working::Working,
    cwd: PathBuf,
    next: Option<Instant>,
    _demo: Option<crate::demo::Transcript>,
}

impl Pane {
    /// What a branch at the message `item` copies: the transcript the reader reads (its file and
    /// the session id it verified) and where to cut it (`Model::branch_cut`). Why not, otherwise.
    pub(crate) fn branch_plan(
        &self,
        item: ItemId,
    ) -> Result<(PathBuf, String, claude_switch_core::branch::Cut), &'static str> {
        if self.model.agent.is_some() {
            return Err("Branch works in the session's own conversation, not in a subagent's.");
        }
        let cut = self.model.branch_cut(item).ok_or("Branch works at the end of a turn that has ended.")?;
        let id = self.model.session_id().ok_or("The session is not verified yet. Try again in a moment.")?;
        let path = self.verified_path.clone().ok_or("The session's transcript is not read yet.")?;
        let named = path.file_stem().is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(id));
        if !named {
            return Err("The transcript Chat reads is not this session's.");
        }
        Ok((path, id.to_owned(), cut))
    }

    fn transcript_end(&self) -> Option<u64> {
        if self.chat.awaiting_transcript {
            // No file yet: everything the CLI writes to it later is newer than this input.
            return Some(0);
        }
        let meta = std::fs::symlink_metadata(self.verified_path.as_ref()?).ok()?;
        meta.is_file().then_some(meta.len())
    }

    pub(crate) fn note_terminal_input(&mut self) {
        self.chat.invalidate_input(&self.model.chat, self.transcript_end());
    }

    /// A wheel report to the program: input, but not one that stops Chat's card key sequences.
    pub(crate) fn note_terminal_wheel(&mut self) {
        self.chat.invalidate_pointer_input(&self.model.chat, self.transcript_end());
    }

    fn note_composer_input(&mut self) {
        self.chat.watermark_input(&self.model.chat, self.transcript_end());
    }

    /// Keys of a message going into the running turn: Claude Code records it as part of that turn
    /// (a `queued_command`), never as a new prompt, so the turn stays the one the transcript shows
    /// (Stop, the gate); only the input's time is noted.
    fn note_composer_input_into_turn(&mut self, into_turn: bool) {
        if into_turn {
            self.chat.note_input();
        } else {
            self.note_composer_input();
        }
    }
    pub fn restore_chat_layout(&mut self, chat: bool) {
        self.chat.enabled = chat;
        self.ui.set_chat(chat);
    }

    pub(crate) fn pinned_id(&self) -> Option<&str> {
        match &self.transcript_source {
            model::Source::File(path) | model::Source::Demo(path) => path.file_stem()?.to_str(),
            model::Source::Discover { verified, .. } => verified.get().map(String::as_str),
        }
    }

    /// What the live record's `id` proves for this pane's source; pins it when it is new.
    fn resolve(&self, id: &str) -> Identity {
        let id = self.pinned_id().unwrap_or(id).to_owned();
        let discover = matches!(self.transcript_source, model::Source::Discover { .. });
        match self.transcript_source.probe(&id) {
            model::Probe::Found(path) if self.transcript_source.pin(&id) => Identity::Attached(path),
            model::Probe::Missing if discover && self.transcript_source.pin(&id) => Identity::Awaiting,
            _ => Identity::Unverified,
        }
    }

    /// The same process now names `id` (`/clear`, `/resume`, compaction): read that session
    /// instead. Refused when the first message was sent for a transcript that never appeared,
    /// or when `id`'s transcript exists but is not this tab's (another folder, invalid ID).
    fn follow(&mut self, id: &str) -> bool {
        if self.chat.awaiting_transcript && self.chat.first_sent {
            return false;
        }
        let Some(source) = self.transcript_source.follow(&self.cwd, id) else { return false };
        // Only a session this tab had attached and verified is a parent: not one it was refused
        // for, nor a new chat whose transcript was never written.
        let verified = self.verified_path.is_some() && !self.chat.identity_unverified;
        let previous = self.pinned_id().filter(|_| verified).map(str::to_owned);
        let end = match source.probe(id) {
            model::Probe::Found(path) => std::fs::symlink_metadata(&path).map_or(u64::MAX, |m| m.len()),
            // A session the CLI has yet to write (`/clear`, compaction) continues this one; an
            // existing transcript (`/resume`) is another conversation and is not linked.
            model::Probe::Missing => {
                self.continued = previous.map(|old| (id.to_owned(), old));
                0
            }
            model::Probe::Rejected => return false,
        };
        let wake = self.wake.clone();
        self.model = model::Model::after(source.clone(), self.cwd.clone(), self.model.generation, move || wake());
        self.transcript_source = source;
        self.verified_path = None;
        self.chat.followed(&self.model.chat, end);
        true
    }

    pub(crate) fn update_identity(&mut self, found: Option<&str>) -> bool {
        let mismatch = self.pinned_id().zip(found).is_some_and(|(pin, id)| !pin.eq_ignore_ascii_case(id));
        let followed = mismatch && found.is_some_and(|id| self.follow(id));
        let refused = mismatch && !followed;
        let identity = found.filter(|_| !refused).map_or(Identity::Unverified, |id| self.resolve(id));
        let (path, awaiting) = match identity {
            Identity::Attached(path) => (Some(path), false),
            Identity::Awaiting => (None, true),
            Identity::Unverified => (None, false),
        };
        let unverified = path.is_none() && !awaiting;
        let changed = followed
            || self.chat.identity_unverified != unverified
            || self.chat.awaiting_transcript != awaiting
            || (refused && self.model.error.as_deref() != Some(SESSION_CHANGED));
        if self.chat.identity_unverified != unverified {
            self.chat.invalidate(&self.model.chat);
        }
        self.verified_path = path;
        self.chat.identity_unverified = unverified;
        self.chat.awaiting_transcript = awaiting;
        if refused {
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
    pub(crate) fn scroll_wake_due(&self, now: Instant) -> bool {
        self.tabs
            .shown()
            .and_then(|tab| tab.pane.as_ref())
            .and_then(|pane| pane.ui.scroll_deadline())
            .is_some_and(|deadline| deadline <= now)
    }
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
        let demo = match crate::demo::Transcript::new_details() {
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
        if self.session_view.demo_working {
            terminal.feed("\r\n✻ Shimmying… (53m 37s · ↓ 27.1k tokens)\r\n  esc to interrupt".as_bytes());
        }
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
        let mut pane = self.new_session_pane(model::Source::Demo(demo.path().to_owned()), demo.cwd().to_owned());
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
        let wake: Wake = std::sync::Arc::new(move || {
            if let Some(proxy) = &proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(Msg::SessionView);
            }
        });
        let model = model::Model::new(source.clone(), cwd.clone(), {
            let wake = wake.clone();
            move || wake()
        });
        let mut chat = chat::Chat::default();
        chat.identity_unverified = true;
        chat.enabled = self.win_file.chat_default;
        let mut ui = ui::Ui::default();
        ui.set_chat(chat.enabled);
        ui.set_panel(self.session_view.markdown);
        Pane {
            model,
            wake,
            continued: None,
            transcript_source: source,
            verified_path: None,
            ui,
            working: Default::default(),
            cwd,
            next: None,
            _demo: None,
            chat,
        }
    }

    pub fn layout_session_view(&mut self, cx: &mut Cx<'_, Msg>, area: Rect) -> Rect {
        self.session_view.area = area;
        if self.chat_active() {
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
            area
        }
    }

    pub fn session_tick(&mut self, cx: &mut Cx<'_, Msg>, output: bool) {
        if cx.window().is_minimized() == Some(true) {
            self.chat_input_tick(cx);
            return;
        }
        let width = cx.size().0;
        let now = cx.now();
        if !self.session_visible(width) {
            self.chat_input_tick(cx);
            return;
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        if output || pane.next.is_none_or(|at| at <= now) {
            pane.model.poll();
            // Chat's panel lists the shells among its sections; the Session view on their page.
            let shells = if pane.chat.enabled {
                pane.chat.detail_side().is_some()
            } else {
                pane.ui.detail_side() == Some(chat::Side::Shells)
            };
            if shells && !pane.ui.overlay_open() {
                for &id in pane.chat.details.visible_shells.iter().take(6) {
                    pane.model.request_shell_tail(id);
                }
                if pane.model.running_shells() > 0 {
                    if pane.chat.enabled {
                        cx.request_redraw_all();
                    } else {
                        cx.request_redraw(self.session_view.pane_area);
                    }
                }
            }
            pane.next = Some(now + POLL);
        }
        if let Some(next) = pane.next {
            cx.wake_at(next);
        }
        self.session_ready(cx);
    }

    pub fn session_ready(&mut self, cx: &mut Cx<'_, Msg>) {
        if !self.session_visible(cx.size().0) || cx.window().is_minimized() == Some(true) {
            self.chat_input_tick(cx);
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
        self.update_working(cx);
    }

    pub fn draw_session_view(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect, pal: &Palette) {
        let rects = self.session_view.splitter.rects(area, AXIS);
        let mouse = self.background_pointer();
        let hot = self.session_view.splitter.hit(area, AXIS, mouse.0, mouse.1);
        let motion = cx.anim().animates();
        let tab = self.tabs.active.unwrap_or(0);
        let hover = self.hover;
        let details_active = self.nothing_over() && !self.picture_overlay() && !self.usage.popup.open;
        let mut hits = Vec::new();
        let diff = look::Diff::from_ansi(&self.term_colours.0.ansi);
        if let Some(pane) = self.shown_pane(area.w) {
            pane.ui.set_diff_colours(diff);
            pane.ui.draw(p, pal, rects.2, &mut pane.model, mouse, cx.now(), motion);
            if let Some(deadline) = pane.ui.scroll_deadline() {
                cx.wake_at(deadline);
            }
            if let Some(side) = pane.ui.detail_side() {
                let area =
                    mdshared_ui::gfx::Rect::new(rects.2.x, rects.2.y + 31.0, rects.2.w, (rects.2.h - 31.0).max(0.0));
                hits = pane.chat.details.draw(
                    p,
                    pal,
                    chat::side::Frame {
                        tab,
                        area,
                        side,
                        sections: false,
                        diff,
                        model: &pane.model,
                        cwd: &pane.cwd,
                        hover: hover.filter(|_| details_active),
                    },
                );
            } else {
                pane.chat.details.visible_shells.clear();
            }
            if pane.ui.animating() && motion {
                cx.animating(rects.2);
            }
            splitter::draw(p, pal, rects, AXIS, hot);
            self.hits.push((rects.1, Target::Splitter));
        }
        if details_active {
            self.hits.extend(hits);
        }
    }

    pub fn draw_session_overlay(&mut self, p: &mut Painter, pal: &Palette, window: Rect) {
        let mouse = self.picture_pointer();
        let radius = crate::windowlook::Cards::for_frame(self, p).decoration(10.0);
        if let Some(reader) = self.shown_reader() {
            reader.draw_overlay(p, pal, window, mouse, radius);
        }
        if let Some(pane) = self.shown_pane(window.w) {
            pane.ui.draw_overlay(p, pal, window, &pane.model, mouse, radius);
        }
    }

    pub fn picture_overlay(&self) -> bool {
        let details = !self.terminal_shown()
            && !self.tabs.workers_shown()
            && self.details_reader.as_ref().is_some_and(|r| r.overlay_open() && self.selected.as_ref() == Some(&r.id));
        details
            || self.session_visible(self.session_view.area.w)
                && self.tabs.shown().and_then(|tab| tab.pane.as_ref()).is_some_and(|pane| pane.ui.overlay_open())
    }

    pub fn session_key(&mut self, width: f32, name: &KeyName, mods: Mods) -> bool {
        // The details conversation has the keys it was clicked for only while the Sessions page
        // keeps the keyboard there: once Tab, Ctrl+F or typing moved it, the reader lets go.
        let list = self.focus == crate::state::Focus::List;
        if let Some(reader) = self.shown_reader() {
            if !list && !reader.overlay_open() {
                reader.blur();
                return false;
            }
            return reader.key(name, mods);
        }
        (self.session_view.text_focus || self.picture_overlay())
            && self.shown_pane(width).is_some_and(|pane| pane.ui.key(name, mods))
    }

    pub fn clear_session_text_focus(&mut self) {
        if let Some(reader) = &mut self.details_reader {
            reader.blur();
        }
        self.session_view.text_focus = false;
        if let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) {
            pane.ui.blur();
        }
    }

    pub fn session_press(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32, clicks: u32) -> bool {
        let mods = cx.mods();
        if let Some(reader) = self.shown_reader() {
            if reader.press(x, y, clicks, mods) {
                self.pressed = None;
                self.focus = crate::state::Focus::List;
                return true;
            }
            return false;
        }
        let width = cx.size().0;
        let overlay = self.picture_overlay();
        if !overlay
            && !self.chat_active()
            && self.session_visible(width)
            && let Some(target) = self.hits.iter().rev().find_map(|(r, t)| {
                (r.contains(x, y) && matches!(t, Target::Chat(chat::Control::Detail(_)))).then_some(*t)
            })
        {
            self.pressed = Some(target);
            return true;
        }
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
        let mods = cx.mods();
        if let Some(reader) = self.shown_reader() {
            return reader.release(x, y, mods);
        }
        let Some(pane) = self.shown_pane(cx.size().0) else { return false };
        let previous = pane.ui.detail_side();
        let action = pane.ui.release(x, y, cx.mods(), &mut pane.model);
        if previous != pane.ui.detail_side() {
            pane.chat.details.reset_scroll();
            pane.chat.details.visible_shells.clear();
        }
        if let Some(panel) = pane.ui.take_panel_change() {
            self.share_markdown_panel(panel);
        }
        match action {
            ui::Release::Miss => false,
            ui::Release::Handled => true,
            ui::Release::Stop(turn) => {
                self.stop_conversation(cx, Some(turn));
                true
            }
            ui::Release::Shell(id) => {
                self.select_shell(Some(id));
                true
            }
            ui::Release::ShellList => {
                self.select_shell(None);
                true
            }
            ui::Release::Rewind(item) => {
                // The right-click menu's "Rewind to here…": the same start and checks.
                if let Some(tab) = self.tabs.active {
                    self.term_woke(cx);
                    self.rewind_from_menu(tab, item, cx.now());
                    self.arm_chat_tick(cx);
                }
                true
            }
            ui::Release::Branch(item) => {
                if let Some(tab) = self.tabs.active {
                    self.branch_at(cx.now(), tab, item);
                }
                true
            }
        }
    }

    /// One pane's Markdown panel width or reading width, for every pane and the next run.
    fn share_markdown_panel(&mut self, panel: ui::Panel) {
        self.session_view.markdown = panel;
        for pane in self.tabs.list.iter_mut().filter_map(|tab| tab.pane.as_mut()) {
            pane.ui.set_panel(panel);
        }
        self.save_window();
    }

    pub fn session_move(&mut self, x: f32, y: f32) -> Option<bool> {
        let area = self.session_view.area;
        if !self.picture_overlay() && self.session_view.splitter.dragging() {
            return Some(self.session_view.splitter.drag(area, AXIS, x, y));
        }
        if !self.splitter.dragging()
            && let Some(reader) = self.shown_reader()
        {
            let changed = reader.moved(x, y);
            return (changed || reader.owns_pointer(x, y)).then_some(changed);
        }
        if let Some(pane) = self.shown_pane(area.w) {
            let changed = pane.ui.moved(x, y);
            return (changed || pane.ui.owns_pointer(x, y)).then_some(changed);
        }
        None
    }

    pub fn session_pointer_cursor(&self, x: f32, y: f32) -> Option<Cursor> {
        if self.tabs.workers_shown() {
            return None;
        }
        if !self.terminal_shown() {
            let reader = self.details_reader.as_ref().filter(|r| self.selected.as_ref() == Some(&r.id))?;
            return reader.pointer_cursor(x, y);
        }
        let area = self.session_view.area;
        if !self.session_visible(area.w) {
            return None;
        }
        if !self.chat_active() && !self.picture_overlay() && self.session_view.splitter.hit(area, AXIS, x, y) {
            return Some(Cursor::ResizeEw);
        }
        self.tabs.shown()?.pane.as_ref()?.ui.pointer_cursor(x, y)
    }

    pub fn session_pointer_left(&mut self) {
        self.mouse = (-1.0, -1.0);
        if let Some(reader) = self.shown_reader() {
            reader.pointer_left();
        }
        let width = self.session_view.area.w;
        if let Some(pane) = self.shown_pane(width) {
            // Re-evaluate hover on re-entry rather than leaving a hot button behind.
            pane.ui.pointer_left();
        }
    }

    pub fn session_wheel(&mut self, width: f32, x: f32, y: f32, dy: f32) -> bool {
        (self.picture_overlay() || self.session_view.pane_area.contains(x, y))
            && self.shown_pane(width).is_some_and(|pane| {
                if pane.ui.detail_side().is_some() && !pane.ui.overlay_open() {
                    pane.chat.details.wheel(dy);
                } else {
                    pane.ui.wheel(x, y, dy);
                }
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
