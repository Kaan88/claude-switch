//! Debounced search worker and projection into the normal selectable session rows.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use claude_switch_core::fulltext::{Hits, Query};
use mdshared_app::job::Job;
use mdshared_ui::sections::Sections;

use crate::state::{AppState, Focus, ROW_H, ViewGroup};

const PAUSE: Duration = Duration::from_millis(220);
pub const SNIPPET_LINE_H: f32 = 17.0;
type Inbox = Arc<Mutex<HashMap<String, Hits>>>;

struct SearchJob {
    inbox: Inbox,
    job: Job<usize>,
}

#[derive(Default)]
pub struct Search {
    pub on: bool,
    pub due: Option<Instant>,
    worker: Option<SearchJob>,
    found: HashMap<String, Hits>,
    skipped: usize,
    failed: bool,
}

impl Search {
    pub fn hits(&self, id: &str) -> Option<&Hits> {
        if self.on { self.found.get(id) } else { None }
    }

    fn cancel(&mut self) {
        if let Some(running) = self.worker.take() {
            running.job.cancel();
        }
    }

    pub fn status(&self, query: &str, visible: usize) -> String {
        if query.trim().is_empty() {
            "Type to search prompts and replies".into()
        } else if self.due.is_some() || self.worker.is_some() {
            format!("Searching\u{2026}  {visible} found")
        } else if self.failed {
            "Could not start search".into()
        } else {
            let result = match visible {
                0 => "No matches".into(),
                1 => "1 session found".into(),
                _ => format!("{visible} sessions found"),
            };
            if self.skipped == 0 { result } else { format!("{result} · {} unreadable", self.skipped) }
        }
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl AppState {
    pub fn toggle_conversation_search(&mut self, now: Instant) {
        self.show_sessions();
        self.focus = Focus::Search;
        self.conversation_search.on = !self.conversation_search.on;
        self.restart_conversation_search(now);
        self.rebuild_view();
        self.sections.scroll = 0.0;
        if self.selected.as_deref().is_none_or(|id| self.entry_of(id).is_none()) {
            self.select_first();
        }
        self.reveal_selected();
    }

    pub fn restart_conversation_search(&mut self, now: Instant) {
        let search = &mut self.conversation_search;
        search.cancel();
        search.found.clear();
        search.skipped = 0;
        search.failed = false;
        search.due = (search.on && !self.search.text().trim().is_empty()).then_some(now + PAUSE);
    }

    pub fn start_conversation_search(&mut self, now: Instant) -> bool {
        if !self.scanned || self.conversation_search.due.is_none_or(|at| at > now) {
            return false;
        }
        self.conversation_search.due = None;
        let Some(query) = Query::new(self.search.text()) else { return false };
        let demo = self.source.is_demo();
        let mut files: Vec<_> = self
            .sessions
            .iter()
            .map(|s| (s.last_active_ms, s.id.clone(), s.file.clone(), demo.then(|| s.last_prompt.clone())))
            .collect();
        let inbox: Inbox = Default::default();
        let updates = inbox.clone();
        let wake = self.wake_fn();
        let job = Job::start(self.wake_fn(), move |cancel, _| {
            files.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let mut skipped = 0;
            for (_, id, file, prompt) in files {
                if cancel.cancelled() {
                    break;
                }
                let publish = |hits: &Hits| {
                    let notify = {
                        let mut batch = updates.lock().unwrap_or_else(|err| err.into_inner());
                        let empty = batch.is_empty();
                        batch.insert(id.clone(), hits.clone());
                        empty
                    };
                    if notify {
                        wake();
                    }
                };
                let result = if let Some(prompt) = prompt {
                    let fixture = crate::conversation_demo::transcript(&file, &prompt);
                    query.scan(fixture.as_bytes(), || cancel.cancelled(), publish)
                } else {
                    File::open(file).and_then(|file| {
                        let len = file.metadata()?.len();
                        query.scan(file.take(len), || cancel.cancelled(), publish)
                    })
                };
                skipped += usize::from(result.is_err());
            }
            skipped
        });
        match job {
            Ok(job) => self.conversation_search.worker = Some(SearchJob { inbox, job }),
            Err(_) => self.conversation_search.failed = true,
        }
        true
    }

    pub fn take_conversation_search(&mut self) -> bool {
        let search = &mut self.conversation_search;
        let Some(worker) = &search.worker else { return false };
        let done = worker.job.take().or_else(|| (!worker.job.running()).then_some(0));
        let incoming = std::mem::take(&mut *worker.inbox.lock().unwrap_or_else(|err| err.into_inner()));
        let changed = !incoming.is_empty();
        search.found.extend(incoming);
        if let Some(skipped) = done {
            search.skipped = skipped;
            search.worker = None;
        }
        if changed {
            self.rebuild_view();
            if self.selected.as_deref().is_none_or(|id| self.entry_of(id).is_none()) {
                self.select_first();
            }
        }
        changed || done.is_some()
    }

    pub fn project_conversation_search(&mut self) {
        let mut rows: Vec<_> = self
            .conversation_search
            .found
            .keys()
            .filter_map(|id| self.by_id.get(id).copied())
            .filter(|&i| {
                let s = &self.sessions[i];
                self.list_session_visible(s, true)
            })
            .collect();
        rows.sort_unstable_by_key(|&i| (std::cmp::Reverse(self.sessions[i].last_active_ms), &self.sessions[i].id));
        let scroll = self.sections.scroll;
        self.sections = Sections::new(self.sections.view, ROW_H);
        self.row_locations.clear();
        self.groups.clear();
        if !rows.is_empty() {
            self.sections.add_group(rows.len(), None);
            for (r, &i) in rows.iter().enumerate() {
                let hits = &self.conversation_search.found[&self.sessions[i].id];
                self.sections
                    .set_height(r, 40.0 + SNIPPET_LINE_H * (hits.shown.len() + usize::from(hits.more > 0)) as f32);
                self.row_locations.insert(i, (0, r));
            }
            self.groups.push(ViewGroup { name: String::new(), key: String::new(), rows });
        }
        self.sections.scroll = self.sections.clamp_scroll(scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;

    fn demo() -> AppState {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        app.apply_scan(crate::demo::scan(&app.config));
        app
    }

    fn finish(app: &mut AppState) {
        let limit = Instant::now() + Duration::from_secs(5);
        while app.conversation_search.worker.is_some() {
            app.take_conversation_search();
            assert!(Instant::now() < limit, "search never finished");
            std::thread::yield_now();
        }
    }

    #[test]
    fn demo_worker_debounces_and_projects_newest_first_with_capped_snippets() {
        let mut app = demo();
        let now = Instant::now();
        app.search.insert("WIDTH splitter");
        app.toggle_conversation_search(now);
        assert!(!app.start_conversation_search(now + PAUSE / 2));
        assert!(app.groups.is_empty() && app.selected.is_none());
        assert!(app.start_conversation_search(now + PAUSE));
        finish(&mut app);
        let rows = &app.groups[0].rows;
        assert_eq!(rows.len(), 2);
        let (first, second) = (&app.sessions[rows[0]], &app.sessions[rows[1]]);
        assert!(first.last_active_ms > second.last_active_ms);
        let hits = app.conversation_search.hits(&first.id).unwrap();
        assert_eq!((hits.shown.len(), hits.more), (3, 2));
        assert_eq!(app.selected.as_ref(), Some(&first.id));
        let second_id = second.id.clone();
        assert!(crate::list::key(&mut app, &mdshared_ui::keys::KeyName::Down));
        assert_eq!(app.selected.as_ref(), Some(&second_id));
    }

    #[test]
    fn clearing_and_retyping_discards_the_old_workers_mailbox() {
        let mut app = demo();
        let now = Instant::now();
        app.search.insert("regression");
        app.toggle_conversation_search(now);
        assert!(app.start_conversation_search(now + PAUSE));
        app.search.clear();
        app.restart_conversation_search(now + PAUSE);
        assert!(app.conversation_search.worker.is_none() && app.conversation_search.due.is_none());
        app.search.insert("not-in-the-fixture");
        app.restart_conversation_search(now + PAUSE);
        assert!(app.start_conversation_search(now + PAUSE * 2));
        finish(&mut app);
        assert!(app.conversation_search.found.is_empty());
    }
}
