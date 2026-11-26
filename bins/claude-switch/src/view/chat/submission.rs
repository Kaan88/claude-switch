//! The only composer PTY writes. A clipboard send has a separate, fail-closed second step.
use std::time::{Duration, Instant};

use claude_switch_core::transcript::ChatInfo;
use mdshared_term::Screen;

use super::{Chat, ChatPhase, Chip, Drawer, Gate, QUIET, Reason, clipboard};
use crate::{state::AppState, tabs::Tab};

const IMAGE_TIMEOUT: Duration = Duration::from_secs(4);
const IMAGE_KEY: &[u8] = b"\x1bv";
pub(super) const CHECK: &str = "Open the terminal to check";
pub(super) const BAD_TEXT: &str = "The draft has unsupported control characters.";
pub(super) const CHANGED_CLIPBOARD: &str = "The clipboard changed. Paste the picture into Chat again.";
const IMAGE_TIMEOUT_NOTICE: &str = "No new CLI image marker. Check the terminal before retrying; draft kept.";
const IMAGE_INTERRUPTED: &str = "Image send interrupted. Check the terminal for an unsent image; draft kept.";
const NO_PASTE: &str = "Bracketed paste is unavailable. Open the terminal to check.";
const ALTERNATE: &str = "The terminal is showing a full-screen view. Return to the CLI prompt first.";
const BUSY_CMD: &str = "A turn is running. Wait for it to finish before running this command.";
const NO_IMAGE_PROMPT: &str = "No empty CLI input line for the picture. Check the terminal.";
pub(super) const REMOVE_ATTACHMENTS: &str = "Remove attachments before running a slash command.";
pub(super) const BAD_COMMAND: &str = "Choose a command or enter a single slash command without control characters.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Draft,
    Override,
    Now,
    Slash,
}

pub(super) struct Queue {
    generation: u64,
    latest: Option<u64>,
}

impl Queue {
    fn completed(&self, info: &ChatInfo) -> bool {
        self.generation == info.generation
            && info.latest.is_some_and(|(at, phase)| phase == ChatPhase::EndTurn && Some(at) > self.latest)
    }
}

pub(super) struct ImageWait {
    due: Instant,
    epoch: u64,
    generation: u64,
    prompt: Option<u64>,
    seq: u32,
    bytes: Vec<u8>,
    demo_text: String,
}

#[derive(Default)]
pub(super) struct CliReport {
    pub model: Option<String>,
    pub effort: Option<&'static str>,
    baseline: Vec<(u64, String)>,
    epoch: u64,
    tracking: bool,
    transcript: Option<(u64, u64)>,
    floor: Option<u64>,
    expects: Option<Choice>,
    confirmed: bool,
    touched_terminal: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Model,
    Effort,
}

fn prompt(screen: &Screen) -> Option<String> {
    let cursor = screen.cursor();
    let line = screen.row_text(cursor.row);
    let payload = line.trim_start().strip_prefix('❯')?;
    if !payload.is_empty() && !payload.starts_with(' ') {
        return None;
    }
    // Different CLI renderers leave the cursor just before or after a trailing blank cell.
    let end = line.chars().count();
    (cursor.col >= end.saturating_sub(1) && cursor.col <= end + 1).then(|| payload.trim().to_owned())
}

fn is_image_marker(text: &str) -> bool {
    text.strip_prefix("[Image #").and_then(|s| s.strip_suffix(']')).is_some_and(|n| {
        !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<u32>().is_ok_and(|n| n > 0)
    })
}

fn clean_prompt(screen: &Screen) -> bool {
    prompt(screen).as_deref() == Some("")
}

fn idle_slash_prompt(screen: &Screen, quiet: bool) -> bool {
    if !quiet || !clean_prompt(screen) {
        return false;
    }
    let row = screen.cursor().row;
    !(row.saturating_sub(3)..(row + 4).min(screen.rows())).any(|i| {
        let row = screen.row_text(i).to_ascii_lowercase();
        row.contains("esc to interrupt") || row.contains("ctrl+c to interrupt")
    })
}

// Read only local-command output rows, not arbitrary Markdown, the prompt itself or scrollback.
fn local_reports(screen: &Screen) -> Vec<(u64, String)> {
    let row = screen.cursor().row;
    (row.saturating_sub(12)..(row + 3).min(screen.rows()))
        .filter_map(|i| {
            let line = screen.row_text(i);
            let text = line.trim_start().strip_prefix('⎿')?.trim();
            Some((screen.screen_start() + i as u64, text.to_owned()))
        })
        .collect()
}

