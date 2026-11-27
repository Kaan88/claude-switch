//! Claude Code's list of blocked calls driven from its card, on a tab gated like a real one and fed
//! the screens recorded from Claude Code 2.1.332 (`blocked_*`).
use super::*;
use crate::view::chat::question::screens::recorded::{
    BLOCKED_IDLE, BLOCKED_ONE, BLOCKED_ONE_APPROVED, BLOCKED_ONE_CLOSED, BLOCKED_ONE_RETRY, BLOCKED_RETRY_STARTED,
    BLOCKED_TWO, BLOCKED_TWO_APPROVE, BLOCKED_TWO_DOWN, BLOCKED_TWO_RETRY, BLOCKED_TWO_SECOND, PERM_TAB_ALLOW,
    recorded,
};
use crate::view::chat::submission::ready_tests::{Live, downs};
use crate::view::chat::{Gate, QUIET, Reason};

const TICK: Duration = Duration::from_millis(100);
const ENTER: &[u8] = b"\r";

fn list(f: &mut Live) -> Shown {
    f.card().blocked.clone().expect("the list's card")
}

#[test]
fn a_blocked_call_is_approved_through_the_clis_own_list_one_checked_key_at_a_time() {
    let mut f = Live::new();
    f.show(BLOCKED_ONE);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Menu)));
    assert_eq!(f.chat().drawer, Drawer::Closed);
    assert!(f.card().menu.is_none(), "never the card of plain choices");
    let shown = list(&mut f);
    assert_eq!(shown.note, "Commands auto mode recently blocked. Approving adds no rule.");
    assert_eq!(shown.detail, ["mkdir probe_blocked_two"]);
    assert_eq!((shown.highlight, shown.esc.as_str()), (0, "close"));
    // A send says to choose first.
    f.send("later");
    assert!(f.writes().is_empty() && f.chat().error == Some(Reason::Menu.label()));

    // Nothing for a list the card has not painted, nor in the moment it appears.
    assert!(f.click(Hit::Approve));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(LIST_CHANGED)));
    let hits = f.paint(1000.0, 900.0);
    assert!(hits.contains(&Hit::Approve) && hits.contains(&Hit::Retry) && hits.contains(&Hit::Escape));
    assert!(!hits.contains(&Hit::Option(0)), "the highlighted call is the one the buttons act on");
    assert!(f.click(Hit::Approve));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(prompt::NOT_SEEN)));
    f.pass(prompt::SEEN);
    // Enter marks the highlighted call; the step ends once the screen shows the mark.
    assert!(f.click(Hit::Approve));
    assert_eq!(f.writes(), [ENTER.to_vec()]);
    assert!(f.card().step.is_some());
    f.show(BLOCKED_ONE_APPROVED);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().error.is_none());
    let shown = list(&mut f);
    assert_eq!(shown.marks, [Mark::Approve]);
    assert_eq!(shown.pending.as_deref(), Some("Esc will approve 1 blocked call"));
    assert_eq!(shown.esc_label(), "Approve 1 blocked");
    // Now Esc applies the mark: only for the list the card painted, and not at once.
    let hits = f.paint(1000.0, 900.0);
    assert!(hits.contains(&Hit::Approve) && !hits.contains(&Hit::Retry), "an approval can only be unmarked");
    assert!(f.click(Hit::Escape));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(prompt::NOT_SEEN)));
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Escape));
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    // The list closes ("Told Claude it may retry …") and the input is idle again.
    f.show(BLOCKED_ONE_CLOSED);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().blocked.is_none() && f.card().error.is_none());
    assert_eq!(f.pass(QUIET), (Gate::Ready, None));
    assert!(f.writes().is_empty());
}

#[test]
fn another_call_is_chosen_with_the_arrows_alone_and_marked_only_once_its_command_showed() {
    let mut f = Live::new();
    f.show(BLOCKED_TWO);
    f.pass(QUIET);
    assert_eq!(list(&mut f).detail, ["mkdir probe_blocked_b"]);
    let hits = f.paint(1000.0, 900.0);
    assert!(hits.contains(&Hit::Option(1)) && !hits.contains(&Hit::Option(0)));
    // A click on the other call moves the highlight there: no Enter, now or later.
    assert!(f.click(Hit::Option(1)));
    assert_eq!(f.writes(), [downs(1)]);
    f.show(BLOCKED_TWO_DOWN);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.writes().is_empty());
    let shown = list(&mut f);
    assert_eq!((shown.highlight, shown.detail.as_slice()), (1, &["mkdir probe_blocked_a".to_owned()][..]));
    // Its command has only just been painted: not yet.
    f.paint(1000.0, 900.0);
    assert!(f.click(Hit::Approve));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(prompt::NOT_SEEN)));
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Approve));
    assert_eq!(f.writes(), [ENTER.to_vec()]);
    f.show(BLOCKED_TWO_SECOND);
    f.pass(TICK);
    assert_eq!(list(&mut f).marks, [Mark::None, Mark::Approve]);
    assert!(f.card().step.is_none());
}

