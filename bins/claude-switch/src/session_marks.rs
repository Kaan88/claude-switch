//! List projection and app-owned marks. No drawing.

use std::time::{Duration, Instant};

use claude_switch_core::grouping::Mode;
use claude_switch_core::sessions::{folder_key, folder_name, group_by_folder};
use claude_switch_core::{Running, Session, clock};
use mdshared_ui::sections::Sections;

use crate::list::model::{self, Filter, Flags};
use crate::source::{ActivityScan, Source};
use crate::state::{AppState, HEAD_H, ROW_H};
use crate::tabs::Tab;

#[cfg(test)]
mod tests;

pub struct UndoHide {
    pub key: String,
    pub folder: bool,
    pub until: Instant,
}

impl AppState {
    pub fn title_of<'a>(&'a self, session: &'a Session) -> &'a str {
        self.marks.name(&session.id).unwrap_or(&session.title)
    }

    pub fn hidden_session(&self, s: &Session) -> bool {
        self.marks.hidden(&s.id) || self.marks.folder_hidden(&folder_key(&s.cwd))
    }

    pub fn listed(&self, s: &Session) -> bool {
        !self.is_archived(s)
            && (self.config.show_archived || !s.archived)
            && (self.config.show_workers || !s.worker)
            && (self.config.show_hidden || !self.hidden_session(s))
    }

    pub fn row_flags(&self, s: &Session) -> Flags {
        let running =
            self.running_ids.contains(&s.id) || self.tabs.by_session(&s.id, |tab| tab.term.running()).is_some();
        Flags {
            running,
            waiting: running && s.turn_ended,
            pinned: self.marks.pinned(&s.id),
            unseen: self.marks.unseen(&s.id, s.transcript_ms),
            hidden: self.hidden_session(s),
        }
    }

    pub fn index_running(&mut self) {
        self.running_ids = self.running.iter().map(|r| r.session_id.clone()).collect();
        self.identify_tabs();
    }

    fn update_running(
        &mut self,
        running: Vec<Running>,
        live: impl Fn(&Tab) -> bool,
        identify: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let running_changed = self.running != running;
        self.running = running;
        if running_changed {
            self.running_ids = self.running.iter().map(|r| r.session_id.clone()).collect();
        }
        if running_changed || self.tabs.list.iter().any(|tab| live(tab) && tab.needs_identity()) {
            identify(self) || running_changed
        } else {
            false
        }
    }

    pub fn apply_activity(&mut self, activity: ActivityScan) -> bool {
        let mut changed = self.update_running(activity.running, |tab| tab.term.running(), Self::identify_tabs);
        if !changed {
            changed |= self.verify_external_owners();
        }
        if changed
            && self.tabs.list.iter().filter_map(|tab| tab.resume_id.as_ref()).any(|id| !self.by_id.contains_key(id))
        {
            self.refresh(Instant::now());
        }
        let mut updated = false;
        let mut selected_updated = false;
        for update in activity.updates {
            if let Some(&i) = self.by_id.get(&update.id) {
                let s = &mut self.sessions[i];
                // A full scan may have applied while this poll was in flight.
                if s.file != update.file || self.grouping.transcript_stamps.get(&update.id) != Some(&update.previous) {
                    continue;
                }
                let at = update.stamp.modified_ms();
                s.transcript_ms = at;
                s.bytes = update.stamp.size();
                s.last_active_ms = s.last_active_ms.max(at);
                s.last_prompt = update.prompt;
                s.turn_ended = update.ended;
                updated = true;
                selected_updated |= self.selected.as_ref() == Some(&update.id);
                self.grouping.transcript_stamps.insert(update.id, update.stamp);
            }
        }
        if updated {
            self.all_groups = group_by_folder(&self.sessions);
            if selected_updated && let Some(id) = self.selected.clone() {
                self.load_details(&id, true);
            }
            changed = true;
        }
        if changed {
            self.see_shown();
        }
        let now = crate::list::now_ms();
        let offset = clock::local_offset_minutes();
        let date_changed =
            self.list_mode() == Mode::Recent && self.recent_local_day != Some((model::local_day(now, offset), offset));
        if changed || date_changed {
            self.rebuild_view_at(now, offset);
        }
        changed || date_changed
    }

    pub fn rebuild_view(&mut self) {
        self.rebuild_view_at(crate::list::now_ms(), clock::local_offset_minutes());
    }

    fn rebuild_view_at(&mut self, now: u64, offset: i32) {
        self.recent_local_day = Some((model::local_day(now, offset), offset));
        self.archive_count = self.sessions.iter().filter(|s| self.is_archived(s)).count();
        self.archive_picks.retain(|id| self.by_id.contains_key(id) && self.marks.archived_at(id).is_some());
        if self.filter == Filter::Archived && self.archive_count == 0 {
            self.filter = Filter::All;
        }
        if self.conversation_search.on && (self.filter != Filter::Archived || !self.search.text().trim().is_empty()) {
            self.project_conversation_search();
            return;
        }
        let groups = self.project_groups(now, offset);
        let no_query = self.search.text().trim().is_empty();
        let scroll = self.sections.scroll;
        self.sections = Sections::new(self.sections.view, ROW_H);
        self.row_locations.clear();
        for (k, g) in groups.iter().enumerate() {
            self.sections.add_group(g.rows.len(), (!g.key.starts_with("thread:")).then_some(HEAD_H));
            self.sections.set_collapsed(k, no_query && self.collapsed.contains(&g.key));
            self.row_locations.extend(g.rows.iter().enumerate().map(|(r, &i)| (i, (k, r))));
        }
        self.groups = groups;
        self.sections.scroll = self.sections.clamp_scroll(scroll);
    }

