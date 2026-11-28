//! Messages waiting in Claude Code's queue. A message sent while a turn runs goes into Claude
//! Code's input, and Claude Code keeps it queued ("Queued · ctrl+x ctrl+s to send now" under it)
//! until it hands it to the running turn at its next step (a `queued_command` record, a prompt
//! "during a turn"), or sends it as the next prompt once the turn ends or is stopped (Esc). Until
//! the transcript shows it, Chat shows it as a quiet bubble at the end of the conversation, in the
//! order it was sent.
//!
//! **Send now** is Claude Code's own: its `chat:sendNow` key, Ctrl+X Ctrl+S, which it names on
//! the screen under its queued messages (Claude Code 2.1.332, `recorder compat` script
//! `queued-now`). With its input empty it hands every queued message to the running turn at once,
//! moving a running command to the background if one holds the turn (the call's result says so),
//! and does not stop the turn. Chat writes those two keys and nothing else, and only while the
//! screen names them, the input reads empty (with text on it the same key would send that text
//! too) and nothing else is on its way: no message is typed twice, and none is taken off the
//! queue by Chat.
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ItemId, ItemKind};

use super::submission::{self, screen};
use super::{Chat, Gate};
use crate::state::AppState;
use crate::tabs::Tab;
use crate::view::model::Model;

/// Claude Code's `chat:sendNow`: Ctrl+X, then Ctrl+S.
pub(super) const SEND_NOW: &[u8] = b"\x18\x13";
/// How long Send now stays dimmed after its keys went, waiting for the transcript.
const SENDING: Duration = Duration::from_secs(5);
pub(super) const NOT_OFFERED: &str = "Claude Code is not offering to send its queued messages now. Check the terminal.";
pub(super) const INPUT_BUSY: &str =
    "Claude Code's input holds text, which its Send now would send too. Send or clear it in the terminal.";
pub(super) const NOT_NOW: &str = "Send now waits for what is on its way to the terminal. Try again in a moment.";

/// A message in Claude Code's queue, as Chat sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Queued {
    /// The draft's text, as the bubble shows it.
    pub text: String,
    pub picture: bool,
    /// The session it went to, and the transcript's length when its Enter went: only a record
    /// written after it can be this message.
    generation: u64,
    floor: u64,
}

impl Queued {
    pub(super) fn new(text: String, picture: bool, generation: u64, floor: u64) -> Self {
        Self { text, picture, generation, floor }
    }
}

/// What the conversation shows for the queue: each message, and whether Send now's keys went.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::view) struct Shown {
    pub messages: Vec<(String, bool)>,
    pub sending: bool,
}

/// Text compared without its spacing: the transcript keeps a message's lines, the screen and a
/// paste may not.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The transcript's prompt is this message: its text (after any attached paths), or a picture
/// with no text.
fn same(prompt: &str, pictures: usize, queued: &Queued) -> bool {
    let (prompt, text) = (squash(prompt), squash(&queued.text));
    if text.is_empty() {
        return queued.picture && pictures > 0;
    }
    if prompt == text {
        return true;
    }
    // Attached files go before the text as paths (quoted when they hold spaces), a picture as
    // its marker: only such a beginning counts.
    let Some(before) = prompt.strip_suffix(&format!(" {text}")) else { return false };
    let mut chars = before.chars();
    let (first, second) = (chars.next(), chars.next());
    matches!(first, Some('"' | '\'' | '/' | '\\' | '~' | '.' | '@' | '['))
        || first.is_some_and(|c| c.is_ascii_alphabetic()) && second == Some(':')
}

impl Chat {
    /// A message went into Claude Code's input while a turn ran and its Enter went.
    pub(super) fn note_queued(&mut self, text: String, picture: bool, generation: u64, floor: u64) {
        if text.trim().is_empty() && !picture {
            return;
        }
        self.queued.push(Queued::new(text, picture, generation, floor));
    }

