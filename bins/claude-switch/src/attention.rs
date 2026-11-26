//! Waiting marks and notification routing. Transcript reads are bounded and off the UI thread.

use std::path::PathBuf;
use std::time::Instant;

use claude_switch_core::config::Attention;
use claude_switch_core::sessions::{TurnActivity, turn_activity};
use mdshared_app::job::Job;
use mdshared_ui::host::Cx;

use crate::state::{AppState, Msg, POLL};
use crate::tabs::{NewTab, Tab};
use crate::tray::{self, Event, Tray};
use crate::view::model::Source;

pub struct TabAttention {
    pub marked: bool,
    /// A pending terminal signal; true means the user already saw the tab.
    bell: Option<bool>,
    permission_visible: bool,
    prompt: Option<u64>,
    consumed: bool,
    started: u64,
    file: Option<PathBuf>,
    transcript: Option<claude_switch_core::Session>,
    stamp: u64,
}

impl Default for TabAttention {
    fn default() -> Self {
        Self {
            marked: false,
            bell: None,
            permission_visible: false,
            prompt: None,
            consumed: false,
            started: claude_switch_core::usage::now_ms(),
            file: None,
            transcript: None,
            stamp: 0,
        }
    }
}

impl TabAttention {
    pub fn terminal(&mut self, screen: &mut mdshared_term::Screen, output: bool, attended: bool) -> bool {
        let bell = screen.take_bell();
        let mut waiting = bell;
        if output {
            let rows = (screen.rows().saturating_sub(18)..screen.rows()).map(|row| screen.row_text(row));
            let permission = permission_prompt(rows);
            waiting |= permission && !self.permission_visible;
            self.permission_visible = permission;
        }
        if waiting {
            self.bell = Some(attended || self.bell == Some(true));
        }
        bell
    }

    pub fn transcript(&self) -> Option<&claude_switch_core::Session> {
        self.transcript.as_ref()
    }

    fn observe(&mut self, sample: Option<(u64, TurnActivity)>, visible: bool, mode: Attention) -> bool {
        let mut waiting = false;
        if let Some((stamp, turn)) = sample {
            if turn.prompt.is_some() && turn.prompt != self.prompt {
                // A bell may have arrived before the first readable record for this tab.
                if self.prompt.is_some() {
                    self.consumed = false;
                    self.marked = false;
                }
                self.prompt = turn.prompt;
            }
            let fresh = turn.waiting_since.map_or(stamp >= self.started, |at| at >= (self.started / 1000) as i64);
            waiting = turn.waiting && fresh;
            self.stamp = stamp;
        }
        let bell = self.bell.take();
        waiting |= bell.is_some();
        let seen = visible || bell == Some(true);
        if seen || mode == Attention::Off {
            self.marked = false;
        }
        if !waiting || std::mem::replace(&mut self.consumed, true) || seen || mode == Attention::Off {
            return false;
        }
        self.marked = true;
        mode == Attention::ToastAndMark
    }
}

fn permission_prompt(rows: impl IntoIterator<Item = String>) -> bool {
    let (mut question, mut allow, mut deny, mut footer) = (false, false, false, false);
    for row in rows {
        let row = row.trim().trim_start_matches(['❯', '>']).trim_start();
        question |= row.starts_with("Do you want to ") || row.starts_with("Would you like to ");
        let option = |label| {
            row.strip_prefix(label).is_some_and(|rest| rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric()))
        };
        allow |= ["1. Yes", "1. Allow"].into_iter().any(option);
        deny |= ["2. No", "3. No", "2. Deny", "3. Deny"].into_iter().any(option);
        footer |= row.starts_with("Esc to cancel");
    }
    question && allow && deny && footer
}

struct Watch {
    id: u64,
    source: Source,
    file: Option<PathBuf>,
    stamp: u64,
    need_transcript: bool,
}

struct Sample {
    id: u64,
    file: PathBuf,
    stamp: u64,
    activity: TurnActivity,
    transcript: Option<claude_switch_core::Session>,
}

