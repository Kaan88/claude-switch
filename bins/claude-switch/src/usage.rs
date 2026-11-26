//! Demand routing and result-only state for usage. Worker jobs return figures or a closed error kind.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use claude_switch_core::Roots;
use claude_switch_core::usage::{
    Attempt, Breakdown, CACHE_NAME, Cache, Demand, Error, Limit, Reading, load_cache, now_ms, save_cache,
};
use mdshared_app::job::Job;

use crate::source::Source;
use crate::state::{AppState, Target};

type FetchJob = Job<Result<Reading, Error>>;
type Fetch = fn(&Roots, &Path, &dyn Fn() -> bool) -> Result<Reading, Error>;

#[derive(Default)]
pub struct Usage {
    readings: Cache,
    attempts: HashMap<String, Attempt>,
    jobs: HashMap<String, FetchJob>,
    sign_in_pending: HashSet<String>,
    file: Option<PathBuf>,
    cache_job: Option<(u64, Job<std::io::Result<()>>)>,
    cache_write: CacheWrite,
    pub popup: Popup,
    pub repaint_at: Option<Instant>,
    pub demo_echo: bool,
    #[cfg(test)]
    pub(crate) demo_keys: Vec<u8>,
}

#[derive(Default)]
pub struct Popup {
    pub open: bool,
    pinned: bool,
    pub due: Option<Instant>,
    pub scroll: f32,
}

impl Popup {
    pub fn takes_keyboard(&self) -> bool {
        self.open && self.pinned
    }
}

#[derive(Default)]
struct CacheWrite {
    revision: u64,
    persisted: u64,
    failures: u32,
    retry: Option<Instant>,
}

impl CacheWrite {
    fn changed(&mut self) {
        self.revision += 1;
        if self.failures >= 5 {
            self.failures = 0;
            self.retry = None;
        }
    }

    fn ready(&self, now: Instant) -> bool {
        self.revision != self.persisted && self.failures < 5 && self.retry.is_none_or(|at| at <= now)
    }

    fn finish(&mut self, revision: u64, success: bool, now: Instant) {
        if success {
            self.persisted = revision;
            self.failures = 0;
            self.retry = None;
        } else {
            self.failures += 1;
            self.retry = (self.failures < 5).then(|| now + Duration::from_secs(5 << (self.failures - 1)));
        }
    }
}