fn reported_effort(line: &str) -> Option<&'static str> {
    let value = line.strip_prefix("Set effort level to ").or_else(|| line.strip_prefix("Effort level: "))?;
    let word = value.split(|c: char| !c.is_ascii_alphabetic()).next()?;
    super::commands::EFFORT.into_iter().chain(["auto"]).find(|s| word.eq_ignore_ascii_case(s))
}

fn reported_model(line: &str) -> Option<String> {
    let value = line.strip_prefix("Set model to ")?;
    let value = value.split_once(" (").map_or(value, |(name, _)| name).trim();
    (!value.is_empty()
        && value.len() <= 70
        && value.chars().all(|c| c.is_ascii_alphanumeric() || " .-_:[]".contains(c)))
    .then(|| value.to_owned())
}

impl Chat {
    pub(super) fn observe_cli(&mut self, info: &ChatInfo, screen: &Screen, now: Instant) -> bool {
        let report = &mut self.cli_report;
        let mut changed = false;
        let usage = info.usage_offset().map(|at| (info.generation, at));
        if let Some((_, offset)) = usage.filter(|u| Some(*u) != report.transcript) {
            report.transcript = usage;
            if report.floor.is_none_or(|floor| offset >= floor) {
                changed |= report.model.take().is_some();
                report.floor = None;
            }
        }
        if report.tracking && self.output_epoch != report.epoch {
            let mut newest_model = None;
            let mut newest_effort = None;
            for (_, line) in local_reports(screen).into_iter().filter(|s| !report.baseline.contains(s)) {
                if let Some(model) = reported_model(&line) {
                    newest_model = Some(model);
                    newest_effort = None;
                }
                newest_effort = reported_effort(&line).or(newest_effort);
            }
            if let Some(model) = newest_model {
                if report.model.as_ref() != Some(&model) {
                    changed = true;
                    report.model = Some(model);
                    changed |= report.effort.take().is_some();
                }
                report.confirmed |= report.expects == Some(Choice::Model);
            }
            if let Some(effort) = newest_effort {
                changed |= report.effort != Some(effort);
                report.effort = Some(effort);
                report.confirmed |= report.expects == Some(Choice::Effort);
            }
            if !self.identity_unverified
                && !screen.modes().alt_screen
                && !crate::attention::screen_permission_prompt(screen)
                && clean_prompt(screen)
                && self.last_output.is_some_and(|at| now.saturating_duration_since(at) >= QUIET)
            {
                report.tracking = false;
                self.slash = false;
                self.suppressed = None;
                self.drawer_focus &= report.touched_terminal;
                if let Some(choice) = report.expects.take().filter(|_| !report.confirmed) {
                    self.drawer = Drawer::Manual;
                    self.error = Some(match choice {
                        Choice::Model => "Claude Code did not report a model change. Check the terminal.",
                        Choice::Effort => "Claude Code did not report an effort change. Check the terminal.",
                    });
                    self.retry = None;
                }
                changed = true;
            }
        }
        changed
    }

    pub(super) fn locked(&self) -> bool {
        self.queue.is_some() || self.image_wait.is_some()
    }

    pub(super) fn interrupt_submission(&mut self) {
        if self.image_wait.take().is_some() {
            self.error = Some(IMAGE_INTERRUPTED);
        } else if self.queue.take().is_some() {
            self.error = Some("Queue cancelled by terminal input or layout change. Draft kept.");
        }
        self.retry = None;
        if self.slash {
            self.cli_report.touched_terminal = true;
        }
    }

    pub(super) fn stop_waiting(&mut self) {
        self.interrupt_submission();
        if self.error != Some(IMAGE_INTERRUPTED) {
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
        self.recall = None;
    }
}

fn blocked(tab: &Tab, now: Instant, intent: Intent) -> Option<&'static str> {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let (gate, reason) = chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now);
    let modes = tab.term.screen.modes();
    if let Some(reason) = reason {
        return Some(reason.label());
    }
    if modes.alt_screen {
        return Some(ALTERNATE);
    }
    if !chat.demo && !modes.bracketed_paste {
        return Some(NO_PASTE);
    }
    if pane.model.error.is_some() && !chat.demo {
        return Some(Reason::Identity.label());
    }
    match intent {
        Intent::Draft if gate != Gate::Ready => Some(CHECK),
        Intent::Override | Intent::Now if pane.model.chat.pending.is_some() => Some(Reason::Tool.label()),
        Intent::Override if gate == Gate::Running => Some("A turn is running. Use Queue to send after it ends."),
        Intent::Now if !chat.demo && !clean_prompt(&tab.term.screen) => {
            Some("The CLI input line is not visibly empty. Wait or use the terminal.")
        }
        Intent::Slash if pane.model.chat.pending.is_some() => Some(Reason::Tool.label()),
        Intent::Slash
            if gate == Gate::Running || pane.model.chat.latest.is_some_and(|(_, p)| p == ChatPhase::Active) =>
        {
            Some(BUSY_CMD)
        }
        Intent::Slash
            if !chat.demo
                && !idle_slash_prompt(
                    &tab.term.screen,
                    chat.last_output.is_some_and(|at| now.saturating_duration_since(at) >= QUIET),
                ) =>
        {
            Some("No quiet, empty Claude Code prompt is visible. Check the terminal.")
        }
        _ => None,
    }
}

