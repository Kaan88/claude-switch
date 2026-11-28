//! The only composer PTY writes. A clipboard send has a separate, fail-closed second step.
pub(super) mod mode;
mod report;
pub(in crate::view) mod screen;
mod stages;

use std::time::{Duration, Instant};

use claude_switch_core::transcript::ChatInfo;

use super::{Chat, ChatPhase, Chip, Drawer, Gate, Reason, clipboard};
use crate::{state::AppState, tabs::Tab};
pub(super) use report::CliReport;
use report::local_reports;
use screen::{clean_prompt, idle_slash_prompt, is_image_marker, prompt};
pub(super) use stages::CommandResult;
pub(super) use stages::Staging;
use stages::{
    CommandWait, Completion, EchoWait, EnterOnly, LinesWait, Owned, OwnedStep, Recovery, Sent, Stamp, Takeback,
    can_confirm, can_withdraw, recovery_ready, shown_exactly, shows_sent,
};

/// Claude Code reads the clipboard's picture through a helper process on Windows, which can take
/// seconds; the wait ends at once when anything else changes.
pub(super) const IMAGE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const ECHO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a slash command waits for the CLI's prompt to go quiet before it is refused.
const COMMAND_WAIT: Duration = Duration::from_secs(3);
/// How long one Ctrl+U may take to show, and how many a put-back message may need.
const TAKEBACK_STEP: Duration = Duration::from_secs(2);
const TAKEBACK_KEYS: u8 = 40;
const CLEAR_LINE: &[u8] = b"\x15";
const TAKEN_BACK: &str = "Stopped before Claude answered. Your message is back in the box.";
const TAKEN_BACK_PICTURE: &str =
    "Stopped before Claude answered. Your message is back in the box; paste its picture again.";
const TAKEBACK_FAILED: &str =
    "Claude Code put your last message back on its input and Chat could not clear it. Clear it in the terminal.";
const TAKEBACK_HELD: &str = "Claude Code put your last message back on its input. Chat left it there, as the message \
     box holds a draft: send or clear it in the terminal, then send the draft.";
const TAKEBACK_OTHER: &str = "After the stop, Claude Code's input holds text that is not a message Chat sent. Send or \
     clear it in the terminal; a held draft goes once the input is empty.";
const TAKEBACK_UNSURE: &str = "Stopped before Claude answered. Your message is back in the box, but Claude Code kept \
     drawing: check that its input is empty before you send.";
const IMAGE_KEY: &[u8] = b"\x1bv";
pub(super) const CHECK: &str = "Open the terminal to check";
pub(super) const NO_QUIET_PROMPT: &str = "No quiet, empty Claude Code prompt is visible. Check the terminal.";
pub(super) const NOT_EMPTY: &str = "The CLI input line is not visibly empty. Wait or use the terminal.";
pub(super) const INPUT_HAS_TEXT: &str =
    "The CLI input line already contains text or an attachment. Check the terminal.";
pub(super) const MENU_OPEN: &str = "Close the CLI menu in the terminal before sending.";
pub(super) const BAD_TEXT: &str = "The draft has unsupported control characters.";
pub(super) const CHANGED_CLIPBOARD: &str = "The clipboard changed. Paste the picture into Chat again.";
const IMAGE_TIMEOUT_NOTICE: &str = "No image marker yet. The CLI may hold a picture; check the terminal. Draft kept.";
const IMAGE_INTERRUPTED: &str = "Image send interrupted. The CLI may hold a picture; check the terminal.";
/// After Enter, how long a command Chat typed on its own may stay on the input before Chat takes
/// Enter for lost; and how long Chat follows it in all (the input may be under a prompt).
const ENTER_CONFIRM: Duration = Duration::from_secs(3);
const OWNED_LIMIT: Duration = Duration::from_secs(60);
const OWNED_PRESSES: u8 = 3;
pub(super) const OWNED_PENDING: &str = "Waiting for Claude Code to take the command Chat typed…";
pub(super) const COMMAND_WITHDRAWN: &str =
    "Claude Code did not take the command Chat typed, so Chat took it off its input again.";
pub(super) const COMMAND_LEFT: &str = "Claude Code did not take the command Chat typed, and Chat could not take it off \
    its input: clear it in the terminal.";
pub(super) const COMMAND_TIMEOUT: &str =
    "The command did not appear on the CLI input line. Check the terminal before retrying.";
pub(super) const COMMAND_INTERRUPTED: &str =
    "Command not submitted. Check the terminal for text left on its input line.";
const LINES_TIMEOUT: &str =
    "The message did not appear on the CLI input line, so Enter was not pressed. Check the terminal.";
const LINES_INTERRUPTED: &str = "Message not submitted. Check the terminal for text left on its input line.";
const ENTER_ONLY: &str = "Input is already in the CLI. Use Press Enter to submit without pasting again.";
const ENTER_WAIT: &str = "Staged input not confirmed. Check the terminal; Press Enter will not paste again.";
const NO_PASTE: &str = "Bracketed paste is unavailable. Open the terminal to check.";
const ALTERNATE: &str = "The terminal is showing a full-screen view. Return to the CLI prompt first.";
pub(super) const FULLSCREEN: &str = "Claude Code is drawing its full-screen view, where Chat cannot send. Type \
    /tui default in the terminal, or turn on Settings › Sessions › Classic terminal view and open a new tab.";
pub(super) const NO_TRANSCRIPT: &str =
    "Waiting for Claude Code to write this chat's transcript. Check the terminal before sending again.";
const BUSY_CMD: &str = "A turn is running. Wait for it to finish before running this command.";
pub(super) const REMOVE_ATTACHMENTS: &str = "Remove attachments before running a slash command.";
pub(super) const BAD_COMMAND: &str = "Choose a command or enter a single slash command without control characters.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Draft,
    Override,
    Now,
    Slash,
    Interrupted,
}

pub(super) struct Queue {
    generation: u64,
    latest: Option<u64>,
    stop: Option<Stop>,
    /// Queued while a new chat's transcript was still to come: the reader starting on it is not
    /// another session.
    awaiting: bool,
    /// Send now was asked for while the CLI's input did not read empty: the draft goes into the
    /// running turn by itself once it does, instead of waiting for the turn's end.
    now: bool,
}

struct Stop {
    stamp: Stamp,
    was_working: bool,
    was_interrupted: bool,
}

impl Queue {
    /// Send now was asked for and waits for an input that reads empty.
    pub(super) fn sends_now(&self) -> bool {
        self.now
    }

