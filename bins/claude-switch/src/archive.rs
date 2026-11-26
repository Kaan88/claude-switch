//! Archive selection, preview, and marks. Transcripts are never changed by an archive action.

use std::collections::HashSet;
use std::time::Instant;

use claude_switch_core::archive::{WEEKS, eligible, older_than};
use claude_switch_core::{Session, launch, sessions::folder_key};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::KeyName;
use mdshared_ui::menu::{Item, Menu};
use mdshared_ui::overlay::{DIALOG_PAD, dialog_card, draw_dialog_button_disabled};

use crate::dialogs::{CANCEL, Dialog, Draw, OK};
use crate::list::{model::Filter, now_ms};
use crate::source::Source;
use crate::state::{AppState, MenuKind};

pub const ARCHIVE_OLD: u32 = 60;
pub const ARCHIVE_ROW: u32 = 61;
pub const ARCHIVE_FOLDER: u32 = 62;
pub const UNARCHIVE_ROW: u32 = 63;
pub const UNARCHIVE_SELECTED: u32 = 64;
pub const UNARCHIVE_ALL: u32 = 65;
pub const REPORT: u32 = 66;
const AGE: u32 = 10;
const PREVIOUS: u32 = 20;
const NEXT: u32 = 21;

#[derive(Clone)]
pub enum Scope {
    Older { weeks: u64, folder: Option<String> },
    Folder(String),
    One(String),
}

impl Scope {
    fn matches(&self, s: &Session, now: u64, modified: u64) -> bool {
        match self {
            Self::Older { weeks, folder } => {
                folder.as_ref().is_none_or(|f| *f == folder_key(&s.cwd))
                    && older_than(now, *weeks, s.last_active_ms.max(modified))
            }
            Self::Folder(key) => *key == folder_key(&s.cwd),
            Self::One(id) => s.id == *id,
        }
    }
}

pub struct ArchiveDialog {
    scope: Scope,
    folders: Vec<(String, String)>,
    folder: usize,
    ids: Vec<String>,
    bytes: u64,
}

impl AppState {
    pub fn is_archived(&self, s: &Session) -> bool {
        self.marks.archived_at(&s.id).is_some()
    }

    pub fn searchable(&self, s: &Session, searching: bool) -> bool {
        claude_switch_core::archive::visible(self.is_archived(s), false, searching, self.listed(s))
    }

    pub fn list_session_visible(&self, s: &Session, searching: bool) -> bool {
        if self.filter == Filter::Archived {
            return self.is_archived(s);
        }
        self.searchable(s, searching) && (self.filter == Filter::All || self.row_flags(s).matches(self.filter))
    }

    pub fn can_archive(&self, s: &Session) -> bool {
        let flags = self.row_flags(s);
        eligible(flags.pinned, flags.running, self.is_archived(s))
    }

    pub fn toggle_archive_pick(&mut self, i: usize) -> bool {
        let Some(s) = self.sessions.get(i).filter(|s| self.filter == Filter::Archived && self.is_archived(s)) else {
            return false;
        };
        if !self.archive_picks.remove(&s.id) {
            self.archive_picks.insert(s.id.clone());
        }
        true
    }

    pub fn archive_menu(&mut self, anchor: Rect) {
        let mut items = vec![Item::new(ARCHIVE_OLD, "Archive older sessions…")];
        self.archive_view_items(&mut items);
        self.menu = Some((MenuKind::Row, Menu::new(items, 0.0, 0.0, false).around(anchor).min_width(230.0)));
    }

    pub fn archive_view_items(&self, items: &mut Vec<Item<u32>>) {
        if self.filter == Filter::Archived {
            items.push(Item::new(REPORT, "Summary report…").sep());
            items.push(
                Item::new(UNARCHIVE_SELECTED, format!("Unarchive selection ({})", self.archive_picks.len()))
                    .enabled(!self.archive_picks.is_empty()),
            );
            items.push(Item::new(UNARCHIVE_ALL, "Unarchive all…"));
        }
    }

