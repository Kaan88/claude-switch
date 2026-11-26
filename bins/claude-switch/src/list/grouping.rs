//! In-memory projection and the selection-scoped similarity worker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use claude_switch_core::grouping::{self, Activity, Mode, Project};
use claude_switch_core::sessions::{ScanCache, Stamp, folder_key, folder_name};
use mdshared_app::job::Job;

use crate::list::model::{self, BUCKETS, Calendar, Filter};
use crate::state::{AppState, ViewGroup};

mod related;
pub(crate) use related::RelatedIndex;

#[derive(Default)]
pub struct Grouping {
    pub scan_cache: Arc<Mutex<ScanCache>>,
    pub projects: HashMap<String, Project>,
    pub transcript_stamps: HashMap<String, Stamp>,
    pub related_index: Arc<RelatedIndex>,
    roots: Vec<usize>,
    threads: HashMap<usize, Vec<usize>>,
    pub expanded: HashSet<String>,
    pub earlier: HashMap<usize, usize>,
    pub project_summary: HashMap<String, String>,
    pub brackets: HashMap<usize, (bool, bool)>,
    pub topics: bool,
    pub related: Vec<(String, Relation)>,
    pub pending: Option<Job<Vec<usize>>>,
    pub related_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Earlier,
    Later,
    Similar,
}

impl Relation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Earlier => "Earlier",
            Self::Later => "Later",
            Self::Similar => "Similar prompts",
        }
    }
}

impl AppState {
    pub(crate) fn rebuild_after_mode_change(&mut self) {
        self.rebuild_view();
        self.sections.scroll = 0.0;
        if self.list_mode() == Mode::Projects {
            self.select_first();
        } else {
            self.reveal_selected();
            if self.selected.as_deref().is_none_or(|id| self.entry_of(id).is_none()) {
                self.select_first();
            }
        }
    }

    pub fn list_mode(&self) -> Mode {
        match self.config.sessions_mode {
            Mode::Topics if !self.grouping.topics => Mode::Recent,
            mode => mode,
        }
    }

    pub fn rebuild_threads(&mut self) {
        let mut ids = HashMap::new();
        for (i, s) in self.sessions.iter().enumerate() {
            ids.entry(s.id.to_ascii_lowercase()).and_modify(|id| *id = None).or_insert(Some(i));
        }
        let parents: Vec<_> = self
            .sessions
            .iter()
            .map(|s| {
                self.marks
                    .parent(&s.id)
                    .or(s.grouping.parent.as_deref())
                    .and_then(|p| ids.get(&p.to_ascii_lowercase()).copied().flatten())
            })
            .collect();
        self.grouping.roots = grouping::thread_roots(&parents);
        self.grouping.threads.clear();
        for (i, &root) in self.grouping.roots.iter().enumerate() {
            self.grouping.threads.entry(root).or_default().push(i);
        }
    }

