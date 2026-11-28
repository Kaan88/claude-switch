//! Pointer and keyboard controls for the terminal tab strip.

use std::time::Instant;

use mdshared_ui::gfx::Rect;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::menu::{Item, Menu};

use crate::dialogs::{self, Dialog, NameFor, Pending};
use crate::state::{AppState, MenuKind, Target};
use crate::tabs::SESSIONS;

const RENAME: u32 = 0;
const CLOSE: u32 = 1;
const CLOSE_OTHERS: u32 = 2;
const COMPACT: u32 = 3;
const COPY_BASE: u32 = 4;
/// Above every `COPY_BASE + account` id.
const OPEN_FOLDER: u32 = 100_000;
const FIND: u32 = 100_001;
const SPLIT_RIGHT: u32 = 100_002;
const SPLIT_DOWN: u32 = 100_003;
const CLOSE_PANE: u32 = 100_004;

#[derive(Clone, Copy)]
pub struct Drag {
    pub id: u64,
    from: (f32, f32),
    pub started: bool,
}

fn crossed(x: f32, y: f32, r: Rect, right: bool) -> bool {
    let mid = r.x + r.w / 2.0;
    r.w > 0.0 && y >= r.y && y < r.bottom() && if right { x >= mid } else { x <= mid }
}

pub fn number(name: &KeyName, mods: Mods, terminal: bool) -> Option<usize> {
    if terminal || !mods.ctrl || mods.shift || mods.alt || mods.control {
        return None;
    }
    match name {
        KeyName::Char(c @ '1'..='9') => Some(*c as usize - '0' as usize),
        _ => None,
    }
}

impl AppState {
    pub fn begin_tab_drag(&mut self, id: u64, x: f32, y: f32) {
        self.tabs.drag = (id != SESSIONS).then_some(Drag { id, from: (x, y), started: false });
    }