    pub fn archive_action(&mut self, now: Instant, action: u32) {
        match action {
            ARCHIVE_OLD => self.ask_archive(Scope::Older { weeks: 2, folder: None }),
            ARCHIVE_ROW => {
                if let Some(s) = self.selected_session() {
                    self.ask_archive(Scope::One(s.id.clone()));
                }
            }
            ARCHIVE_FOLDER => {
                if let Some(key) =
                    self.context_folder.take().or_else(|| self.selected_session().map(|s| folder_key(&s.cwd)))
                    && !key.is_empty()
                {
                    self.ask_archive(Scope::Folder(key));
                }
            }
            UNARCHIVE_ROW => {
                if let Some(s) = self.selected_session() {
                    self.unarchive(now, Some(HashSet::from([s.id.clone()])));
                }
            }
            UNARCHIVE_SELECTED => self.unarchive(now, Some(self.archive_picks.clone())),
            UNARCHIVE_ALL => {
                use crate::dialogs::Pending;
                self.dialog = Some(Dialog::Confirm {
                    title: "Unarchive all sessions?".into(),
                    text: "Removes all Claude Switch archive marks. Hidden sessions keep their Hide marks.".into(),
                    ok: "Unarchive all".into(),
                    then: Pending::UnarchiveAll,
                });
            }
            REPORT => self.ask_archive_report(None),
            _ => {}
        }
    }

    fn ask_archive(&mut self, scope: Scope) {
        self.show_sessions();
        let folders = self
            .all_groups
            .iter()
            .filter(|g| !g.cwd.as_os_str().is_empty())
            .map(|g| (folder_key(&g.cwd), g.cwd.display().to_string()))
            .collect();
        let mut d = ArchiveDialog { scope, folders, folder: 0, ids: Vec::new(), bytes: 0 };
        d.preview(self);
        self.dialog = Some(Dialog::Archive(Box::new(d)));
    }

    fn archive_preview(&mut self, d: &ArchiveDialog) {
        let result = match &self.source {
            Source::Real(roots) => {
                launch::with_archive_check(&roots.clone(), &self.config.clone(), |busy| self.archive_commit(d, busy))
            }
            Source::Demo => Ok(self.archive_commit(d, &HashSet::new())),
        };
        match result {
            Ok(Some(n)) => self.ask_archive_report(Some((n, d.ids.len() - n))),
            Ok(None) => {}
            Err(e) => self.warn("Archive not changed", e.to_string()),
        }
    }

    fn archive_commit(&mut self, d: &ArchiveDialog, busy: &HashSet<String>) -> Option<usize> {
        let now = now_ms();
        let before = self.marks.clone();
        let mut n = 0;
        for id in &d.ids {
            let Some(s) = self.by_id.get(id).map(|&i| &self.sessions[i]) else { continue };
            if busy.contains(&id.to_ascii_lowercase()) || !self.can_archive(s) {
                continue;
            }
            let modified =
                if self.source.is_demo() { s.transcript_ms } else { claude_switch_core::modified_ms(&s.file) };
            if modified != 0 && d.scope.matches(s, now, modified) && self.marks.archive(id, &s.file, Some(now)) {
                n += 1;
            }
        }
        if n == 0 {
            self.warn("Nothing archived", "No previewed session is still eligible.".into());
            return None;
        }
        if !self.save_marks() {
            self.marks = before;
            return None;
        }
        self.after_archive_change();
        Some(n)
    }

    pub fn unarchive(&mut self, now: Instant, ids: Option<HashSet<String>>) {
        let before = self.marks.clone();
        let n = match ids {
            Some(ids) => {
                let mut n = 0;
                for id in ids {
                    if let Some(&i) = self.by_id.get(&id)
                        && self.marks.archived_at(&id).is_some()
                        && self.marks.archive(&id, &self.sessions[i].file, None)
                    {
                        n += 1;
                    }
                }
                n
            }
            None => self.marks.clear_archive(),
        };
        if n == 0 {
            return;
        }
        if !self.save_marks() {
            self.marks = before;
            return;
        }
        self.after_archive_change();
        self.toast(format!("Unarchived {n} session(s)"), now);
    }

