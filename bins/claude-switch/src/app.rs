//! The window loop's side of the state: the host's events routed to the open layer (palette,
//! drop-down, menu, dialog, then the window), timers, and the frame drawn from top to bottom.

use claude_switch_core::usage::Demand;
use mdshared_ui::chrome::{TITLE_H, WinButton};
use mdshared_ui::dropdown::Outcome as DdOutcome;
use mdshared_ui::gfx::Rect;
use mdshared_ui::host::{Button, Cx, KeyPress};
use mdshared_ui::keys::KeyName;
use mdshared_ui::palette::Outcome as PalOutcome;
use mdshared_ui::splitter::Axis;

use crate::actions;
use crate::dialogs::{self, Dialog};
use crate::state::{AppState, Focus, MenuKind, Msg, POLL, Target};
use crate::termkeys;
use crate::topbar::{self, Bar};
use crate::{list, winfile};

mod events;
mod pointer;

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
        let top = self.main_top.max(TITLE_H);
        Rect::new(0.0, top, w, h - top)
    }

    fn timers(&self, cx: &mut Cx<'_, Msg>) {
        for at in [
            self.next_poll,
            self.conversation_search.due.filter(|_| self.scanned),
            self.tip.deadline(),
            self.toasts.deadline(),
            self.undo_hide.as_ref().map(|u| u.until),
            self.marks_flush_at,
            self.usage_warnings.next,
            self.usage.popup.due,
            self.usage.cache_retry_at().filter(|_| self.config.show_usage),
        ]
        .into_iter()
        .flatten()
        {
            cx.wake_at(at);
        }
    }

    pub(crate) fn save_window(&mut self) {
        if let Err(error) = self.write_window() {
            self.error("Could not save window settings", error.to_string());
        }
    }

    pub(crate) fn write_window(&mut self) -> Result<(), claude_switch_core::Refusal> {
        self.win_file.split = self.splitter.to_setting();
        self.win_file.session_split = self.session_view.splitter.to_setting();
        let panel = self.session_view.markdown;
        (self.win_file.markdown_split, self.win_file.markdown_reading) =
            (panel.split.to_setting(), panel.reading.key().to_owned());
        winfile::save(self.source.window_file().as_deref(), &self.win_file)
    }

    fn activate(&mut self, cx: &mut Cx<'_, Msg>, t: Target, x: f32) {
        let now = cx.now();
        match t {
            Target::Caption(WinButton::Close) => {
                self.request_window_close(cx);
            }
            Target::TabClose(id) => self.ask_close_tab(id),
            Target::RestoreTabs => self.begin_tab_restore(now),
            Target::DismissTabs => self.dismiss_tab_restore(),
            Target::SessionToggle => self.toggle_session_view(),
            Target::ChatMode(enabled) => self.set_chat_layout(enabled),
            Target::Chat(control) => self.chat_control(cx, control),
            Target::Caption(WinButton::Max) => cx.toggle_maximize(),
            Target::Caption(WinButton::Min) => cx.minimize(),
            Target::SearchClear => {
                self.search.clear();
                self.after_search();
            }
            Target::SearchConversation => self.toggle_conversation_search(now),
            Target::NewSession => self.command(now, actions::NEW_SESSION, None),
            Target::Refresh => self.command(now, actions::REFRESH, None),
            Target::UsageLines => self.open_usage(now, true),
            Target::UsageRefresh => {
                let slug = self.current().slug.clone();
                self.request_usage(now, Demand::Refresh, Some(&slug));
            }
            Target::UsageOpenAs => self.usage_open_as(claude_switch_core::usage::now_ms()),
            Target::MenuButton => {
                if let Some(r) = self.rect_of(t) {
                    self.main_menu(r);
                }
            }
            Target::OpenDefault => self.command(now, actions::OPEN_LAST, None),
            Target::FolderOpen => self.edit_folder_open(),
            Target::ArchiveMenu => {
                if let Some(r) = self.rect_of(t) {
                    self.archive_menu(r);
                }
            }
            Target::ArchivePick(entry) => {
                if let Some(i) = self.session_at(entry) {
                    self.toggle_archive_pick(i);
                }
            }
            Target::OpenAs => self.command(now, actions::OPEN_AS, self.rect_of(t)),
            Target::OpenCopy => self.command(now, actions::OPEN_COPY, None),
            Target::Handovers => self.ask_compact(true),
            Target::ListMode(mode) if self.list_mode() != mode => {
                self.config.sessions_mode = mode;
                self.save_config();
                self.rebuild_after_mode_change();
            }
            Target::ListGrouping => {
                use claude_switch_core::grouping::Mode;
                match self.list_mode() {
                    Mode::Projects => self.config.group_by_folder ^= true,
                    Mode::Recent => self.config.show_sittings ^= true,
                    _ => return,
                }
                self.save_config();
                self.rebuild_view();
                self.reveal_selected();
            }
            Target::ThreadToggle(i) => self.toggle_thread(i),
            Target::Related(i) if self.selected == self.grouping.related_id => {
                if let Some((id, _)) = self.grouping.related.get(i).cloned() {
                    self.select(&id);
                    self.reveal_selected();
                }
            }
            Target::ListFilter(filter) => {
                if filter != self.filter {
                    self.archive_picks.clear();
                }
                self.filter = filter;
                self.rebuild_view();
                self.sections.scroll = 0.0;
                if filter != list::model::Filter::Unseen
                    && self.selected.as_deref().is_none_or(|id| self.entry_of(id).is_none())
                {
                    self.select_first();
                }
            }
            Target::Pin(e) => {
                if let Some(i) = self.session_at(e) {
                    self.toggle_pin(i);
                }
            }
            Target::UndoHide => self.undo_hidden(now),
            Target::InfoClose => self.info = None,
            Target::Info(topbar::RESET_SETTINGS) => self.reset_settings(now),
            Target::Dialog(id) => dialogs::click(self, now, id, x),
            _ => {}
        }
    }

    fn after_search(&mut self) {
        if self.conversation_search.on {
            self.restart_conversation_search(std::time::Instant::now());
        }
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

    fn dropdown_outcome(&mut self, now: std::time::Instant, o: DdOutcome) {
        if o == DdOutcome::Opened {
            self.usage.popup = Default::default();
            self.request_usage(now, Demand::Automatic, None);
        }
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
            let anchor = match kind {
                MenuKind::Main => self.rect_of(Target::MenuButton),
                MenuKind::OpenAs => self.rect_of(Target::OpenAs),
                MenuKind::Row => Some(Rect::new(self.mouse.0, self.mouse.1, 0.0, 0.0)),
                MenuKind::Tab(tab) => return self.tab_menu_chosen(cx.now(), tab, id),
                MenuKind::Chat(tab, mode) => return self.chat_menu_chosen(cx, tab, mode, id),
            };
            self.command(cx.now(), id, anchor);
        }
    }

    fn key(&mut self, cx: &mut Cx<'_, Msg>, k: KeyPress) {
        let now = cx.now();
        let Some(name) = k.name.clone() else {
            // A key the toolkit has no name for may still have typed text for a program.
            if !self.chat_key(cx, &k) && self.terminal_shown() && self.nothing_over() && !self.picture_overlay() {
                self.term_key(now, &k);
            }
            return;
        };
        let (mods, text) = (k.mods, k.text.as_deref());
        if self.source.is_demo() && self.usage.demo_echo && name == KeyName::F(8) && mods.none() {
            self.open_usage(now, false);
            return;
        }
        let bar = Bar::layout(cx.size().0);
        if crate::view::chat::layout_shortcut(
            &name,
            mods,
            self.palette.is_some(),
            self.tabs.shown().is_some_and(|tab| tab.pane.is_some()),
        ) {
            self.palette = None;
            self.set_chat_layout(!self.chat_active());
            return;
        }
        if self.palette.is_some()
            && let Some(id) = crate::tabcontrols::number(&name, mods, false).and_then(|n| self.tabs.numbered(n))
        {
            self.palette = None;
            self.show_tab(id);
            return;
        }
        if let Some(p) = &mut self.palette {
            let o = p.key(&name, mods, text);
            self.palette_outcome(cx, o);
            return;
        }
        if self.accounts_dd.is_open() {
            let o = self.accounts_dd.key(bar.account, &name, mods);
            self.dropdown_outcome(now, o);
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
        if self.usage.popup.takes_keyboard() {
            match name {
                KeyName::Escape | KeyName::Tab => self.usage.popup = Default::default(),
                KeyName::Enter if mods.shift => self.activate(cx, Target::UsageOpenAs, 0.0),
                KeyName::Enter | KeyName::Space => self.activate(cx, Target::UsageRefresh, 0.0),
                _ => {}
            }
            return;
        }
        if self.session_key(cx.size().0, &name, mods) {
            return;
        }
        // The shortcuts that stay with the window in every tab, then a terminal's own keys.
        if let Some(number) = crate::tabcontrols::number(&name, mods, self.terminal_shown()) {
            if let Some(id) = self.tabs.numbered(number) {
                self.show_tab(id);
            }
            return;
        }
        let shortcut = termkeys::shortcut(&name, mods);
        if shortcut == Some(actions::SESSION_VIEW) && self.chat_key(cx, &k) {
            return;
        }
        if let Some(id) = shortcut
            && (id != actions::SESSION_VIEW || self.tabs.shown().is_some_and(|t| t.pane.is_some()))
        {
            self.command(now, id, None);
            return;
        }
        if self.terminal_shown() {
            if !self.picture_overlay() && !self.chat_key(cx, &k) {
                self.term_key(now, &k);
            }
            Self::cursor_moved(cx);
            return;
        }
        if matches!(name, KeyName::Char('f' | 'F')) && mods.ctrl && mods.shift && !mods.alt {
            self.key_used = true;
            self.toggle_conversation_search(now);
            return;
        }
        if name == KeyName::Escape && self.conversation_search.on && !self.search.is_empty() {
            self.search.clear();
            self.after_search();
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
                if name == KeyName::Enter && mods.shift && self.rect_of(Target::UsageLines).is_some() {
                    self.open_usage(now, true);
                } else {
                    let o = self.accounts_dd.key(bar.account, &name, mods);
                    self.dropdown_outcome(now, o);
                }
            }
            Focus::List => {
                if name == KeyName::Space
                    && self
                        .selected
                        .as_ref()
                        .and_then(|id| self.by_id.get(id))
                        .copied()
                        .is_some_and(|i| self.toggle_archive_pick(i))
                {
                    self.key_used = true;
                } else if name == KeyName::Delete && !mods.ctrl && !mods.alt && !mods.shift {
                    self.hide_from_keyboard(now);
                } else if list::key(self, &name) {
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
            self.dropdown_outcome(cx.now(), o);
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
            dialogs::settings_event(self, o);
            return;
        }
        if self.dialog.is_some() {
            self.pressed = hover;
            if hover == Some(Target::Dialog(dialogs::FIELD)) {
                dialogs::click(self, cx.now(), dialogs::FIELD, x);
            }
            return;
        }
        if matches!(hover, Some(Target::UsageLines | Target::UsageRefresh | Target::UsageOpenAs | Target::UsagePopup)) {
            self.pressed = hover;
            return;
        }
        self.usage.popup = Default::default();
        if self.chat_press(cx, hover, x, y, clicks) {
            self.clear_session_text_focus();
            return;
        }
        if self.session_press(cx, x, y, clicks) {
            return;
        }
        match hover {
            Some(Target::Tab(id)) => {
                self.show_tab(id);
                self.begin_tab_drag(id, x, y);
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
                self.dropdown_outcome(cx.now(), o);
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
                if cx.mods().ctrl
                    && let Some(i) = self.session_at(e)
                    && self.toggle_archive_pick(i)
                {
                    return;
                }
                let row = self.session_at(e).is_some();
                list::click(self, e);
                if clicks == 2 && row {
                    self.command(cx.now(), actions::OPEN_LAST, None);
                }
            }
            Some(t) => self.pressed = Some(t),
            None => {}
        }
    }

    fn release(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) {
        if self.chat_end_drag(Some((x, y))) {
            return;
        }
        if let Some(t @ Target::Chat(_)) = self.pressed {
            self.pressed = None;
            if self.nothing_over() && self.hit(x, y) == Some(t) {
                self.activate(cx, t, x);
            }
            return;
        }
        if self.tabs.drag.take().is_some() {
            return;
        }
        if matches!(
            self.pressed,
            Some(Target::UsageLines | Target::UsageRefresh | Target::UsageOpenAs | Target::UsagePopup)
        ) {
            if let Some(t) = self.pressed.take().filter(|t| self.hit(x, y) == Some(*t)) {
                self.activate(cx, t, x);
            }
            return;
        }
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
            self.dropdown_outcome(cx.now(), o);
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
            dialogs::settings_event(self, o);
            return;
        }
        if let Some(t) = self.pressed.take()
            && self.hit(x, y) == Some(t)
        {
            self.activate(cx, t, x);
        }
    }
}
