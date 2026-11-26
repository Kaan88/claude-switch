//! Chat's conservative PTY input gate and per-tab state.
mod input;
mod layout;
mod paint;

pub use input::Control;
pub use input::layout_shortcut;
pub use layout::Layout;
pub use paint::STRIP_RESERVE;
pub use paint::strip_button;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ChatInfo, ChatPhase};

use crate::compact_dialog::editor::Editor;

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
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Chip {
    Path(PathBuf),
    Clipboard(u32),
}

#[derive(Default)]
pub struct Chat {
    pub enabled: bool,
    pub(crate) identity_unverified: bool,
    drawer: Drawer,
    drawer_height: Option<f32>,
    resizing: Option<(f32, f32)>,
    drawer_focus: bool,
    reason: Option<Reason>,
    suppressed: Option<Reason>,
    side_hidden: bool,
    side: Side,
    side_scroll: usize,
    side_max: usize,
    editor: Editor,
    editor_drag: bool,
    chips: Vec<Chip>,
    chip_page: usize,
    cell_width: f32,
    recall: Option<usize>,
    error: Option<&'static str>,
    sample: Option<(u64, Option<(u64, ChatPhase)>)>,
    fresh: bool,
    require_prompt: Option<Option<u64>>,
    input_floor: Option<u64>,
    stopped: Option<(u64, u64)>,
    pending: Option<(u64, Instant)>,
    bell: bool,
    slash: bool,
    last_output: Option<Instant>,
    last_gate: Option<Gate>,
    demo: bool,
    demo_progress: bool,
    layout: Layout,
}

impl Chat {
    pub fn demo() -> Self {
        Self { enabled: true, demo: true, ..Self::default() }
    }

    pub fn output(&mut self, now: Instant, output: bool, bell: bool) {
        if output {
            self.last_output = Some(now);
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
                    self.slash = false;
                    self.suppressed = None;
                }
                _ => {
                    self.fresh = false;
                    self.require_prompt = Some(info.prompt);
                    self.pending = None;
                    self.input_floor = None;
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
        if crate::attention::screen_permission_prompt(&term.screen) {
            return (Gate::Check, Some(Reason::Permission));
        }
        if self.demo && self.demo_progress && !self.bell && !self.slash && info.pending.is_none() {
            return (Gate::Ready, None);
        }
        let mut decision = assess(Signals {
            phase: info.latest.map(|(_, phase)| phase),
            fresh: self.fresh,
            pending_for: info.pending.map(|id| {
                self.pending
                    .filter(|(known, _)| *known == id)
                    .map_or(Duration::ZERO, |(_, at)| now.saturating_duration_since(at))
            }),
            quiet_for: self.last_output.map_or(Duration::ZERO, |at| now.saturating_duration_since(at)),
            title,
            bell: self.bell,
            slash: self.slash,
            ended: term.ended.is_some() || !self.demo && !term.running(),
            bracketed: modes.bracketed_paste,
            alternate: modes.alt_screen,
        });
        if self.reason == Some(Reason::Tool) && self.pending.is_some() && decision.1.is_none() {
            decision = (Gate::Check, Some(Reason::Tool));
        }
        decision
    }

    fn react(&mut self, reason: Option<Reason>) -> bool {
        let before = (self.drawer, self.reason);
        self.reason = reason;
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
        self.fresh
            && !self.identity_unverified
            && self.reason.is_none()
            && info.pending.is_none()
            && info.latest.is_some_and(|(_, phase)| phase == ChatPhase::Active)
            && info.prompt.is_some_and(|prompt| self.stopped != Some((info.generation, prompt)))
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
}

fn recall<'a>(history: impl Iterator<Item = &'a str>, cursor: &mut Option<usize>, up: bool) -> Option<&'a str> {
    let next = match (*cursor, up) {
        (None, true) => 0,
        (Some(i), true) => i + 1,
        (Some(0), false) => {
            *cursor = None;
            return Some("");
        }
        (Some(i), false) => i - 1,
        (None, false) => return None,
    };
    let prompt = history.filter(|text| !text.trim().is_empty()).nth(next)?;
    *cursor = Some(next);
    Some(prompt)
}

fn send_bytes(chips: &[Chip], text: &str) -> Option<Vec<u8>> {
    // Never let pasted content smuggle control sequences out of a bracketed paste.
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || chips.iter().any(|chip| matches!(chip, Chip::Clipboard(_)))
    {
        return None;
    }
    let text = text.replace(['\n', '\t'], " ");
    let mut bytes = Vec::new();
    let paths: Vec<_> = chips
        .iter()
        .filter_map(|chip| match chip {
            Chip::Path(path) => Some(path.clone()),
            Chip::Clipboard(_) => None,
        })
        .collect();
    let paths = super::attach::paths(&paths);
    if !paths.is_empty() {
        bytes.extend(mdshared_term::input::encode_paste(&paths, true));
        if !text.is_empty() {
            bytes.push(b' ');
        }
    }
    if !text.is_empty() {
        bytes.extend(mdshared_term::input::encode_paste(&text, true));
    }
    if bytes.is_empty() {
        return None;
    }
    bytes.push(b'\r');
    Some(bytes)
}

#[cfg(test)]
mod tests;