    fn after_archive_change(&mut self) {
        self.rebuild_view();
        if self.selected.as_deref().is_none_or(|id| self.entry_of(id).is_none()) {
            self.select_first();
        }
    }
}

impl ArchiveDialog {
    fn preview(&mut self, st: &AppState) {
        let now = now_ms();
        self.bytes = 0;
        self.ids.clear();
        for s in &st.sessions {
            if (!s.worker || st.config.show_workers) && self.scope.matches(s, now, s.transcript_ms) && st.can_archive(s)
            {
                self.ids.push(s.id.clone());
                self.bytes = self.bytes.saturating_add(s.bytes);
            }
        }
    }

    pub fn draw(&self, d: &mut Draw<'_>, p: &mut Painter, window: Rect) {
        let older = matches!(self.scope, Scope::Older { .. });
        let card = dialog_card(500.0, if older { 295.0 } else { 191.0 }, window.w, window.h);
        let footer = d.card(p, card);
        let title = match &self.scope {
            Scope::Older { .. } => "Archive older sessions",
            Scope::Folder(_) => "Archive folder",
            Scope::One(_) => "Archive session",
        };
        d.title(p, card, title);
        let x = card.x + DIALOG_PAD;
        let w = card.w - 2.0 * DIALOG_PAD;
        let mut y = card.y + 58.0;
        let pal = d.pal;
        if let Scope::Older { weeks, .. } = self.scope {
            p.text(Face::Ui, 12.0, x, y + 7.0, "Last active more than", pal.muted, w);
            y += 19.0;
            let bw = (w - 24.0) / 4.0;
            for (i, age) in WEEKS.into_iter().enumerate() {
                let r = Rect::new(x + i as f32 * (bw + 8.0), y, bw, 32.0);
                d.button(p, r, &format!("{age} weeks"), weeks == age, AGE + i as u32);
            }
            y += 45.0;
            let folder = self.folder.checked_sub(1).and_then(|i| self.folders.get(i)).map_or("All folders", |(_, s)| s);
            let r = Rect::new(x, y, w, 32.0);
            p.frame(r, 5.0, pal.field, pal.line);
            p.text_center(Face::Ui, 13.0, Rect::new(x + 38.0, y, w - 76.0, r.h), folder, pal.text);
            if !self.folders.is_empty() {
                d.button(p, Rect::new(x + 2.0, y + 2.0, 34.0, 28.0), "‹", false, PREVIOUS);
                d.button(p, Rect::new(r.right() - 36.0, y + 2.0, 34.0, 28.0), "›", false, NEXT);
            }
            y += 40.0;
        }
        let count = format!("{} session(s) · {}", self.ids.len(), mdshared_app::fmt::size(self.bytes));
        p.text(Face::Semibold, 15.0, x, y + 12.0, &count, pal.text, w);
        p.text(
            Face::Ui,
            12.0,
            x,
            y + 35.0,
            "Pinned and running sessions are skipped. Nothing is moved or deleted.",
            pal.muted,
            w,
        );
        if self.ids.is_empty() {
            d.footer(p, footer, &[("Cancel", true, CANCEL)]);
            let width = mdshared_ui::overlay::dialog_button_w(p, "Archive");
            let x = footer.right() - DIALOG_PAD - mdshared_ui::overlay::dialog_button_w(p, "Cancel") - 8.0 - width;
            draw_dialog_button_disabled(p, &pal, Rect::new(x, footer.cy() - 16.0, width, 32.0), "Archive", false);
        } else {
            d.footer(p, footer, &[("Archive", false, OK), ("Cancel", true, CANCEL)]);
        }
    }

    pub fn key(&mut self, st: &AppState, key: &KeyName) {
        let Scope::Older { weeks, .. } = self.scope else { return };
        let at = WEEKS.iter().position(|&n| n == weeks).unwrap_or(0);
        let id = match key {
            KeyName::Left => AGE + at.saturating_sub(1) as u32,
            KeyName::Right => AGE + (at + 1).min(WEEKS.len() - 1) as u32,
            KeyName::Up => PREVIOUS,
            KeyName::Down => NEXT,
            _ => return,
        };
        self.change(st, id);
    }

