//! Messages in Claude Code's queue: shown until the transcript has them, Send now as Claude Code's
//! own key.
use std::time::Duration;

use claude_switch_core::transcript::{Item, ItemId, ItemKind};

use super::*;
use crate::view::chat::question::screens::recorded::{
    PERM_BASH, PLAIN_IDLE, PLAIN_QUEUED, PLAIN_WORKING_TYPED, QUEUED_NOW_SENT, QUEUED_TWO, WORKING_TOOL, recorded,
};
use crate::view::chat::submission::ready_tests::{Live, TYPED_WHILE_BUSY};

const SECOND: Duration = Duration::from_secs(1);

fn paste(text: &str) -> Vec<Vec<u8>> {
    vec![format!("\x1b[200~{text}\x1b[201~").into_bytes()]
}

/// A turn that runs a call the screen shows running, pending for a while.
fn running_call() -> Live {
    let mut f = Live::new();
    f.turn();
    let pane = f.tab().pane.as_mut().unwrap();
    let call = pane.model.chat.prompt.map(|p| p + 5);
    pane.model.chat.pending = call;
    // Pending for a while, as a real session's records note it (`Chat::observe`).
    let since = f.now;
    f.chat().pending = call.map(|call| (call, since));
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.pass(SECOND);
    f
}

/// Sends `text` into the running turn as Chat does: pasted, then Enter once the input shows it
/// (the input is drawn as `shown` reads it).
fn queue(f: &mut Live, text: &str, shown: &str) {
    f.send(text);
    assert_eq!(f.writes(), paste(text), "pasted into Claude Code's empty input");
    f.show(shown);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()], "Enter once the input shows it");
}

/// The transcript records a message as Claude Code hands it in.
fn handed_in(f: &mut Live, offset: u64, text: &str) {
    let kind = ItemKind::Prompt { text: text.into(), pictures: Vec::new(), synthetic: false, queued: true };
    f.tab().pane.as_mut().unwrap().model.items.insert(ItemId { offset, block: 0 }, Item { timestamp: None, kind });
}

fn shown(f: &mut Live) -> Vec<String> {
    f.chat().queued_shown().messages.into_iter().map(|(text, _)| text).collect()
}

#[test]
fn claude_code_names_its_send_now_key_under_its_queued_messages() {
    assert!(screen::send_now_offered(&recorded(QUEUED_TWO)));
    assert!(screen::send_now_offered(&recorded(PLAIN_QUEUED)), "also drawn without Unicode");
    assert!(screen::clean_prompt(&recorded(QUEUED_TWO)), "the input reads empty under the queue");
    assert!(!screen::send_now_offered(&recorded(QUEUED_NOW_SENT)), "handed in: nothing is queued");
    assert!(!screen::send_now_offered(&recorded(WORKING_TOOL)));
}

/// The owner's request (2026-11-28): Enter during a turn queues the message, and Chat shows it
/// waiting for its turn until Claude Code hands it in. A call the screen shows running no longer
/// holds the draft in the box until the turn ends.
#[test]
fn a_message_sent_while_a_call_runs_waits_in_claude_codes_queue_until_the_transcript_has_it() {
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    let chat = f.chat();
    assert!(chat.editor.text.is_empty() && !chat.locked() && chat.error.is_none(), "the box is free");
    assert_eq!(shown(&mut f), [TYPED_WHILE_BUSY]);
    // Claude Code shows it queued: the bubble stays.
    f.show(PLAIN_QUEUED);
    f.pass(SECOND);
    assert_eq!(shown(&mut f), [TYPED_WHILE_BUSY]);
    // It hands the message to the turn (a `queued_command` record): the transcript shows it, and
    // the bubble goes. Nothing more was typed.
    handed_in(&mut f, 900, TYPED_WHILE_BUSY);
    f.pass(SECOND / 10);
    assert!(shown(&mut f).is_empty());
    assert!(f.writes().is_empty());
}

#[test]
fn a_call_just_written_still_holds_the_draft() {
    // A permission prompt is drawn as soon as a call is written: within that moment the draft is
    // held in the box, as before.
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    let pane = f.tab().pane.as_mut().unwrap();
    let call = pane.model.chat.prompt.map(|p| p + 5);
    pane.model.chat.pending = call;
    let since = f.now;
    f.chat().pending = call.map(|call| (call, since));
    f.pass(SECOND / 10);
    f.send(TYPED_WHILE_BUSY);
    assert!(f.writes().is_empty());
    assert!(f.chat().queue.is_some() && shown(&mut f).is_empty());
}