    pub fn project_groups(&mut self, now: u64, offset: i32) -> Vec<ViewGroup> {
        let calendar = Calendar::at(now, offset);
        let query = self.search.text().trim();
        let archive = self.filter == Filter::Archived;
        let newest = |&i: &usize| (std::cmp::Reverse(self.sessions[i].last_active_ms), i);
        self.grouping.topics = self.sessions.iter().any(|s| self.marks.topic(&s.id).is_some());
        self.grouping.earlier.clear();
        self.grouping.project_summary.clear();
        self.grouping.brackets.clear();
        let mode = self.list_mode();
        let keep_pins_with_group =
            matches!(mode, Mode::Threads | Mode::Topics) || (mode == Mode::Projects && !self.config.group_by_folder);
        let mut rows: Vec<_> = self
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if !self.list_session_visible(s, !query.is_empty()) {
                    return None;
                }
                if !query.is_empty() {
                    let matched = model::score(query, self.marks.name(&s.id), s, &folder_name(&s.cwd)).is_some()
                        || s.branch.as_ref().is_some_and(|b| mdshared_app::fuzzy::score(query, b).is_some())
                        || self.marks.topic(&s.id).is_some_and(|t| mdshared_app::fuzzy::score(query, t).is_some())
                        || (mode == Mode::Projects
                            && !self.config.group_by_folder
                            && self.project(i).is_some_and(|p| mdshared_app::fuzzy::score(query, &p.name).is_some()));
                    if !matched {
                        return None;
                    }
                }
                Some(i)
            })
            .collect();
        rows.sort_unstable_by_key(newest);

        let mut groups = Vec::<ViewGroup>::new();
        let mut by_key = HashMap::new();
        let mut buckets: [Vec<usize>; 5] = Default::default();
        let mut pinned = Vec::new();
        for i in rows {
            let s = &self.sessions[i];
            let (key, name) = match mode {
                Mode::Threads => {
                    let root = self.grouping.roots.get(i).copied().unwrap_or(i);
                    (format!("thread:{}", self.sessions[root].id), String::new())
                }
                _ if !keep_pins_with_group && self.marks.pinned(&s.id) => {
                    pinned.push(i);
                    continue;
                }
                Mode::Recent if !archive => {
                    buckets[calendar.bucket(s.last_active_ms)].push(i);
                    continue;
                }
                Mode::Topics => match self.marks.topic(&s.id) {
                    Some(topic) => (format!("topic:{}", topic.to_lowercase()), topic.to_owned()),
                    None => ("topic-none".into(), "No topic".into()),
                },
                Mode::Projects if !self.config.group_by_folder => self.project(i).map_or_else(
                    || (format!("project:{}", folder_key(&s.cwd)), folder_name(&s.cwd)),
                    |p| (format!("project:{}", p.key), p.name.clone()),
                ),
                _ => (format!("folder:{}", folder_key(&s.cwd)), folder_name(&s.cwd)),
            };
            let index = *by_key.entry(key.clone()).or_insert_with(|| {
                groups.push(ViewGroup { name, key, rows: Vec::new() });
                groups.len() - 1
            });
            groups[index].rows.push(i);
        }

        if keep_pins_with_group {
            groups.sort_by_key(|g| !g.rows.iter().any(|&i| self.marks.pinned(&self.sessions[i].id)));
        }
        if mode == Mode::Threads {
            for g in &mut groups {
                if g.rows.len() > 1 {
                    self.grouping.earlier.insert(g.rows[0], g.rows.len() - 1);
                    if query.is_empty() && !self.grouping.expanded.contains(&g.key) {
                        g.rows.truncate(1);
                    }
                }
            }
        }
        for g in &groups {
            if g.key.starts_with("project:") {
                let (mut running, mut waiting) = (0, 0);
                for &i in &g.rows {
                    let flags = self.row_flags(&self.sessions[i]);
                    running += usize::from(flags.running);
                    waiting += usize::from(flags.waiting);
                }
                self.grouping.project_summary.insert(g.key.clone(), format!("{running} running · {waiting} waiting"));
            }
        }
        if !pinned.is_empty() {
            groups.insert(0, ViewGroup { name: "Pinned".into(), key: "pinned".into(), rows: pinned });
        }
        for (i, mut rows) in buckets.into_iter().enumerate().filter(|(_, rows)| !rows.is_empty()) {
            if self.config.show_sittings {
                rows = self.cluster_rows(rows, offset);
            }
            groups.push(ViewGroup { name: BUCKETS[i].into(), key: format!("recent:{i}"), rows });
        }
        groups
    }

    pub fn project(&self, i: usize) -> Option<&Project> {
        self.grouping.projects.get(&folder_key(&self.sessions.get(i)?.cwd))
    }

    pub(crate) fn project_heading_summary(&self, group: &ViewGroup, now: u64, offset: i32) -> Option<String> {
        let counts = self.grouping.project_summary.get(&group.key)?;
        let newest = self.sessions[*group.rows.first()?].last_active_ms;
        Some(format!("{} · {counts}", crate::list::ago_at(now, newest, offset)))
    }

    fn cluster_rows(&mut self, rows: Vec<usize>, offset: i32) -> Vec<usize> {
        let keys: Vec<_> = rows
            .iter()
            .map(|&i| self.project(i).map_or_else(|| folder_key(&self.sessions[i].cwd), |p| p.key.clone()))
            .collect();
        let activity: Vec<_> = rows
            .iter()
            .zip(&keys)
            .map(|(&i, key)| {
                let s = &self.sessions[i];
                Activity { project: key, first: s.grouping.first_active, last: s.last_active_ms }
            })
            .collect();
        let ids = grouping::sittings(&activity, offset);
        let mut grouped = vec![Vec::new(); rows.len()];
        let mut order = Vec::new();
        for (i, session) in rows.into_iter().enumerate() {
            let group = &mut grouped[ids[i]];
            if group.is_empty() {
                order.push(ids[i]);
            }
            group.push(session);
        }
        let mut out = Vec::new();
        for i in order {
            let group = &grouped[i];
            if group.len() > 1 {
                for (r, &i) in group.iter().enumerate() {
                    self.grouping.brackets.insert(i, (r == 0, r + 1 == group.len()));
                }
            }
            out.extend(group);
        }
        out
    }

    pub fn thread_key(&self, i: usize) -> Option<String> {
        let root = *self.grouping.roots.get(i)?;
        Some(format!("thread:{}", self.sessions.get(root)?.id))
    }

    pub fn toggle_thread(&mut self, i: usize) {
        let Some(key) = self.thread_key(i) else { return };
        if self.grouping.expanded.remove(&key) {
            let selected = self.selected.as_deref().and_then(|id| self.by_id.get(id)).copied();
            if selected != Some(i) && selected.and_then(|j| self.thread_key(j)).as_deref() == Some(&key) {
                self.select(&self.sessions[i].id.clone());
            }
        } else {
            self.grouping.expanded.insert(key);
        }
        self.rebuild_view();
    }

    /// Brings the selected row into view, opening its thread if necessary.
    pub fn reveal_selected(&mut self) {
        if self.list_mode() == Mode::Threads
            && !self.conversation_search.on
            && let Some(id) = &self.selected
            && self.entry_of(id).is_none()
            && let Some(key) = self.by_id.get(id).and_then(|&i| self.thread_key(i))
            && self.grouping.expanded.insert(key)
        {
            self.rebuild_view();
        }
        if let Some(e) = self.selected.as_deref().and_then(|id| self.entry_of(id)) {
            self.sections.scroll = self.sections.reveal(e);
        }
    }

    pub fn related_visible(&self, s: &claude_switch_core::Session) -> bool {
        (self.config.show_hidden || !self.hidden_session(s))
            && (self.config.show_workers || !s.worker)
            && (self.config.show_archived || !s.archived)
    }

    pub fn record_copy(&mut self, id: &str, from: &str) -> bool {
        let previous = self.marks.clone();
        if !self.marks.set_parent(id, from) {
            self.error("Not opened", "Could not record the copy’s source session.".into());
            return false;
        }
        if !self.save_marks() {
            self.marks = previous;
            return false;
        }
        true
    }

    pub fn cancel_related(&mut self) {
        if let Some(job) = self.grouping.pending.take() {
            job.cancel();
        }
        self.grouping.related_id = None;
        self.grouping.related.clear();
    }

    pub fn load_related(&mut self, i: usize) {
        self.cancel_related();
        let s = &self.sessions[i];
        let root = self.grouping.roots.get(i).copied().unwrap_or(i);
        let thread = self.grouping.threads.get(&root).map(Vec::as_slice).unwrap_or(&[]);
        let visible = |j: usize| self.related_visible(&self.sessions[j]);
        let compare = |a: &claude_switch_core::Session, b: &claude_switch_core::Session| {
            a.last_active_ms.cmp(&b.last_active_ms).then_with(|| b.id.cmp(&a.id))
        };
        let mut related: Vec<_> = thread.iter().copied().filter(|&j| j != i && visible(j)).collect();
        related.sort_unstable_by(|&a, &b| compare(&self.sessions[b], &self.sessions[a]));
        let related = related
            .into_iter()
            .map(|j| {
                let other = &self.sessions[j];
                (other.id.clone(), if compare(other, s).is_gt() { Relation::Later } else { Relation::Earlier })
            })
            .collect();
        self.grouping.related = related;
        self.grouping.related_id = Some(s.id.clone());
        let id = s.id.clone();
        let index = Arc::clone(&self.grouping.related_index);
        self.grouping.pending =
            Job::start(self.wake_fn(), move |cancel, _| index.ranks(&id, || cancel.cancelled())).ok();
    }

    pub fn take_related(&mut self) -> bool {
        let Some(job) = self.grouping.pending.take_if(|j| !j.running()) else { return false };
        if self.selected == self.grouping.related_id
            && let Some(&i) = self.selected.as_deref().and_then(|id| self.by_id.get(id))
        {
            let root = self.grouping.roots.get(i);
            // Apply live marks/visibility after ranking, without copying any candidate prompts.
            let similar: Vec<_> = job
                .take()
                .unwrap_or_default()
                .into_iter()
                .map(|j| self.grouping.related_index.id(j))
                .filter(|&id| {
                    self.by_id
                        .get(id)
                        .is_some_and(|&j| self.grouping.roots.get(j) != root && self.related_visible(&self.sessions[j]))
                })
                .take(5)
                .map(|id| (id.to_owned(), Relation::Similar))
                .collect();
            self.grouping.related.extend(similar);
        }
        true
    }
}

#[cfg(test)]
mod tests;
