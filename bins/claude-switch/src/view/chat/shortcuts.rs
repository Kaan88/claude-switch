use std::time::Instant;

use mdshared_ui::host::Cx;
use mdshared_ui::keys::KeyName;
use mdshared_ui::menu::{Item, Menu};

use super::{Chat, Control, commands};
use crate::dialogs::{Dialog, Pending};
use crate::state::{AppState, MenuKind, Msg, Target};
use crate::tabs::Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuMode {
    Editor,
    Attach,
    Model,
    Effort,
}

impl AppState {
    pub(super) fn chat_attach_menu(&mut self) {
        let Some(tab) = self.tabs.shown() else { return };
        let Some(anchor) = self.hits.iter().find_map(|(r, t)| (*t == Target::Chat(Control::Attach)).then_some(*r))
        else {
            return;
        };
        let items = vec![Item::new(0, "Choose files…"), Item::new(1, "Paste picture")];
        self.menu = Some((MenuKind::Chat(tab.id, MenuMode::Attach), Menu::new(items, 0.0, 0.0, false).around(anchor)));
    }

    pub fn chat_editor_menu(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        self.clear_session_text_focus();
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        if !chat.enabled {
            return;
        }
        chat.drawer_focus = false;
        chat.editor_drag = false;
        let (fx, fy) = chat.editor_point(x, y);
        let at = chat.editor.hit(fx, fy);
        if !chat.editor.selection().contains(&at) && chat.editor.selection().end != at {
            chat.editor.point(fx, fy, false);
        }
        let selected = !chat.editor.selection().is_empty();
        let items = vec![
            Item::new(0, "Cut").enabled(selected && !chat.locked()),
            Item::new(1, "Copy").enabled(selected),
            Item::new(2, "Paste").enabled(!chat.locked()),
            Item::new(3, "Select all").enabled(!chat.editor.text.is_empty()).sep(),
        ];
        self.menu = Some((MenuKind::Chat(tab.id, MenuMode::Editor), Menu::new(items, x, y, false)));
    }

    pub(super) fn chat_choice_menu(&mut self, control: Control) {
        let Some(tab) = self.tabs.shown() else { return };
        let choices = if control == Control::Model { &commands::MODELS[..] } else { &commands::EFFORT[..] };
        let mode = if control == Control::Model { MenuMode::Model } else { MenuMode::Effort };
        let mut items: Vec<_> = choices.iter().enumerate().map(|(i, label)| Item::new(i as u32, *label)).collect();
        if mode == MenuMode::Model {
            items.push(Item::new(choices.len() as u32, "Open the CLI’s own /model menu…").sep());
        }
        let Some(anchor) = self.hits.iter().find_map(|(r, t)| (*t == Target::Chat(control)).then_some(*r)) else {
            return;
        };
        self.menu = Some((MenuKind::Chat(tab.id, mode), Menu::new(items, 0.0, 0.0, false).around(anchor)));
    }

    pub fn chat_menu_chosen(&mut self, cx: &mut Cx<'_, Msg>, id: u64, mode: MenuMode, index: u32) {
        if self.tabs.active != Some(id) || !self.chat_active() {
            return;
        }
        let chat = &mut self.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
        match mode {
            MenuMode::Editor => {
                match index {
                    0 if !chat.locked() => chat.editor_copy(true),
                    1 => chat.editor_copy(false),
                    2 if !chat.locked() => chat.paste_clipboard(false),
                    3 => chat.editor.select_all(),
                    _ => (),
                }
                if matches!(index, 0 | 2) {
                    self.chat_editor_changed();
                }
            }
            MenuMode::Attach if !chat.locked() => {
                if index == 0 {
                    let cwd = self.tabs.shown().unwrap().pane.as_ref().unwrap().cwd.clone();
                    self.attach_picker(Some(&cwd), true);
                } else if index == 1 {
                    chat.paste_clipboard(true);
                }
            }
            MenuMode::Attach => {}
            MenuMode::Model if index as usize == commands::MODELS.len() => {
                self.chat_run_slash(cx, "/model".into(), false);
            }
            MenuMode::Model | MenuMode::Effort => {
                let name = if mode == MenuMode::Model { "model" } else { "effort" };
                if let Some(value) = commands::choices(name).get(index as usize) {
                    self.chat_run_slash(cx, format!("/{name} {value}"), false);
                }
            }
        }
    }

