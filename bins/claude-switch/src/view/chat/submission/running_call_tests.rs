//! Refusal kind 1 (notes/refusals.md): a send refused only because the transcript holds a running
//! call, while the screen shows the turn working over an empty input. The screen tells it apart
//! from a permission question, which has no input box.
use super::ready_tests::{Live, TYPED_WHILE_BUSY};
use super::*;
use crate::view::chat::question::screens::recorded::{PERM_BASH, PLAIN_WORKING_TYPED};

const SECOND: Duration = Duration::from_secs(1);

/// A command running for a while, as the reports showed it (cursor on the empty input).
const RUNNING_CALL: &str = "cursor 7 2
@size 100 12
● Bash(sleep 60)
  ⎿  Running… (8s · timeout 10m)
     (ctrl+b to run in background)

✻ Working… (12s · ↓ 40 tokens)

────────────────────────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on · esc to interrupt
";

/// The transcript's turn runs a call first seen longer ago than the silence that opens the
/// terminal.
fn pending_call(screen: &str) -> Live {
    let mut f = Live::new();
    f.turn();
    let pane = f.tab().pane.as_mut().unwrap();
    let call = pane.model.chat.prompt.map(|p| p + 5);
    pane.model.chat.pending = call;
    let since = f.now - 6 * SECOND;
    f.chat().pending = call.map(|call| (call, since));
    f.show(screen);
    f.pass(SECOND);
    f
}

#[test]
fn a_running_call_over_an_empty_input_takes_a_message_into_the_turn() {
    let mut f = pending_call(RUNNING_CALL);
    assert_eq!(f.pass(SECOND / 10), (Gate::Running, None), "a slow tool, not a question");
    let now = f.now;
    assert_eq!(blocked(f.tab(), now, Intent::Now), None);
    f.send(TYPED_WHILE_BUSY);
    assert_eq!(f.writes(), [format!("\x1b[200~{TYPED_WHILE_BUSY}\x1b[201~").into_bytes()], "the paste only");
    // Claude Code shows it on the input (recorded from 2.1.332).
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()], "Enter once the input shows it");
}

#[test]
fn a_command_or_the_override_during_a_running_call_says_a_turn_runs() {
    let mut f = pending_call(RUNNING_CALL);
    let now = f.now;
    assert_eq!(blocked(f.tab(), now, Intent::Slash), Some(BUSY_CMD));
    assert_eq!(blocked(f.tab(), now, Intent::Override), Some("A turn is running. Use Queue to send after it ends."));
    // A stop's take-back still waits for the call (unchanged).
    assert_eq!(blocked(f.tab(), now, Intent::Interrupted), Some(Reason::Tool.label()));
}

#[test]
fn a_permission_question_with_the_same_transcript_is_still_a_question() {
    let mut f = pending_call(PERM_BASH);
    let (_, reason) = f.pass(SECOND);
    assert!(matches!(reason, Some(Reason::Permission | Reason::Approval)), "{reason:?}");
    f.send(TYPED_WHILE_BUSY);
    for _ in 0..3 {
        f.pass(SECOND);
    }
    assert!(f.writes().is_empty(), "nothing typed into the question");
    let now = f.now;
    assert!(blocked(f.tab(), now, Intent::Slash).is_some_and(|e| e != BUSY_CMD));
}