    fn change(&mut self, st: &AppState, id: u32) {
        let Scope::Older { weeks, folder } = &mut self.scope else { return };
        match id {
            PREVIOUS => self.folder = (self.folder + self.folders.len()) % (self.folders.len() + 1),
            NEXT => self.folder = (self.folder + 1) % (self.folders.len() + 1),
            id if (AGE..AGE + WEEKS.len() as u32).contains(&id) => *weeks = WEEKS[(id - AGE) as usize],
            _ => return,
        }
        *folder = self.folder.checked_sub(1).and_then(|i| self.folders.get(i)).map(|(key, _)| key.clone());
        self.preview(st);
    }

    pub fn click(mut self: Box<Self>, st: &mut AppState, id: u32) {
        match id {
            CANCEL => {}
            OK if !self.ids.is_empty() => st.archive_preview(&self),
            _ => {
                self.change(st, id);
                st.dialog = Some(Dialog::Archive(self));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winfile::WindowFile;

    fn demo() -> AppState {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.apply_scan(crate::demo::scan(&st.config));
        st
    }

    #[test]
    fn preview_is_read_only_and_commit_rechecks_the_previewed_ids() {
        let mut st = demo();
        let id = st.sessions[7].id.clone();
        let path = st.sessions[7].file.clone();
        st.ask_archive(Scope::One(id.clone()));
        let Some(Dialog::Archive(preview)) = st.dialog.take() else { panic!("missing preview") };
        assert_eq!(preview.ids, vec![id.clone()]);
        assert_eq!(preview.bytes, st.sessions[7].bytes);
        assert!(st.marks.archived_at(&id).is_none());
        assert!(st.archive_commit(&preview, &HashSet::from([id.clone()])).is_none());
        st.marks.pin(&id, &path, true);
        assert!(st.archive_commit(&preview, &HashSet::new()).is_none());
        st.marks.pin(&id, &path, false);
        assert_eq!(st.archive_commit(&preview, &HashSet::new()), Some(1));
        assert!(st.marks.archived_at(&id).is_some() && st.entry_of(&id).is_none());
        let title = st.sessions[7].title.clone();
        st.search.set_text(&title);
        st.rebuild_view();
        assert!(st.entry_of(&id).is_some(), "archived matches are searchable");
    }

    #[test]
    fn archived_checkbox_selection_and_unarchive_all_keep_hide_state() {
        let mut st = demo();
        let i = 80;
        let id = st.sessions[i].id.clone();
        let file = st.sessions[i].file.clone();
        st.marks.hide(&id, &file, true);
        assert!(!st.toggle_archive_pick(i));
        st.filter = Filter::Archived;
        st.rebuild_view();
        assert_eq!(st.archive_count, 3);
        assert!(st.entry_of(&id).is_some() && st.toggle_archive_pick(i));
        st.archive_action(Instant::now(), UNARCHIVE_SELECTED);
        assert_eq!(st.archive_count, 2);
        assert!(st.archive_picks.is_empty() && st.entry_of(&id).is_none() && st.marks.hidden(&id));
        st.unarchive(Instant::now(), None);
        assert_eq!(st.archive_count, 0);
        assert_eq!(st.filter, Filter::All);
        assert!(st.marks.hidden(&id));
    }

    #[test]
    fn folder_preview_only_admits_eligible_sessions_in_that_folder() {
        let mut st = demo();
        let folder = folder_key(&st.sessions[0].cwd);
        st.ask_archive(Scope::Folder(folder.clone()));
        let Some(Dialog::Archive(preview)) = st.dialog.take() else { panic!("missing preview") };
        assert!(!preview.ids.is_empty());
        for id in &preview.ids {
            let s = &st.sessions[st.by_id[id]];
            assert_eq!(folder_key(&s.cwd), folder);
            assert!(st.can_archive(s));
        }
        assert!(!preview.ids.contains(&st.sessions[0].id), "running");
    }
}