    fn completed(&self, info: &ChatInfo) -> bool {
        self.generation == info.generation
            && info.latest.is_some_and(|(at, phase)| phase == ChatPhase::EndTurn && Some(at) > self.latest)
    }

    fn interrupted(&self, tab: &Tab, now: Instant) -> bool {
        let Some(stop) = &self.stop else { return false };
        let pane = tab.pane.as_ref().unwrap();
        let screen = &tab.term.screen;
        stop.stamp.matches(tab)
            && pane.chat.output_epoch != stop.stamp.epoch
            && !busy_title(&tab.program_title)
            && (stop.was_working || !stop.was_interrupted && screen::interrupted(screen))
            && idle_slash_prompt(screen, pane.chat.quiet(now))
    }
}

pub(super) struct ImageWait {
    due: Instant,
    stamp: Stamp,
    seq: u32,
    bytes: Vec<u8>,
    demo_text: String,
    into_turn: bool,
}

fn busy_title(title: &str) -> bool {
    matches!(title, "Claude Code — Working" | "Claude Code — Running")
}

impl Chat {
    pub(super) fn locked(&self) -> bool {
        self.queue.is_some() || self.waiting().is_some() || self.enter_only()
    }

    /// A draft queued behind the running turn, as Queue makes it.
    #[cfg(test)]
    pub(super) fn queue_draft(&mut self, generation: u64) {
        self.queue = Some(Queue { generation, latest: None, stop: None, awaiting: false, now: false });
    }

    pub(super) fn interrupt_submission(&mut self) {
        self.interrupt_card();
        self.interrupt_staging();
    }

    /// Terminal input that leaves card key sequences alone (a wheel report): what was staged for a
    /// send no longer knows the screen.
    pub(super) fn interrupt_staging(&mut self) {
        // Whatever the CLI's input holds now may be the user's own.
        (self.staging.takeback, self.staging.command) = (None, None);
        // A command Chat typed on its own is no longer followed (and never cleared): the input
        // may hold the user's own text now, or belong to another session.
        // After its Enter it most likely went; before (being withdrawn), it did not.
        if let Some(owned) = self.staging.owned.take() {
            self.staging.result = Some(match owned.step {
                OwnedStep::Entered => CommandResult::Unknown,
                OwnedStep::Withdraw { .. } => CommandResult::Left,
            });
        }
        self.staging.unseen = None;
        self.staging.sent.clear();
        self.staging.stopped = false;
        if let Some(wait) = self.image_wait.take() {
            self.staging.recovery = Some(Recovery::Picture(Stamp { epoch: self.output_epoch, ..wait.stamp }));
            self.error = Some(IMAGE_INTERRUPTED);
        } else if let Some(wait) = self.staging.echo.take() {
            self.staging.recovery = Some(Recovery::Command(Stamp { epoch: self.output_epoch, ..wait.stamp }));
            self.error = Some(COMMAND_INTERRUPTED);
        } else if let Some(wait) = self.staging.lines.take() {
            self.staging.recovery =
                Some(Recovery::Manual(Stamp { epoch: self.output_epoch, ..wait.stamp }, wait.picture));
            self.error = Some(LINES_INTERRUPTED);
        } else if let Some(retry) = self.staging.retry.take() {
            self.staging.recovery =
                Some(Recovery::Manual(Stamp { epoch: self.output_epoch, ..retry.stamp }, retry.picture));
            self.error = self.recovery_notice();
        } else if self.queue.take().is_some() {
            self.error = Some("Queue cancelled by terminal input or layout change. Draft kept.");
        } else if let Some(recovery) = self.staging.recovery {
            self.staging.recovery = Some(recovery.after_input(self.output_epoch));
        }
        self.retry = None;
        if self.slash {
            self.cli_report.touched_terminal = true;
        }
    }

    pub(super) fn stop_waiting(&mut self) {
        self.interrupt_submission();
        if self.staging.recovery.is_none() {
            self.error = None;
        }
    }

    pub(super) fn clipboard_sequence(&self) -> Option<u32> {
        self.chips.iter().find_map(|c| match c {
            Chip::Clipboard(p) => Some(p.seq),
            Chip::Path(_) => None,
        })
    }

    fn refused(&mut self, error: &'static str, retry: Option<Intent>) {
        self.error = Some(error);
        self.retry = retry;
    }

    fn submitted(&mut self) {
        self.error = None;
        self.retry = None;
        self.queue = None;
        self.editor.set(String::new());
        self.editor.clear_history();
        self.commands.open = false;
        self.chips.clear();
        self.chip_page = 0;
        self.recall.end();
    }
}

/// Why a send of this kind cannot go now; `None` when it can.
pub(super) fn blocked(tab: &Tab, now: Instant, intent: Intent) -> Option<&'static str> {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let (gate, reason) = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
    let modes = tab.term.screen.modes();
    if let Some(reason) = reason {
        return Some(reason.label());
    }
    if let Some(recovery) = chat.staging.recovery.filter(|recovery| !recovery_ready(tab, now, *recovery)) {
        return Some(recovery.notice());
    }
    if chat.staging.owned.is_some() {
        return Some(OWNED_PENDING);
    }
    if chat.awaiting_transcript && chat.first_sent {
        return Some(NO_TRANSCRIPT);
    }
    if modes.alt_screen {
        // A tab started without the classic renderer is most likely showing the CLI's own
        // fullscreen renderer; say how to leave it. Otherwise it is another full-screen view.
        return Some(if chat.classic_renderer { ALTERNATE } else { FULLSCREEN });
    }
    if !chat.demo && !modes.bracketed_paste {
        return Some(NO_PASTE);
    }
    if screen::menu(&tab.term.screen) {
        return Some(MENU_OPEN);
    }
    if pane.model.error.is_some() && !chat.demo {
        return Some(Reason::Identity.label());
    }
    if !chat.demo && matches!(intent, Intent::Draft | Intent::Override) {
        if prompt(&tab.term.screen).is_some_and(|row| !row.is_empty()) {
            return Some(INPUT_HAS_TEXT);
        }
        if intent == Intent::Draft && gate == Gate::Ready && !idle_slash_prompt(&tab.term.screen, chat.quiet(now)) {
            return Some(CHECK);
        }
    }
    match intent {
        Intent::Interrupted if pane.model.chat.pending.is_some() => Some(Reason::Tool.label()),
        Intent::Interrupted if !idle_slash_prompt(&tab.term.screen, chat.quiet(now)) => Some(CHECK),
        Intent::Draft if gate != Gate::Ready => Some(CHECK),
        Intent::Override | Intent::Now if pane.model.chat.pending.is_some() => Some(Reason::Tool.label()),
        Intent::Override if gate == Gate::Running => Some("A turn is running. Use Queue to send after it ends."),
        Intent::Now if !chat.demo && !clean_prompt(&tab.term.screen) => Some(NOT_EMPTY),
        Intent::Slash if pane.model.chat.pending.is_some() => Some(Reason::Tool.label()),
        // The transcript's running turn counts only while the screen shows one: a command's own
        // record stays the newest and Active when the turn before it had no `end_turn` (Claude
        // Code 2.1.332), while the CLI is plainly at rest. Over an idle input not yet quiet
        // (right after `/model`) the gate reads Running from the transcript alone: the command
        // waits for the input to rest (`CommandWait`), and a turn that shows meanwhile refuses it.
        Intent::Slash
            if gate == Gate::Running
                && (chat.demo
                    || screen::working(&tab.term.screen)
                    || busy_title(&tab.program_title)
                    || !idle_slash_prompt(&tab.term.screen, true))
                || pane.model.chat.latest.is_some_and(|(_, p)| p == ChatPhase::Active)
                    && screen::working(&tab.term.screen) =>
        {
            Some(BUSY_CMD)
        }
        Intent::Slash
            if !chat.demo && (gate == Gate::Running || !idle_slash_prompt(&tab.term.screen, chat.quiet(now))) =>
        {
            Some(NO_QUIET_PROMPT)
        }
        _ => None,
    }
}