#[derive(Default)]
pub struct Notifications {
    pub tray: Tray,
    pub hidden: bool,
    pub demo: bool,
    next: Option<Instant>,
    job: Option<Job<Vec<Sample>>>,
    watched_bells: Vec<u64>,
}

fn tab_name(st: &AppState, tab: &Tab) -> String {
    tab.rename
        .as_deref()
        .or_else(|| tab.session.as_deref().and_then(|id| st.marks.name(id)))
        .map_or_else(|| tab.title().into_owned(), str::to_owned)
}

impl AppState {
    pub fn setup_notifications(&mut self, cx: &mut Cx<'_, Msg>) {
        let proxy = self.proxy.clone();
        self.notifications.tray.attach(cx.window(), move |event| {
            if let Some(p) = &proxy
                && let Ok(p) = p.lock()
            {
                p.send(Msg::Tray(event));
            }
        });
        if self.notifications.demo {
            self.config.notifications.tray_icon = true;
            self.config.notifications.close_to_tray = true;
            self.usage.seed_notification_demo();
            let previous = self.tabs.active;
            let id = self.tabs.open(
                NewTab {
                    label: "Build notes · Demo".into(),
                    color: self.account_color("work"),
                    session: Some("attention-demo".into()),
                    account: Some("work".into()),
                },
                crate::term::Term::empty(self.grid),
            );
            self.tabs.active = previous;
            if let Some(tab) = self.tabs.get_mut(id) {
                tab.term.feed(b"Claude Code demo\r\nBuild notes are ready.\r\n\x07");
                tab.attention.terminal(&mut tab.term.screen, true, false);
            }
        }
        self.attention_tick(cx);
        if self.notifications.demo {
            self.usage_warning_reading(cx.now(), "main", None);
            self.open_usage(cx.now(), true);
        }
    }

    pub fn sync_tray(&mut self, cx: &mut Cx<'_, Msg>) {
        let enabled = self.config.notifications.tray_icon;
        let tip = if enabled { self.usage_tray_tip() } else { String::new() };
        // Never strand a hidden window if the setting is changed or Explorer loses its icon.
        self.notifications.tray.sync(enabled, tip);
        if self.notifications.hidden && !self.notifications.tray.enabled() {
            self.show_from_tray(cx);
        }
    }

    pub fn request_window_close(&mut self, cx: &mut Cx<'_, Msg>) {
        self.sync_tray(cx);
        if self.config.notifications.close_to_tray && self.notifications.tray.enabled() {
            self.notifications.hidden = true;
            self.term_focus(false);
            cx.window().set_visible(false);
        } else if self.ask_quit() {
            self.quit = true;
        }
    }

    fn show_from_tray(&mut self, cx: &mut Cx<'_, Msg>) {
        self.notifications.hidden = false;
        tray::show(cx.window());
    }

    pub fn tray_event(&mut self, cx: &mut Cx<'_, Msg>, event: Event) {
        match event {
            Event::Open => self.show_from_tray(cx),
            Event::ExplorerRestarted => self.notifications.tray.explorer_restarted(),
            Event::BalloonClick => {
                if let Some(tab) = self.notifications.tray.balloon_done() {
                    if let Some(id) = tab {
                        self.show_tab(id);
                    }
                    self.show_from_tray(cx);
                }
            }
            Event::BalloonDone => {
                self.notifications.tray.balloon_done();
            }
            Event::Menu(at) => {
                let mut tabs = self
                    .tabs
                    .list
                    .iter()
                    .filter(|t| t.term.running() && t.pane.is_some() || self.notifications.demo && t.session.is_some())
                    .map(|t| (t.id, tab_name(self, t)))
                    .collect::<Vec<_>>();
                let mut labels = tabs.drain(..).map(|(id, label)| (Some(id), label)).collect::<Vec<_>>();
                labels.extend(["New session", "Open window", "Quit"].map(|s| (None, s.to_owned())));
                let names = labels.iter().map(|(_, name)| name.clone()).collect::<Vec<_>>();
                if let Some(i) = self.notifications.tray.menu(at, &names).filter(|&i| i < labels.len()) {
                    self.show_from_tray(cx);
                    match labels[i].0 {
                        Some(id) => self.show_tab(id),
                        None if i == labels.len() - 3 && self.dialog.is_none() => {
                            self.command(cx.now(), crate::actions::NEW_SESSION, None);
                        }
                        None if i == labels.len() - 1 && self.ask_quit() => self.quit = true,
                        None => {}
                    }
                }
            }
        }
        self.attention_tick(cx);
        cx.request_redraw_all();
    }