    pub fn save_marks(&mut self) -> bool {
        self.marks_flush_at = None;
        if let Source::Real(roots) = &self.source {
            if self.marks_read_only {
                self.warn(
                    "Could not save session marks",
                    "marks.json is read-only in this window. Restart after repairing it.".into(),
                );
                return false;
            } else if let Err(e) = self.marks.save(roots) {
                self.error("Could not save session marks", e.to_string());
                return false;
            }
        }
        true
    }

    pub fn prune_marks(&mut self) {
        if !self.source.is_demo()
            && !self.marks_read_only
            && self.marks.prune(self.sessions.iter().map(|s| (s.id.as_str(), s.file.as_path())), crate::list::now_ms())
        {
            self.save_marks();
        }
    }

    pub fn see_shown(&mut self) {
        if !self.window_focused || self.marks_read_only {
            return;
        }
        let id = if self.terminal_shown() {
            self.tabs.shown().filter(|tab| !tab.needs_identity()).and_then(|tab| tab.session.as_deref())
        } else {
            self.selected.as_deref()
        };
        let Some(s) = id.and_then(|id| self.by_id.get(id)).map(|&i| &self.sessions[i]) else { return };
        if self.marks.see(&s.id, &s.file, s.transcript_ms) {
            if !self.source.is_demo() {
                self.marks_flush_at = Some(Instant::now() + Duration::from_millis(650));
            }
            if self.filter == Filter::Unseen {
                self.rebuild_view();
            }
        }
    }

    /// Whether the session's mark now carries `name` (blank: no custom name).
    pub fn rename_session(&mut self, id: &str, name: &str) -> bool {
        let Some(&i) = self.by_id.get(id) else { return false };
        if self.edit_marks(|st| st.marks.rename(id, &st.sessions[i].file, name)) {
            self.rebuild_view();
        }
        self.marks.name(id).unwrap_or_default() == name.trim()
    }

    pub fn toggle_pin(&mut self, i: usize) {
        let Some(s) = self.sessions.get(i) else { return };
        if self.is_archived(s) {
            return;
        }
        let pin = !self.marks.pinned(&s.id);
        if self.edit_marks(|st| {
            let s = &st.sessions[i];
            st.marks.pin(&s.id, &s.file, pin)
        }) {
            self.rebuild_view();
        }
    }

    fn edit_marks(&mut self, edit: impl FnOnce(&mut Self) -> bool) -> bool {
        let previous = self.marks.clone();
        let flush = self.marks_flush_at;
        if !edit(self) {
            return false;
        }
        if self.save_marks() {
            return true;
        }
        self.marks = previous;
        self.marks_flush_at = flush;
        false
    }

    pub fn toggle_hidden(&mut self, now: Instant, folder: bool) {
        let Some(s) = self.selected.as_ref().and_then(|id| self.by_id.get(id)).map(|&i| &self.sessions[i]) else {
            return;
        };
        let key = if folder { folder_key(&s.cwd) } else { s.id.clone() };
        let hide = if folder { !self.marks.folder_hidden(&key) } else { !self.marks.hidden(&key) };
        self.set_hidden(now, &key, folder, hide);
    }

    pub fn hide_from_keyboard(&mut self, now: Instant) {
        if let Some(s) = self.selected_session()
            && !self.hidden_session(s)
        {
            let key = s.id.clone();
            self.set_hidden(now, &key, false, true);
        }
    }

    pub fn toggle_group_hidden(&mut self, now: Instant, key: String) {
        let hide = !self.marks.folder_hidden(&key);
        self.set_hidden(now, &key, true, hide);
    }

    fn set_hidden(&mut self, now: Instant, key: &str, folder: bool, hide: bool) -> bool {
        let saved = self.edit_marks(|st| {
            if folder {
                st.marks.hide_folder(key, hide)
            } else if let Some(&i) = st.by_id.get(key) {
                st.marks.hide(key, &st.sessions[i].file, hide)
            } else {
                false
            }
        });
        if saved {
            self.rebuild_view();
            if self.selected.as_deref().is_some_and(|id| self.entry_of(id).is_none()) {
                if self.filter == Filter::Unseen {
                    self.selected = None;
                    self.preview = None;
                    self.session_size.cancel();
                    self.cancel_related();
                } else {
                    self.select_first();
                }
            }
            if hide {
                self.undo_hide = Some(UndoHide { key: key.into(), folder, until: now + Duration::from_secs(6) });
            }
        }
        saved
    }

    pub fn undo_hidden(&mut self, now: Instant) {
        let Some(undo) = self.undo_hide.take().filter(|u| u.until >= now) else { return };
        let hidden = if undo.folder {
            self.marks.folder_hidden(&undo.key)
        } else {
            self.by_id.contains_key(&undo.key) && self.marks.hidden(&undo.key)
        };
        if hidden && !self.set_hidden(now, &undo.key, undo.folder, false) {
            self.undo_hide = Some(undo);
        }
    }
}

pub fn folder_label(s: &Session) -> String {
    folder_name(&s.cwd)
}
