//! Handover instead of Claude Code's own compaction (Settings › Handovers), in Claude Switch's
//! tabs only. While it is on, the CLI tabs Switch starts get `DISABLE_AUTO_COMPACT=1` (manual
//! `/compact` still works) and this watcher reads each such tab's transcript tail off the UI
//! thread. Once the context passes the threshold and the turn has ended, it asks first (the
//! handover dialog, opened for that tab) or makes the handover with the configured summarizer
//! and opens the new chat in the same account, linked to the old one (handover provenance).
//! It never types into the old chat and never forks it; the old tab stays open, marked.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::compact::{self, AutoHandover};
use claude_switch_core::transcript::{ChatPhase as Phase, Reader};
use claude_switch_core::{Roots, Session};
use mdshared_app::job::Job;
use mdshared_ui::host::Cx;

use crate::source::Source;
use crate::state::{AppState, Msg};

pub const ENV: (&str, &str) = ("DISABLE_AUTO_COMPACT", "1");
const POLL: Duration = Duration::from_secs(5);
const TAIL: usize = 512 * 1024;
/// The context window when nothing says otherwise, and the long-context one.
const WINDOW: u64 = 200_000;
const LONG_WINDOW: u64 = 1_000_000;
/// Where Claude Code compacts by default, as a share of the window (see DESIGN.md).
pub const CLAUDE_PERCENT: u8 = 95;

/// Per tab: what the watcher knows and did.
#[derive(Default)]
pub struct TabAuto {
    /// Started with Claude Code's own compaction off; only these tabs are watched.
    pub managed: bool,
    stamp: u64,
    /// Asked or handed over for this session; never again for it.
    pub done: Option<String>,
}

#[derive(Default)]
pub struct Watcher {
    job: Option<Job<Vec<Sample>>>,
    next: Option<Instant>,
    making: Option<Making>,
}

struct Making {
    session: Session,
    from: String,
    account: String,
    job: Job<Result<String, String>>,
}

#[derive(Debug)]
struct Sample {
    tab: u64,
    session: String,
    stamp: u64,
    context: u64,
    model: Option<String>,
    ended: bool,
}

/// Whether the model runs with a 1M window without a `[1m]` suffix (Claude Code's model docs:
/// Fable, Sonnet 5 and later, Haiku 5.5 and later, Opus 4.7 and later).
fn long_by_default(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    if model.contains("fable") {
        return true;
    }
    let version = |family: &str| -> Option<(u32, u32)> {
        let rest = model.split_once(&format!("{family}-"))?.1;
        let mut parts = rest.split(['-', '.', '[']).map(|p| p.parse::<u32>().ok());
        let major = parts.next()??;
        // A date-like tail (20250929) is not a minor version.
        let minor = parts.next().flatten().filter(|m| *m < 100).unwrap_or(0);
        Some((major, minor))
    };
    version("sonnet").is_some_and(|v| v.0 >= 5)
        || version("haiku").is_some_and(|v| v >= (5, 5))
        || version("opus").is_some_and(|v| v >= (4, 7))
}

/// The auto-compact window: a configured one, else the model's context window (1M for `[1m]`
/// and long-by-default models unless 1M context is turned off; also once the context is past
/// the ordinary window, which it otherwise cannot be).
pub fn window(model: Option<&str>, context: u64, limits: &Limits) -> u64 {
    if let Some(window) = limits.window.filter(|w| (100_000..=LONG_WINDOW).contains(w)) {
        return window.max(context);
    }
    let long = model.is_some_and(|m| m.to_ascii_lowercase().contains("[1m]") || long_by_default(m));
    if long && !limits.no_long || context > WINDOW { LONG_WINDOW } else { WINDOW }
}

/// The context size that triggers a handover: the owner's percentage, else Claude Code's own
/// point (its percentage override only lowers it, as in Claude Code).
pub fn threshold(window: u64, set: Option<u8>, claude_override: Option<u8>) -> u64 {
    let claude = claude_override.filter(|p| (1..=100).contains(p)).map_or(CLAUDE_PERCENT, |p| p.min(CLAUDE_PERCENT));
    window * u64::from(set.unwrap_or(claude)) / 100
}

