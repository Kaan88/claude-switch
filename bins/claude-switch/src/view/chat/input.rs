use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::transcript::ItemKind;
use mdshared_ui::host::{Cx, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::{Chat, Chip, Drawer, Side, clipboard, recall, submission};
use crate::state::{AppState, Msg, Target};
use crate::tabs::Tab;

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
    Compact,
    Clear,
    Commands,
    Suggestion(usize),
    Argument(usize, usize),
    PinCommand(usize),
    CommandBackground,
    Stop,
    Remove(usize),
    MoreChips,
}

impl AppState {
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
        self.clear_session_text_focus();
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        if pane.chat.enabled != enabled {
            tab.term.stop_pointer();
            pane.chat.enabled = enabled;
            pane.chat.drawer_focus = false;
            pane.chat.resizing = None;
            pane.chat.commands.open = false;
            pane.chat.interrupt_submission();
            if enabled {
                pane.ui.chat_conversation(&mut pane.model);
            } else {
                pane.ui.set_chat(false);
            }
            self.win_file.chat_default = enabled;
            self.save_window();
        }
    }

    pub fn chat_input_tick(&mut self, cx: &mut Cx<'_, Msg>) {
        let active = self.tabs.active;
        let may_write_shown =
            self.nothing_over() && !self.picture_overlay() && cx.window().is_minimized() != Some(true);
        let now = cx.now();
        let pending = self.tabs.list.iter().any(Self::has_chat_work);
        // A transcript worker or timer may beat the PTY wake; check the latest screen before writing.
        if pending {
            let woke = self.pump_terms();
            if woke.output || woke.strip || woke.all {
                cx.request_redraw_all();
            }
        }
        if self.progress_chat_tabs(now, may_write_shown) {
            cx.request_redraw_all();
        }
        for tab in &self.tabs.list {
            let Some(pane) = &tab.pane else { continue };
            if Self::has_chat_work(tab) || Some(tab.id) == active && pane.chat.enabled && pane.chat.commands.loading() {
                cx.wake_at(now + SUBMISSION_POLL);
            }
            if let Some(deadline) = submission::stage_deadline(&pane.chat) {
                cx.wake_at(deadline);
            }
            if Some(tab.id) == active
                && pane.chat.recovery_notice().is_some()
                && let Some(quiet) = pane.chat.last_output.map(|at| at + super::QUIET).filter(|at| *at > now)
            {
                cx.wake_at(quiet);
            }
        }
    }

    pub(super) fn progress_chat_tabs(&mut self, now: Instant, may_write_shown: bool) -> bool {
        let active = self.tabs.active;
        let mut redraw = false;
        for tab in &mut self.tabs.list {
            let shown = Some(tab.id) == active;
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
            let catalog = chat.commands.collect();
            let decision = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
            let changed = chat.react(decision.1) | (chat.last_gate.replace(decision.0) != Some(decision.0));
            let may_write = !shown || may_write_shown && !chat.drawer_focus;
            let advanced = Self::advance_submission(tab, now, may_write);
            redraw |= shown && (changed || catalog || reported || advanced || collected);
        }
        redraw
    }

    fn has_chat_work(tab: &Tab) -> bool {
        tab.pane
            .as_ref()
            .is_some_and(|pane| pane.chat.enabled && (pane.chat.queue.is_some() || pane.chat.waiting().is_some()))
    }

    pub(super) fn arm_chat_tick(&self, cx: &mut Cx<'_, Msg>) {
        if self.tabs.list.iter().any(Self::has_chat_work) {
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
        if mods.none()
            && let Some(key) = k.name.as_ref()
            && self.chat_picker_key(cx, key)
        {
            return true;
        }
        let Some(tab) = self.tabs.shown_mut() else { return true };
        let pane = tab.pane.as_mut().expect("Chat pane");
        let chat = &mut pane.chat;
        let mut changed = false;
        match k.name.as_ref() {
            Some(KeyName::Enter) if !mods.shift && !mods.ctrl && !mods.alt => self.chat_control(cx, Control::Send),
            Some(KeyName::Char('c' | 'C')) if mods.ctrl && !mods.alt => chat.editor_copy(false),
            Some(KeyName::Char('a' | 'A')) if mods.ctrl && !mods.alt => chat.editor.select_all(),
            _ if chat.locked() => {}
            Some(KeyName::Char('x' | 'X')) if mods.ctrl && !mods.alt => {
                chat.editor_copy(true);
                changed = true;
            }
            Some(KeyName::Char('v' | 'V')) if mods.ctrl && !mods.alt => {
                chat.paste_clipboard(mods.shift);
                changed = true;
            }
            Some(KeyName::Up | KeyName::Down)
                if !mods.shift && !mods.ctrl && !mods.alt && (chat.editor.text.is_empty() || chat.recall.is_some()) =>
            {
                let history = pane.model.items.values().rev().filter_map(|item| match &item.kind {
                    ItemKind::Prompt { text, synthetic: false, .. } => Some(text.as_str()),
                    _ => None,
                });
                if let Some(prompt) = recall(history, &mut chat.recall, k.name == Some(KeyName::Up)) {
                    chat.editor.set(prompt.to_owned());
                    chat.editor.move_to(prompt.len(), false);
                }
            }
            Some(name) => {
                chat.editor.key(name, mods, k.text.as_deref());
                changed = chat.editor.rows.is_empty();
            }
            None => {
                if let Some(text) = &k.text {
                    chat.editor.insert(text);
                    changed = true;
                }
            }
        }
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
        if !chat.locked() {
            chat.editor.insert(text);
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
        chat.drawer_focus = target == Some(Target::Terminal);
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
            Control::Model | Control::Effort => return self.chat_choice_menu(control),
            Control::Context => return self.chat_run_slash(cx, "/context".into(), false),
            Control::Compact => return self.chat_run_slash(cx, "/compact".into(), false),
            Control::Clear => return self.chat_run_slash(cx, "/clear".into(), false),
            Control::Commands | Control::Suggestion(_) | Control::Argument(_, _) | Control::PinCommand(_) => {
                return self.chat_picker_action(cx, control);
            }
            _ => {}
        }
        if matches!(control, Control::Send | Control::Override | Control::SendNow | Control::PressEnter) {
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
            Control::Detail(hit) => {
                if super::side::activate(pane, tab.id, hit) {
                    self.session_view.text_focus = true;
                }
            }
            Control::Remove(i) if i < chat.chips.len() && !chat.locked() => {
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
                    chat.chips.push(chip);
                }
            }
            chat.chip_page = chat.attachment_pages() - 1;
        }
    }
}

impl Chat {
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
            self.editor.insert(&text);
        } else if let Some(picture) = picture() {
            self.chips.retain(|c| !matches!(c, Chip::Clipboard(_)));
            self.chips.push(Chip::Clipboard(picture));
            if self.error == Some(submission::CHANGED_CLIPBOARD) {
                self.error = None;
            }
        } else if picture_only {
            self.error = Some("The clipboard doesn’t contain an available picture.");
            self.retry = None;
        }
        self.recall = None;
    }

    pub(super) fn editor_copy(&mut self, cut: bool) {
        let selection = self.editor.selection();
        if !selection.is_empty() && platform::set_clipboard_text(&self.editor.text[selection]) && cut {
            self.editor.insert("");
        }
    }

    pub(super) fn editor_point(&self, x: f32, y: f32) -> (f32, f32) {
        let rect = self.layout.editor;
        (x - rect.x - 8.0, y - rect.y)
    }
}
