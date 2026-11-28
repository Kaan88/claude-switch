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

/// A message of several lines, pasted and waiting to show on the input line before Enter.
pub(super) struct LinesWait {
    pub stamp: Stamp,
    pub due: Instant,
    pub expected: String,
    pub text: String,
    pub picture: bool,
    pub allow_working: bool,
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

/// The message Chat last submitted, kept until the CLI's input shows idle again or the terminal
/// gets other input: interrupted before any answer, Claude Code puts it back on its input.
pub(super) struct Sent {
    /// What the CLI's input showed for it.
    pub expected: String,
    /// The draft's text.
    pub text: String,
    pub picture: bool,
}

/// The CLI's input being emptied of a message it put back: one Ctrl+U at a time, each after
/// newer output than the one before.
pub(super) struct Takeback {
    pub epoch: u64,
    pub presses: u8,
    pub due: Instant,
    /// Which of the sent messages the input shows.
    pub index: usize,
    /// The output epoch of a first reading of the input as empty while output still came: a
    /// second one of newer output confirms it.
    pub clean: Option<u64>,
}

/// A slash command held for a moment because the CLI's prompt was not quiet yet.
pub(super) struct CommandWait {
    pub due: Instant,
    pub line: String,
    pub from_editor: bool,
}

#[derive(Default)]
pub(in crate::view::chat) struct Staging {
    pub(super) echo: Option<EchoWait>,
    pub(super) lines: Option<LinesWait>,
    pub(super) retry: Option<EnterOnly>,
    pub(super) recovery: Option<Recovery>,
    /// The messages Chat submitted since the last one sent at an idle input (those sent while
    /// its turn ran follow it): any of them may be the one Claude Code puts back after a Stop.
    pub(super) sent: Vec<Sent>,
    /// Stop went since the last of them: text left on the input that is none of them is said.
    pub(super) stopped: bool,
    pub(super) takeback: Option<Takeback>,
    pub(super) command: Option<CommandWait>,
}

impl Chat {
    pub(in crate::view::chat) fn waiting(&self) -> Option<&'static str> {
        if self.staging.echo.is_some() {
            Some("Waiting for Claude Code to show the command on its input line…")
        } else if self.staging.lines.is_some() {
            Some("Waiting for Claude Code to show the message on its input line…")
        } else if self.image_wait.is_some() {
            Some("Waiting for Claude Code to confirm the picture…")
        } else if self.staging.takeback.is_some() {
            Some("Taking your message back from Claude Code's input…")
        } else if self.staging.command.is_some() {
            Some("Waiting for Claude Code's prompt to run the command…")
        } else if self.mode_step.is_some() {
            // Part of the chat's work, so it also ends in a hidden tab.
            Some("Waiting for Claude Code to change the permission mode…")
        } else {
            self.question_notice()
        }
    }

    /// The last message's turn is over or under way for good: nothing to take back.
    pub(in crate::view::chat) fn forget_sent(&mut self) {
        if self.staging.takeback.is_none() {
            self.staging.sent.clear();
            self.staging.stopped = false;
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
    if expected.contains('\n') {
        return screen::shows_lines(&tab.term.screen, expected);
    }
    // A command line longer than the terminal is wide wraps onto more rows.
    let shows = |expected: &str| {
        prompt(&tab.term.screen).as_deref() == Some(expected) || screen::shows_wrapped(&tab.term.screen, expected)
    };
    // Text pasted after a picture's marker follows it without a space (recorded from Claude Code
    // 2.1.332, `recorder compat`'s `queued` script); a space there is taken too.
    shows(expected) || without_marker_space(expected).is_some_and(|joined| shows(&joined))
}

/// `[Image #1] text` as `[Image #1]text`; `None` when it does not start with a picture's marker.
fn without_marker_space(text: &str) -> Option<String> {
    let (marker, rest) = text.split_once("] ")?;
    let marker = format!("{marker}]");
    screen::is_image_marker(&marker).then(|| format!("{marker}{rest}"))
}

#[cfg(test)]
#[test]
fn a_pictures_marker_may_be_followed_by_its_text_without_a_space() {
    assert_eq!(without_marker_space("[Image #1] look at this").as_deref(), Some("[Image #1]look at this"));
    assert_eq!(without_marker_space("[Image #12] a] b").as_deref(), Some("[Image #12]a] b"));
    assert_eq!(without_marker_space("[Pasted text #1] a"), None);
    assert_eq!(without_marker_space("no marker"), None);
}