impl AppState {
    pub(super) fn chat_send(tab: &mut Tab, now: Instant, force: bool) {
        let intent = if !force {
            Intent::Draft
        } else if tab.pane.as_ref().unwrap().chat.queue.is_some() {
            Intent::Now
        } else {
            Intent::Override
        };
        let error = blocked(tab, now, intent);
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.image_wait.is_some() || chat.queue.is_some() && !force {
            return;
        }
        if chat.editor.text.trim().is_empty() && chat.chips.is_empty() {
            return;
        }
        let Some(bytes) = send_bytes(&chat.chips, &chat.editor.text) else {
            chat.refused(BAD_TEXT, None);
            return;
        };
        if !force
            && error == Some(CHECK)
            && chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now) == (Gate::Running, None)
        {
            chat.queue =
                Some(Queue { generation: pane.model.chat.generation, latest: pane.model.chat.latest.map(|x| x.0) });
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
            if !chat.demo && !clean_prompt(&tab.term.screen) {
                chat.queue = None;
                chat.refused(NO_IMAGE_PROMPT, None);
                return;
            }
            // The guard is armed before the first write but no Enter is present in that write.
            let wait = ImageWait {
                due: now + IMAGE_TIMEOUT,
                epoch: chat.output_epoch,
                generation: pane.model.chat.generation,
                prompt: pane.model.chat.prompt,
                seq,
                bytes,
                demo_text,
            };
            chat.queue = None;
            chat.error = None;
            chat.retry = None;
            chat.image_wait = Some(wait);
            if let Err(error) = write(tab, IMAGE_KEY, "") {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.image_wait = None;
                chat.refused(error, None);
            }
        } else {
            finish(tab, &bytes, &demo_text);
        }
    }

    pub(super) fn advance_submission(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
        let pane = tab.pane.as_ref().unwrap();
        let chat = &pane.chat;
        if let Some(wait) = &chat.image_wait {
            let modes = tab.term.screen.modes();
            let veto = pane.model.chat.generation != wait.generation
                || pane.model.chat.prompt != wait.prompt
                || pane.model.error.is_some()
                || chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now).1.is_some()
                || pane.model.chat.pending.is_some()
                || modes.alt_screen
                || !chat.demo && (!modes.bracketed_paste || clipboard::sequence() != Some(wait.seq));
            let result = if veto {
                Some(Err(IMAGE_INTERRUPTED))
            } else if now >= wait.due {
                Some(Err(IMAGE_TIMEOUT_NOTICE))
            } else if may_write
                && chat.output_epoch != wait.epoch
                && prompt(&tab.term.screen).as_deref().is_some_and(is_image_marker)
            {
                Some(Ok(()))
            } else {
                None
            };
            if let Some(result) = result {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                let wait = chat.image_wait.take().unwrap();
                if let Err(error) = result {
                    chat.refused(error, None);
                    chat.open_drawer();
                    chat.drawer_focus = false;
                } else {
                    let demo_text = if wait.demo_text.is_empty() { "[Picture]" } else { &wait.demo_text };
                    finish(tab, &wait.bytes, demo_text);
                }
                return true;
            }
        }
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
            if may_write
                && queue.completed(&pane.model.chat)
                && blocked(tab, now, Intent::Draft).is_none()
                && (chat.demo || clean_prompt(&tab.term.screen))
            {
                tab.pane.as_mut().unwrap().chat.queue = None;
                Self::chat_send(tab, now, false);
                return true;
            }
        }
        if let Some(intent) = chat.retry {
            let error = blocked(tab, now, intent);
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            let changed = chat.error != error;
            chat.error = error;
            if error.is_none() {
                chat.retry = None;
            }
            return changed;
        }
        if chat.error == Some(NO_IMAGE_PROMPT) && clean_prompt(&tab.term.screen) {
            tab.pane.as_mut().unwrap().chat.error = None;
            return true;
        }
        false
    }

    pub(super) fn submit_slash(tab: &mut Tab, now: Instant, line: &str, from_editor: bool) {
        let error = blocked(tab, now, Intent::Slash);
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        if chat.locked() {
            chat.refused("Cancel the queued or pending send before running a command.", None);
            return;
        }
        if from_editor && !chat.chips.is_empty() {
            chat.refused(REMOVE_ATTACHMENTS, None);
            return;
        }
        let Some(bytes) = slash_bytes(line) else {
            chat.refused(BAD_COMMAND, None);
            return;
        };
        if let Some(error) = error {
            chat.refused(error, Some(Intent::Slash));
            return;
        }
        chat.cli_report.baseline = local_reports(&tab.term.screen);
        chat.cli_report.epoch = chat.output_epoch;
        chat.cli_report.tracking = !chat.demo;
        chat.cli_report.expects = match line.split_once(' ') {
            Some(("/model", arg)) if !arg.trim().is_empty() => Some(Choice::Model),
            Some(("/effort", arg)) if !arg.trim().is_empty() => Some(Choice::Effort),
            _ => None,
        };
        chat.cli_report.confirmed = false;
        chat.cli_report.touched_terminal = false;
        if !chat.demo {
            pane.note_terminal_input();
            pane.chat.cli_report.floor = pane.chat.input_floor;
        }
        if let Err(error) = write(tab, &bytes, line) {
            tab.pane.as_mut().unwrap().chat.refused(error, None);
            return;
        }
        let pane = tab.pane.as_mut().unwrap();
        let chat = &mut pane.chat;
        chat.commands.open = false;
        if from_editor {
            chat.submitted();
        } else {
            chat.error = None;
            chat.retry = None;
        }
        chat.slash = !chat.demo;
        chat.suppressed = None;
        chat.react(chat.slash.then_some(Reason::Slash));
        // Only inline model/effort settings use an automatically dismissible drawer. Leave
        // results from /context and the CLI's own interactive pickers available for inspection.
        if chat.demo || !line.starts_with("/model ") && !line.starts_with("/effort ") {
            chat.drawer = Drawer::Manual;
        }
    }
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
    // Literal typed command, not a paste placeholder. No terminal control besides our own Enter.
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\r');
    Some(bytes)
}