impl AppState {
    pub(super) fn chat_send(tab: &mut Tab, now: Instant, force: bool) {
        let pane = tab.pane.as_ref().unwrap();
        let chat = &pane.chat;
        let running = || chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now).0 == Gate::Running;
        let intent = if !force {
            // While a turn runs, a message goes into Claude Code's visibly empty input at once, as
            // Send now does and as typing in the terminal would: Claude Code hands it to the
            // running turn, and the box is free for the next one. Otherwise it is held.
            let empty = clean_prompt(&tab.term.screen);
            if chat.queue.is_none() && empty && running() && blocked(tab, now, Intent::Now).is_none() {
                Intent::Now
            } else {
                Intent::Draft
            }
        } else if chat.queue.is_some() && (chat.demo || clean_prompt(&tab.term.screen) || running()) {
            // Into the visibly empty input, also while a turn runs.
            Intent::Now
        } else {
            // No readable prompt and no turn: the explicit override.
            Intent::Override
        };
        Self::send_with_intent(tab, now, intent);
    }

    fn send_with_intent(tab: &mut Tab, now: Instant, intent: Intent) {
        clear_recovery(tab, now);
        let error = blocked(tab, now, intent);
        let drafted =
            tab.pane.as_ref().is_some_and(|p| !p.chat.editor.text.trim().is_empty() || !p.chat.chips.is_empty());
        if drafted && let Some(error) = error.filter(|e| super::screen_report::worth(e)) {
            super::screen_report::save(tab, now, error);
        }
        let stamp = Stamp::at(tab);
        // Into a running turn, as the gate reads it now: Send now also goes into an idle input (no
        // turn, or one that ended while the draft waited), which is an ordinary send.
        let into_turn = intent == Intent::Now && {
            let pane = tab.pane.as_ref().unwrap();
            pane.chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now).0 == Gate::Running
        };
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.waiting().is_some() || chat.enter_only() || chat.queue.is_some() && intent == Intent::Draft {
            return;
        }
        if chat.editor.text.trim().is_empty() && chat.chips.is_empty() {
            return;
        }
        chat.recall.end();
        let Some(bytes) = send_bytes(&chat.chips, &chat.editor.text) else {
            chat.refused(BAD_TEXT, None);
            return;
        };
        // Whatever passes by itself is waited for, not refused: a running turn, a prompt the CLI
        // is still drawing, a command's output, a new chat's transcript. The draft is held and
        // goes out once the CLI's input is ready. So does one the CLI's input still holds text
        // for: never cleared by Chat, it goes once that text was sent or cleared in the terminal.
        let waits = matches!(error, Some(e) if e == CHECK || e == NO_TRANSCRIPT || e == INPUT_HAS_TEXT
            || e == Reason::Slash.label() || e == OWNED_PENDING);
        // A picture needs the visibly empty input for its marker, whoever asked for the send.
        let picture_waits =
            chat.clipboard_sequence().is_some() && error.is_none() && !chat.demo && !clean_prompt(&tab.term.screen);
        if intent == Intent::Draft && waits || picture_waits {
            chat.queue = Some(Queue {
                generation: pane.model.chat.generation,
                latest: pane.model.chat.latest.map(|x| x.0),
                stop: None,
                awaiting: chat.awaiting_transcript,
                now: false,
            });
            chat.error = None;
            chat.retry = None;
            return;
        }
        // Send now while the input does not read empty: it may be mid-redraw, or hold what was
        // typed in the terminal. The draft stays queued and goes by itself once it reads empty.
        if intent == Intent::Now
            && error == Some(NOT_EMPTY)
            && let Some(queue) = chat.queue.as_mut()
        {
            queue.now = true;
            chat.error = None;
            chat.retry = None;
            return;
        }
        if let Some(error) = error {
            chat.refused(error, Some(intent));
            return;
        }
        let demo_text: String = chat.editor.text.chars().take(400).collect();
        if let Some(seq) = chat.clipboard_sequence() {
            if !chat.demo && clipboard::sequence() != Some(seq) {
                chat.queue = None;
                chat.refused(CHANGED_CLIPBOARD, None);
                return;
            }
            // The guard is armed before the first write but no Enter is present in that write.
            let wait = ImageWait { due: now + IMAGE_TIMEOUT, stamp, seq, bytes, demo_text, into_turn };
            chat.queue = None;
            chat.error = None;
            chat.retry = None;
            chat.image_wait = Some(wait);
            if let Err(error) = write(tab, IMAGE_KEY, "") {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.image_wait = None;
                chat.staging.recovery = Some(Recovery::Picture(stamp));
                chat.refused(error, None);
            }
            tab.pane.as_mut().unwrap().note_composer_input_into_turn(into_turn);
        } else {
            finish(tab, now, &bytes, Completion::Message(demo_text), None, into_turn);
        }
    }

    pub(super) fn chat_stop(tab: &mut Tab, now: Instant) {
        let stamp = Stamp::at(tab);
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now) != (Gate::Running, None)
            || !chat.can_stop(&pane.model.chat)
        {
            return;
        }
        let stop = Stop {
            stamp,
            was_working: busy_title(&tab.program_title) || screen::working(&tab.term.screen),
            was_interrupted: screen::interrupted(&tab.term.screen),
        };
        chat.stopped = pane.model.chat.prompt.map(|p| (pane.model.chat.generation, p));
        match write(tab, b"\x1b", "Stopped") {
            Ok(()) => {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.staging.stopped = true;
                if let Some(queue) = &mut chat.queue {
                    queue.stop = Some(stop);
                }
            }
            Err(error) => tab.pane.as_mut().unwrap().chat.refused(error, None),
        }
    }

    pub(super) fn advance_submission(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
        let changed =
            super::question::advance(tab, now, may_write) | mode::advance(tab, now) | clear_recovery(tab, now);
        let pane = tab.pane.as_ref().unwrap();
        let chat = &pane.chat;
        if let Some(wait) = &chat.image_wait {
            let veto = !can_confirm(tab, now, wait.stamp, wait.into_turn)
                || !chat.demo && clipboard::sequence() != Some(wait.seq);
            let result = if veto {
                Some(Err(IMAGE_INTERRUPTED))
            } else if now >= wait.due {
                Some(Err(IMAGE_TIMEOUT_NOTICE))
            } else if may_write && chat.output_epoch != wait.stamp.epoch {
                prompt(&tab.term.screen).filter(|row| is_image_marker(row)).map(Ok)
            } else {
                None
            };
            if let Some(result) = result {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let wait = chat.image_wait.take().unwrap();
                match result {
                    Err(error) => {
                        chat.staging.recovery =
                            Some(Recovery::Picture(Stamp { epoch: chat.output_epoch, ..wait.stamp }));
                        chat.refused(error, None);
                    }
                    Ok(marker) => {
                        let text = if wait.demo_text.is_empty() { "[Picture]".into() } else { wait.demo_text };
                        finish(tab, now, &wait.bytes, Completion::Message(text), Some(marker), wait.into_turn);
                    }
                }
                return true;
            }
        }
        if let Some(wait) = &chat.staging.lines {
            let result = if !can_confirm(tab, now, wait.stamp, wait.into_turn) {
                Some(Err(LINES_INTERRUPTED))
            } else if now >= wait.due {
                Some(Err(LINES_TIMEOUT))
            } else if may_write
                && chat.output_epoch != wait.stamp.epoch
                && shows_sent(tab, &wait.expected, wait.into_turn)
            {
                Some(Ok(()))
            } else {
                None
            };
            if let Some(result) = result {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let wait = chat.staging.lines.take().unwrap();
                match result {
                    Err(error) => {
                        let stamp = Stamp { epoch: chat.output_epoch, ..wait.stamp };
                        chat.staging.recovery = Some(Recovery::Manual(stamp, wait.picture));
                        chat.refused(error, None);
                    }
                    Ok(()) => submit_enter(
                        tab,
                        EnterOnly {
                            stamp: wait.stamp,
                            expected: wait.expected,
                            completion: Completion::Message(wait.text),
                            picture: wait.picture,
                            into_turn: wait.into_turn,
                        },
                    ),
                }
                return true;
            }
        }
        if let Some(wait) = &chat.staging.echo {
            let result = if !can_confirm(tab, now, wait.stamp, false) {
                Some(Err(COMMAND_INTERRUPTED))
            } else if now >= wait.due {
                Some(Err(COMMAND_TIMEOUT))
            } else if may_write && chat.output_epoch != wait.stamp.epoch && shown_exactly(tab, &wait.line) {
                Some(Ok(()))
            } else {
                None
            };
            if let Some(result) = result {
                let screen = &tab.term.screen;
                let (shows, clean, hidden) =
                    (shown_exactly(tab, &wait.line), clean_prompt(screen), prompt(screen).is_none());
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let wait = chat.staging.echo.take().unwrap();
                match result {
                    // A command Chat typed on its own and did not enter: exactly its text comes off
                    // the input again, once the input can be read (a permission prompt drawn over
                    // it hides it; its card answers it meanwhile).
                    // (The demo has no CLI to take a command, nor anything to take off.)
                    Err(_) if !wait.from_editor && (shows || hidden) && !chat.demo => {
                        let epoch = chat.output_epoch.wrapping_sub(1);
                        let step = OwnedStep::Withdraw { presses: 0 };
                        let (due, last) = (now, now + OWNED_LIMIT);
                        let stamp = wait.stamp;
                        let seen = now;
                        chat.staging.owned = Some(Owned { line: wait.line, stamp, step, epoch, due, last, seen });
                        chat.error = None;
                        chat.retry = None;
                    }
                    Err(error) if !wait.from_editor && clean => {
                        chat.staging.result = Some(CommandResult::Withdrawn);
                        chat.refused(error, None);
                    }
                    Err(error) => {
                        if !wait.from_editor {
                            chat.staging.result = Some(CommandResult::Left);
                        }
                        chat.staging.recovery =
                            Some(Recovery::Command(Stamp { epoch: chat.output_epoch, ..wait.stamp }));
                        chat.refused(error, None);
                    }
                    Ok(()) => {
                        let (line, from_editor, stamp) = (wait.line.clone(), wait.from_editor, wait.stamp);
                        submit_enter(
                            tab,
                            EnterOnly {
                                stamp: wait.stamp,
                                expected: wait.line.clone(),
                                completion: Completion::Command { line: wait.line, from_editor },
                                picture: false,
                                into_turn: false,
                            },
                        );
                        let chat = &mut tab.pane.as_mut().unwrap().chat;
                        if !from_editor && chat.staging.retry.is_none() && chat.demo {
                            // No CLI behind the demo to let go of it.
                            chat.staging.result = Some(CommandResult::Taken);
                        } else if !from_editor && chat.staging.retry.is_none() {
                            // Enter went: the input must let go of the line.
                            let (epoch, due, last) = (chat.output_epoch, now + ENTER_CONFIRM, now + OWNED_LIMIT);
                            let step = OwnedStep::Entered;
                            let seen = now;
                            chat.staging.owned = Some(Owned { line, stamp, step, epoch, due, last, seen });
                        }
                    }
                }
                return true;
            }
        }
        if chat.queue.as_ref().is_some_and(|q| q.awaiting && q.generation != pane.model.chat.generation)
            && !chat.awaiting_transcript
        {
            // The new chat's transcript appeared: the queued draft follows it.
            let (generation, latest) = (pane.model.chat.generation, pane.model.chat.latest.map(|x| x.0));
            let queue = tab.pane.as_mut().unwrap().chat.queue.as_mut().unwrap();
            (queue.generation, queue.latest, queue.awaiting) = (generation, latest, false);
            return true;
        }
        if let Some(wait) = &chat.staging.command {
            let error = blocked(tab, now, Intent::Slash);
            if error.is_none() && may_write {
                let wait = tab.pane.as_mut().unwrap().chat.staging.command.take().unwrap();
                Self::submit_slash(tab, now, &wait.line, wait.from_editor);
                return true;
            }
            if now >= wait.due || error.is_some_and(|e| e != NO_QUIET_PROMPT) {
                if let Some(error) = error.filter(|e| super::screen_report::worth(e)) {
                    super::screen_report::save(tab, now, error);
                }
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.staging.command = None;
                chat.refused(error.unwrap_or(NO_QUIET_PROMPT), Some(Intent::Slash));
                return true;
            }
        }
        if follow_owned(tab, now, may_write) || watch_unseen(tab, now) {
            return true;
        }
        if take_back(tab, now, may_write) {
            return true;
        }
        let pane = tab.pane.as_ref().unwrap();
        let chat = &pane.chat;
        if let Some(queue) = &chat.queue {
            if chat.identity_unverified
                || pane.model.error.is_some()
                || pane.model.chat.generation != queue.generation
                || chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now).0 == Gate::Ended
            {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.queue = None;
                chat.refused("The session changed or ended. The queue was cancelled; draft kept.", None);
                return true;
            }
            let intent = if queue.interrupted(tab, now) {
                Intent::Interrupted
            } else if queue.now {
                Intent::Now
            } else {
                Intent::Draft
            };
            // The screen's idle input releases it as a newer completion does: the transcript may
            // never show one (an interrupted turn, a command, a resumed session).
            let idle = chat.screen_idle(&tab.term, now);
            // Never past keys of a card, a mode change or Ctrl+B still on their way: the draft
            // stays queued until they have gone (the send itself would refuse and drop the queue).
            if may_write
                && chat.waiting().is_none()
                && !chat.enter_only()
                && (matches!(intent, Intent::Interrupted | Intent::Now) || idle || queue.completed(&pane.model.chat))
                && blocked(tab, now, intent).is_none()
                && (chat.demo || clean_prompt(&tab.term.screen))
            {
                tab.pane.as_mut().unwrap().chat.queue = None;
                Self::send_with_intent(tab, now, intent);
                return true;
            }
        }
        if chat.staging.retry.as_ref().is_some_and(|retry| !retry.stamp.matches(tab)) {
            tab.pane.as_mut().unwrap().chat.interrupt_submission();
            return true;
        }
        if let Some(intent) = chat.retry {
            let error = blocked(tab, now, intent);
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            let updated = chat.error != error;
            chat.error = error;
            if error.is_none() {
                chat.retry = None;
            }
            return changed || updated;
        }
        changed
    }

    pub(super) fn submit_slash(tab: &mut Tab, now: Instant, line: &str, from_editor: bool) {
        clear_recovery(tab, now);
        let error = blocked(tab, now, Intent::Slash);
        // A command refused for a running turn while the screen reads idle, or for a screen Chat
        // could not read: worth a report (a prompt still settling is waited for first).
        let at_rest = error == Some(BUSY_CMD) && tab.pane.as_ref().is_some_and(|p| p.chat.screen_idle(&tab.term, now));
        let report = error
            .filter(|e| super::screen_report::worth(e) && *e != NO_QUIET_PROMPT)
            .or(at_rest.then_some("A turn is running (the screen reads idle)"));
        if let Some(why) = report {
            super::screen_report::save(tab, now, why);
        }
        let stamp = Stamp::at(tab);
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.locked() {
            chat.refused("Cancel the queued or pending send before running a command.", None);
            return;
        }
        chat.recall.end();
        if from_editor && !chat.chips.is_empty() {
            chat.refused(REMOVE_ATTACHMENTS, None);
            return;
        }
        let Some(bytes) = slash_bytes(line) else {
            chat.refused(BAD_COMMAND, None);
            return;
        };
        if error == Some(NO_QUIET_PROMPT) && !chat.demo && chat.staging.command.is_none() {
            // The prompt may only be settling (a turn just ended, a key just went): the command
            // waits a moment for it and runs by itself.
            chat.staging.command = Some(CommandWait { due: now + COMMAND_WAIT, line: line.to_owned(), from_editor });
            chat.commands.open = false;
            chat.error = None;
            chat.retry = None;
            return;
        }
        if let Some(error) = error {
            chat.refused(error, Some(Intent::Slash));
            return;
        }
        pane.note_composer_input();
        if let Err(error) = write(tab, &bytes, line) {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.staging.recovery = Some(Recovery::Command(stamp));
            chat.refused(error, None);
            return;
        }
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.commands.open = false;
        chat.error = None;
        chat.retry = None;
        chat.staging.unseen = None;
        chat.staging.echo = Some(EchoWait { stamp, due: now + ECHO_TIMEOUT, line: line.to_owned(), from_editor });
    }

    pub(super) fn retry_enter(tab: &mut Tab, now: Instant) {
        let chat = &tab.pane.as_ref().unwrap().chat;
        let Some(retry) = &chat.staging.retry else { return };
        if !can_confirm(tab, now, retry.stamp, retry.into_turn) || !shown_exactly(tab, &retry.expected) {
            tab.pane.as_mut().unwrap().chat.refused(ENTER_WAIT, None);
            return;
        }
        let retry = tab.pane.as_mut().unwrap().chat.staging.retry.take().unwrap();
        submit_enter(tab, retry);
    }
}