    /// Takes off the bubbles of messages the transcript now shows (handed to the turn, or sent as
    /// a prompt), each by the first prompt after it that reads the same, in order. At an idle
    /// input Claude Code's queue is empty: what is left goes too. True when something changed.
    pub(super) fn reconcile_queued(&mut self, model: &Model, idle: bool, now: Instant) -> bool {
        let before = (self.queued.len(), self.send_now.is_some());
        if self.send_now.is_some_and(|at| now.saturating_duration_since(at) >= SENDING) {
            self.send_now = None;
        }
        if self.queued.is_empty() {
            self.send_now = None;
            return before != (self.queued.len(), self.send_now.is_some());
        }
        let generation = model.chat.generation;
        self.queued.retain(|q| q.generation == generation);
        let mut claimed: Vec<ItemId> = Vec::new();
        let mut kept = Vec::with_capacity(self.queued.len());
        for queued in std::mem::take(&mut self.queued) {
            let from = ItemId { offset: queued.floor, block: 0 };
            let found = model.items.range(from..).find_map(|(id, item)| match &item.kind {
                ItemKind::Prompt { text, pictures, synthetic: false, .. }
                    if !claimed.contains(id) && same(text, pictures.len(), &queued) =>
                {
                    Some(*id)
                }
                _ => None,
            });
            match found {
                Some(id) => claimed.push(id),
                None => kept.push(queued),
            }
        }
        self.queued = if idle { Vec::new() } else { kept };
        if self.queued.is_empty() {
            self.send_now = None;
        }
        before != (self.queued.len(), self.send_now.is_some())
    }

    /// The bubbles the conversation shows.
    pub(in crate::view) fn queued_shown(&self) -> Shown {
        Shown {
            messages: self.queued.iter().map(|q| (q.text.clone(), q.picture)).collect(),
            sending: self.send_now.is_some(),
        }
    }

    /// The session changed or its program ended: what was queued is not shown any more. True when
    /// there was something.
    pub(super) fn forget_queued(&mut self) -> bool {
        let had = !self.queued.is_empty() || self.send_now.is_some();
        self.queued.clear();
        self.send_now = None;
        had
    }
}

/// Why Send now cannot go; `None` when it can.
pub(super) fn send_now_blocked(tab: &Tab, now: Instant) -> Option<&'static str> {
    let pane = tab.pane.as_ref()?;
    let chat = &pane.chat;
    let term = &tab.term;
    if chat.queued.is_empty() || chat.send_now.is_some() {
        return Some(NOT_NOW);
    }
    // Something of Chat's is on its way: keys of a message, or a command it typed (its own, as a
    // handover's `/rename`) that is followed until Claude Code takes it.
    if chat.waiting().is_some() || chat.enter_only() || chat.image_wait.is_some() || chat.command_in_flight() {
        return Some(NOT_NOW);
    }
    let (gate, reason) = chat.decision(&pane.model.chat, term, &tab.program_title, now);
    if let Some(reason) = reason {
        return Some(reason.label());
    }
    if term.screen.modes().alt_screen || gate != Gate::Running {
        return Some(NOT_OFFERED);
    }
    if !screen::send_now_offered(&term.screen) {
        return Some(NOT_OFFERED);
    }
    if !screen::clean_prompt(&term.screen) {
        return Some(INPUT_BUSY);
    }
    None
}

impl AppState {
    /// Send now on a queued message: Claude Code's own key for it, once, under the checks above.
    pub(super) fn chat_send_queued(tab: &mut Tab, now: Instant) {
        if let Some(error) = send_now_blocked(tab, now) {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.error = Some(error);
            chat.send_now_refused = Some(error);
            return;
        }
        tab.pane.as_mut().unwrap().note_composer_input_into_turn(true);
        match submission::write(tab, SEND_NOW, "") {
            Ok(()) => {
                let chat = &mut tab.pane.as_mut().unwrap().chat;
                chat.send_now = Some(now);
                // The refusal an earlier click got is over.
                if chat.send_now_refused.take().is_some_and(|refused| chat.error == Some(refused)) {
                    chat.error = None;
                }
            }
            Err(error) => tab.pane.as_mut().unwrap().chat.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests;
