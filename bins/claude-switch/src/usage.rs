//! Demand routing and result-only state for usage. Worker jobs return figures or a closed error kind.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::usage::{
    Attempt, CACHE_NAME, Cache, Demand, Error, Limit, Reading, load_cache, now_ms, save_cache,
};
use mdshared_app::job::Job;

use crate::source::Source;
use crate::state::{AppState, Target};

type FetchJob = Job<Result<Reading, Error>>;

#[derive(Default)]
pub struct Usage {
    readings: Cache,
    attempts: HashMap<String, Attempt>,
    jobs: HashMap<String, FetchJob>,
    file: Option<PathBuf>,
    cache_job: Option<Job<()>>,
    cache_dirty: bool,
    pub popup: Popup,
    pub repaint_at: Option<Instant>,
}

#[derive(Default)]
pub struct Popup {
    pub open: bool,
    pinned: bool,
    pub due: Option<Instant>,
    pub scroll: f32,
}

impl Usage {
    pub fn new(source: &Source) -> Self {
        match source {
            Source::Real(roots) => {
                let file = roots.config_file().with_file_name(CACHE_NAME);
                Self { readings: load_cache(&file), file: Some(file), ..Self::default() }
            }
            Source::Demo => {
                let now = now_ms();
                let limit = |key: &str, used, minutes: i64| Limit {
                    key: key.into(),
                    used,
                    reset: Some((now / 1000) as i64 + minutes * 60),
                };
                let reading = Reading {
                    read_at: now.saturating_sub(180_000),
                    limits: vec![
                        limit("five_hour", 42.0, 130),
                        limit("seven_day", 64.0, 4320),
                        limit("seven_day_opus", 86.0, 4320),
                        limit("seven_day_sonnet", 37.0, 4320),
                    ],
                };
                let mut work = Attempt::default();
                work.finish(Instant::now(), Err(Error::Expired));
                Self {
                    readings: Cache::from([("main".into(), reading)]),
                    attempts: HashMap::from([("work".into(), work)]),
                    ..Self::default()
                }
            }
        }
    }

    pub fn reading(&self, slug: &str) -> Option<&Reading> {
        self.readings.get(slug)
    }

    pub fn attempt(&self, slug: &str) -> Option<&Attempt> {
        self.attempts.get(slug)
    }

    pub fn can_refresh(&self, slug: &str, now: Instant) -> bool {
        let age = self.cached_age(slug, now_ms());
        self.attempt(slug).unwrap_or(&Attempt::default()).allowed(Demand::Refresh, now, age)
    }

    fn cached_age(&self, slug: &str, now: u64) -> Option<Duration> {
        self.reading(slug).map(|r| Duration::from_millis(now.saturating_sub(r.read_at)))
    }

    pub fn disable(&mut self) {
        self.popup = Popup::default();
        self.repaint_at = None;
        for job in self.jobs.values() {
            job.cancel();
        }
    }
}

impl AppState {
    pub fn request_usage(&mut self, now: Instant, demand: Demand, only: Option<&str>) {
        if !usage_allowed(self.config.show_usage, &self.source) {
            return;
        }
        let clock = now_ms();
        for a in &self.config.accounts {
            if only.is_some_and(|slug| slug != a.slug) {
                continue;
            }
            let wake = self.wake_fn();
            let usage = &mut self.usage;
            let cached_age = usage.cached_age(&a.slug, clock);
            let attempt = usage.attempts.entry(a.slug.clone()).or_default();
            if !attempt.allowed(demand, now, cached_age) {
                continue;
            }
            attempt.start(now);
            let dir = a.cli_dir.clone();
            match Job::start(wake, move |cancel, _| crate::usage_fetch::fetch(&dir, || !cancel.cancelled())) {
                Ok(job) => {
                    usage.jobs.insert(a.slug.clone(), job);
                }
                Err(_) => attempt.finish(now, Err(Error::Network)),
            }
        }
    }

    pub fn take_usage_jobs(&mut self, now: Instant) -> bool {
        let wake = self.wake_fn();
        let usage = &mut self.usage;
        let mut changed = false;
        usage.jobs.retain(|slug, job| {
            if job.running() {
                return true;
            }
            let result = job.take().unwrap_or(Err(Error::Network));
            let attempt = usage.attempts.entry(slug.clone()).or_default();
            // An opt-out also prevents a late result from changing the displayed or saved data.
            attempt.finish(now, result.as_ref().map(|_| ()).map_err(|e| *e));
            changed = true;
            if job.cancelled() {
                return false;
            }
            if let Ok(reading) = result
                && self.config.accounts.iter().any(|a| &a.slug == slug)
            {
                usage.readings.insert(slug.clone(), reading);
                usage.cache_dirty = true;
            }
            false
        });
        if usage.cache_job.as_ref().is_some_and(|job| !job.running()) {
            usage.cache_job = None;
        }
        if self.config.show_usage
            && usage.cache_dirty
            && usage.cache_job.is_none()
            && let Some(file) = usage.file.clone()
        {
            usage.cache_dirty = false;
            usage.readings.retain(|slug, _| self.config.accounts.iter().any(|a| &a.slug == slug));
            let readings = usage.readings.clone();
            usage.cache_job = Job::start(wake, move |_, _| {
                let _ = save_cache(&file, &readings);
            })
            .ok();
        }
        changed
    }