    pub(super) fn chat_open_picker(&mut self, open: bool) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        chat.commands.open = open;
        chat.commands.reset();
        if open {
            chat.recall.cancel_load();
            chat.drawer_focus = false;
            if let crate::source::Source::Real(roots) = &self.source {
                let account =
                    tab.account.as_deref().and_then(|slug| claude_switch_core::account(&self.config, slug).ok());
                chat.commands.load(
                    account.map(|account| account.cli_dir.clone()),
                    Some(roots.primary_cli_dir()),
                    pane.cwd.clone(),
                );
            }
        }
    }

    pub(super) fn chat_editor_changed(&mut self) {
        let Some(chat) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()).map(|p| &mut p.chat) else { return };
        chat.recall.cancel_load();
        if chat.error == Some(super::submission::BAD_TEXT)
            && !chat.editor.text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        {
            chat.error = None;
        }
        if chat.error == Some(super::submission::BAD_COMMAND)
            && (!chat.editor.text.trim_start().starts_with('/')
                || super::submission::slash_bytes(chat.editor.text.trim_matches(' ')).is_some())
        {
            chat.error = None;
        }
        let open = chat.editor.text.starts_with('/') && !chat.editor.text.contains('\n');
        self.chat_open_picker(open);
    }

    pub(super) fn chat_picker_key(&mut self, cx: &mut Cx<'_, Msg>, key: &KeyName) -> bool {
        let Some(chat) = self.tabs.shown_mut().and_then(|t| t.pane.as_mut()).map(|p| &mut p.chat) else { return false };
        if !chat.commands.open {
            return false;
        }
        let pins = &self.win_file.chat_pins;
        match key {
            KeyName::Escape => chat.commands.open = false,
            KeyName::Up | KeyName::Down => chat.commands.step(&chat.editor.text, pins, *key == KeyName::Down),
            KeyName::Tab => {
                if let Some(index) = chat.commands.chosen(&chat.editor.text, pins) {
                    chat.fill_suggestion(index);
                }
            }
            KeyName::Enter => {
                let draft = &chat.editor.text;
                let chosen = chat.commands.chosen(draft, pins).map(|i| chat.commands.catalog[i].name.as_str());
                let typed = draft.starts_with('/');
                let exact = chat.commands.catalog.iter().any(|c| c.name == commands::query(draft));
                let line = if let Some(name) =
                    chosen.filter(|_| chat.commands.moved || typed && !exact && !commands::query(draft).is_empty())
                {
                    commands::fill(name, draft)
                } else if typed {
                    draft.clone()
                } else {
                    chat.error = Some("Choose a command with the arrow keys or click a row.");
                    chat.retry = None;
                    return true;
                };
                self.chat_run_slash(cx, line, typed);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn chat_picker_action(&mut self, cx: &mut Cx<'_, Msg>, control: Control) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let chat = &mut pane.chat;
        if control == Control::Commands {
            let open = !chat.commands.open;
            self.chat_open_picker(open);
            return;
        }
        if !chat.commands.open {
            return;
        }
        match control {
            Control::Suggestion(i) => chat.fill_suggestion(i),
            Control::Argument(i, n) => {
                let Some(cmd) = chat.commands.catalog.get(i) else { return };
                let Some(value) = commands::choices(&cmd.name).get(n) else { return };
                let line = format!("/{} {value}", cmd.name);
                let typed = chat.editor.text.starts_with('/');
                self.chat_run_slash(cx, line, typed);
            }
            Control::PinCommand(i) => {
                let Some(cmd) = chat.commands.catalog.get(i) else { return };
                let old = self.win_file.chat_pins.clone();
                let pins = &mut self.win_file.chat_pins;
                if let Some(at) = pins.iter().position(|p| *p == cmd.name) {
                    pins.remove(at);
                } else if pins.len() < 128 {
                    pins.push(cmd.name.clone());
                } else {
                    chat.error = Some("Too many pinned commands. Unpin one first.");
                    chat.retry = None;
                    return;
                }
                chat.commands.reset();
                if let Err(err) = self.write_window() {
                    self.win_file.chat_pins = old;
                    self.error("Could not save command pins", err.to_string());
                }
            }
            _ => {}
        }
    }

    pub(super) fn chat_run_slash(&mut self, cx: &mut Cx<'_, Msg>, line: String, from_editor: bool) {
        self.term_woke(cx);
        let Some(tab) = self.tabs.shown_mut() else { return };
        if let Some(confirm) = Self::prepare_slash(tab, cx.now(), &line, from_editor) {
            self.dialog = Some(confirm);
        }
        self.arm_chat_tick(cx);
    }

    pub(super) fn prepare_slash(tab: &mut Tab, now: Instant, line: &str, from_editor: bool) -> Option<Dialog> {
        let pane = tab.pane.as_mut()?;
        let chat = &mut pane.chat;
        chat.commands.open = false;
        let line = line.trim_matches(' ');
        if from_editor && !chat.locked() && chat.editor.text != line {
            chat.editor.set(line.to_owned());
            chat.editor.move_to(line.len(), false);
        }
        if let Some(action) = commands::confirm_name(line) {
            Some(Dialog::Confirm {
                title: format!("{action} this conversation?"),
                text: if action == "Clear" {
                    "This asks Claude Code to start a new conversation with empty context.".into()
                } else {
                    "Claude Code will summarize the conversation to make room in its context. This can take a moment."
                        .into()
                },
                ok: action.into(),
                then: Pending::ChatSlash {
                    tab: tab.id,
                    session: tab.session.clone(),
                    line: line.to_owned(),
                    from_editor,
                },
            })
        } else {
            Self::submit_slash(tab, now, line, from_editor);
            None
        }
    }

    pub fn chat_confirm_slash(
        &mut self,
        now: Instant,
        id: u64,
        session: Option<String>,
        line: &str,
        from_editor: bool,
    ) {
        self.pump_terms();
        let Some(tab) = self.tabs.shown_mut().filter(|t| t.id == id) else { return };
        let Some(pane) = &mut tab.pane else { return };
        if tab.session != session || !pane.chat.enabled {
            pane.chat.error = Some("The session changed. Run the command again in the correct tab.");
            pane.chat.retry = None;
            return;
        }
        Self::submit_slash(tab, now, line, from_editor);
    }
}

impl Chat {
    fn fill_suggestion(&mut self, index: usize) {
        let Some(cmd) = self.commands.catalog.get(index) else { return };
        if self.locked() || !self.editor.text.trim().is_empty() && !self.editor.text.starts_with('/') {
            self.error = Some("Finish or clear the current draft before filling in a command.");
            self.retry = None;
            return;
        }
        let line = commands::fill(&cmd.name, &self.editor.text);
        let at = line.len();
        self.editor.set(line);
        self.editor.move_to(at, false);
        self.commands.reset();
    }
}