    pub fn attention_tick(&mut self, cx: &mut Cx<'_, Msg>) {
        self.sync_tray(cx);
        self.usage_warning_present(cx);
        let mode = self.config.notifications.attention;
        let now = cx.now();
        let present = cx.focused() && !self.notifications.hidden && cx.window().is_minimized() != Some(true);
        let mut samples = Vec::new();
        let mut sampled_bells = Vec::new();
        if self.notifications.job.as_ref().is_some_and(|job| !job.running()) {
            samples = self.notifications.job.take().and_then(|job| job.take()).unwrap_or_default();
            sampled_bells = std::mem::take(&mut self.notifications.watched_bells);
        }
        let mut notices = Vec::new();
        let mut changed = false;
        for tab in &mut self.tabs.list {
            let visible = present && self.tabs.active == Some(tab.id);
            let sample = samples.iter().position(|s| s.id == tab.id).map(|i| {
                let sample = samples.swap_remove(i);
                tab.attention.file = Some(sample.file);
                if let Some(transcript) = sample.transcript {
                    tab.attention.transcript = Some(transcript);
                }
                (sample.stamp, sample.activity)
            });
            if visible && tab.attention.bell.is_some() {
                tab.attention.bell = Some(true);
            }
            let held = if mode != Attention::Off
                && !self.source.is_demo()
                && tab.term.running()
                && !sampled_bells.contains(&tab.id)
            {
                tab.attention.bell.take()
            } else {
                None
            };
            let before = tab.attention.marked;
            if tab.attention.observe(sample, visible || held == Some(true), mode) {
                notices.push(tab.id);
            }
            if held.is_some() {
                tab.attention.bell = held;
            }
            changed |= before != tab.attention.marked;
        }
        let due = self.notifications.next.is_none_or(|at| at <= now);
        let bell = self.tabs.list.iter().any(|tab| tab.attention.bell.is_some());
        if mode != Attention::Off && self.notifications.job.is_none() && !self.source.is_demo() && (due || bell) {
            let watch = self
                .tabs
                .list
                .iter()
                .filter(|tab| tab.term.running())
                .filter_map(|tab| {
                    Some(Watch {
                        id: tab.id,
                        source: tab.pane.as_ref()?.transcript_source.clone(),
                        file: tab.attention.file.clone(),
                        stamp: tab.attention.stamp,
                        need_transcript: tab.attention.transcript.is_none()
                            && tab.session.as_ref().is_none_or(|id| !self.by_id.contains_key(id)),
                    })
                })
                .collect::<Vec<_>>();
            if !watch.is_empty() {
                self.notifications.next = Some(now + POLL);
                self.notifications.watched_bells =
                    watch.iter().filter_map(|w| self.tabs.get(w.id)?.attention.bell.map(|_| w.id)).collect();
                self.notifications.job = Job::start(self.wake_fn(), move |_, _| {
                    watch
                        .into_iter()
                        .filter_map(|w| {
                            let file = w.file.or_else(|| w.source.find_file())?;
                            let stamp = claude_switch_core::modified_ms(&file);
                            if stamp == 0 || stamp == w.stamp {
                                return None;
                            }
                            let activity = turn_activity(&file)?;
                            let transcript = w
                                .need_transcript
                                .then(|| claude_switch_core::sessions::read_transcript(&file))
                                .flatten();
                            Some(Sample { id: w.id, activity, file, stamp, transcript })
                        })
                        .collect()
                })
                .ok();
                if self.notifications.job.is_none() {
                    // A thread creation failure must not lose a terminal signal.
                    for id in self.notifications.watched_bells.drain(..) {
                        let visible = present && self.tabs.active == Some(id);
                        if let Some(tab) = self.tabs.get_mut(id) {
                            let before = tab.attention.marked;
                            if tab.attention.observe(None, visible, mode) {
                                notices.push(id);
                            }
                            changed |= before != tab.attention.marked;
                        }
                    }
                }
            }
        }
        if changed {
            cx.request_redraw_all();
        }
        for id in notices {
            let Some(tab) = self.tabs.get(id) else { continue };
            let text = format!("{} is waiting for you", tab_name(self, tab));
            self.notifications.tray.notice("Claude Switch", &text, Some(id));
            tray::flash(cx.window());
            if self.notifications.demo {
                self.toast(text, now);
            }
        }
        if mode != Attention::Off
            && self.tabs.list.iter().any(|t| t.term.running() && t.pane.is_some())
            && let Some(at) = self.notifications.next
            && at > now
        {
            cx.wake_at(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(prompt: u64) -> Option<(u64, TurnActivity)> {
        Some((10, TurnActivity { prompt: Some(prompt), waiting: true, waiting_since: None }))
    }

    #[test]
    fn bell_and_transcript_share_one_notice_until_a_new_user_record() {
        let mut tab = TabAttention { started: 0, ..Default::default() };
        let mode = Attention::ToastAndMark;
        tab.observe(Some((1, TurnActivity { prompt: Some(1), ..Default::default() })), true, mode);
        tab.bell = Some(false);
        assert!(tab.observe(None, false, mode));
        assert!(tab.marked);
        assert!(!tab.observe(waiting(1), false, mode));
        tab.observe(None, true, mode);
        assert!(!tab.marked);
        assert!(!tab.observe(waiting(1), false, mode));
        assert!(tab.observe(waiting(2), false, mode));
    }

    #[test]
    fn attended_old_or_disabled_turns_do_not_alert_later() {
        let mut tab = TabAttention { started: 20, ..Default::default() };
        assert!(!tab.observe(waiting(1), false, Attention::ToastAndMark));
        tab.started = 0;
        assert!(!tab.observe(waiting(2), true, Attention::ToastAndMark));
        assert!(!tab.observe(waiting(2), false, Attention::ToastAndMark));
        assert!(!tab.observe(waiting(3), false, Attention::MarkOnly));
        assert!(tab.marked);
        assert!(!tab.observe(waiting(4), false, Attention::Off));
        assert!(!tab.marked);
    }

    #[test]
    fn a_bell_observed_in_the_foreground_does_not_alert_after_switching_tabs() {
        let mut tab = TabAttention { started: 0, ..Default::default() };
        let mode = Attention::ToastAndMark;
        tab.observe(waiting(1), true, mode);
        let mut screen = mdshared_term::Screen::new(90, 12, 20);
        screen.feed(b"\x07");
        tab.terminal(&mut screen, true, true);
        assert!(!tab.observe(waiting(2), false, mode));
        assert!(!tab.marked);
        assert!(tab.observe(waiting(3), false, mode));
    }

    #[test]
    fn permission_requires_a_complete_interactive_prompt_and_signals_its_appearance_once() {
        let mut tab = TabAttention::default();
        let mut screen = mdshared_term::Screen::new(90, 12, 20);
        screen.feed(b"Do you want to proceed?\r\n1. Yes\r\n3. No\r\n");
        tab.terminal(&mut screen, true, false);
        assert!(tab.bell.is_none(), "a question and options alone are not a permission UI");
        screen.feed(b"Esc to cancel");
        tab.terminal(&mut screen, true, false);
        assert_eq!(tab.bell.take(), Some(false));
        tab.terminal(&mut screen, true, false);
        assert!(tab.bell.is_none());
    }
}
