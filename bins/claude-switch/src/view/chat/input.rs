use std::path::PathBuf;
use std::time::{Duration, Instant};

use mdshared_ui::host::{Cx, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::{Chat, Chip, Drawer, Side, clipboard, submission};
use crate::state::{AppState, Msg, Target};
use crate::tabs::Tab;
use crate::view::model::Model;

const SUBMISSION_POLL: Duration = Duration::from_millis(250);

pub fn layout_shortcut(name: &KeyName, mods: Mods, palette: bool, session_tab: bool) -> bool {
    palette && session_tab && *name == KeyName::F(6) && mods.none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Drawer,
    Open,
    Grip,
    Panel,
    Side(Side),
    Detail(super::side::Hit),
    Shells,
    Editor,
    Attach,
    Send,
    Override,
    SendNow,
    PressEnter,
    CancelSend,
    Model,
    Effort,
    Context,
    Mode,
    Compact,
    Clear,
    Commands,
    /// Own command `n` from Settings › Chat.
    Own(usize),
    Suggestion(usize),
    Argument(usize, usize),
    PinCommand(usize),
    CommandBackground,
    /// A row of the `@` or Ctrl+R pop-up.
    Popup(usize),
    /// The CLI's suggested next prompt.
    NextPrompt,
    Stop,
    Remove(usize),
    MoreChips,
    Question(super::question::Hit),
}

impl AppState {
    pub(in crate::view) fn select_shell(&mut self, id: Option<claude_switch_core::transcript::ItemId>) {
        let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) else { return };
        if id.is_some_and(|id| !pane.model.shells.iter().any(|shell| shell.tool == id)) {
            return;
        }
        pane.chat.details.reset_scroll();
        if let Some(id) = id {
            pane.chat.details.select_shell(id, pane.model.generation);
        }
        if pane.chat.enabled {
            pane.chat.side = Side::Shells;
            pane.chat.side_hidden = false;
        } else {
            pane.ui.show_details(Side::Shells);
        }
    }

    fn detail_control(&mut self, hit: super::side::Hit) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let side = if pane.chat.enabled {
            pane.chat.side
        } else if let Some(side) = pane.ui.detail_side() {
            side
        } else {
            return;
        };
        match super::side::activate(pane, tab.id, side, hit) {
            super::side::Activated::None => {}
            super::side::Activated::Focus => self.session_view.text_focus = true,
            super::side::Activated::Stop(id) => self.ask_shell_manager(id),
        }
    }

    pub fn chat_paste(&mut self) -> bool {
        if !self.chat_takes_input() {
            return false;
        }
        let chat = &mut self.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
        if !chat.locked() {
            chat.paste_clipboard(false);
            self.chat_editor_changed();
        }
        true
    }

    pub fn chat_active(&self) -> bool {
        self.tabs.shown().and_then(|tab| tab.pane.as_ref()).is_some_and(|pane| pane.chat.enabled)
    }

    pub fn chat_takes_input(&self) -> bool {
        self.tabs
            .shown()
            .and_then(|tab| tab.pane.as_ref())
            .is_some_and(|pane| pane.chat.enabled && !(pane.chat.drawer_focus && pane.chat.drawer != Drawer::Closed))
    }

    pub fn set_chat_layout(&mut self, enabled: bool) {
        if self.set_shown_tab_chat(enabled) {
            self.win_file.chat_default = enabled;
            self.save_window();
        }
    }

    /// Switches only the shown tab between Chat and the terminal, leaving the saved default
    /// alone. True when it changed.
    pub fn set_shown_tab_chat(&mut self, enabled: bool) -> bool {
        self.clear_session_text_focus();
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let Some(pane) = &mut tab.pane else { return false };
        if pane.chat.enabled != enabled {
            tab.term.stop_pointer();
            pane.chat.enabled = enabled;
            pane.chat.drawer_focus = false;
            pane.chat.resizing = None;
            pane.chat.commands.open = false;
            pane.chat.recall.cancel_load();
            pane.chat.interrupt_submission();
            if enabled {
                pane.ui.chat_conversation(&mut pane.model);
            } else {
                pane.ui.set_chat(false);
            }
            return true;
        }
        false
    }

    pub fn chat_input_tick(&mut self, cx: &mut Cx<'_, Msg>) {
        let active = self.tabs.active;
        let now = cx.now();
        let pending = self.tabs.list.iter().any(Self::has_chat_work);
        // A transcript worker or timer may beat the PTY wake; check the latest screen before writing.
        if pending {
            let woke = self.pump_terms();
            if woke.output || woke.strip || woke.all {
                cx.request_redraw_all();
            }
        }
        let window_allows_writes = self.chat_window_allows_writes(cx.window().is_minimized() == Some(true));
        if self.progress_chat_tabs(now, window_allows_writes) {
            cx.request_redraw_all();
        }
        if window_allows_writes && self.send_cli_rename(now) {
            cx.request_redraw_all();
        }
        if window_allows_writes && self.in_place_tick(now) {
            cx.request_redraw_all();
        }
        if self.cli_rename_waiting() || self.in_place_waiting() {
            cx.wake_at(now + SUBMISSION_POLL);
        }
        for tab in &self.tabs.list {
            let Some(pane) = &tab.pane else { continue };
            if Self::has_chat_work(tab) || Some(tab.id) == active && pane.chat.enabled && pane.chat.commands.loading() {
                cx.wake_at(now + SUBMISSION_POLL);
            }
            if let Some(deadline) = submission::stage_deadline(&pane.chat, now) {
                cx.wake_at(deadline);
            }
            // The gate reads the screen's rest: look again when the quiet time is up.
            if Some(tab.id) == active
                && pane.chat.enabled
                && let Some(settled) = pane.chat.next_settle(now)
            {
                cx.wake_at(settled);
            }
        }
    }

    pub(super) fn chat_window_allows_writes(&self, minimized: bool) -> bool {
        !minimized && self.nothing_over() && !self.picture_overlay()
    }

    pub(super) fn progress_chat_tabs(&mut self, now: Instant, window_allows_writes: bool) -> bool {
        let active = self.tabs.active;
        let menus_off = !self.config.chat.answer_menus;
        let mut redraw = false;
        for tab in &mut self.tabs.list {
            let shown = Some(tab.id) == active;
            if let Some(pane) = &mut tab.pane {
                pane.chat.menus_off = menus_off;
            }
            if !shown && !Self::has_chat_work(tab) {
                continue;
            }
            let Some(pane) = &mut tab.pane else { continue };
            let collected = if !shown {
                if pane.next.is_none_or(|at| at <= now) {
                    pane.model.poll();
                    pane.next = Some(now + SUBMISSION_POLL);
                }
                pane.model.collect()
            } else {
                false
            };
            let chat = &mut pane.chat;
            if pane.model.error.is_some() {
                chat.invalidate(&pane.model.chat);
            } else if pane.model.session_id().is_some() {
                chat.observe(&pane.model.chat, now);
            }
            if !chat.enabled {
                continue;
            }
            let reported = chat.observe_cli(&pane.model.chat, &tab.term.screen, now);
            let catalog = chat.commands.collect() | chat.popups.collect();
            let suggested = shown && chat.popups.read_next_prompt(&tab.term.screen);
            // The card first: the gate asks it whether it answers the pending call's menu.
            let card = chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen);
            chat.settle(chat.screen_idle(&tab.term, now));
            let decision = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
            let tool = decision.1 == Some(super::Reason::Tool) && chat.reason != Some(super::Reason::Tool);
            let changed = card | chat.react(decision.1) | (chat.last_gate.replace(decision.0) != Some(decision.0));
            let may_write = window_allows_writes && (!shown || !chat.drawer_focus);
            let recalled = !chat.locked()
                && chat.recall.advance(&mut chat.editor, &pane.model, shown && may_write && !chat.commands.open);
            let advanced = Self::advance_submission(tab, now, may_write);
            if tool {
                // Chat says Claude Code may be waiting and cannot show why: keep what it read.
                super::screen_report::save(tab, now, super::Reason::Tool.label());
            }
            redraw |= shown && (changed || catalog || suggested || reported || advanced || collected || recalled);
        }
        redraw
    }

    fn has_chat_work(tab: &Tab) -> bool {
        tab.pane
            .as_ref()
            .is_some_and(|pane| pane.chat.enabled && (pane.chat.queue.is_some() || pane.chat.waiting().is_some()))
    }

    pub(in crate::view) fn arm_chat_tick(&self, cx: &mut Cx<'_, Msg>) {
        let listing = self.tabs.shown().and_then(|t| t.pane.as_ref()).is_some_and(|p| p.chat.popups.loading_files());
        if listing || self.tabs.list.iter().any(Self::has_chat_work) {
            cx.wake_at(cx.now() + SUBMISSION_POLL);
        }
    }

    /// A key over Chat is either for the editor or for the explicitly focused drawer.
    pub fn chat_key(&mut self, cx: &mut Cx<'_, Msg>, k: &KeyPress) -> bool {
        if !self.chat_active() || !self.nothing_over() || self.picture_overlay() {
            return false;
        }
        let mods = k.mods;
        // Reuse the app's already-reserved pane shortcut; no new chord steals PTY input.
        if crate::termkeys::shortcut(k.name.as_ref().unwrap_or(&KeyName::Escape), mods)
            == Some(crate::actions::SESSION_VIEW)
        {
            self.chat_control(cx, Control::Drawer);
            return true;
        }
        if !self.chat_takes_input() {
            return false;
        }
        self.clear_session_text_focus();
        if let Some(name) = k.name.as_ref()
            && let Some(tab) = self.tabs.shown_mut()
            && super::question::key(tab, cx.now(), name, mods, k.text.as_deref())
        {
            self.arm_chat_tick(cx);
            return true;
        }
        if mods.none()
            && let Some(key) = k.name.as_ref()
            && self.chat_picker_key(cx, key)
        {
            return true;
        }
        if let Some(key) = k.name.as_ref()
            && let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut())
            && pane.chat.popup_key(&pane.model, key, mods, k.text.as_deref())
        {
            return true;
        }
        let Some(tab) = self.tabs.shown_mut() else { return true };
        let pane = tab.pane.as_mut().expect("Chat pane");
        let chat = &mut pane.chat;
        let changed = match k.name.as_ref() {
            Some(KeyName::Enter) if !mods.shift && !mods.ctrl && !mods.alt => {
                self.chat_control(cx, Control::Send);
                false
            }
            name => chat.editor_key(&pane.model, name, mods, k.text.as_deref()),
        };
        if changed {
            self.chat_editor_changed();
        }
        true
    }

    pub fn chat_text(&mut self, _cx: &mut Cx<'_, Msg>, text: &str) -> bool {
        if !self.chat_takes_input() {
            return false;
        }
        let chat = &mut self.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
        if super::question::text(chat, text) || chat.popup_text(text) {
            return true;
        }
        if !chat.locked() {
            chat.insert_text(text);
            self.chat_editor_changed();
        }
        true
    }

    pub fn note_chat_terminal_input(&mut self) {
        if let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) {
            pane.note_terminal_input();
        }
    }

    pub fn chat_press(&mut self, cx: &Cx<'_, Msg>, target: Option<Target>, x: f32, y: f32, clicks: u32) -> bool {
        if !self.chat_active() || self.picture_overlay() {
            return false;
        }
        let Some(tab) = self.tabs.shown_mut() else { return false };
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.recall.cancel_load();
        if !matches!(
            target,
            Some(Target::Chat(
                Control::Editor
                    | Control::Commands
                    | Control::Suggestion(_)
                    | Control::Argument(_, _)
                    | Control::PinCommand(_)
                    | Control::CommandBackground
                    | Control::Send
            ))
        ) {
            chat.commands.open = false;
        }
        if !matches!(target, Some(Target::Chat(Control::Editor | Control::Popup(_) | Control::CommandBackground))) {
            chat.popups.close();
        }
        chat.drawer_focus = target == Some(Target::Terminal);
        chat.question.focus &= matches!(target, Some(Target::Chat(Control::Question(_))));
        match target {
            Some(Target::Chat(Control::Grip)) => {
                chat.resizing = Some((y, chat.layout.drawer.h));
                chat.drawer_focus = true;
                true
            }
            Some(Target::Chat(Control::Editor)) => {
                let (x, y) = chat.editor_point(x, y);
                if clicks >= 3 {
                    chat.editor.select_all();
                } else if clicks == 2 {
                    chat.editor.word_at(x, y);
                } else {
                    chat.editor.point(x, y, cx.mods().shift);
                    chat.editor_drag = true;
                }
                true
            }
            Some(Target::Chat(Control::CommandBackground)) => true,
            Some(Target::Chat(_)) => {
                self.pressed = target;
                true
            }
            _ => false,
        }
    }

    pub fn chat_move(&mut self, x: f32, y: f32) -> Option<bool> {
        if !self.chat_active() || self.picture_overlay() || !self.nothing_over() {
            return None;
        }
        let chat = &mut self.tabs.shown_mut()?.pane.as_mut()?.chat;
        if chat.resize_drawer(y) {
            return Some(true);
        }
        if chat.editor_drag {
            let (x, y) = chat.editor_point(x, y);
            let changed = chat.editor.point(x, y, true);
            return Some(changed);
        }
        if chat.commands.open && chat.layout.suggestions.contains(x, y) {
            return Some(false);
        }
        None
    }

    pub fn chat_end_drag(&mut self, point: Option<(f32, f32)>) -> bool {
        self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()).is_some_and(|pane| pane.chat.end_drag(point))
    }

    pub fn chat_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) else { return false };
        let chat = &mut pane.chat;
        if !chat.enabled {
            return false;
        }
        if chat.commands.open && chat.layout.suggestions.contains(x, y) {
            chat.commands.step(&chat.editor.text, &self.win_file.chat_pins, dy < 0.0);
        } else if chat.popups.open.is_some() && chat.layout.suggestions.contains(x, y) {
            let count = chat.popup_items(&pane.model).map_or(0, |(_, items)| items.len());
            chat.popups.step(count, dy < 0.0);
        } else if chat.question_wheel(x, y, dy) {
            // A permission prompt longer than its card scrolls.
        } else if chat.layout.editor.contains(x, y) {
            chat.editor.wheel(dy);
        } else if chat.layout.side.w > 0.0 && chat.layout.side.contains(x, y) {
            chat.details.wheel(dy);
        } else {
            return false;
        }
        true
    }

    pub fn chat_control(&mut self, cx: &mut Cx<'_, Msg>, control: Control) {
        match control {
            Control::Stop => return self.stop_conversation(cx, None),
            Control::Detail(hit) => return self.detail_control(hit),
            Control::Shells => {
                self.select_shell(None);
                return;
            }
            Control::Model | Control::Effort => return self.chat_choice_menu(control),
            Control::Question(hit) => {
                // As Send: read the newest screen before matching the question on it.
                self.term_woke(cx);
                if let Some(tab) = self.tabs.shown_mut().filter(|tab| tab.pane.as_ref().is_some_and(|p| p.chat.enabled))
                {
                    super::question::press(tab, cx.now(), hit);
                }
                return self.arm_chat_tick(cx);
            }
            Control::Context => return self.chat_run_slash(cx, "/context".into(), false),
            Control::Mode => {
                // As Send: read the newest screen before the checks.
                self.term_woke(cx);
                if let Some(tab) = self.tabs.shown_mut().filter(|tab| tab.pane.as_ref().is_some_and(|p| p.chat.enabled))
                {
                    AppState::chat_mode(tab, cx.now());
                }
                return self.arm_chat_tick(cx);
            }
            Control::Compact => return self.chat_run_slash(cx, "/compact".into(), false),
            Control::Clear => return self.chat_run_slash(cx, "/clear".into(), false),
            Control::Own(n) => {
                let Some(line) = self.config.chat.commands.get(n).filter(|c| !c.is_empty()).cloned() else { return };
                return self.chat_run_slash(cx, line, false);
            }
            Control::Commands | Control::Suggestion(_) | Control::Argument(_, _) | Control::PinCommand(_) => {
                return self.chat_picker_action(cx, control);
            }
            Control::Popup(i) => {
                if let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) {
                    pane.chat.popup_choose(&pane.model, i);
                }
                return;
            }
            Control::NextPrompt => {
                if let Some(pane) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()) {
                    pane.chat.fill_next_prompt();
                }
                return;
            }
            _ => {}
        }
        if matches!(control, Control::Send | Control::Override | Control::SendNow | Control::PressEnter) {
            self.end_chat_recall();
            // Don't let a pre-click tick submit the queue before processing an explicit Send now.
            // Stop returned above through `stop_conversation`, which wakes the terminals itself.
            self.term_woke(cx);
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        if !chat.enabled {
            return;
        }
        if control == Control::Send && !chat.locked() && chat.editor.text.trim_start().starts_with('/') {
            let line = chat.editor.text.clone();
            self.chat_run_slash(cx, line, true);
            return;
        }
        match control {
            Control::Drawer => chat.toggle_drawer(),
            Control::Open if chat.drawer == Drawer::Closed => chat.open_drawer(),
            Control::Open => chat.drawer_focus = true,
            Control::Panel => chat.side_hidden = !chat.side_hidden,
            Control::Side(side) => {
                chat.side = side;
                chat.details.reset_scroll();
            }
            Control::Remove(i) if i < chat.chips.len() && !chat.locked() => {
                chat.recall.end();
                let clipboard = matches!(chat.chips.remove(i), Chip::Clipboard(_));
                chat.chip_page = chat.chip_page.min(chat.attachment_pages() - 1);
                if clipboard && chat.error == Some(submission::CHANGED_CLIPBOARD) {
                    chat.error = None;
                }
                if chat.chips.is_empty() && chat.error == Some(submission::REMOVE_ATTACHMENTS) {
                    chat.error = None;
                }
            }
            Control::MoreChips => chat.chip_page = (chat.chip_page + 1) % chat.attachment_pages(),
            Control::Attach if !chat.locked() => self.chat_attach_menu(),
            Control::CancelSend => chat.stop_waiting(),
            Control::Send | Control::Override | Control::SendNow
                if control == Control::Send
                    || (control == Control::SendNow && chat.queue.is_some())
                    || (control == Control::Override
                        && chat.queue.is_none()
                        && chat.error == Some(submission::CHECK)) =>
            {
                Self::chat_send(tab, cx.now(), control != Control::Send);
            }
            Control::PressEnter => Self::retry_enter(tab, cx.now()),
            _ => {}
        }
        self.arm_chat_tick(cx);
    }

    pub(super) fn end_chat_recall(&mut self) {
        // A pre-send tick can finish an older-history load; disarm it before refreshing the terminal.
        if let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) {
            pane.chat.recall.end();
        }
    }

    pub(in crate::view) fn stop_conversation(
        &mut self,
        cx: &mut Cx<'_, Msg>,
        expected: Option<crate::view::working::Turn>,
    ) {
        self.term_woke(cx);
        if !self.nothing_over() || self.picture_overlay() || self.usage.popup.open {
            return;
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let info = &pane.model.chat;
        if pane.model.error.is_none()
            && expected.is_none_or(|key| key.0 == info.generation && Some(key.1) == info.prompt)
            && pane.chat.stop_ready(info, &tab.term, &tab.program_title, cx.now())
        {
            // One Stop path for the button and the working line: it also lets a queued draft
            // learn that the turn was interrupted (submission.rs).
            Self::chat_stop(tab, cx.now());
            self.update_working(cx);
        }
    }

    pub fn chat_attach(&mut self, id: u64, files: &[PathBuf]) {
        if let Some(pane) = self.tabs.get_mut(id).and_then(|t| t.pane.as_mut()) {
            let chat = &mut pane.chat;
            if chat.locked() {
                chat.error = Some("Cancel the queued or pending send before adding files.");
                chat.retry = None;
                return;
            }
            for path in files {
                if super::super::attach::paths(std::slice::from_ref(path)).is_empty() {
                    chat.error = Some("A file path could not be attached.");
                    continue;
                }
                let chip = Chip::Path(path.clone());
                if !chat.chips.contains(&chip) {
                    chat.recall.end();
                    chat.chips.push(chip);
                }
            }
            chat.chip_page = chat.attachment_pages() - 1;
        }
    }
}