pub(super) fn send_bytes(chips: &[Chip], text: &str) -> Option<Vec<u8>> {
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) {
        return None;
    }
    let text = text.replace(['\n', '\t'], " ");
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

fn finish(tab: &mut Tab, paste: &[u8], demo_text: &str) {
    let pane = tab.pane.as_mut().unwrap();
    if !pane.chat.demo {
        pane.note_terminal_input();
    }
    let result = if paste.is_empty() { Ok(()) } else { write(tab, paste, "") };
    if let Err(error) = result.and_then(|()| write(tab, b"\r", demo_text)) {
        tab.pane.as_mut().unwrap().chat.refused(error, None);
        return;
    }
    let pane = tab.pane.as_mut().unwrap();
    if pane.chat.demo {
        pane.chat.invalidate(&pane.model.chat);
    }
    pane.chat.submitted();
}

pub(super) fn write(tab: &mut Tab, bytes: &[u8], demo_text: &str) -> Result<(), &'static str> {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    #[cfg(test)]
    chat.writes.push(bytes.to_vec());
    if chat.demo {
        if bytes == IMAGE_KEY {
            tab.term.feed("\r\n❯ [Image #1]".as_bytes());
            chat.output_epoch = chat.output_epoch.wrapping_add(1);
        } else if !bytes.starts_with(b"\x1b[200~") {
            let demo_text: String = demo_text.chars().filter(|c| !c.is_control() || *c == '\n').collect();
            tab.term.feed(
                format!("\r\n> {}\r\n[Demo input — no process started]\r\n", demo_text.replace('\n', "\r\n"))
                    .as_bytes(),
            );
            chat.demo_progress = true;
        }
    } else {
        let pty =
            tab.term.pty.as_ref().filter(|p| tab.term.ended.is_none() && p.running()).ok_or("The session ended.")?;
        pty.write(bytes).map_err(|_| "The terminal write failed. Check the drawer before retrying.")?;
    }
    tab.term.scroll = 0;
    tab.term.selection = None;
    Ok(())
}

pub(super) fn image_deadline(chat: &Chat) -> Option<Instant> {
    chat.image_wait.as_ref().map(|wait| wait.due)
}

#[cfg(test)]
mod tests;
