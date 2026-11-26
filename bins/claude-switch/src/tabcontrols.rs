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
        items.extend([
            Item::new(COMPACT, "Compact and open as…").enabled(tab.resume_id.is_some()),
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
            COMPACT => {
                self.ask_tab_compact(id, None);
            }
            CLOSE_OTHERS => {
                if self.tabs.list.iter().any(|tab| tab.id != id && tab.term.running()) {
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

    #[test]
    fn digit_shortcuts_leave_terminal_input_and_altgr_alone() {
        let ctrl = Mods { ctrl: true, ..Default::default() };
        assert_eq!(number(&KeyName::Char('9'), ctrl, false), Some(9));
        assert_eq!(number(&KeyName::Char('0'), ctrl, false), None);
        assert_eq!(number(&KeyName::Char('2'), ctrl, true), None);
        assert_eq!(number(&KeyName::Char('2'), Mods { alt: true, ..ctrl }, false), None);
        assert_eq!(
            crate::termkeys::key(Some(&KeyName::Char('2')), ctrl, None, &Default::default(), false),
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
            Some(Dialog::Confirm { then: Pending::Copy { id: ref got, ref account }, .. }) if got == &id && account == "work"
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