impl Usage {
    pub fn cache_retry_at(&self) -> Option<Instant> {
        self.cache_job.is_none().then_some(self.cache_write.retry).flatten()
    }
    pub fn new(source: &Source) -> Self {
        match source {
            Source::Real(roots) => {
                let file = roots.config_file().with_file_name(CACHE_NAME);
                Self { readings: load_cache(&file), file: Some(file), ..Self::default() }
            }
            Source::Demo => {
                let now = now_ms();
                let limit =
                    |key: &str, used, minutes: i64| Limit::legacy(key, used, Some((now / 1000) as i64 + minutes * 60));
                let reading = Reading {
                    read_at: now.saturating_sub(180_000),
                    limits: vec![
                        limit("five_hour", 42.0, 130),
                        limit("seven_day", 64.0, 4320),
                        limit("seven_day_opus", 86.0, 4320),
                        limit("seven_day_sonnet", 37.0, 4320),
                    ],
                    breakdown: vec![
                        Breakdown { label: "Claude Code".into(), percent: Some(39.0) },
                        Breakdown { label: "Chats".into(), percent: Some(25.0) },
                        Breakdown { label: "Cowork".into(), percent: None },
                        Breakdown { label: "Other".into(), percent: None },
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

    pub fn seed_notification_demo(&mut self) {
        let now = now_ms();
        let limit = |key, used, hours: i64| Limit::legacy(key, used, Some((now / 1000) as i64 + hours * 3600));
        for (slug, week, opus) in [("main", 96.0, 86.0), ("work", 27.0, 32.0)] {
            let reading = Reading {
                read_at: now,
                limits: vec![
                    limit("five_hour", 38.0, 2),
                    limit("seven_day", week, 72),
                    limit("seven_day_opus", opus, 72),
                ],
                breakdown: vec![],
            };
            self.readings.insert(slug.into(), reading);
        }
        self.attempts.remove("work");
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
        self.sign_in_pending.clear();
        for job in self.jobs.values() {
            job.cancel();
        }
    }
}

impl AppState {
    pub fn start_usage_hover_demo(&mut self, now: Instant) {
        if !self.source.is_demo() || !self.usage.demo_echo {
            return;
        }
        let Some(tab) = self.tabs.shown_mut() else { return };
        if let Some(pane) = &mut tab.pane {
            pane.chat.enabled = false;
        }
        tab.term.feed(b"\r\nType here while the usage preview is open:\r\n> \x1b[?25h");
        self.open_usage(now, false);
    }

    pub fn sync_usage_setting(&mut self) -> bool {
        let Source::Real(roots) = &self.source else { return false };
        let enabled = claude_switch_core::config::usage_enabled(roots);
        let changed = enabled != self.config.show_usage;
        self.apply_committed_usage(enabled);
        changed
    }

    pub fn disable_usage(&mut self) {
        self.usage.disable();
        self.clear_usage_notices();
    }

    pub fn request_usage(&mut self, now: Instant, demand: Demand, only: Option<&str>) {
        self.request_usage_with(now, demand, only, crate::usage_fetch::fetch);
    }

    fn request_usage_with(&mut self, now: Instant, demand: Demand, only: Option<&str>, fetch: Fetch) {
        if !usage_allowed(self.config.show_usage, &self.source) {
            return;
        }
        let crate::source::Source::Real(roots) = &self.source else { return };
        let clock = now_ms();
        for a in &self.config.accounts {
            if only.is_some_and(|slug| slug != a.slug) {
                continue;
            }
            let wake = self.wake_fn();
            let usage = &mut self.usage;
            let cached_age = usage.cached_age(&a.slug, clock);
            let attempt = usage.attempts.entry(a.slug.clone()).or_default();
            if attempt.busy && matches!(demand, Demand::SignIn) {
                usage.sign_in_pending.insert(a.slug.clone());
            }
            if !attempt.allowed(demand, now, cached_age) {
                continue;
            }
            attempt.start(now);
            let dir = a.cli_dir.clone();
            let roots = roots.clone();
            match Job::start(wake, move |cancel, _| fetch(&roots, &dir, &|| !cancel.cancelled())) {
                Ok(job) => {
                    usage.jobs.insert(a.slug.clone(), job);
                }
                Err(_) => attempt.finish(now, Err(Error::SettingsUnavailable)),
            }
        }
    }

    pub fn take_usage_jobs(&mut self, now: Instant) -> bool {
        self.take_usage_jobs_with(now, crate::usage_fetch::fetch)
    }

    fn take_usage_jobs_with(&mut self, now: Instant, fetch: Fetch) -> bool {
        let mut changed = self.sync_usage_setting();
        let wake = self.wake_fn();
        let usage = &mut self.usage;
        let mut accepted = Vec::new();
        usage.jobs.retain(|slug, job| {
            if job.running() {
                return true;
            }
            let result = job.take().unwrap_or(Err(Error::SettingsUnavailable));
            let attempt = usage.attempts.entry(slug.clone()).or_default();
            // An opt-out also prevents a late result from changing the displayed or saved data.
            attempt.finish(now, result.as_ref().map(|_| ()).map_err(|e| *e));
            changed = true;
            if job.cancelled() || !self.config.show_usage {
                return false;
            }
            if let Ok(reading) = result
                && self.config.accounts.iter().any(|a| &a.slug == slug)
            {
                let previous = usage.readings.insert(slug.clone(), reading);
                accepted.push((slug.clone(), previous));
                usage.cache_write.changed();
            }
            false
        });
        if let Some((rev, job)) = usage.cache_job.take_if(|(_, job)| !job.running()) {
            usage.cache_write.finish(rev, job.take().is_some_and(|r| r.is_ok()), now);
        }
        if self.config.show_usage
            && usage.cache_write.ready(now)
            && usage.cache_job.is_none()
            && let Some(file) = usage.file.clone()
        {
            usage.readings.retain(|slug, _| self.config.accounts.iter().any(|a| &a.slug == slug));
            let readings = usage.readings.clone();
            let rev = usage.cache_write.revision;
            match Job::start(wake, move |_, _| save_cache(&file, &readings)) {
                Ok(job) => usage.cache_job = Some((rev, job)),
                Err(_) => usage.cache_write.finish(rev, false, now),
            }
        }
        let mut retry = Vec::new();
        usage.sign_in_pending.retain(|slug| {
            if usage.jobs.contains_key(slug) {
                true
            } else {
                retry.push(slug.clone());
                false
            }
        });
        for slug in retry {
            self.request_usage_with(now, Demand::SignIn, Some(&slug), fetch);
        }
        for (slug, previous) in accepted {
            self.usage_warning_reading(now, &slug, previous.as_ref());
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
        let alternatives = self.usage_constrained(now_ms()).is_some();
        self.request_usage(now, Demand::Automatic, (!alternatives).then_some(slug.as_str()));
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
            && !matches!(
                target,
                Some(
                    Target::UsageLines
                        | Target::UsagePopup
                        | Target::UsageRefresh
                        | Target::UsageOpenAs
                        | Target::Account
                )
            )
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
    fn a_failed_save_stays_dirty_retries_are_bounded_and_old_success_cannot_clear_new_data() {
        let mut cache = CacheWrite::default();
        let now = Instant::now();
        cache.changed();
        assert!(cache.ready(now));
        cache.finish(1, false, now);
        assert_eq!(cache.persisted, 0);
        assert!(!cache.ready(now + Duration::from_secs(4)));
        assert!(cache.ready(now + Duration::from_secs(5)));
        for _ in 1..5 {
            cache.finish(1, false, now);
        }
        assert!(cache.retry.is_none() && !cache.ready(now + Duration::from_secs(360)));
        cache.changed();
        assert!(cache.ready(now));
        cache.finish(1, true, now);
        assert!(cache.ready(now));
        cache.finish(2, true, now);
        assert!(!cache.ready(now));
    }

    #[test]
    fn demo_and_disabled_modes_do_not_schedule_a_fetch() {
        let mut demo = AppState::new(Source::Demo, Some(true), WindowFile::default());
        for demand in [Demand::Automatic, Demand::Refresh, Demand::SignIn] {
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
        assert!(!app.usage.popup.takes_keyboard());
        assert!(app.nothing_over(), "hover is not a keyboard overlay");
        assert!(!app.usage_hover(due, Some(Target::UsagePopup)));
        assert!(app.usage_hover(due, Some(Target::ListBlank)));
        app.open_usage(due, true);
        assert!(!app.usage_hover(due, None));
        assert!(app.usage.popup.open);
        assert!(app.usage.popup.takes_keyboard());
        app.open_usage(due, true);
        assert!(!app.usage.popup.open);
    }

    #[test]
    fn hovering_usage_keeps_terminal_text_and_enter_in_the_terminal() {
        use mdshared_ui::host::KeyPress;
        use mdshared_ui::keys::{KeyName, Mods};
        let mut app = AppState::new(Source::Demo, Some(true), WindowFile::default());
        app.start_demo_session();
        app.usage.demo_echo = true;
        app.window_focused = true;
        let now = Instant::now();
        app.start_usage_hover_demo(now);
        assert!(app.terminal_focused());
        for (name, text) in [(KeyName::Char('x'), Some("x".into())), (KeyName::Enter, None)] {
            app.term_key(now, &KeyPress { name: Some(name), mods: Mods::default(), text, repeat: false });
        }
        assert_eq!(app.usage.demo_keys, b"x\r");
        assert!(app.usage.popup.open && !app.usage.popup.takes_keyboard());
    }

    #[test]
    fn other_windows_opt_out_drops_a_completed_but_unaccepted_reading() {
        let folder = std::env::temp_dir().join(format!("cs-w3-optout-{}", std::process::id()));
        let roots = claude_switch_core::Roots { home: folder.clone(), appdata: folder.clone() };
        let mut second = AppState::new(Source::Demo, None, WindowFile::default());
        second.source = Source::Real(roots.clone());
        second.usage.readings.clear();
        second.usage.popup = Popup { open: true, pinned: true, ..Popup::default() };
        let (wake, completed) = std::sync::mpsc::channel();
        let job = Job::start(
            move || {
                let _ = wake.send(());
            },
            |_, _| Ok(Reading { read_at: 42, limits: vec![], breakdown: vec![] }),
        )
        .unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        // The wake comes from inside the job, just before it marks itself finished.
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!job.running());
        second.usage.jobs.insert("main".into(), job);
        claude_switch_core::config::set_usage(&roots, false).unwrap();
        assert!(second.take_usage_jobs(Instant::now()));
        assert!(!second.config.show_usage && !second.usage.popup.open);
        assert!(second.usage.reading("main").is_none());
        assert!(second.usage.jobs.is_empty());
    }
}

#[cfg(test)]
#[path = "usage/recovery_tests.rs"]
mod recovery_tests;
