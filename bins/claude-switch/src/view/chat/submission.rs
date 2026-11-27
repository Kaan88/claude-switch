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
pub(super) use stages::Staging;
use stages::{
    CommandWait, Completion, EchoWait, EnterOnly, LinesWait, Recovery, Sent, Stamp, Takeback, can_confirm,
    recovery_ready, shown_exactly,
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
const IMAGE_KEY: &[u8] = b"\x1bv";
pub(super) const CHECK: &str = "Open the terminal to check";
pub(super) const NO_QUIET_PROMPT: &str = "No quiet, empty Claude Code prompt is visible. Check the terminal.";
pub(super) const NOT_EMPTY: &str = "The CLI input line is not visibly empty. Wait or use the terminal.";
pub(super) const BAD_TEXT: &str = "The draft has unsupported control characters.";
pub(super) const CHANGED_CLIPBOARD: &str = "The clipboard changed. Paste the picture into Chat again.";
const IMAGE_TIMEOUT_NOTICE: &str = "No image marker yet. The CLI may hold a picture; check the terminal. Draft kept.";
const IMAGE_INTERRUPTED: &str = "Image send interrupted. The CLI may hold a picture; check the terminal.";
const COMMAND_TIMEOUT: &str = "The command did not appear on the CLI input line. Check the terminal before retrying.";
const COMMAND_INTERRUPTED: &str = "Command not submitted. Check the terminal for text left on its input line.";
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
}

struct Stop {
    stamp: Stamp,
    was_working: bool,
    was_interrupted: bool,
}

impl Queue {
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
    allow_working: bool,
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
        self.queue = Some(Queue { generation, latest: None, stop: None, awaiting: false });
    }

    pub(super) fn interrupt_submission(&mut self) {
        self.interrupt_card();
        self.interrupt_staging();
    }

    /// Terminal input that leaves card key sequences alone (a wheel report): what was staged for a
    /// send no longer knows the screen.
    pub(super) fn interrupt_staging(&mut self) {
        // Whatever the CLI's input holds now may be the user's own.
        (self.staging.sent, self.staging.takeback, self.staging.command) = (None, None, None);
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
        return Some("Close the CLI menu in the terminal before sending.");
    }
    if pane.model.error.is_some() && !chat.demo {
        return Some(Reason::Identity.label());
    }
    if !chat.demo && matches!(intent, Intent::Draft | Intent::Override) {
        if prompt(&tab.term.screen).is_some_and(|row| !row.is_empty()) {
            return Some("The CLI input line already contains text or an attachment. Check the terminal.");
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
        Intent::Slash
            if gate == Gate::Running || pane.model.chat.latest.is_some_and(|(_, p)| p == ChatPhase::Active) =>
        {
            Some(BUSY_CMD)
        }
        Intent::Slash if !chat.demo && !idle_slash_prompt(&tab.term.screen, chat.quiet(now)) => Some(NO_QUIET_PROMPT),
        _ => None,
    }
}

