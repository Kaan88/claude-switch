//! Selected-session statistics worker and its bounded, in-memory cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use claude_switch_core::Session;
use claude_switch_core::stats::{self, Cached, Stats};
use mdshared_app::job::Job;

enum ResultKind {
    Real(Cached),
    Demo(Stats),
    Unavailable,
}

#[derive(Default)]
pub struct Size {
    pub current: Option<Stats>,
    pub pending: bool,
    worker: Option<(PathBuf, Job<ResultKind>)>,
    cached: HashMap<PathBuf, Cached>,
}

impl Size {
    pub fn cancel(&mut self) {
        if let Some((_, job)) = self.worker.take() {
            job.cancel();
        }
        self.current = None;
        self.pending = false;
    }

    pub fn start(&mut self, session: &Session, demo: bool, wake: impl Fn() + Send + Sync + 'static) {
        self.cancel();
        let path = session.file.clone();
        let file = path.clone();
        let cached = self.cached.get(&path).copied();
        let prompt = demo.then(|| session.last_prompt.clone());
        self.worker = Job::start(wake, move |cancel, _| {
            if let Some(prompt) = prompt {
                let json = crate::conversation_demo::transcript(&file, &prompt);
                let Ok(mut stats) = stats::collect(json.as_bytes(), || cancel.cancelled()) else {
                    return ResultKind::Unavailable;
                };
                stats.subagents = usize::from(file.file_stem().is_some_and(|stem| stem == "0"));
                ResultKind::Demo(stats)
            } else {
                stats::read(&file, cached, || cancel.cancelled())
                    .ok()
                    .flatten()
                    .map_or(ResultKind::Unavailable, ResultKind::Real)
            }
        })
        .ok()
        .map(|job| (path, job));
        self.pending = self.worker.is_some();
    }

    pub fn take(&mut self, selected: Option<&Path>) -> bool {
        let Some((path, job)) = &self.worker else { return false };
        let Some(result) = job.take().or_else(|| (!job.running()).then_some(ResultKind::Unavailable)) else {
            return false;
        };
        if selected == Some(path.as_path()) {
            self.current = match result {
                ResultKind::Real(cached) => {
                    if self.cached.len() >= 128 && !self.cached.contains_key(path) {
                        self.cached.clear();
                    }
                    self.cached.insert(path.clone(), cached);
                    Some(cached.stats)
                }
                ResultKind::Demo(stats) => Some(stats),
                ResultKind::Unavailable => None,
            };
        }
        self.pending = false;
        self.worker = None;
        true
    }
}

impl Drop for Size {
    fn drop(&mut self) {
        self.cancel();
    }
}