    pub fn open_usage(&mut self, now: Instant, pinned: bool) {
        if !self.config.show_usage {
            return;
        }
        let popup = &mut self.usage.popup;
        if pinned && popup.pinned {
            *popup = Popup::default();
            return;
        }
        *popup = Popup { open: true, pinned, ..Popup::default() };
        let slug = self.current().slug.clone();
        self.request_usage(now, Demand::Automatic, Some(&slug));
    }

    pub fn usage_hover(&mut self, now: Instant, target: Option<Target>) -> bool {
        let blocked = self.dialog.is_some()
            || self.menu.is_some()
            || self.palette.is_some()
            || self.accounts_dd.is_open()
            || self.picture_overlay();
        let popup = &mut self.usage.popup;
        if blocked {
            let was_open = popup.open;
            *popup = Popup::default();
            return was_open;
        }
        if popup.open
            && !popup.pinned
            && !matches!(target, Some(Target::UsageLines | Target::UsagePopup | Target::UsageRefresh | Target::Account))
        {
            *popup = Popup::default();
            return true;
        }
        if !popup.open && target == Some(Target::UsageLines) {
            popup.due.get_or_insert(now + mdshared_ui::anim::document::TIP_DELAY);
        } else {
            popup.due = None;
        }
        false
    }

    pub fn usage_hover_due(&mut self, now: Instant) -> bool {
        if self.usage.popup.due.take_if(|at| *at <= now).is_some()
            && self.hover == Some(Target::UsageLines)
            && self.nothing_over()
            && !self.picture_overlay()
        {
            self.open_usage(now, false);
            return self.usage.popup.open;
        }
        false
    }

    pub fn running_for_usage(&self, slug: &str) -> usize {
        let mut ids: HashSet<_> =
            self.running.iter().filter(|r| r.account == slug).map(|r| r.session_id.as_str()).collect();
        let mut new_sessions = 0;
        for tab in
            self.tabs.list.iter().filter(|t| t.account.as_deref() == Some(slug) && t.term.running() && t.pane.is_some())
        {
            if let Some(id) = &tab.session {
                ids.insert(id);
            } else {
                new_sessions += 1;
            }
        }
        ids.len() + new_sessions
    }
}

fn usage_allowed(enabled: bool, source: &Source) -> bool {
    enabled && !source.is_demo()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winfile::WindowFile;

    #[test]
    fn demo_and_disabled_modes_do_not_schedule_a_fetch() {
        let mut demo = AppState::new(Source::Demo, Some(true), WindowFile::default());
        for demand in [Demand::Automatic, Demand::Refresh] {
            demo.request_usage(Instant::now(), demand, None);
            assert!(demo.usage.jobs.is_empty());
        }
        assert!(demo.usage.file.is_none());
        assert!(demo.usage.reading("main").unwrap().limits.iter().any(|limit| limit.used >= 80.0));
        assert!(demo.usage.reading("work").is_none());
        let real = Source::Real(claude_switch_core::Roots { home: PathBuf::new(), appdata: PathBuf::new() });
        assert!(!usage_allowed(false, &real));
        assert!(usage_allowed(true, &real));
    }

    #[test]
    fn hover_opens_after_the_toolkit_delay_and_click_pins_until_dismissed() {
        let mut app = AppState::new(Source::Demo, Some(true), WindowFile::default());
        let now = Instant::now();
        let due = now + mdshared_ui::anim::document::TIP_DELAY;
        app.hover = Some(Target::UsageLines);
        app.usage_hover(now, app.hover);
        assert!(!app.usage_hover_due(due - Duration::from_millis(1)));
        assert!(app.usage_hover_due(due));
        assert!(!app.usage_hover(due, Some(Target::UsagePopup)));
        assert!(app.usage_hover(due, Some(Target::ListBlank)));
        app.open_usage(due, true);
        assert!(!app.usage_hover(due, None));
        assert!(app.usage.popup.open);
        app.open_usage(due, true);
        assert!(!app.usage.popup.open);
    }
}