impl Chat {
    /// A key for the message box (Enter to send is the caller's). Modifiers reach the editor
    /// whole: Shift extends, Ctrl moves and deletes by word. Up and Down recall earlier prompts
    /// only without modifiers. Returns whether the draft changed.
    pub(super) fn editor_key(&mut self, model: &Model, name: Option<&KeyName>, mods: Mods, text: Option<&str>) -> bool {
        let shortcut = mods.ctrl && !mods.alt;
        match name {
            Some(KeyName::Char('c' | 'C')) if shortcut => {
                self.editor_copy(false);
                false
            }
            Some(KeyName::Char('a' | 'A')) if shortcut => {
                self.recall.cancel_load();
                self.editor.select_all();
                false
            }
            _ if self.locked() => false,
            Some(KeyName::Char(c @ ('z' | 'Z' | 'y' | 'Y'))) if shortcut => {
                // Ctrl+Z undoes; Ctrl+Y and Ctrl+Shift+Z redo, as in Windows text boxes.
                self.undo_or_redo(matches!(c, 'y' | 'Y') || mods.shift)
            }
            Some(KeyName::Char('x' | 'X')) if shortcut => {
                self.editor_copy(true);
                true
            }
            Some(KeyName::Char('v' | 'V')) if shortcut => {
                self.paste_clipboard(mods.shift);
                true
            }
            Some(key @ (KeyName::Up | KeyName::Down | KeyName::Escape))
                if mods.none() && self.recall_key(model, key) =>
            {
                false
            }
            Some(name) => self.edit_key(name, mods, text),
            None => text.is_some_and(|text| {
                self.insert_text(text);
                true
            }),
        }
    }

