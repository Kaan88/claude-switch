//! Chat's conservative PTY input gate and per-tab state.
mod clipboard;
mod commands;
mod composer;
mod editor;
pub(crate) mod effort;
mod in_place;
mod input;
mod layout;
mod paint;
mod popups;
pub(super) mod question;
#[cfg(test)]
mod readme_shots;
mod recall;
mod rename;
mod screen_report;
mod shortcuts;
#[cfg(test)]
mod shots;
pub(super) mod side;
pub(super) mod submission;
mod terminal_card;
mod undo;
mod welcome;

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
/// How long a submitted message may take to show on the screen before the screen as it stands
/// counts again (the CLI normally redraws within milliseconds).
const SENT_WAIT: Duration = Duration::from_secs(5);
/// A key in the message box this recently: the user is typing there.
const TYPING: Duration = Duration::from_secs(1);

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
    /// A CLI menu or dialog the card in the conversation shows.
    Menu,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tool => "Claude Code may be waiting for your answer in the terminal.",
            Self::Bell => "The terminal rang its bell. Check the terminal.",
            Self::Slash => "Claude Code commands are shown in the terminal.",
            Self::Permission => "Answer the permission question in the terminal.",
            Self::Identity => "The current session could not be verified. Check the terminal.",
            Self::Ended => "Claude Code has ended.",
            Self::Question => "Answer Claude's question above.",
            Self::Approval => "Answer Claude's request above.",
            Self::Menu => "Choose in Claude Code's menu above, or open the terminal.",
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
    /// The screen shows the CLI's input idle: a quiet, visibly empty prompt and no working line.
    idle: bool,
    /// The screen shows the CLI's working line.
    working: bool,
    /// The CLI's input holds text.
    typed: bool,
    /// The screen shows the CLI's input at rest (empty or not): quiet, with no working line.
    resting: bool,
}