    pub fn move_tab_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = &mut self.tabs.drag else { return false };
        let began = !drag.started && (x - drag.from.0).abs().max((y - drag.from.1).abs()) >= 5.0;
        drag.started |= began;
        if !drag.started {
            return false;
        }
        let id = drag.id;
        let Some(at) = self.tabs.index(id) else { return false };
        let hit = |id| self.hits.iter().find_map(|(r, t)| (*t == Target::Tab(id)).then_some(*r));
        // Use the last painted rectangles, including when a single motion crosses several tabs.
        // After a move, old hits are discarded until the strip paints at its new positions.
        let mut to = at;
        for (j, tab) in self.tabs.list.iter().enumerate().take(at).rev() {
            if !hit(tab.id).is_some_and(|r| crossed(x, y, r, false)) {
                break;
            }
            to = j;
        }
        for (j, tab) in self.tabs.list.iter().enumerate().skip(at + 1) {
            if !hit(tab.id).is_some_and(|r| crossed(x, y, r, true)) {
                break;
            }
            to = j;
        }
        if to == at {
            return began;
        }
        let tab = self.tabs.list.remove(at);
        self.tabs.list.insert(to, tab);
        self.hits.retain(|(_, target)| !matches!(target, Target::Tab(_) | Target::TabClose(_)));
        true
    }

    pub fn tab_menu(&mut self, id: u64, anchor: Rect) {
        let Some(tab) = self.tabs.get(id) else { return };
        let can_copy = tab.resume_id.as_ref().is_some_and(|id| self.by_id.contains_key(id));
        let mut items: Vec<_> = self
            .config
            .accounts
            .iter()
            .enumerate()
            .map(|(i, account)| {
                Item::new(COPY_BASE + i as u32, format!("Open a copy as {}", account.name)).enabled(can_copy)
            })
            .collect();
        let folder = self.tab_folder(id).is_some();
        items.extend([
            Item::new(COMPACT, "Compact and open as…").enabled(tab.resume_id.is_some()),
            Item::new(OPEN_FOLDER, "Open working folder").enabled(folder),
            Item::new(FIND, "Find…").hint("Ctrl+Shift+F"),
            Item::new(SPLIT_RIGHT, "Split right").enabled(self.can_split(id)),
            Item::new(SPLIT_DOWN, "Split down").enabled(self.can_split(id)),
            Item::new(CLOSE_PANE, "Close pane").enabled(tab.shows_panes()).sep(),
            Item::new(RENAME, "Rename tab…").sep(),
            Item::new(CLOSE, "Close"),
            Item::new(CLOSE_OTHERS, "Close others").enabled(self.tabs.list.len() > 1),
        ]);
        self.menu = Some((MenuKind::Tab(id), Menu::new(items, 0.0, 0.0, false).around(anchor)));
    }

    pub fn tab_menu_chosen(&mut self, now: Instant, id: u64, item: u32) {
        let Some(tab) = self.tabs.get(id) else { return };
        match item {
            RENAME => {
                let title = tab
                    .rename
                    .as_deref()
                    .or_else(|| tab.session.as_deref().and_then(|id| self.marks.name(id)))
                    .unwrap_or_else(|| {
                        if tab.term.ended.is_some() || tab.program_title.trim().is_empty() {
                            &tab.label
                        } else {
                            &tab.program_title
                        }
                    });
                self.dialog = Some(dialogs::name_dialog("Rename tab", title, NameFor::Tab(id)));
            }
            CLOSE => self.ask_close_tab(id),
            FIND => {
                self.show_tab(id);
                self.open_find();
            }
            SPLIT_RIGHT | SPLIT_DOWN => {
                self.show_tab(id);
                let axis = if item == SPLIT_RIGHT { crate::panes::Axis::Right } else { crate::panes::Axis::Down };
                self.split_pane(now, id, axis);
            }
            CLOSE_PANE => {
                self.show_tab(id);
                self.ask_close_pane(now, id);
            }
            OPEN_FOLDER => {
                if let Some(folder) = self.tab_folder(id) {
                    self.open_working_folder(now, &folder);
                }
            }
            COMPACT => {
                self.ask_tab_compact(id, None);
            }
            CLOSE_OTHERS => {
                if self.tabs.list.iter().any(|tab| tab.id != id && tab.any_running()) {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Close the other tabs?".into(),
                        text: "Their running programs will end.".into(),
                        ok: "Close".into(),
                        then: Pending::CloseOtherTabs(id),
                    });
                } else {
                    self.close_other_tabs(id);
                }
            }
            copy => {
                if let (Some(session), Some(account)) =
                    (self.recheck_tab_identity(id), self.config.accounts.get((copy - COPY_BASE) as usize))
                {
                    self.open_copy_as(now, &session, account.slug.clone());
                }
            }
        }
    }

    /// The folder a tab works in: the one its Claude session was started in, else its session's
    /// as the list knows it. A shell tab has none.
    pub(crate) fn tab_folder(&self, id: u64) -> Option<std::path::PathBuf> {
        let tab = self.tabs.get(id)?;
        let listed = || {
            let session = tab.resume_id.as_ref().or(tab.session.as_ref())?;
            Some(self.sessions.get(*self.by_id.get(session)?)?.cwd.clone())
        };
        tab.cwd.clone().or_else(listed).filter(|cwd| !cwd.as_os_str().is_empty())
    }

    /// A tab that runs a Claude session renames the session: its mark (list, search, the tab
    /// label, restored tabs) and, through the Chat view's gate, Claude Code's own name. A plain
    /// terminal tab, or a session the list does not know yet, only changes its label.
    pub fn rename_tab(&mut self, now: Instant, id: u64, name: &str) {
        let name = name.trim();
        let Some(tab) = self.tabs.get(id) else { return };
        let session = tab.pane.as_ref().and(tab.resume_id.clone().or_else(|| tab.session.clone()));
        let Some(session) = session.filter(|session| self.by_id.contains_key(session)) else {
            if let Some(tab) = self.tabs.get_mut(id) {
                tab.rename = (!name.is_empty()).then(|| name.to_owned());
            }
            return;
        };
        if !self.rename_session(&session, name) {
            return; // Not saved; the marks error was reported.
        }
        let live = !name.is_empty() && self.recheck_tab_identity(id).as_deref() == Some(session.as_str());
        let Some(tab) = self.tabs.get_mut(id) else { return };
        tab.rename = None;
        tab.cli_rename = live.then(|| (session, name.to_owned()));
        if live {
            self.toast("Renamed. Claude Code gets /rename from the Chat view when its prompt is idle.".into(), now);
        }
    }

    pub fn close_other_tabs(&mut self, keep: u64) {
        if self.tabs.get(keep).is_none() {
            return;
        }
        self.show_tab(keep);
        self.tabs.list.retain(|tab| tab.id == keep);
        self.rebuild_view();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, tabs::NewTab, term::Term};

    fn tab(app: &mut AppState, session: Option<String>) -> u64 {
        app.tabs.open(NewTab { label: "Tab".into(), color: [0; 3], session, account: None }, Term::empty((80, 24)))
    }

    #[test]
    fn a_tab_works_in_its_launch_folder_else_in_its_sessions_and_a_shell_tab_in_none() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let shell = tab(&mut app, None);
        assert_eq!(app.tab_folder(shell), None);
        let launched = tab(&mut app, None);
        app.tabs.get_mut(launched).unwrap().cwd = Some(r"C:\work\trail".into());
        assert_eq!(app.tab_folder(launched), Some(r"C:\work\trail".into()));
        let listed = app.sessions.iter().find(|s| !s.cwd.as_os_str().is_empty()).expect("demo sessions");
        let (id, cwd) = (listed.id.clone(), listed.cwd.clone());
        let resumed = tab(&mut app, Some(id));
        assert_eq!(app.tab_folder(resumed), Some(cwd));
    }

    #[test]
    fn only_an_existing_absolute_folder_is_opened() {
        assert!(crate::run::open_folder(std::path::Path::new("relative")).is_err());
        assert!(crate::run::open_folder(&std::env::current_exe().unwrap()).is_err(), "never a file");
        assert!(crate::run::open_folder(&std::env::temp_dir().join("cs-no-such-folder-81723")).is_err());
    }

    #[test]
    fn digit_shortcuts_leave_terminal_input_and_altgr_alone() {
        let ctrl = Mods { ctrl: true, ..Default::default() };
        assert_eq!(number(&KeyName::Char('9'), ctrl, false), Some(9));
        assert_eq!(number(&KeyName::Char('0'), ctrl, false), None);
        assert_eq!(number(&KeyName::Char('2'), ctrl, true), None);
        assert_eq!(number(&KeyName::Char('2'), Mods { alt: true, ..ctrl }, false), None);
        assert_eq!(
            crate::termkeys::key(
                Some(&KeyName::Char('2')),
                ctrl,
                None,
                &Default::default(),
                false,
                crate::termkeys::Keyboard::Classic,
            ),
            crate::termkeys::KeyAction::Send(vec![0])
        );
    }

    #[test]
    fn drag_crosses_the_neighbour_midpoint_and_keeps_the_active_id() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let mut open = || {
            app.tabs
                .open(NewTab { label: "fake".into(), color: [0; 3], session: None, account: None }, Term::empty((8, 4)))
        };
        let (a, b, c) = (open(), open(), open());
        app.show_tab(a);
        app.hits.extend([
            (Rect::new(100.0, 40.0, 90.0, 40.0), Target::Tab(b)),
            (Rect::new(190.0, 40.0, 110.0, 40.0), Target::Tab(c)),
        ]);
        app.begin_tab_drag(a, 50.0, 60.0);
        assert!(app.move_tab_drag(144.0, 60.0));
        assert_eq!(app.tabs.numbered(2), Some(a));
        assert!(app.move_tab_drag(245.0, 60.0));
        assert_eq!(app.tabs.numbered(1), Some(SESSIONS));
        assert_eq!(app.tabs.numbered(2), Some(b));
        assert_eq!(app.tabs.numbered(4), Some(a));
        assert_eq!(app.tabs.active, Some(a));
        app.close_other_tabs(c);
        assert_eq!(app.tabs.active, Some(c));
        assert_eq!(app.tabs.list.len(), 1);
    }

    #[test]
    fn renaming_a_claude_tab_renames_the_session_and_queues_the_cli_rename() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let session = app.sessions.iter().find(|s| s.desktop_id.is_none()).unwrap().clone();
        let tab = app.tabs.open(
            NewTab { label: "tab".into(), color: [0; 3], session: Some(session.id.clone()), account: None },
            Term::empty((8, 4)),
        );
        let pane = app.new_session_pane(crate::view::model::Source::Demo(session.file.clone()), session.cwd.clone());
        app.tabs.get_mut(tab).unwrap().pane = Some(pane);
        let now = Instant::now();
        app.rename_tab(now, tab, "  toolkit ");
        assert_eq!(app.marks.name(&session.id), Some("toolkit"), "the list and search use the mark");
        let t = app.tabs.get(tab).unwrap();
        assert_eq!(t.rename, None, "the label follows the mark, which also survives a restart");
        assert_eq!(t.cli_rename, Some((session.id.clone(), "toolkit".into())));
        app.rename_tab(now, tab, "");
        assert_eq!(app.marks.name(&session.id), None, "blank goes back to the derived name");
        assert_eq!(app.tabs.get(tab).unwrap().cli_rename, None, "and asks Claude Code for nothing");
    }

    #[test]
    fn renaming_a_plain_terminal_tab_changes_only_its_label() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let session = app.sessions[0].id.clone();
        let shell = app
            .tabs
            .open(NewTab { label: "pwsh".into(), color: [0; 3], session: None, account: None }, Term::empty((8, 4)));
        let marks = app.marks.clone();
        app.rename_tab(Instant::now(), shell, "build");
        let t = app.tabs.get(shell).unwrap();
        assert_eq!((t.rename.as_deref(), t.cli_rename.as_ref()), (Some("build"), None));
        assert_eq!(app.marks.name(&session), marks.name(&session));
    }

    #[test]
    fn tab_actions_target_the_clicked_session_without_changing_list_selection() {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        let selected = app.selected.clone();
        let session = app.sessions.iter().find(|s| Some(&s.id) != selected.as_ref() && s.desktop_id.is_none()).unwrap();
        let id = session.id.clone();
        let tab = app.tabs.open(
            NewTab { label: "tab".into(), color: [0; 3], session: Some(id.clone()), account: Some("main".into()) },
            Term::empty((8, 4)),
        );
        let now = Instant::now();
        app.tab_menu_chosen(now, tab, COPY_BASE + 1);
        assert!(matches!(
            app.dialog,
            Some(Dialog::Confirm { then: Pending::Copy { id: ref got, ref account, .. }, .. }) if got == &id && account == "work"
        ));
        app.tab_menu_chosen(now, tab, COMPACT);
        assert!(matches!(app.dialog, Some(Dialog::Compact(_))));
        app.tab_menu_chosen(now, tab, RENAME);
        dialogs::text_input(&mut app, "custom label");
        dialogs::click(&mut app, now, dialogs::OK, 0.0);
        assert_eq!(app.tabs.get(tab).unwrap().rename.as_deref(), Some("custom label"));
        assert_eq!(app.selected, selected);
    }
}