#[test]
fn retry_marks_with_r_and_a_second_r_makes_it_an_approval() {
    let mut f = Live::new();
    f.show(BLOCKED_TWO);
    f.pass(QUIET);
    f.paint(1000.0, 900.0);
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Retry));
    assert_eq!(f.writes(), [b"r".to_vec()]);
    f.show(BLOCKED_TWO_RETRY);
    f.pass(TICK);
    let shown = list(&mut f);
    assert_eq!(shown.marks, [Mark::Retry, Mark::None]);
    assert_eq!(shown.esc_label(), "Retry 1 blocked");
    // What the CLI was recorded doing with a call marked to retry: `r` again, nothing else.
    let hits = f.paint(1000.0, 900.0);
    assert!(hits.contains(&Hit::Retry) && !hits.contains(&Hit::Approve));
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Approve));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(NOT_OFFERED)));
    assert!(f.click(Hit::Retry));
    assert_eq!(f.writes(), [b"r".to_vec()]);
    f.show(BLOCKED_TWO_APPROVE);
    f.pass(TICK);
    assert_eq!(list(&mut f).marks, [Mark::Approve, Mark::None]);

    // Esc on a retry closes the list and Claude Code starts the turn at once.
    let mut f = Live::new();
    f.show(BLOCKED_ONE_RETRY);
    f.pass(QUIET);
    f.paint(1000.0, 900.0);
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Escape));
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.turn();
    f.show(BLOCKED_RETRY_STARTED);
    assert_eq!(f.pass(TICK), (Gate::Running, None));
    assert!(f.card().step.is_none() && f.card().blocked.is_none() && f.writes().is_empty());
}

#[test]
fn a_list_that_changed_since_the_card_painted_it_gets_no_key() {
    let mut f = Live::new();
    f.show(BLOCKED_TWO);
    f.pass(QUIET);
    f.paint(1000.0, 900.0);
    f.pass(prompt::SEEN);
    // The highlight moved in the terminal: Approve would now mark the other call.
    f.tab().term.screen = recorded(BLOCKED_TWO_DOWN);
    assert!(f.click(Hit::Approve));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(LIST_CHANGED)));
    // Marked in the terminal since: Esc would now apply it.
    let mut f = Live::new();
    f.show(BLOCKED_TWO);
    f.pass(QUIET);
    f.paint(1000.0, 900.0);
    f.pass(prompt::SEEN);
    f.tab().term.screen = recorded(BLOCKED_TWO_SECOND);
    assert!(f.click(Hit::Escape));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(LIST_CHANGED)));
    // The mark does not appear: the step stops and says so, with no further key.
    let mut f = Live::new();
    f.show(BLOCKED_ONE);
    f.pass(QUIET);
    f.paint(1000.0, 900.0);
    f.pass(prompt::SEEN);
    assert!(f.click(Hit::Approve));
    assert_eq!(f.writes(), [ENTER.to_vec()]);
    f.pass(RESULT);
    assert!(f.writes().is_empty() && f.card().step.is_none());
    assert_eq!(f.card().error, Some(NOT_CLOSED));
}

#[test]
fn esc_that_only_closes_the_list_needs_no_wait_and_other_tabs_are_plain_panels() {
    let mut f = Live::new();
    f.show(BLOCKED_ONE);
    f.pass(QUIET);
    assert!(f.click(Hit::Escape));
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.show(BLOCKED_IDLE);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().blocked.is_none());

    // The dialog's other tabs have nothing to approve: read-only rows with Close.
    let mut f = Live::new();
    f.show(PERM_TAB_ALLOW);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Menu)));
    assert!(f.card().blocked.is_none());
    assert!(f.card().menu.as_ref().is_some_and(|menu| menu.panel() && menu.esc));

    // With "Answer menus from the chat" off there is no card and no key.
    let mut f = Live::new();
    f.app.config.chat.answer_menus = false;
    f.show(BLOCKED_ONE);
    f.pass(QUIET);
    assert!(f.card().blocked.is_none() && f.card().menu.is_none());
    f.click(Hit::Approve);
    f.click(Hit::Escape);
    assert!(f.writes().is_empty());
}