#[test]
fn several_queued_messages_keep_their_order_and_each_goes_when_the_transcript_has_it() {
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    f.show(PLAIN_QUEUED);
    f.pass(SECOND);
    // The second one, as the screen shows it on the input.
    let second = "hello while busy";
    f.send(second);
    assert_eq!(f.writes(), paste(second));
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    assert_eq!(shown(&mut f), [TYPED_WHILE_BUSY, second], "in the order they were sent");
    // The transcript has one of the two (the same text): the first bubble goes, the second stays.
    f.show(QUEUED_TWO);
    handed_in(&mut f, 900, TYPED_WHILE_BUSY);
    f.pass(SECOND / 10);
    assert_eq!(shown(&mut f), [second]);
    handed_in(&mut f, 950, second);
    f.pass(SECOND / 10);
    assert!(shown(&mut f).is_empty());
}

#[test]
fn a_message_claude_code_sends_as_the_next_prompt_goes_too() {
    // After Esc (Chat's Stop) or the turn's end Claude Code sends its queue as the next turn
    // (`queued-esc`): an ordinary prompt.
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    let kind =
        ItemKind::Prompt { text: TYPED_WHILE_BUSY.into(), pictures: Vec::new(), synthetic: false, queued: false };
    f.tab().pane.as_mut().unwrap().model.items.insert(ItemId { offset: 990, block: 0 }, Item { timestamp: None, kind });
    f.pass(SECOND / 10);
    assert!(shown(&mut f).is_empty());
}

#[test]
fn at_an_idle_input_nothing_is_left_in_claude_codes_queue() {
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.pass(SECOND);
    assert!(shown(&mut f).is_empty());
}

#[test]
fn another_session_forgets_the_queue() {
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    f.tab().pane.as_mut().unwrap().model.chat.generation += 1;
    f.pass(SECOND / 10);
    assert!(shown(&mut f).is_empty());
}

#[test]
fn send_now_is_claude_codes_own_key_written_once_and_only_while_it_is_offered() {
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    // The screen does not name the key (the queue is not drawn yet): nothing is written.
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    let now = f.now;
    AppState::chat_send_queued(f.tab(), now);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().error, Some(NOT_OFFERED));
    // A permission prompt: its card answers first.
    f.show(PERM_BASH);
    f.pass(SECOND);
    let now = f.now;
    AppState::chat_send_queued(f.tab(), now);
    assert!(f.writes().is_empty());
    // Claude Code shows the queue and names Ctrl+X Ctrl+S: those two keys, and nothing else.
    f.show(QUEUED_TWO);
    f.pass(SECOND);
    let now = f.now;
    AppState::chat_send_queued(f.tab(), now);
    assert_eq!(f.writes(), [SEND_NOW.to_vec()]);
    assert!(f.chat().error.is_none() && f.chat().queued_shown().sending);
    // A second click while they are on their way writes nothing.
    AppState::chat_send_queued(f.tab(), now);
    assert!(f.writes().is_empty());
    // Handed in: the bubble goes, and with it the dimmed Send now.
    f.show(QUEUED_NOW_SENT);
    handed_in(&mut f, 900, TYPED_WHILE_BUSY);
    f.pass(SECOND / 10);
    assert_eq!(f.chat().queued_shown(), Shown::default());
}

#[test]
fn send_now_never_sends_text_on_claude_codes_input() {
    // With text on its input Claude Code's Send now would send that text too.
    let mut f = running_call();
    queue(&mut f, TYPED_WHILE_BUSY, PLAIN_WORKING_TYPED);
    let mut screen = recorded(QUEUED_TWO);
    assert!(screen::send_now_offered(&screen));
    // The user types in the terminal: the input holds text under the queue.
    screen.feed(b"typed in the terminal");
    let pane = f.tab();
    pane.term.screen = screen;
    let now = f.now;
    f.chat().output(now, true, false);
    f.pass(SECOND);
    AppState::chat_send_queued(f.tab(), now + SECOND);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().error, Some(INPUT_BUSY));
}