/// Interrupted before any answer, Claude Code takes the turn out of the conversation and puts the
/// prompt back on its input (2.1.332), where it would block every later send. While that input
/// shows exactly the message Chat last submitted, with no turn running and nothing else typed
/// into the terminal since, Chat empties it with Ctrl+U (one line per key, each after the screen
/// showed the one before) and gives the text back to the message box. Only an empty box with no
/// attachments and no held draft takes it back: otherwise the message stays on the CLI input,
/// with a notice, so that no text is ever cleared without a place to go.
fn take_back(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let screen = &tab.term.screen;
    if chat.staging.sent.is_empty() {
        return false;
    }
    let resting = chat.screen_resting(&tab.term, now);
    let Some(step) = &chat.staging.takeback else {
        let settled = may_write && chat.waiting().is_none() && !chat.enter_only() && resting && !clean_prompt(screen);
        if !settled {
            return false;
        }
        let Some(index) = chat.staging.sent.iter().position(|sent| shown_exactly(tab, &sent.expected)) else {
            // Something else rests on the input after a stop: the next send would wait for it
            // without a word.
            if !chat.staging.stopped {
                return false;
            }
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.staging.sent.clear();
            chat.staging.stopped = false;
            chat.refused(TAKEBACK_OTHER, None);
            return true;
        };
        let box_free = chat.editor.text.trim().is_empty() && chat.chips.is_empty() && chat.queue.is_none();
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        if box_free {
            let epoch = chat.output_epoch.wrapping_sub(1);
            chat.staging.takeback = Some(Takeback { epoch, presses: 0, due: now + TAKEBACK_STEP, clean: None, index });
        } else {
            // The message stays where the user can see it, and the box keeps its draft (a held
            // one is let go when the user types in the terminal, and stays in the box).
            chat.staging.sent.clear();
            chat.staging.stopped = false;
            chat.refused(TAKEBACK_HELD, None);
        }
        return true;
    };
    if clean_prompt(screen) {
        // The input reads empty. Settled (quiet), that is so. With output still coming (a turn
        // that starts by itself, a redraw) one reading may be a screen caught half way, with
        // part of the message still on the input: it counts once a second reading of newer
        // output, both after the last Ctrl+U's, reads empty too.
        let epoch = chat.output_epoch;
        let fresh = epoch != step.epoch;
        let confirmed = chat.quiet(now) || fresh && step.clean.is_some_and(|first| first != epoch);
        if confirmed {
            give_back(&mut tab.pane.as_mut().unwrap().chat, false);
            return true;
        }
        if now >= step.due {
            // No Enter went with the keys, so the message was not sent: it comes back to the
            // box rather than nowhere, with a word to check what the CLI input still holds.
            give_back(&mut tab.pane.as_mut().unwrap().chat, true);
            return true;
        }
        if fresh && step.clean.is_none() {
            tab.pane.as_mut().unwrap().chat.staging.takeback.as_mut().unwrap().clean = Some(epoch);
        }
        return false;
    }
    if step.clean.is_some() {
        // Not two readings in a row.
        tab.pane.as_mut().unwrap().chat.staging.takeback.as_mut().unwrap().clean = None;
    }
    let chat = &tab.pane.as_ref().unwrap().chat;
    let Some(step) = &chat.staging.takeback else { return false };
    let input = prompt(screen).is_some() || screen::shows_some_input(screen);
    if step.presses >= TAKEBACK_KEYS || now >= step.due || screen::working(screen) || !input {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.staging.takeback = None;
        chat.staging.sent.clear();
        chat.refused(TAKEBACK_FAILED, None);
        return true;
    }
    if !may_write || chat.output_epoch == step.epoch {
        return false;
    }
    let (presses, index) = (step.presses + 1, step.index);
    tab.pane.as_mut().unwrap().note_composer_input();
    let written = write(tab, CLEAR_LINE, "");
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match written {
        Ok(()) => {
            chat.staging.takeback =
                Some(Takeback { epoch: chat.output_epoch, presses, due: now + TAKEBACK_STEP, clean: None, index })
        }
        Err(error) => {
            chat.staging.takeback = None;
            chat.staging.sent.clear();
            chat.refused(error, None);
        }
    }
    true
}