/// What Claude Code is told about compaction for an account, as far as Switch can read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub percent: Option<u8>,
    pub window: Option<u64>,
    pub no_long: bool,
}

fn tokens(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase();
    let (digits, scale) = match text.strip_suffix('k') {
        Some(d) => (d, 1_000),
        None => text.strip_suffix('m').map_or((text.as_str(), 1), |d| (d, 1_000_000)),
    };
    digits.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0).map(|n| (n * scale as f64) as u64)
}

/// `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW` and
/// `CLAUDE_CODE_DISABLE_1M_CONTEXT` from the account's `settings.json` `env` or Switch's own
/// environment (which its launches inherit), and `autoCompactWindow` in that file. A per-session
/// `/autocompact` or `--autocompact` is not visible to Switch.
pub fn account_limits(cli_dir: &std::path::Path) -> Limits {
    let settings = claude_switch_core::read_bounded(&cli_dir.join("settings.json"), 1 << 20)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_default();
    let var = |name: &str| settings["env"][name].as_str().map(str::to_owned).or_else(|| std::env::var(name).ok());
    Limits {
        percent: var("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE")
            .and_then(|p| p.trim().parse::<u8>().ok())
            .filter(|p| (1..=100).contains(p)),
        window: var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
            .as_deref()
            .and_then(tokens)
            .or_else(|| settings["autoCompactWindow"].as_u64()),
        no_long: var("CLAUDE_CODE_DISABLE_1M_CONTEXT").is_some_and(|v| v.trim() == "1"),
    }
}

/// Whether this sample calls for a handover now.
fn due(sample: &Sample, prefs: &compact::Preferences, limits: &Limits) -> Option<u8> {
    if !sample.ended || prefs.auto == AutoHandover::Off {
        return None;
    }
    let window = window(sample.model.as_deref(), sample.context, limits);
    let at = threshold(window, prefs.auto_percent, limits.percent);
    (sample.context >= at).then(|| (sample.context * 100 / window.max(1)).min(100) as u8)
}

fn read(file: &std::path::Path) -> Option<(u64, Option<String>, bool)> {
    let chat = Reader::open(file, TAIL).ok()?.chat();
    let ended = chat.pending.is_none() && chat.latest.is_some_and(|(_, phase)| phase == Phase::EndTurn);
    Some((chat.context?, chat.model, ended))
}

/// A tab whose current session is watched: started with compaction off, running, verified, and
/// not yet asked or handed over for this session.
fn watching(tab: &crate::tabs::Tab) -> bool {
    tab.auto.managed && tab.term.running() && tab.resume_id.is_some() && tab.auto.done != tab.resume_id
}

impl AppState {
    /// The launch environment for a Claude tab while the feature is on.
    pub fn auto_handover_env(&self, launch: &mut claude_switch_core::Launch) -> bool {
        let on = self.config.compact.auto != AutoHandover::Off && !self.source.is_demo();
        if on {
            launch.env.insert(ENV.0.into(), ENV.1.into());
        }
        on
    }

    pub fn auto_handover_tick(&mut self, cx: &mut Cx<'_, Msg>) {
        let now = cx.now();
        self.take_auto_samples();
        self.take_auto_handover();
        let watched = self.config.compact.auto != AutoHandover::Off && self.tabs.list.iter().any(watching);
        if !watched || self.source.is_demo() {
            return;
        }
        if self.compact.auto.job.is_none() && self.compact.auto.next.is_none_or(|at| at <= now) {
            let watch: Vec<_> = self
                .tabs
                .list
                .iter()
                .filter(|t| watching(t))
                .filter_map(|t| {
                    let file = t.pane.as_ref()?.verified_transcript()?.to_owned();
                    Some((t.id, t.resume_id.clone()?, file, t.auto.stamp))
                })
                .collect();
            self.compact.auto.next = Some(now + POLL);
            self.compact.auto.job = Job::start(self.wake_fn(), move |cancel, _| {
                watch
                    .into_iter()
                    .take_while(|_| !cancel.cancelled())
                    .filter_map(|(tab, session, file, old): (u64, String, PathBuf, u64)| {
                        let stamp = claude_switch_core::modified_ms(&file);
                        if stamp == 0 || stamp == old {
                            return None;
                        }
                        let (context, model, ended) = read(&file)?;
                        Some(Sample { tab, session, stamp, context, model, ended })
                    })
                    .collect()
            })
            .ok();
        }
        if let Some(at) = self.compact.auto.next {
            cx.wake_at(at.max(now + Duration::from_millis(50)));
        }
    }