    pub(super) fn edit_key(&mut self, key: &KeyName, mods: Mods, text: Option<&str>) -> bool {
        self.recall.cancel_load();
        let edited = self.editor.key(key, mods, text);
        if edited {
            self.recall.end();
        }
        edited
    }

    pub(super) fn insert_text(&mut self, text: &str) {
        self.recall.end();
        self.editor.insert(text);
    }

    pub(super) fn resize_drawer(&mut self, y: f32) -> bool {
        let Some((start, height)) = self.resizing else { return false };
        self.drawer_height = Some((height + start - y).max(52.0));
        true
    }

    pub(super) fn end_drag(&mut self, point: Option<(f32, f32)>) -> bool {
        if let Some((x, y)) = point {
            self.resize_drawer(y);
            if self.editor_drag {
                let (x, y) = self.editor_point(x, y);
                self.editor.point(x, y, true);
            }
        }
        self.resizing.take().is_some() | std::mem::take(&mut self.editor_drag)
    }

    pub(super) fn paste_clipboard(&mut self, picture_only: bool) {
        self.paste_from(picture_only, platform::clipboard_text, clipboard::picture);
    }

    pub(super) fn paste_from(
        &mut self,
        picture_only: bool,
        text: impl FnOnce() -> Option<String>,
        picture: impl FnOnce() -> Option<clipboard::Picture>,
    ) {
        if !picture_only && let Some(text) = text().filter(|s| !s.is_empty()) {
            self.recall.end();
            self.editor.paste(&text);
        } else if let Some(picture) = picture() {
            self.recall.end();
            self.chips.retain(|c| !matches!(c, Chip::Clipboard(_)));
            self.chips.push(Chip::Clipboard(picture));
            if self.error == Some(submission::CHANGED_CLIPBOARD) {
                self.error = None;
            }
        } else if picture_only {
            self.error = Some("The clipboard doesn’t contain an available picture.");
            self.retry = None;
        }
        self.recall.cancel_load();
    }

    /// Ctrl+Z, or redo. The restored draft is the one to send, so prompt browsing ends.
    pub(super) fn undo_or_redo(&mut self, redo: bool) -> bool {
        self.recall.cancel_load();
        let changed = if redo { self.editor.redo() } else { self.editor.undo() };
        if changed {
            self.recall.end();
        }
        changed
    }

    pub(super) fn editor_copy(&mut self, cut: bool) {
        self.editor_copy_to(cut, platform::set_clipboard_text);
    }

    /// Copy or Cut through `set_clipboard`; Cut removes the selection only once the clipboard took
    /// it. Returns whether the draft changed.
    pub(super) fn editor_copy_to(&mut self, cut: bool, set_clipboard: impl FnOnce(&str) -> bool) -> bool {
        let selection = self.editor.selection();
        if selection.is_empty() || !set_clipboard(&self.editor.text[selection]) || !cut {
            return false;
        }
        self.recall.end();
        self.editor.delete_selection()
    }

    pub(super) fn editor_point(&self, x: f32, y: f32) -> (f32, f32) {
        let rect = self.layout.editor;
        (x - rect.x - 8.0, y - rect.y)
    }
}