/// A command Chat typed on its own, after its keys. Entered: taken once the input lets go of it
/// (another text at rest, or an empty input); still exactly on the input after `ENTER_CONFIRM`,
/// Enter was lost and the line is withdrawn. An input Chat cannot read (a panel the command
/// opened, a prompt a card answers) is waited for until `OWNED_LIMIT`: the line still read there
/// is left (said), else the command ran and following ends without a word (said after all should
/// the input later show exactly the line at rest: `watch_unseen`).
/// Withdrawn: Ctrl+U, one press at a time, only while the input shows exactly the line at rest and
/// the gate would let a key go (`can_withdraw`), until it reads empty; time a turn runs does not
/// count against the limit. Text of anyone else's on the input is never cleared; terminal input
/// ends the following (`interrupt_staging`; after Enter the result is unknown). A command
/// that does not run ends Claude Code's command tracking with it, and the box says what happened.
fn follow_owned(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let Some(owned) = &chat.staging.owned else { return false };
    let screen = &tab.term.screen;
    let fresh = chat.output_epoch != owned.epoch;
    let readable = prompt(screen).is_some();
    let exactly = shown_exactly(tab, &owned.line);
    let resting = chat.screen_resting(&tab.term, now);
    let end = |tab: &mut Tab, result: CommandResult, notice: Option<&'static str>| {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.staging.owned = None;
        chat.staging.result = Some(result);
        if result != CommandResult::Taken {
            // Nothing of it runs: the command's tracking would wait for an input that never
            // empties by itself.
            chat.slash = false;
            chat.cli_report.tracking = false;
        }
        if let Some(notice) = notice {
            chat.refused(notice, None);
        }
        true
    };
    match owned.step {
        OwnedStep::Entered => {
            // One reading caught half way through a redraw is not enough: the input at rest, or
            // read empty.
            if fresh && readable && !exactly && (resting || clean_prompt(screen)) {
                return end(tab, CommandResult::Taken, None);
            }
            if now >= owned.last {
                if exactly {
                    return end(tab, CommandResult::Left, Some(COMMAND_LEFT));
                }
                // The line was not read on the input: a command that opens a panel (`/config`,
                // `/mcp`, a bare `/model`) hides it, and ran. Following ends without a word,
                // unless the input later shows exactly the line at rest (`watch_unseen`).
                let line = owned.line.clone();
                end(tab, CommandResult::Taken, None);
                tab.pane.as_mut().unwrap().chat.staging.unseen = Some(line);
                return true;
            }
            if now < owned.due || !(exactly && resting) {
                return false;
            }
            // Enter did not take: the line comes off again.
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            let owned = chat.staging.owned.as_mut().unwrap();
            owned.step = OwnedStep::Withdraw { presses: 0 };
            owned.epoch = chat.output_epoch.wrapping_sub(1);
            owned.seen = now;
            true
        }
        OwnedStep::Withdraw { presses } => {
            // A turn that runs holds the Ctrl+U back (`can_withdraw`): its time does not count
            // against the limit, so the line still comes off once the turn is over.
            let running =
                screen::working(screen) || busy_title(&tab.program_title) || pane.model.chat.pending.is_some();
            if running {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let owned = chat.staging.owned.as_mut().unwrap();
                owned.last += now.saturating_duration_since(owned.seen);
                owned.seen = owned.seen.max(now);
                return false;
            }
            tab.pane.as_mut().unwrap().chat.staging.owned.as_mut().unwrap().seen = now;
            let chat = &tab.pane.as_ref().unwrap().chat;
            let Some(owned) = &chat.staging.owned else { return false };
            if clean_prompt(screen) && (presses == 0 || fresh) {
                return end(tab, CommandResult::Withdrawn, Some(COMMAND_WITHDRAWN));
            }
            if now >= owned.last || presses >= OWNED_PRESSES {
                return end(tab, CommandResult::Left, Some(COMMAND_LEFT));
            }
            if readable && !exactly && !clean_prompt(screen) && fresh {
                // Someone else's text: never cleared.
                return end(tab, CommandResult::Left, Some(COMMAND_LEFT));
            }
            if !may_write || !fresh || !exactly || !resting || !can_withdraw(tab, now, owned.stamp) {
                return false;
            }
            tab.pane.as_mut().unwrap().note_composer_input();
            let written = write(tab, CLEAR_LINE, "");
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            match written {
                Ok(()) => {
                    let owned = chat.staging.owned.as_mut().unwrap();
                    owned.step = OwnedStep::Withdraw { presses: presses + 1 };
                    owned.epoch = chat.output_epoch;
                    true
                }
                Err(_) => end(tab, CommandResult::Left, Some(COMMAND_LEFT)),
            }
        }
    }
}

