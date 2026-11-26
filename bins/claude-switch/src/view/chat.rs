//! Chat's conservative PTY input gate and per-tab state.
mod clipboard;
mod commands;
mod composer;
mod editor;
mod in_place;
pub(crate) mod effort;
mod input;
mod layout;
mod paint;
pub(super) mod question;
mod recall;
mod rename;
mod shortcuts;
pub(super) mod side;
pub(super) mod submission;

pub(crate) use commands::valid_name as valid_command_name;
pub use input::Control;
pub use input::layout_shortcut;
pub use layout::Layout;
pub use paint::STRIP_RESERVE;
pub use paint::strip_button;
pub use shortcuts::MenuMode;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ChatInfo, ChatPhase};

use editor::ComposerEditor;
use submission::{ImageWait, Intent, Queue};

pub const QUIET: Duration = Duration::from_millis(750);
const TOOL_WAIT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Tool,
    Bell,
    Slash,
    Permission,
    Identity,
    Ended,
    /// Claude asked a question the card in the conversation can answer.
    Question,
    /// A permission prompt the card in the conversation can answer.
    Approval,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tool => "Waiting for your answer in the terminal",
            Self::Bell => "The terminal rang its bell. Check the terminal.",
            Self::Slash => "Claude Code commands are shown in the terminal.",
            Self::Permission => "Answer the permission question in the terminal.",
            Self::Identity => "The current session could not be verified. Check the terminal.",
            Self::Ended => "Claude Code has ended.",
            Self::Question => "Answer Claude's question above.",
            Self::Approval => "Answer Claude's request above.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Ready,
    Running,
    Check,
    Ended,
}

#[derive(Clone, Copy)]
struct Signals<'a> {
    phase: Option<ChatPhase>,
    fresh: bool,
    pending_for: Option<Duration>,
    quiet_for: Duration,
    title: &'a str,
    bell: bool,
    slash: bool,
    ended: bool,
    bracketed: bool,
    alternate: bool,
}

