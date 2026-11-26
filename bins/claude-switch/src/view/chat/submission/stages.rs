use std::time::Instant;

use super::{COMMAND_INTERRUPTED, Chat, IMAGE_INTERRUPTED, Tab, busy_title, clean_prompt, prompt, screen};

#[derive(Clone, Copy)]
pub(super) struct Stamp {
    pub epoch: u64,
    pub(super) generation: u64,
    pub(super) prompt: Option<u64>,
}

impl Stamp {
    pub fn at(tab: &Tab) -> Self {
        let pane = tab.pane.as_ref().unwrap();
        Self { epoch: pane.chat.output_epoch, generation: pane.model.chat.generation, prompt: pane.model.chat.prompt }
    }

    pub fn matches(self, tab: &Tab) -> bool {
        let next = Self::at(tab);
        self.generation == next.generation && self.prompt == next.prompt
    }
}

pub(super) enum Completion {
    Message(String),
    Command { line: String, from_editor: bool },
}

pub(super) struct EchoWait {
    pub stamp: Stamp,
    pub due: Instant,
    pub line: String,
    pub from_editor: bool,
}

pub(super) struct EnterOnly {
    pub stamp: Stamp,
    pub expected: String,
    pub completion: Completion,
    pub picture: bool,
    pub allow_working: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Recovery {
    Picture(Stamp),
    Command(Stamp),
    Manual(Stamp, bool),
}

impl Recovery {
    fn stamp(self) -> Stamp {
        match self {
            Self::Picture(stamp) | Self::Command(stamp) | Self::Manual(stamp, _) => stamp,
        }
    }

    pub fn after_input(self, epoch: u64) -> Self {
        let stamp = Stamp { epoch, ..self.stamp() };
        match self {
            Self::Picture(_) => Self::Picture(stamp),
            Self::Command(_) => Self::Command(stamp),
            Self::Manual(_, picture) => Self::Manual(stamp, picture),
        }
    }

    pub fn notice(self) -> &'static str {
        match self {
            Self::Picture(_) => IMAGE_INTERRUPTED,
            Self::Command(_) => COMMAND_INTERRUPTED,
            Self::Manual(_, true) => "The CLI may hold a picture and pasted text. Check the terminal before sending.",
            Self::Manual(_, false) => "Text may still be on the CLI input line. Check the terminal before sending.",
        }
    }
}

#[derive(Default)]
pub(in crate::view::chat) struct Staging {
    pub(super) echo: Option<EchoWait>,
    pub(super) retry: Option<EnterOnly>,
    pub(super) recovery: Option<Recovery>,
}

impl Chat {
    pub(in crate::view::chat) fn waiting(&self) -> Option<&'static str> {
        if self.staging.echo.is_some() {
            Some("Waiting for Claude Code to show the command on its input line…")
        } else if self.image_wait.is_some() {
            Some("Waiting for Claude Code to confirm the picture…")
        } else {
            None
        }
    }

    pub(in crate::view::chat) fn enter_only(&self) -> bool {
        self.staging.retry.is_some()
    }

    pub(in crate::view::chat) fn recovery_notice(&self) -> Option<&'static str> {
        self.staging.recovery.map(Recovery::notice)
    }

    pub(in crate::view::chat) fn quiet(&self, now: Instant) -> bool {
        self.last_output.is_some_and(|at| now.saturating_duration_since(at) >= crate::view::chat::QUIET)
    }
}

pub(super) fn recovery_ready(tab: &Tab, now: Instant, recovery: Recovery) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    // A freshly rendered clean row after the wait (or a direct terminal key) is required.
    // The same empty screen snapshot that preceded the first write is not clearance.
    chat.output_epoch != recovery.stamp().epoch
        && chat.quiet(now)
        && clean_prompt(&tab.term.screen)
        && !screen::working(&tab.term.screen)
        && !busy_title(&tab.program_title)
        && tab.pane.as_ref().unwrap().model.chat.pending.is_none()
        && !chat.identity_unverified
        && !chat.bell
        && !chat.slash
}

pub(super) fn can_confirm(tab: &Tab, now: Instant, stamp: Stamp, allow_working: bool) -> bool {
    let pane = tab.pane.as_ref().unwrap();
    let modes = tab.term.screen.modes();
    let (gate, reason) = pane.chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
    stamp.matches(tab)
        && pane.model.error.is_none()
        && reason.is_none()
        && gate != super::Gate::Ended
        && !modes.alt_screen
        && (pane.chat.demo || modes.bracketed_paste)
        && pane.model.chat.pending.is_none()
        && !screen::menu(&tab.term.screen)
        && (allow_working || !(busy_title(&tab.program_title) || screen::working(&tab.term.screen)))
}

pub(super) fn shown_exactly(tab: &Tab, expected: &str) -> bool {
    prompt(&tab.term.screen).as_deref() == Some(expected)
}