/// A command taken for run while the input could not be read: once the input can be read at
/// rest, it showing exactly the line means the command never ran. Said, and its tracking ends.
fn watch_unseen(tab: &mut Tab, now: Instant) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    let Some(line) = &chat.staging.unseen else { return false };
    if chat.staging.owned.is_some() || prompt(&tab.term.screen).is_none() || !chat.screen_resting(&tab.term, now) {
        return false;
    }
    let left = shown_exactly(tab, line);
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    chat.staging.unseen = None;
    if left {
        chat.staging.result = Some(CommandResult::Left);
        chat.slash = false;
        chat.cli_report.tracking = false;
        chat.refused(COMMAND_LEFT, None);
    }
    left
}

/// The take-back is over: the message goes back to the box (in front of anything there, which
/// the locked composer should never hold), with a word to check the terminal when the CLI input
/// could not be read as empty for sure.
fn give_back(chat: &mut Chat, unsure: bool) {
    let index = chat.staging.takeback.take().map_or(0, |step| step.index);
    let sent = chat.staging.sent.swap_remove(index);
    chat.staging.sent.clear();
    chat.staging.stopped = false;
    chat.queue = None;
    let text = match chat.editor.text.trim().is_empty() {
        true => sent.text,
        false => format!("{}\n\n{}", sent.text, chat.editor.text),
    };
    chat.editor.set(text);
    chat.error = Some(match (unsure, sent.picture) {
        (true, _) => TAKEBACK_UNSURE,
        (false, true) => TAKEN_BACK_PICTURE,
        (false, false) => TAKEN_BACK,
    });
    chat.retry = None;
}