fn assess(s: Signals<'_>) -> (Gate, Option<Reason>) {
    if s.ended {
        return (Gate::Ended, Some(Reason::Ended));
    }
    if s.bell {
        return (Gate::Check, Some(Reason::Bell));
    }
    if s.slash {
        return (Gate::Check, Some(Reason::Slash));
    }
    if s.pending_for.is_some_and(|age| age >= TOOL_WAIT) && s.quiet_for >= QUIET {
        return (Gate::Check, Some(Reason::Tool));
    }
    // OSC title strings are not a protocol. Only an explicit busy label is a veto; no
    // title (including an apparently idle title) is accepted as proof of readiness.
    let busy_title = matches!(s.title, "Claude Code — Working" | "Claude Code — Running");
    if s.pending_for.is_some() || busy_title || s.fresh && s.phase == Some(ChatPhase::Active) {
        return (Gate::Running, None);
    }
    if s.fresh && s.phase == Some(ChatPhase::EndTurn) && s.quiet_for >= QUIET && s.bracketed && !s.alternate {
        (Gate::Ready, None)
    } else {
        (Gate::Check, None)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Drawer {
    #[default]
    Closed,
    Manual,
    Auto(Reason),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Tasks,
    Agents,
    Shells,
    Files,
    Outputs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Chip {
    Path(PathBuf),
    Clipboard(clipboard::Picture),
}

#[derive(Default)]
pub struct Chat {
    pub enabled: bool,
    pub(crate) identity_unverified: bool,
    /// The live record pinned a session whose transcript Claude Code has not written yet.
    pub(crate) awaiting_transcript: bool,
    /// A message went from Chat while awaiting; later sends wait for that transcript.
    pub(super) first_sent: bool,
    /// The tab was started with Claude Code's classic renderer (Settings).
    pub(crate) classic_renderer: bool,
    question: question::Card,
    mode_step: Option<submission::mode::ModeStep>,
    /// The permission mode as the screen last showed it.
    mode_shown: Option<&'static str>,
    drawer: Drawer,
    drawer_height: Option<f32>,
    resizing: Option<(f32, f32)>,
    drawer_focus: bool,
    reason: Option<Reason>,
    suppressed: Option<Reason>,
    side_hidden: bool,
    side: Side,
    pub(super) details: side::Details,
    shell_count: usize,
    editor: ComposerEditor,
    editor_drag: bool,
    chips: Vec<Chip>,
    chip_page: usize,
    recall: recall::Recall,
    error: Option<&'static str>,
    retry: Option<Intent>,
    queue: Option<Queue>,
    image_wait: Option<ImageWait>,
    staging: submission::Staging,
    commands: commands::Picker,
    cli_report: submission::CliReport,
    output_epoch: u64,
    sample: Option<(u64, Option<(u64, ChatPhase)>)>,
    fresh: bool,
    require_prompt: Option<Option<u64>>,
    input_floor: Option<u64>,
    /// The floor for the next reader's records, kept across its first reset (after a follow).
    next_floor: Option<u64>,
    stopped: Option<(u64, u64)>,
    pending: Option<(u64, Instant)>,
    bell: bool,
    slash: bool,
    last_output: Option<Instant>,
    last_gate: Option<Gate>,
    demo: bool,
    demo_progress: bool,
    #[cfg(test)]
    writes: Vec<Vec<u8>>,
    #[cfg(test)]
    fail_enter_once: bool,
    layout: Layout,
}

impl Chat {
    pub fn demo() -> Self {
        Self { enabled: true, demo: true, ..Self::default() }
    }

    pub(super) fn seed_permission_demo(&mut self) {
        self.editor.set("Keep this draft until I confirm.".into());
        self.react(Some(Reason::Permission));
    }

    pub fn output(&mut self, now: Instant, output: bool, bell: bool) {
        if output {
            self.last_output = Some(now);
            self.output_epoch = self.output_epoch.wrapping_add(1);
        }
        if bell {
            self.bell = true;
            self.suppressed = None;
        }
    }

    pub(super) fn invalidate(&mut self, info: &ChatInfo) {
        self.fresh = false;
        self.require_prompt = Some(info.prompt);
    }

    pub(super) fn invalidate_input(&mut self, info: &ChatInfo, end: Option<u64>) {
        self.interrupt_submission();
        self.watermark_input(info, end);
    }

    pub(super) fn watermark_input(&mut self, info: &ChatInfo, end: Option<u64>) {
        self.invalidate(info);
        self.input_floor = Some(end.unwrap_or(u64::MAX));
    }

    fn observe(&mut self, info: &ChatInfo, now: Instant) {
        let sample = (info.generation, info.latest);
        if self.sample != Some(sample) {
            match self.sample {
                Some((generation, prior))
                    if generation == sample.0 && sample.1.map(|(at, _)| at) > prior.map(|(at, _)| at) =>
                {
                    if self.require_prompt.is_some_and(|old| info.prompt > old)
                        && self.input_floor.is_none_or(|floor| info.prompt.is_some_and(|p| p >= floor))
                    {
                        self.require_prompt = None;
                        self.input_floor = None;
                    }
                    self.fresh = self.require_prompt.is_none();
                    if self.reason == Some(Reason::Bell) || self.suppressed == Some(Reason::Bell) {
                        self.bell = false;
                    }
                    if self.require_prompt.is_none() {
                        self.slash = false;
                    }
                    self.suppressed = None;
                }
                _ => {
                    self.fresh = false;
                    self.require_prompt = Some(info.prompt);
                    self.pending = None;
                    self.input_floor = self.next_floor.take();
                }
            }
            self.sample = Some(sample);
        }
        if self.pending.map(|(id, _)| id) != info.pending {
            self.pending = info.pending.map(|id| (id, if self.demo { now - TOOL_WAIT } else { now }));
            if self.demo && self.last_output.is_none() {
                self.last_output = Some(now - QUIET);
            }
            if self.suppressed == Some(Reason::Tool) {
                self.suppressed = None;
            }
        }
    }

    fn decision(&self, info: &ChatInfo, term: &crate::term::Term, title: &str, now: Instant) -> (Gate, Option<Reason>) {
        let modes = term.screen.modes();
        if self.identity_unverified {
            return (Gate::Check, Some(Reason::Identity));
        }
        if term.ended.is_some() || !self.demo && !term.running() {
            return (Gate::Ended, Some(Reason::Ended));
        }
        if crate::attention::screen_permission_prompt(&term.screen) {
            // The card answers it when it names the pending call; the terminal otherwise.
            let card = info.pending.is_some() && self.question.approval && self.question.answerable == info.pending;
            return (Gate::Check, Some(if card { Reason::Approval } else { Reason::Permission }));
        }
        if self.demo
            && self.demo_progress
            && !self.bell
            && !self.slash
            && info.pending.is_none()
            && !(self.fresh && info.latest.is_some_and(|(_, phase)| phase == ChatPhase::Active))
        {
            return (Gate::Ready, None);
        }
        // No transcript yet, so no turn has started and no old completion can mislead. The
        // composer still needs a quiet, visibly empty prompt (`submission::blocked`).
        let first = self.first_send_open(info);
        let mut decision = assess(Signals {
            phase: if first { Some(ChatPhase::EndTurn) } else { info.latest.map(|(_, phase)| phase) },
            fresh: self.fresh || first,
            pending_for: info.pending.map(|id| {
                self.pending
                    .filter(|(known, _)| *known == id)
                    .map_or(Duration::ZERO, |(_, at)| now.saturating_duration_since(at))
            }),
            quiet_for: self.last_output.map_or(Duration::ZERO, |at| now.saturating_duration_since(at)),
            title,
            bell: self.bell,
            slash: self.slash,
            ended: false,
            bracketed: modes.bracketed_paste,
            alternate: modes.alt_screen,
        });
        if self.reason == Some(Reason::Tool) && self.pending.is_some() && decision.1.is_none() {
            decision = (Gate::Check, Some(Reason::Tool));
        }
        // The card answers it: the same refusal, without sending the user to the terminal.
        if decision.1 == Some(Reason::Tool) && info.pending.is_some() && self.question.answerable == info.pending {
            decision.1 = Some(if self.question.approval { Reason::Approval } else { Reason::Question });
        }
        decision
    }

    /// The first message of a new chat may go before its transcript exists.
    fn first_send_open(&self, info: &ChatInfo) -> bool {
        self.awaiting_transcript && !self.first_sent && info.latest.is_none() && info.pending.is_none()
    }

    /// The tab now follows another session of the same process: nothing staged carries over.
    /// `end` is the followed transcript's length then: only records after it count as new.
    pub(super) fn followed(&mut self, info: &ChatInfo, end: u64) {
        self.interrupt_submission();
        self.first_sent = false;
        self.stopped = None;
        self.pending = None;
        self.watermark_input(info, Some(end));
        self.next_floor = Some(end);
    }

    fn react(&mut self, reason: Option<Reason>) -> bool {
        let before = (self.drawer, self.reason);
        self.reason = reason;
        // A question or request the card can answer does not open the drawer, and closes one it opened.
        let reason = reason.filter(|r| !matches!(r, Reason::Question | Reason::Approval));
        match (self.drawer, reason) {
            (Drawer::Closed, Some(reason)) if Some(reason) != self.suppressed => self.drawer = Drawer::Auto(reason),
            (Drawer::Auto(_), Some(reason)) => self.drawer = Drawer::Auto(reason),
            (
                Drawer::Auto(Reason::Tool | Reason::Bell | Reason::Slash | Reason::Permission | Reason::Identity),
                None,
            ) => {
                self.drawer = Drawer::Closed;
                self.drawer_focus = false;
                self.resizing = None;
            }
            _ => {}
        }
        before != (self.drawer, self.reason)
    }

    fn can_stop(&self, info: &ChatInfo) -> bool {
        self.reason.is_none() && self.fresh_turn_to_stop(info)
    }

    fn fresh_turn_to_stop(&self, info: &ChatInfo) -> bool {
        self.fresh
            && !self.identity_unverified
            && info.pending.is_none()
            && info.latest.is_some_and(|(_, phase)| phase == ChatPhase::Active)
            && info.prompt.is_some_and(|prompt| self.stopped != Some((info.generation, prompt)))
    }

    pub(super) fn is_demo(&self) -> bool {
        self.demo
    }

    pub(super) fn stop_ready(&self, info: &ChatInfo, term: &crate::term::Term, title: &str, now: Instant) -> bool {
        self.fresh_turn_to_stop(info)
            && !term.screen.modes().alt_screen
            && self.decision(info, term, title, now) == (Gate::Running, None)
    }

    fn toggle_drawer(&mut self) {
        if self.drawer == Drawer::Closed {
            self.open_drawer();
        } else {
            self.suppressed = self.reason;
            self.drawer = Drawer::Closed;
            self.drawer_focus = false;
            self.resizing = None;
        }
    }

    pub(super) fn open_drawer(&mut self) {
        self.drawer = Drawer::Manual;
        self.drawer_focus = true;
    }

    pub(super) fn shell_manager_focus(&mut self) -> bool {
        if self.enabled {
            self.open_drawer();
            self.commands.open = false;
        }
        !self.locked() && self.reason.is_none() && self.recovery_notice().is_none()
    }

    pub(super) fn detail_side(&self) -> Option<Side> {
        (self.enabled && !self.side_hidden && self.layout.side.w > 0.0).then_some(self.side)
    }
}

#[cfg(test)]
mod tests;
