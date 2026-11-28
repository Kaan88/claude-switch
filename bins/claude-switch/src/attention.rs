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

pub struct TabAttention {
    pub marked: bool,
    /// A pending terminal signal; true means the user already saw the tab.
    bell: Option<bool>,
    permission_visible: bool,
    prompt: Option<u64>,
    consumed: bool,
    started: u64,
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
            let permission = screen_permission_prompt(screen);
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

    pub fn clear_transcript(&mut self) {
        self.marked = false;
        self.prompt = None;
        self.consumed = false;
        self.transcript = None;
        self.stamp = 0;
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

/// How many rows above its footer a permission prompt's choices and question may use.
const PROMPT_ROWS: usize = 24;
/// How many rows one long choice may wrap onto.
const CHOICE_WRAP: usize = 4;

pub(crate) fn screen_permission_prompt(screen: &mdshared_term::Screen) -> bool {
    // The footer is the last row with text on the screen; a rule or two under it (left by a
    // redraw) are passed over. A prompt takes the input box's place, so nothing is under its
    // footer: prompt text left in the conversation has the input row and its status rows below
    // it, and is not the last text. The cursor is no guide. Claude Code hides it and leaves it
    // where it last wrote: at the second prompt of two calls made in one message it sat fifteen
    // rows up, on the first call's output row (2.1.332, the `perm_two_after` fixture).
    let rule = |text: &str| text.trim().chars().all(|c| matches!(c, '─' | '━' | '═'));
    let bottom = (0..screen.rows()).rev().find_map(|row| {
        let text = screen.row_text(row);
        (!text.trim().is_empty() && !rule(&text)).then_some((row, text))
    });
    let Some((footer, text)) = bottom else { return false };
    if !text.trim_start().starts_with("Esc to cancel") {
        return false;
    }
    // A prompt takes the input box's place: with the box on screen, the rows are the user's own
    // text (a draft, the message as sent) or the conversation.
    if crate::view::chat::input_box_shown(screen) {
        return false;
    }
    let rows: Vec<_> = (footer.saturating_sub(PROMPT_ROWS)..footer)
        .rev()
        .map(|row| screen.row_text(row))
        .filter(|s| !s.trim().is_empty())
        .collect();
    let mut rows = rows.iter().map(|s| s.trim().trim_start_matches(['❯', '>']).trim_start());
    let option = |row: &str, labels: &[&str]| {
        labels.iter().any(|label| {
            row.strip_prefix(label).is_some_and(|rest| rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric()))
        })
    };
    let numbered = |row: &str| {
        row.split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()))
    };
    // Bottom up: each choice is its numbered row and the rows a long choice wraps onto below it.
    let mut choice = || {
        let mut wrapped = 0;
        for row in rows.by_ref() {
            if numbered(row) {
                return Some(row);
            }
            wrapped += 1;
            if wrapped > CHOICE_WRAP {
                return None;
            }
        }
        None
    };
    let Some(deny) = choice() else { return false };
    // The last choice declines ("N. No…"); the choices above it count down to "1. Yes" or
    // "1. Allow" (two to five choices, such as "Yes, and switch to auto mode").
    let Some(count) = (2..=5).find(|n| option(deny, &[&format!("{n}. No"), &format!("{n}. Deny")])) else {
        return false;
    };
    for n in (1..count).rev() {
        let Some(row) = choice().filter(|r| r.starts_with(&format!("{n}. "))) else { return false };
        if n == 1 && !option(row, &["1. Yes", "1. Allow"]) {
            return false;
        }
    }
    rows.next().is_some_and(|row| row.starts_with("Do you want to ") || row.starts_with("Would you like to "))
}

struct Watch {
    id: u64,
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

    fn sync_tray(&mut self, cx: &mut Cx<'_, Msg>, force: bool) {
        let enabled = self.config.notifications.tray_icon;
        if enabled {
            self.sync_usage_setting();
        }
        let tip = if enabled { self.usage_tray_tip() } else { String::new() };
        // Never strand a hidden window if the setting is changed or Explorer loses its icon.
        self.notifications.tray.sync(enabled, tip, force);
        if self.notifications.hidden && !self.notifications.tray.enabled() {
            self.show_from_tray(cx);
        }
    }

    pub fn request_window_close(&mut self, cx: &mut Cx<'_, Msg>) {
        self.sync_tray(cx, self.config.notifications.close_to_tray);
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
        self.sync_tray(cx, false);
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
            let sample = samples.iter().position(|s| s.id == tab.id).and_then(|i| {
                let sample = samples.swap_remove(i);
                if tab.pane.as_ref()?.verified_transcript() != Some(sample.file.as_path()) {
                    return None;
                }
                if let Some(transcript) = sample.transcript {
                    tab.attention.transcript = Some(transcript);
                }
                Some((sample.stamp, sample.activity))
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
                        file: tab.pane.as_ref()?.verified_transcript().map(std::path::Path::to_owned),
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
                            let file = w.file?;
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
        screen.feed(b"Do you want to proceed?\r\n1. Yes\r\n2. No\r\n");
        tab.terminal(&mut screen, true, false);
        assert!(tab.bell.is_none(), "a question and options alone are not a permission UI");
        screen.feed(b"Esc to cancel");
        tab.terminal(&mut screen, true, false);
        assert_eq!(tab.bell.take(), Some(false));
        tab.terminal(&mut screen, true, false);
        assert!(tab.bell.is_none());
    }

    #[test]
    fn a_permission_prompt_may_offer_up_to_five_consecutive_choices() {
        let prompt = |rows: &[&str]| {
            let mut screen = mdshared_term::Screen::new(90, 14, 0);
            let rows: String = rows.iter().map(|row| format!("{row}\r\n")).collect();
            screen.feed(
                format!(
                    "Bash command\r\n  npm test\r\nDo you want to proceed?\r\n{rows}Esc to cancel and Tab to amend"
                )
                .as_bytes(),
            );
            screen_permission_prompt(&screen)
        };
        assert!(prompt(&[
            "❯ 1. Yes",
            "  2. Yes, and don't ask again for: npm test *",
            "  3. Yes, and switch to auto mode",
            "  4. No"
        ]));
        assert!(prompt(&["❯ 1. Yes", "  2. No"]));
        assert!(!prompt(&["❯ 1. Yes", "  3. No"]), "a missing choice is not the CLI's menu");
        assert!(!prompt(&["❯ 1. Maybe", "  2. No"]));
    }
}
