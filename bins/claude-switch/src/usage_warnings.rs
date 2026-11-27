//! Threshold notices and an account suggestion; all input is already sanitized usage data.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use claude_switch_core::config::{WarningWindows, claim_warning, load_warning_windows};
use claude_switch_core::usage::{Demand, Limit, Reading, Slot};
use claude_switch_core::{Account, sessions};
use mdshared_ui::host::Cx;

use crate::{
    source::Source,
    state::{AppState, Msg},
};

const THRESHOLDS: [u8; 2] = [80, 95];
const FRESH_MS: u64 = 300_000;

#[derive(Default)]
pub struct Warnings {
    // Keep the highest threshold per account, slot, label and reset window. A reading that
    // moves down does not re-arm a notice. Unknown resets conservatively share a single window.
    seen: WarningWindows,
    pub next: Option<Instant>,
    deferred: VecDeque<String>,
    pending: VecDeque<(String, Limit, u8)>,
    flash: bool,
}

fn active(limit: &Limit, now: u64) -> bool {
    limit.slot != Slot::Other && limit.reset.is_none_or(|at| at > (now / 1000) as i64)
}

impl Warnings {
    pub fn new(source: &Source) -> Self {
        match source {
            Source::Real(roots) => Self { seen: load_warning_windows(roots), ..Self::default() },
            Source::Demo => Self::default(),
        }
    }

    fn observe(&mut self, slug: &str, reading: &Reading, previous: Option<&Reading>) -> Vec<(Limit, u8)> {
        let mut notices = Vec::new();
        let clock = reading.read_at;
        self.seen.retain(|(_, _, _, end), _| end.is_none_or(|at| at > (clock / 1000) as i64));
        for limit in reading.limits.iter().filter(|limit| active(limit, clock)) {
            let same_window =
                |old: &&Limit| old.slot == limit.slot && old.label == limit.label && old.reset == limit.reset;
            let prior_used = previous.and_then(|old| old.limits.iter().find(same_window)).map_or(0.0, |old| old.used);
            let baseline = THRESHOLDS.into_iter().rev().find(|&n| f64::from(n) <= prior_used).unwrap_or(0);
            let slot = match limit.slot {
                Slot::Session => "session",
                Slot::Week => "week",
                Slot::Model => "model",
                Slot::Other => continue,
            };
            let entry =
                self.seen.entry((slug.into(), slot.into(), limit.label.clone(), limit.reset)).or_insert(baseline);
            for threshold in THRESHOLDS {
                if threshold > *entry && limit.used >= f64::from(threshold) {
                    notices.push((limit.clone(), threshold));
                    *entry = threshold;
                }
            }
        }
        notices
    }
}

fn has_room(reading: &Reading, constrained: &[&Limit], now: u64) -> bool {
    let limits = reading.limits.iter().filter(|limit| active(limit, now)).collect::<Vec<_>>();
    now.saturating_sub(reading.read_at) <= FRESH_MS
        && !limits.is_empty()
        && limits.iter().all(|limit| limit.used < f64::from(THRESHOLDS[0]))
        && constrained.iter().all(|from| {
            limits.iter().any(|to| to.slot == from.slot && (from.slot != Slot::Model || to.label == from.label))
        })
}

impl AppState {
    pub fn usage_active_in_app(&self, slug: &str) -> bool {
        self.tabs
            .list
            .iter()
            .any(|tab| tab.account.as_deref() == Some(slug) && tab.pane.is_some() && tab.term.running())
    }

    pub fn usage_warning_tick(&mut self, now: Instant) {
        if self.source.is_demo() || !self.config.show_usage || !self.config.notifications.usage_warnings {
            self.usage_warnings.next = None;
            return;
        }
        if self.usage_warnings.next.is_some_and(|at| at > now) {
            return;
        }
        self.retry_usage_warnings(now);
        let slugs = self
            .config
            .accounts
            .iter()
            .filter(|a| self.usage_active_in_app(&a.slug))
            .map(|a| a.slug.clone())
            .collect::<Vec<_>>();
        self.usage_warnings.next =
            (!slugs.is_empty() || !self.usage_warnings.pending.is_empty()).then_some(now + Duration::from_secs(15));
        for slug in slugs {
            self.request_usage(now, Demand::Automatic, Some(&slug));
        }
    }

    pub fn usage_warning_reading(&mut self, now: Instant, slug: &str, previous: Option<&Reading>) {
        if !self.config.show_usage
            || !self.config.notifications.usage_warnings
            || !(self.usage_active_in_app(slug) || self.notifications.demo)
        {
            return;
        }
        let Some(reading) = self.usage.reading(slug) else { return };
        let notices = self.usage_warnings.observe(slug, reading, previous);
        self.usage_warnings.pending.extend(notices.into_iter().map(|(limit, n)| (slug.into(), limit, n)));
        self.retry_usage_warnings(now);
    }