fn assess(s: Signals<'_>) -> (Gate, Option<Reason>) {
    if s.ended {
        return (Gate::Ended, Some(Reason::Ended));
    }
    // The screen is what the next key meets. A quiet, visibly empty input with no working line is
    // ready whatever the transcript last said: it lags, it has no record for a resumed session's
    // first prompt, and after an interruption its last record still reads as a running turn.
    if s.idle && s.bracketed && !s.alternate {
        return (Gate::Ready, None);
    }
    // A bell under the CLI's working line asks for nothing: the turn is running.
    if s.bell && !s.working {
        return (Gate::Check, Some(Reason::Bell));
    }
    if s.slash {
        return (Gate::Check, Some(Reason::Slash));
    }
    // A pending tool with a silent terminal may be waiting for an answer Chat cannot read; under
    // the CLI's working line it is only slow.
    // So may a turn whose terminal has been silent for seconds with no working line: the CLI
    // then shows something Chat has no card for.
    let silent_tool = s.pending_for.is_some_and(|age| age >= TOOL_WAIT) && s.quiet_for >= QUIET;
    let silent_turn = s.fresh && s.phase == Some(ChatPhase::Active) && s.quiet_for >= TOOL_WAIT && !s.typed;
    if (silent_tool || silent_turn) && !s.working {
        return (Gate::Check, Some(Reason::Tool));
    }
    // OSC title strings are not a protocol. Only an explicit busy label is a veto; no
    // title (including an apparently idle title) is accepted as proof of readiness.
    let busy_title = matches!(s.title, "Claude Code — Working" | "Claude Code — Running");
    // An input at rest with no working line is no running turn, whatever the transcript's last
    // record says (interrupted before any answer, the prompt is its last record for good).
    let active = s.fresh && s.phase == Some(ChatPhase::Active) && !s.resting;
    if s.working || s.pending_for.is_some() || busy_title || active {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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
    /// Settings › Chat › "Answer menus from the chat" is off: no card for the CLI's menus,
    /// pickers or permission prompts; they stay in the terminal.
    pub(crate) menus_off: bool,
    question: question::Card,
    /// The part of the terminal screen Chat cannot read, shown in the conversation.
    terminal_card: terminal_card::Card,
    mode_step: Option<submission::mode::ModeStep>,
    /// The permission mode as the screen last showed it.
    mode_shown: Option<&'static str>,
    drawer: Drawer,
    drawer_height: Option<f32>,
    resizing: Option<(f32, f32)>,
    drawer_focus: bool,
    /// When a key last went to the message box: the terminal does not take the keyboard from
    /// someone typing there.
    typed_at: Option<Instant>,
    /// The conversation has a selection (set each tick from the pane): Ctrl+C is meant for it, so
    /// the terminal does not take the keyboard by itself.
    selection: bool,
    reason: Option<Reason>,
    suppressed: Option<Reason>,
    /// The Details panel is closed: by the user's own choice, else because it has no section.
    side_hidden: bool,
    /// The user opened (`true`) or closed the panel: it then stays so, whatever it has to show.
    side_choice: Option<bool>,
    side: Side,
    pub(super) details: side::Details,
    shell_count: usize,
    editor: ComposerEditor,
    editor_drag: bool,
    /// The word a double click took: a drag from it goes by whole words.
    editor_words: Option<std::ops::Range<usize>>,
    chips: Vec<Chip>,
    chip_page: usize,
    recall: recall::Recall,
    error: Option<&'static str>,
    retry: Option<Intent>,
    queue: Option<Queue>,
    image_wait: Option<ImageWait>,
    staging: submission::Staging,
    commands: commands::Picker,
    popups: popups::Popups,
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
    /// When the terminal last got input from the user or from Chat.
    last_input: Option<Instant>,
    /// When this tab last saved a screen report (`screen_report`).
    last_report: Option<Instant>,
    /// A message or command Chat submitted: the output it was written after, and when.
    sent: Option<(u64, Instant)>,
    last_gate: Option<Gate>,
    demo: bool,
    demo_progress: bool,
    #[cfg(test)]
    writes: Vec<Vec<u8>>,
    #[cfg(test)]
    fail_enter_once: bool,
    /// A test's tab: gated like a real one (not the demo), with its writes only recorded.
    #[cfg(test)]
    pub(super) scripted: bool,
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

    /// As [`Chat::invalidate_input`], for a mouse report that cannot type or pick anything by
    /// itself: card key sequences in flight go on, each re-checking the screen before its key.
    pub(super) fn invalidate_pointer_input(&mut self, info: &ChatInfo, end: Option<u64>) {
        self.interrupt_staging();
        self.watermark_input(info, end);
    }

    /// A key went to the CLI that leaves the turn as it is.
    pub(super) fn note_input(&mut self) {
        self.last_input = Some(Instant::now());
    }

    pub(super) fn watermark_input(&mut self, info: &ChatInfo, end: Option<u64>) {
        self.invalidate(info);
        self.input_floor = Some(end.unwrap_or(u64::MAX));
        self.last_input = Some(Instant::now());
    }

    /// The tab has no program behind it and is not the demo: a test drives its screen.
    pub(super) fn scripted(&self) -> bool {
        #[cfg(test)]
        return self.scripted;
        #[cfg(not(test))]
        false
    }

    /// What the screen shows is newer than the last input: the CLI had time to react to a key
    /// (it may show nothing for one), and a message Chat submitted has shown (the input emptied,
    /// the working line appeared) or is long overdue.
    fn settled(&self, now: Instant) -> bool {
        let shown =
            |(epoch, at): (u64, Instant)| self.output_epoch != epoch || now.saturating_duration_since(at) >= SENT_WAIT;
        self.sent.is_none_or(shown) && self.last_input.is_none_or(|at| now.saturating_duration_since(at) >= QUIET)
    }

    /// The screen shows the CLI's input idle: a visibly empty prompt under the cursor, no working
    /// line, no menu or permission prompt, quiet since the last output and input.
    pub(super) fn screen_idle(&self, term: &crate::term::Term, now: Instant) -> bool {
        !self.demo
            && self.settled(now)
            && !term.screen.modes().alt_screen
            && submission::screen::idle_slash_prompt(&term.screen, self.quiet(now))
    }

    /// The screen shows the CLI's input at rest, empty or holding text: under the cursor, quiet
    /// since the last output and input, with no working line.
    pub(in crate::view) fn screen_resting(&self, term: &crate::term::Term, now: Instant) -> bool {
        !self.demo
            && self.settled(now)
            && self.quiet(now)
            && !term.screen.modes().alt_screen
            && !submission::screen::working(&term.screen)
            && (submission::screen::prompt(&term.screen).is_some()
                || submission::screen::shows_some_input(&term.screen))
    }

    /// An idle input ends what was noted about the screen before it: a bell asked for nothing that
    /// is still there, and the last message was not put back on the input.
    pub(super) fn settle(&mut self, idle: bool) {
        if idle {
            self.bell = false;
            self.sent = None;
            self.forget_sent();
        }
    }

    /// When the screen's rest could next change what Chat says: the end of the quiet time after
    /// the last output or input, and of the silence that makes a turn look stuck.
    pub(super) fn next_settle(&self, now: Instant) -> Option<Instant> {
        let after = |at: Option<Instant>, time: Duration| at.map(|at| at + time).filter(|at| *at > now);
        [after(self.last_output, QUIET), after(self.last_input, QUIET), after(self.last_output, TOOL_WAIT)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Chat wrote Enter after a message or command.
    pub(super) fn note_sent(&mut self) {
        self.sent = Some((self.output_epoch, Instant::now()));
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
        if term.ended.is_some() || !self.demo && !self.scripted() && !term.running() {
            return (Gate::Ended, Some(Reason::Ended));
        }
        // A question card answers its own menu, whatever its words.
        let question_card =
            info.pending.is_some() && self.question.answerable == info.pending && !self.question.approval;
        if crate::attention::screen_permission_prompt(&term.screen)
            || !question_card && question::screens::permission_like(&term.screen)
        {
            // The card answers it: the pending call's own card when the prompt names that call,
            // else the card that shows the prompt as the screen has it. The terminal only for a
            // prompt Chat cannot read whole.
            let card = info.pending.is_some() && self.question.approval && self.question.answerable == info.pending;
            let card =
                card || self.question.prompt_answers() && crate::attention::screen_permission_prompt(&term.screen);
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
            idle: self.screen_idle(term, now),
            working: !self.demo && submission::screen::working(&term.screen),
            typed: self.demo || submission::screen::prompt(&term.screen).is_some_and(|text| !text.is_empty()),
            resting: self.screen_resting(term, now),
        });
        if self.reason == Some(Reason::Tool)
            && self.pending.is_some()
            && decision.1.is_none()
            && decision.0 != Gate::Ready
            && !(!self.demo && submission::screen::working(&term.screen))
        {
            decision = (Gate::Check, Some(Reason::Tool));
        }
        // The card answers it: the same refusal, without sending the user to the terminal.
        if decision.1 == Some(Reason::Tool) && info.pending.is_some() && self.question.answerable == info.pending {
            decision.1 = Some(if self.question.approval { Reason::Approval } else { Reason::Question });
        }
        // A CLI menu shows as a card: nothing is sent past it and the drawer stays closed.
        if (self.question.menu.is_some() || self.question.blocked_shown())
            && decision.0 != Gate::Ended
            && decision.1.is_none_or(|r| matches!(r, Reason::Slash | Reason::Tool | Reason::Bell))
        {
            decision = (Gate::Check, Some(Reason::Menu));
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

    /// As `react`, and when the terminal opens for a question Chat cannot answer (Claude Code may
    /// be waiting, a permission prompt Chat cannot read whole), it takes the keyboard so the answer
    /// can be typed at once; not while someone types in the message box, whose next keys would
    /// land on that prompt.
    fn react_at(&mut self, reason: Option<Reason>, now: Instant) -> bool {
        let closed = self.drawer == Drawer::Closed;
        let changed = self.react(reason);
        if closed && matches!(self.drawer, Drawer::Auto(Reason::Tool | Reason::Permission)) && !self.keeps_keyboard(now)
        {
            self.drawer_focus = true;
        }
        changed
    }

    /// The keyboard stays with Chat rather than going to the terminal by itself: a key went to the
    /// message box a moment ago, the box or a card's field holds text, or the conversation has a
    /// selection. The next key (Enter, a digit, Ctrl+C) would otherwise land on Claude Code's prompt.
    pub(super) fn keeps_keyboard(&self, now: Instant) -> bool {
        self.typed_at.is_some_and(|at| now.saturating_duration_since(at) < TYPING)
            || !self.editor.text.trim().is_empty()
            || !self.chips.is_empty()
            || !self.question.field.text().is_empty()
            || self.selection
    }

    /// Whether the conversation has a selection, from the pane, each tick.
    pub(super) fn set_selection(&mut self, selection: bool) {
        self.selection = selection;
    }

    fn react(&mut self, reason: Option<Reason>) -> bool {
        let before = (self.drawer, self.reason);
        self.reason = reason;
        // A question or request the card can answer does not open the drawer, and closes one it opened.
        // A command's results and menus now show in the conversation: a slash command alone does
        // not open it either. Nor does a screen Chat cannot read (Claude Code may be waiting, the
        // bell): `terminal_card` shows it in the conversation and opens the drawer on a click.
        let reason = reason.filter(|r| {
            !matches!(
                r,
                Reason::Question | Reason::Approval | Reason::Menu | Reason::Slash | Reason::Tool | Reason::Bell
            )
        });
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

    /// The session has no message yet and none on its way: Chat greets, with the message box in
    /// the middle of the window.
    pub(super) fn welcome(&self, model: &crate::view::model::Model) -> bool {
        self.enabled
            && model.agent.is_none()
            && model.items.is_empty()
            && !model.has_earlier
            && model.error.is_none()
            && (self.awaiting_transcript || model.loaded)
            && !self.first_sent
            && self.queue.is_none()
            && self.waiting().is_none()
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