fn clear_recovery(tab: &mut Tab, now: Instant) -> bool {
    let Some(recovery) = tab.pane.as_ref().unwrap().chat.staging.recovery else { return false };
    if !recovery_ready(tab, now, recovery) {
        return false;
    }
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    chat.staging.recovery = None;
    chat.error = None;
    chat.retry = None;
    true
}

pub(super) fn slash_bytes(line: &str) -> Option<Vec<u8>> {
    let name = line.strip_prefix('/')?.split_ascii_whitespace().next()?;
    if line.as_bytes().get(1).is_none_or(u8::is_ascii_whitespace)
        || !super::commands::valid_name(name)
        || line.len() > 4096
        || line.chars().any(char::is_control)
    {
        return None;
    }
    // Never include Enter: the next stage must see this exact text in the live input row.
    Some(line.as_bytes().to_vec())
}

/// Built-in commands that only change settings or show a panel and never start a model turn. Any
/// other command (skills, plugins, custom prompts, unknown names) is taken to start one.
const LOCAL_COMMANDS: [&str; 22] = [
    "model",
    "effort",
    "help",
    "context",
    "cost",
    "usage",
    "status",
    "config",
    "tui",
    "permissions",
    "mcp",
    "theme",
    "doctor",
    "login",
    "logout",
    "add-dir",
    "agents",
    "hooks",
    "memory",
    "output-style",
    "vim",
    "exit",
];

fn local_command(line: &str) -> bool {
    let name = line.strip_prefix('/').and_then(|s| s.split_ascii_whitespace().next()).unwrap_or("");
    LOCAL_COMMANDS.iter().any(|c| name.eq_ignore_ascii_case(c))
}

pub(super) fn send_bytes(chips: &[Chip], text: &str) -> Option<Vec<u8>> {
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) {
        return None;
    }
    // Line breaks stay: the bracketed paste keeps them in one message.
    let text = text.replace('\t', " ");
    let paths: Vec<_> = chips
        .iter()
        .filter_map(|chip| match chip {
            Chip::Path(path) => Some(path.clone()),
            Chip::Clipboard(_) => None,
        })
        .collect();
    let paths = super::super::attach::paths(&paths);
    let mut payload = paths;
    if !payload.is_empty() && !text.is_empty() {
        payload.push(' ');
    }
    payload.push_str(&text);
    if payload.is_empty() && !chips.iter().any(|chip| matches!(chip, Chip::Clipboard(_))) {
        return None;
    }
    Some(if payload.is_empty() { Vec::new() } else { mdshared_term::input::encode_paste(&payload, true) })
}