    fn take_auto_samples(&mut self) {
        if !self.compact.auto.job.as_ref().is_some_and(|job| !job.running()) {
            return;
        }
        let samples = self.compact.auto.job.take().and_then(|job| job.take()).unwrap_or_default();
        for sample in samples {
            let Some(tab) = self.tabs.get_mut(sample.tab) else { continue };
            if tab.resume_id.as_deref() != Some(sample.session.as_str()) {
                continue;
            }
            tab.auto.stamp = sample.stamp;
            let limits = tab
                .account
                .as_deref()
                .and_then(|a| claude_switch_core::account(&self.config, a).ok())
                .map(|a| account_limits(&a.cli_dir))
                .unwrap_or_default();
            if let Some(percent) = due(&sample, &self.config.compact, &limits) {
                self.auto_handover_due(sample.tab, &sample.session, percent);
            }
        }
    }

    fn auto_handover_due(&mut self, tab: u64, session: &str, percent: u8) {
        let ask = self.config.compact.auto == AutoHandover::Ask;
        if self.compact.auto.making.is_some() || ask && (self.dialog.is_some() || !self.nothing_over()) {
            // Read it again at the next poll, even if the transcript does not change meanwhile.
            if let Some(t) = self.tabs.get_mut(tab) {
                t.auto.stamp = 0;
            }
            return;
        }
        let Some(t) = self.tabs.get(tab) else { return };
        let Some(account) = t.account.clone() else { return };
        let Some(source) = self.by_id.get(session).and_then(|&i| self.sessions.get(i)).cloned() else { return };
        let automatic = self.config.compact.auto == AutoHandover::Automatic;
        let ready = compact::provider(&self.config.compact.model)
            .is_some_and(|provider| self.config.compact.consents.contains(provider));
        if let Some(t) = self.tabs.get_mut(tab) {
            t.auto.done = Some(session.to_owned());
        }
        if automatic && ready {
            self.start_auto_handover(source, account, percent);
        } else {
            self.ask_tab_compact(tab, Some(&account));
            let why = if automatic { " Approve a summary model once to let it run by itself." } else { "" };
            self.compact_notice(format!(
                "The context is at {percent}% of its window. Make a handover to continue in a new chat.{why}"
            ));
        }
    }

    fn start_auto_handover(&mut self, session: Session, account: String, percent: u8) {
        let Source::Real(roots) = &self.source else { return };
        let roots: Roots = roots.clone();
        let program = match crate::compact_runner::opencode() {
            Ok(program) => program,
            Err(e) => return self.warn("Automatic handover did not start", e),
        };
        let model = self.config.compact.model.clone();
        let from = self.title_of(&session).to_owned();
        let data_session = session.clone();
        let name = from.clone();
        let job = Job::start(self.wake_fn(), move |cancel, _| {
            let data = compact::Handover::read_cancel(data_session, name, || cancel.cancelled())
                .map_err(|_| "Could not read the transcript.".to_string())?;
            crate::compact_runner::make(&roots, &program, &model, &data, data.focus().unwrap_or(""), || {
                cancel.cancelled()
            })
        });
        match job {
            Ok(job) => {
                self.compact.auto.making = Some(Making { session, from, account, job });
                self.toast(format!("Context at {percent}%: making a handover for a new chat…"), Instant::now());
            }
            Err(_) => self.warn("Automatic handover did not start", "Could not start the handover worker.".into()),
        }
    }