impl AppState {
    pub(super) fn chat_send(tab: &mut Tab, now: Instant, force: bool) {
        let pane = tab.pane.as_ref().unwrap();
        let chat = &pane.chat;
        let intent = if !force {
            Intent::Draft
        } else if chat.queue.is_some()
            && (chat.demo
                || clean_prompt(&tab.term.screen)
                || chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now).0 == Gate::Running)
        {
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
        let stamp = Stamp::at(tab);
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
        // goes out once the CLI's input is ready.
        let waits = matches!(error, Some(e) if e == CHECK || e == NO_TRANSCRIPT || e == Reason::Slash.label());
        // A picture needs the visibly empty input for its marker, whoever asked for the send.
        let picture_waits =
            chat.clipboard_sequence().is_some() && error.is_none() && !chat.demo && !clean_prompt(&tab.term.screen);
        if intent == Intent::Draft && waits || picture_waits {
            chat.queue = Some(Queue {
                generation: pane.model.chat.generation,
                latest: pane.model.chat.latest.map(|x| x.0),
                stop: None,
                awaiting: chat.awaiting_transcript,
            });
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
            let wait = ImageWait {
                due: now + IMAGE_TIMEOUT,
                stamp,
                seq,
                bytes,
                demo_text,
                allow_working: intent == Intent::Now,
            };
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
            tab.pane.as_mut().unwrap().note_composer_input();
        } else {
            finish(tab, now, &bytes, Completion::Message(demo_text), None, intent == Intent::Now);
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
                if let Some(queue) = &mut tab.pane.as_mut().unwrap().chat.queue {
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
            let veto = !can_confirm(tab, now, wait.stamp, wait.allow_working)
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
                        finish(tab, now, &wait.bytes, Completion::Message(text), Some(marker), wait.allow_working);
                    }
                }
                return true;
            }
        }
        if let Some(wait) = &chat.staging.lines {
            let result = if !can_confirm(tab, now, wait.stamp, wait.allow_working) {
                Some(Err(LINES_INTERRUPTED))
            } else if now >= wait.due {
                Some(Err(LINES_TIMEOUT))
            } else if may_write && chat.output_epoch != wait.stamp.epoch && shown_exactly(tab, &wait.expected) {
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
                            allow_working: wait.allow_working,
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
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let wait = chat.staging.echo.take().unwrap();
                match result {
                    Err(error) => {
                        chat.staging.recovery =
                            Some(Recovery::Command(Stamp { epoch: chat.output_epoch, ..wait.stamp }));
                        chat.refused(error, None);
                    }
                    Ok(()) => submit_enter(
                        tab,
                        EnterOnly {
                            stamp: wait.stamp,
                            expected: wait.line.clone(),
                            completion: Completion::Command { line: wait.line, from_editor: wait.from_editor },
                            picture: false,
                            allow_working: false,
                        },
                    ),
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
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.staging.command = None;
                chat.refused(error.unwrap_or(NO_QUIET_PROMPT), Some(Intent::Slash));
                return true;
            }
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
            let intent = if queue.interrupted(tab, now) { Intent::Interrupted } else { Intent::Draft };
            // The screen's idle input releases it as a newer completion does: the transcript may
            // never show one (an interrupted turn, a command, a resumed session).
            let idle = chat.screen_idle(&tab.term, now);
            // Never past keys of a card, a mode change or Ctrl+B still on their way: the draft
            // stays queued until they have gone (the send itself would refuse and drop the queue).
            if may_write
                && chat.waiting().is_none()
                && !chat.enter_only()
                && (intent == Intent::Interrupted || idle || queue.completed(&pane.model.chat))
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
        chat.staging.echo = Some(EchoWait { stamp, due: now + ECHO_TIMEOUT, line: line.to_owned(), from_editor });
    }

    pub(super) fn retry_enter(tab: &mut Tab, now: Instant) {
        let chat = &tab.pane.as_ref().unwrap().chat;
        let Some(retry) = &chat.staging.retry else { return };
        if !can_confirm(tab, now, retry.stamp, retry.allow_working) || !shown_exactly(tab, &retry.expected) {
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
/// showed the one before) and gives the text back to the message box when that is empty.
fn take_back(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let screen = &tab.term.screen;
    let Some(sent) = &chat.staging.sent else { return false };
    let resting = chat.screen_resting(&tab.term, now);
    let Some(step) = &chat.staging.takeback else {
        let starts = may_write
            && chat.waiting().is_none()
            && !chat.enter_only()
            && resting
            && !clean_prompt(screen)
            && shown_exactly(tab, &sent.expected);
        if starts {
            let epoch = chat.output_epoch.wrapping_sub(1);
            tab.pane.as_mut().unwrap().chat.staging.takeback =
                Some(Takeback { epoch, presses: 0, due: now + TAKEBACK_STEP });
        }
        return starts;
    };
    if clean_prompt(screen) {
        if !chat.quiet(now) {
            return false;
        }
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        let sent = chat.staging.sent.take().unwrap();
        chat.staging.takeback = None;
        if chat.editor.text.trim().is_empty() && chat.chips.is_empty() && chat.queue.is_none() {
            chat.editor.set(sent.text);
            chat.error = Some(if sent.picture { TAKEN_BACK_PICTURE } else { TAKEN_BACK });
            chat.retry = None;
        }
        return true;
    }
    let input = prompt(screen).is_some() || screen::shows_some_input(screen);
    if step.presses >= TAKEBACK_KEYS || now >= step.due || screen::working(screen) || !input {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        (chat.staging.sent, chat.staging.takeback) = (None, None);
        chat.refused(TAKEBACK_FAILED, None);
        return true;
    }
    if !may_write || chat.output_epoch == step.epoch {
        return false;
    }
    let presses = step.presses + 1;
    tab.pane.as_mut().unwrap().note_composer_input();
    let written = write(tab, CLEAR_LINE, "");
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match written {
        Ok(()) => {
            chat.staging.takeback = Some(Takeback { epoch: chat.output_epoch, presses, due: now + TAKEBACK_STEP })
        }
        Err(error) => {
            (chat.staging.sent, chat.staging.takeback) = (None, None);
            chat.refused(error, None);
        }
    }
    true
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

fn finish(
    tab: &mut Tab,
    now: Instant,
    paste: &[u8],
    completion: Completion,
    marker: Option<String>,
    allow_working: bool,
) {
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
    tab.pane.as_mut().unwrap().note_composer_input();
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
    if expected.contains('\n') {
        // Several lines: Enter only once the input line shows them (or the CLI's paste marker).
        let Completion::Message(text) = completion else { unreachable!("commands are one line") };
        chat.error = None;
        chat.retry = None;
        let due = now + ECHO_TIMEOUT;
        chat.staging.lines = Some(LinesWait { stamp, due, expected, text, picture, allow_working });
        return;
    }
    submit_enter(tab, EnterOnly { stamp, expected, completion, picture, allow_working });
}

fn submit_enter(tab: &mut Tab, input: EnterOnly) {
    let (demo_text, command) = match &input.completion {
        Completion::Message(text) => (text.as_str(), None),
        Completion::Command { line, .. } => (line.as_str(), Some(line.as_str())),
    };
    let baseline = command.map(|_| local_reports(&tab.term.screen));
    tab.pane.as_mut().unwrap().note_composer_input();
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
            chat.staging.sent = Some(Sent { expected, text, picture });
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

pub(super) fn stage_deadline(chat: &Chat) -> Option<Instant> {
    chat.image_wait
        .as_ref()
        .map(|wait| wait.due)
        .or_else(|| chat.staging.echo.as_ref().map(|wait| wait.due))
        .or_else(|| chat.staging.lines.as_ref().map(|wait| wait.due))
        .or_else(|| chat.staging.command.as_ref().map(|wait| wait.due))
        .or_else(|| chat.staging.takeback.as_ref().map(|step| step.due))
        .or_else(|| chat.question.deadline())
        .or_else(|| chat.mode_deadline())
}

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod ready_tests;
#[cfg(test)]
mod tests;