    pub fn clear_usage_notices(&mut self) {
        self.usage_warnings.deferred.clear();
        self.usage_warnings.pending.clear();
        self.usage_warnings.flash = false;
        self.notifications.tray.discard_usage_notices();
    }

    fn retry_usage_warnings(&mut self, now: Instant) {
        let clock = claude_switch_core::usage::now_ms();
        let pending = &mut self.usage_warnings.pending;
        pending.retain(|(slug, limit, _)| active(limit, clock) && self.config.accounts.iter().any(|a| &a.slug == slug));
        while let Some((slug, limit, threshold)) = self.usage_warnings.pending.front() {
            let claimed = match &self.source {
                Source::Demo => Ok(true),
                Source::Real(roots) => claim_warning(
                    roots,
                    (slug.clone(), limit.slot.id().into(), limit.label.clone(), limit.reset),
                    *threshold,
                ),
            };
            let Ok(claimed) = claimed else {
                self.usage_warnings.next.get_or_insert(now + Duration::from_secs(15));
                break;
            };
            let (slug, limit, threshold) = self.usage_warnings.pending.pop_front().expect("pending claim");
            if !claimed {
                continue;
            }
            let window = match limit.slot {
                Slot::Session => "the session".into(),
                Slot::Week => "the week".into(),
                _ => limit.label,
            };
            let reset = limit.reset.map_or_else(
                || "reset time unavailable".into(),
                |at| {
                    let civil = crate::usage_fetch::local_time(at);
                    let day = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][usize::from(civil.weekday)];
                    format!("resets {day} {:02}:{:02}", civil.hour, civil.minute)
                },
            );
            let text = format!("{}: {threshold}% of {window} used, {reset}", self.account_name(&slug));
            let native = self.notifications.tray.notice("Claude Switch usage", &text, None);
            if self.window_focused || self.notifications.demo {
                self.toast(text, now);
            } else if !native {
                let deferred = &mut self.usage_warnings.deferred;
                if deferred.len() == 16 {
                    deferred.pop_front();
                }
                deferred.push_back(text);
                self.usage_warnings.flash = true;
            }
        }
    }

    pub fn usage_warning_present(&mut self, cx: &mut Cx<'_, Msg>) {
        if !self.config.show_usage || !self.config.notifications.usage_warnings {
            self.clear_usage_notices();
            return;
        }
        if cx.focused() && !self.notifications.hidden && cx.window().is_minimized() != Some(true) {
            let mut pending = std::mem::take(&mut self.usage_warnings.deferred);
            while let Some(text) = pending.pop_front() {
                self.toast(text, cx.now());
                cx.request_redraw_all();
            }
            self.usage_warnings.flash = false;
        } else if std::mem::take(&mut self.usage_warnings.flash) {
            crate::tray::flash(cx.window());
        }
    }

    pub fn usage_constrained(&self, clock: u64) -> Option<(&str, Vec<&Limit>)> {
        if !self.config.notifications.usage_warnings || !self.config.show_usage {
            return None;
        }
        let tab = self.tabs.shown()?;
        if !self.notifications.demo && !tab.term.running() {
            return None;
        }
        if !(tab.session.as_ref().is_some_and(|id| self.by_id.contains_key(id))
            || tab.attention.transcript().is_some()
            || self.notifications.demo && tab.pane.is_some())
        {
            return None;
        }
        let slug = tab.account.as_deref()?;
        let constrained = self
            .usage
            .reading(slug)?
            .limits
            .iter()
            .filter(|limit| active(limit, clock) && limit.used >= f64::from(THRESHOLDS[0]))
            .collect::<Vec<_>>();
        (!constrained.is_empty()).then_some((slug, constrained))
    }

    pub fn usage_switch_account(&self, clock: u64) -> Option<&Account> {
        let (slug, constrained) = self.usage_constrained(clock)?;
        self.config.accounts.iter().find(|a| {
            a.slug != slug
                && self.statuses.get(&a.slug).is_none_or(|status| status.cli_logged_in || self.notifications.demo)
                && self.usage.reading(&a.slug).is_some_and(|r| has_room(r, &constrained, clock))
        })
    }

    pub fn usage_open_as(&mut self, clock: u64) {
        let Some(account) = self.usage_switch_account(clock).map(|a| a.slug.clone()) else { return };
        let Some(tab) = self.tabs.shown() else { return };
        if !self.source.is_demo() {
            let id = tab.id;
            self.usage.popup = Default::default();
            self.ask_tab_compact(id, Some(&account));
            return;
        }
        let session = tab
            .session
            .as_ref()
            .and_then(|id| self.by_id.get(id))
            .map(|&i| self.sessions[i].clone())
            .or_else(|| tab.attention.transcript().cloned())
            .or_else(|| {
                if !self.notifications.demo {
                    return None;
                }
                sessions::read_transcript(&tab.pane.as_ref()?.transcript_source.find_file()?)
            });
        self.usage.popup = Default::default();
        if let Some(session) = session {
            self.ask_compact_for(session, Some(&account), false);
        }
    }

    pub fn usage_tray_tip(&self) -> String {
        if !self.config.show_usage {
            return "Claude Switch · Usage off".into();
        }
        self.config
            .accounts
            .iter()
            .map(|a| {
                let name = a.name.split_whitespace().collect::<Vec<_>>().join(" ");
                let figures = crate::usage_ui::hairlines(self.usage.reading(&a.slug))
                    .iter()
                    .map(|l| format!("{} {:.0}%", l.label, l.used))
                    .collect::<Vec<_>>()
                    .join(", ");
                if figures.is_empty() { format!("{name}: no reading") } else { format!("{name}: {figures}") }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn week(used: f64, reset: i64) -> Reading {
        Reading { read_at: 0, limits: vec![Limit::legacy("seven_day", used, Some(reset))], breakdown: vec![] }
    }

    #[test]
    fn threshold_steps_are_independent_and_only_rearm_after_reset() {
        let mut w = Warnings::default();
        let raised = |w: &mut Warnings, used, reset| {
            w.observe("main", &week(used, reset), None).into_iter().map(|(_, n)| n).collect::<Vec<_>>()
        };
        assert!(raised(&mut w, 79.9, 100).is_empty());
        assert_eq!(raised(&mut w, 80.0, 100), [80]);
        assert!(raised(&mut w, 81.0, 100).is_empty());
        assert_eq!(raised(&mut w, 95.0, 100), [95]);
        assert!(raised(&mut w, 21.0, 100).is_empty());
        assert!(raised(&mut w, 99.0, 100).is_empty());
        assert_eq!(raised(&mut w, 99.0, 200), [80, 95]);
        assert_eq!(w.observe("work", &week(99.0, 200), None).len(), 2);
    }

    #[test]
    fn cached_high_is_not_a_crossing_and_alternatives_must_be_current_and_comparable() {
        let mut w = Warnings::default();
        assert!(w.observe("main", &week(82.0, 100), Some(&week(80.0, 100))).is_empty());
        assert_eq!(w.observe("main", &week(96.0, 100), Some(&week(82.0, 100))).len(), 1);
        let high = week(95.0, 1000);
        assert!(has_room(&week(30.0, 1000), &[&high.limits[0]], 0));
        assert!(!has_room(&week(30.0, 1000), &[&high.limits[0]], FRESH_MS + 1));
        assert!(!has_room(&week(81.0, 1000), &[&high.limits[0]], 0));
        let model = Limit::legacy("seven_day_opus", 95.0, Some(1000));
        assert!(!has_room(&week(30.0, 1000), &[&model], 0));
    }

    #[test]
    fn failed_notice_claim_stays_pending_until_the_ledger_is_writable() {
        let scratch = crate::testing::Scratch::new("cs-w3-warning-retry");
        let folder = scratch.to_path_buf();
        let roots = claude_switch_core::Roots { home: folder.clone(), appdata: folder.clone() };
        let ledger = roots.config_file().with_file_name("usage-notices.json");
        std::fs::create_dir_all(&ledger).unwrap();
        let mut app = AppState::new(Source::Demo, None, crate::winfile::WindowFile::default());
        app.source = Source::Real(roots.clone());
        let limit = Limit::legacy("seven_day", 82.0, None);
        app.usage_warnings.pending.push_back(("main".into(), limit, 80));
        let now = Instant::now();
        app.retry_usage_warnings(now);
        assert_eq!(app.usage_warnings.pending.len(), 1);
        assert!(app.usage_warnings.next.is_some());
        std::fs::remove_dir(&ledger).unwrap();
        app.retry_usage_warnings(now);
        assert!(app.usage_warnings.pending.is_empty());
        assert_eq!(load_warning_windows(&roots).values().copied().collect::<Vec<_>>(), [80]);
    }

    #[test]
    fn demo_offer_opens_handover_for_the_visible_tab_without_starting_a_program() {
        let mut st = AppState::new(Source::Demo, Some(true), crate::winfile::WindowFile::default());
        st.notifications.demo = true;
        st.start_demo_session();
        st.usage.seed_notification_demo();
        let now = claude_switch_core::usage::now_ms();
        assert_eq!(st.usage_switch_account(now).map(|a| a.slug.as_str()), Some("work"));
        st.usage_open_as(now);
        assert!(matches!(st.dialog, Some(crate::dialogs::Dialog::Compact(_))));
        assert_eq!(st.tabs.running(), 0);
    }
}