    fn take_auto_handover(&mut self) {
        if !self.compact.auto.making.as_ref().is_some_and(|m| !m.job.running()) {
            return;
        }
        let Some(Making { session, from, account, job }) = self.compact.auto.making.take() else { return };
        let result = job
            .take()
            .unwrap_or_else(|| Err("The handover worker stopped.".into()))
            .and_then(|text| self.open_auto_handover(&session, &from, &account, &text));
        match result {
            Ok(true) => {}
            Ok(false) => {
                self.toast("Handed over: the new chat continues from a summary.".into(), Instant::now());
            }
            Err(e) => self.warn("Automatic handover failed", e),
        }
    }

    fn open_auto_handover(&mut self, session: &Session, from: &str, account: &str, text: &str) -> Result<bool, String> {
        let Source::Real(roots) = &self.source else { return Err("Demo".into()) };
        let roots = roots.clone();
        crate::compact_open::validate_prompt(text)?;
        let saved =
            compact::save(&roots, &session.id, text).map_err(|e| format!("Could not keep the handover: {e}"))?;
        let name = crate::compact_open::names::suggest(&self.marks, &self.sessions, from);
        // In place by default: the same tab and process continue (`handover_in_place`).
        if let Some(tab) = self.in_place_target(&session.id, Some(account)) {
            self.continue_in_place(tab, session, (from.to_owned(), name), text, saved)?;
            return Ok(true);
        }
        let agent = self.config.compact.default_agent.clone().filter(|default| {
            let found = claude_switch_core::account(&self.config, account)
                .map(|target| crate::compact_dialog::agents::discover(&roots, target, &session.cwd, || false))
                .unwrap_or_default();
            crate::compact_dialog::agents::checked_default(default, &found, account).is_ok()
        });
        let request = crate::compact_open::prepare(
            &roots,
            &self.config,
            session,
            (from.to_owned(), &name),
            account,
            text,
            &saved,
        )?
        .with_agent(agent.as_deref())?;
        self.open_handover_tab(request)?;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_first_opens_the_handover_dialog_for_that_tab_once_per_session() {
        use crate::{dialogs::Dialog, tabs::NewTab, term::Term};
        let mut st = AppState::new(Source::Demo, Some(false), Default::default());
        st.apply_scan(crate::demo::scan(&st.config));
        let session = st.sessions.iter().find(|s| s.desktop_id.is_none()).unwrap().id.clone();
        let new =
            NewTab { label: "t".into(), color: [0; 3], session: Some(session.clone()), account: Some("main".into()) };
        let tab = st.tabs.open(new, Term::empty((8, 4)));
        st.tabs.get_mut(tab).unwrap().auto.managed = true;
        st.config.compact.auto = AutoHandover::Ask;
        st.dialog = Some(Dialog::About);
        st.auto_handover_due(tab, &session, 91);
        assert!(matches!(st.dialog, Some(Dialog::About)), "never over another dialog");
        assert_eq!(st.tabs.get(tab).unwrap().auto.done, None, "asked again later");
        st.dialog = None;
        st.auto_handover_due(tab, &session, 91);
        assert_eq!(st.compact_source(), Some((session.as_str(), Some(tab))));
        let Some(Dialog::Compact(dialog)) = &st.dialog else { panic!("handover dialog") };
        assert!(dialog.notice_text().is_some_and(|t| t.contains("91%")));
        let t = st.tabs.get(tab).unwrap();
        assert_eq!(t.auto.done, t.resume_id, "not again for this session");
        st.tabs.get_mut(tab).unwrap().resume_id = Some("another".into());
        let t = st.tabs.get(tab).unwrap();
        assert_ne!(t.auto.done, t.resume_id, "a new session in the tab is watched again");
    }

    #[test]
    fn only_switch_started_claude_tabs_get_compaction_off_while_on() {
        let mut st = AppState::new(
            Source::Real(Roots { home: "C:/none".into(), appdata: "C:/none".into() }),
            None,
            Default::default(),
        );
        let mut launch = claude_switch_core::launch::shell(std::path::Path::new("C:/"));
        assert!(!st.auto_handover_env(&mut launch));
        assert!(!launch.env.contains_key(ENV.0));
        st.config.compact.auto = AutoHandover::Automatic;
        assert!(st.auto_handover_env(&mut launch));
        assert_eq!(launch.env.get(ENV.0).map(String::as_str), Some("1"));
    }

    fn sample(context: u64, ended: bool) -> Sample {
        Sample { tab: 1, session: "s".into(), stamp: 1, context, model: Some("claude-opus".into()), ended }
    }

    #[test]
    fn window_follows_the_model_the_settings_and_the_observed_context() {
        let none = Limits::default();
        assert_eq!(window(Some("claude-sonnet-4-5-20250929"), 10, &none), WINDOW);
        assert_eq!(window(Some("claude-opus-4-6"), 10, &none), WINDOW);
        assert_eq!(window(Some("claude-sonnet-4-6[1m]"), 10, &none), LONG_WINDOW);
        for long in ["claude-opus-4-7", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-2"]
        {
            assert_eq!(window(Some(long), 10, &none), LONG_WINDOW, "{long}");
        }
        assert_eq!(window(Some("claude-haiku-4-5-20251001"), 10, &none), WINDOW);
        let short = Limits { no_long: true, ..Default::default() };
        assert_eq!(window(Some("claude-opus-5-5"), 10, &short), WINDOW);
        assert_eq!(window(None, WINDOW + 1, &short), LONG_WINDOW, "the context proves the window");
        let set = Limits { window: Some(400_000), ..Default::default() };
        assert_eq!(window(Some("claude-opus-5-5"), 10, &set), 400_000);
        let tiny = Limits { window: Some(5), ..Default::default() };
        assert_eq!(window(None, 10, &tiny), WINDOW, "an implausible configured window is ignored");
        assert_eq!(
            [tokens("500k"), tokens("1M"), tokens("250000"), tokens("x")],
            [Some(500_000), Some(1_000_000), Some(250_000), None]
        );
    }

    #[test]
    fn threshold_prefers_the_owner_then_claude_codes_override_then_its_default() {
        assert_eq!(threshold(200_000, Some(60), Some(80)), 120_000);
        assert_eq!(threshold(200_000, None, Some(80)), 160_000);
        assert_eq!(threshold(200_000, None, Some(100)), threshold(200_000, None, None), "an override only lowers");
        assert_eq!(threshold(200_000, None, None), 200_000 * u64::from(CLAUDE_PERCENT) / 100);
        assert_eq!(threshold(200_000, None, Some(0)), 200_000 * u64::from(CLAUDE_PERCENT) / 100);
    }

    #[test]
    fn due_only_after_the_turn_ended_past_the_threshold_and_while_on() {
        let none = Limits::default();
        let mut prefs = compact::Preferences { auto: AutoHandover::Ask, auto_percent: Some(50), ..Default::default() };
        assert_eq!(due(&sample(99_999, true), &prefs, &none), None);
        assert_eq!(due(&sample(100_000, false), &prefs, &none), None, "never during a turn");
        assert_eq!(due(&sample(100_000, true), &prefs, &none), Some(50));
        prefs.auto = AutoHandover::Off;
        assert_eq!(due(&sample(190_000, true), &prefs, &none), None);
    }

    #[test]
    fn account_settings_env_is_read_for_the_limits() {
        let dir = std::env::temp_dir().join(format!("cs-auto-limits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = serde_json::json!({"env": {
            "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": "70",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "300k",
            "CLAUDE_CODE_DISABLE_1M_CONTEXT": "1",
        }});
        std::fs::write(dir.join("settings.json"), settings.to_string()).unwrap();
        assert_eq!(account_limits(&dir), Limits { percent: Some(70), window: Some(300_000), no_long: true });
        std::fs::remove_dir_all(dir).unwrap();
    }
}