fn finish(tab: &mut Tab, now: Instant, paste: &[u8], completion: Completion, marker: Option<String>, into_turn: bool) {
    let stamp = Stamp::at(tab);
    let picture = marker.is_some();
    let payload = std::str::from_utf8(paste)
        .unwrap_or("")
        .strip_prefix("\x1b[200~")
        .and_then(|s| s.strip_suffix("\x1b[201~"))
        .unwrap_or("")
        .trim()
        // The paste carries line breaks as CR, as a terminal pastes them.
        .replace('\r', "\n");
    let expected = match marker {
        Some(marker) if !payload.is_empty() => format!("{marker} {payload}"),
        Some(marker) => marker,
        None => payload,
    };
    tab.pane.as_mut().unwrap().note_composer_input_into_turn(into_turn);
    if !paste.is_empty()
        && let Err(error) = write(tab, paste, "")
    {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.queue = None;
        chat.staging.recovery = Some(Recovery::Manual(stamp, picture));
        chat.refused(error, None);
        return;
    }
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    // Into a running turn, a permission prompt or a menu may be drawn between the paste and Enter
    // (its default is Yes): Enter goes only once the input line shows the text. So it does for
    // several lines (or the CLI's paste marker). The demo has no CLI to show it.
    let into_turn = into_turn && !chat.demo;
    if expected.contains('\n') || into_turn && matches!(completion, Completion::Message(_)) {
        let Completion::Message(text) = completion else { unreachable!("commands are one line") };
        chat.error = None;
        chat.retry = None;
        let due = now + ECHO_TIMEOUT;
        chat.staging.lines = Some(LinesWait { stamp, due, expected, text, picture, into_turn });
        return;
    }
    submit_enter(tab, EnterOnly { stamp, expected, completion, picture, into_turn });
}

fn submit_enter(tab: &mut Tab, input: EnterOnly) {
    let (demo_text, command) = match &input.completion {
        Completion::Message(text) => (text.as_str(), None),
        Completion::Command { line, .. } => (line.as_str(), Some(line.as_str())),
    };
    let baseline = command.map(|_| local_reports(&tab.term.screen));
    tab.pane.as_mut().unwrap().note_composer_input_into_turn(input.into_turn);
    if write(tab, b"\r", demo_text).is_err() {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.queue = None;
        chat.staging.retry = Some(input);
        chat.refused(ENTER_ONLY, None);
        return;
    }
    let pane = tab.pane.as_mut().unwrap();
    let chat = &mut pane.chat;
    chat.note_sent();
    match input.completion {
        Completion::Message(_) => {
            chat.first_sent |= chat.awaiting_transcript;
            let (expected, text, picture) = (input.expected, chat.editor.text.clone(), input.picture);
            // One sent at an idle input starts a turn: those before it are done with.
            if !input.into_turn {
                chat.staging.sent.clear();
            }
            chat.staging.sent.push(Sent { expected, text, picture });
            chat.staging.stopped = false;
            chat.submitted();
        }
        Completion::Command { line, from_editor } => {
            // A command that may start a model turn counts as the new chat's first send.
            chat.first_sent |= chat.awaiting_transcript && !local_command(&line);
            chat.cli_report.begin(baseline.unwrap_or_default(), chat.output_epoch, chat.demo, chat.input_floor, &line);
            if from_editor {
                chat.submitted();
            } else {
                chat.error = None;
                chat.retry = None;
            }
            chat.slash = !chat.demo;
            chat.suppressed = None;
            chat.react(chat.slash.then_some(Reason::Slash));
            if chat.demo {
                chat.drawer = Drawer::Manual;
            }
        }
    }
}

pub(super) fn write(tab: &mut Tab, bytes: &[u8], demo_text: &str) -> Result<(), &'static str> {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    #[cfg(test)]
    {
        chat.writes.push(bytes.to_vec());
        if bytes == b"\r" && std::mem::take(&mut chat.fail_enter_once) {
            return Err("Simulated Enter write failure");
        }
    }
    if chat.demo {
        if bytes == IMAGE_KEY {
            tab.term.feed("\r\n❯ [Image #1]".as_bytes());
            chat.output_epoch = chat.output_epoch.wrapping_add(1);
        } else if bytes.starts_with(b"/") {
            tab.term.feed(format!("\r\n❯ {demo_text}").as_bytes());
            chat.output_epoch = chat.output_epoch.wrapping_add(1);
        } else if let Some(text) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.strip_prefix("\x1b[200~"))
            .and_then(|s| s.strip_suffix("\x1b[201~"))
            .filter(|s| s.contains('\r'))
        {
            // The demo shows a pasted message of several lines as the CLI's input would.
            tab.term.feed(format!("\r\n❯ {}", text.replace('\r', "\r\n  ")).as_bytes());
            chat.output_epoch = chat.output_epoch.wrapping_add(1);
        } else if !bytes.starts_with(b"\x1b[200~") {
            let demo_text: String = demo_text.chars().filter(|c| !c.is_control() || *c == '\n').collect();
            tab.term.feed(
                format!("\r\n> {}\r\n[Demo input — no process started]\r\n", demo_text.replace('\n', "\r\n"))
                    .as_bytes(),
            );
            chat.demo_progress = true;
        }
    } else if !chat.scripted() {
        let pty =
            tab.term.pty.as_ref().filter(|p| tab.term.ended.is_none() && p.running()).ok_or("The session ended.")?;
        pty.write(bytes).map_err(|_| "The terminal write failed. Check the drawer before retrying.")?;
    }
    tab.term.scroll = 0;
    tab.term.selection = None;
    Ok(())
}

/// The next stage deadline still ahead. One already past is never armed again: its stage acts on
/// it at the next tick, which the pending work's poll brings anyway, and re-arming an instant
/// that has gone would wake the window at once, over and over.
pub(super) fn stage_deadline(chat: &Chat, now: Instant) -> Option<Instant> {
    [
        chat.image_wait.as_ref().map(|wait| wait.due),
        chat.staging.echo.as_ref().map(|wait| wait.due),
        chat.staging.lines.as_ref().map(|wait| wait.due),
        chat.staging.command.as_ref().map(|wait| wait.due),
        chat.staging.takeback.as_ref().map(|step| step.due),
        chat.staging.owned.as_ref().map(|owned| owned.due.max(now + Duration::from_millis(250)).min(owned.last)),
        chat.question.deadline(),
        chat.mode_deadline(),
    ]
    .into_iter()
    .flatten()
    .filter(|due| *due > now)
    .min()
}

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
pub(in crate::view::chat) mod ready_tests;
#[cfg(test)]
mod tests;
