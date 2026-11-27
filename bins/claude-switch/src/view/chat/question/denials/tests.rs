//! The current turn's denied permissions: which calls count, how the card shows them, and that
//! its buttons write nothing but the `/permissions` command, under that command's own checks.
use super::*;
use crate::view::chat::question::screens::recorded::{BLOCKED_IDLE, BLOCKED_ONE, BLOCKED_TYPED};
use crate::view::chat::submission::ready_tests::Live;
use crate::view::chat::{Gate, QUIET};
use claude_switch_core::transcript::Item;

const TICK: Duration = Duration::from_millis(100);
const SECOND: Duration = Duration::from_secs(1);
const CLASSIFIER: &str = "The server-side auto mode classifier judged this action dangerous (it gave no explanation).";

fn denial(kind: DenialKind) -> Denial {
    let (reason, rule_source) = match kind {
        DenialKind::AutoMode => (CLASSIFIER, None),
        DenialKind::Rule => {
            ("Permission to use Bash with command mkdir probe_d4 has been denied.", Some("flagSettings".to_owned()))
        }
        _ => ("The user doesn't want to proceed with this tool use.", None),
    };
    Denial { kind, before_run: (kind == DenialKind::User).then_some(true), reason: reason.into(), rule_source }
}

impl Live {
    /// A tool call in the transcript at `offset`: a shell command, or a file for another tool.
    fn call(&mut self, offset: u64, tool: &str, target: &str, state: ToolState, denial: Option<Denial>) {
        let shell = tool == "Bash";
        let kind = ItemKind::Tool {
            id: format!("toolu_{offset}"),
            name: tool.into(),
            summary: target.into(),
            file_path: (!shell).then(|| target.to_owned()),
            state,
            duration_ms: None,
            pictures: vec![],
            subagent: None,
            denial: denial.map(Box::new),
            card: shell.then(|| ToolCard::Shell { command: target.into(), result: vec![] }),
        };
        let model = &mut self.tab().pane.as_mut().unwrap().model;
        model.items.insert(ItemId { offset, block: 0 }, Item { kind, timestamp: None });
    }

    fn denied(&mut self, offset: u64, tool: &str, target: &str, kind: DenialKind) {
        self.call(offset, tool, target, ToolState::Failed, Some(denial(kind)));
    }

    fn current(&mut self) -> Vec<(String, DenialKind)> {
        let model = &self.tab().pane.as_ref().unwrap().model;
        current(model).into_iter().map(|d| (d.target, d.denial.kind)).collect()
    }
}

#[test]
fn current_means_the_newest_prompts_turn_without_what_ran_after_all() {
    let mut f = Live::new();
    // The newest prompt is at 10 (the fixture's): a denial before it is an earlier turn's.
    f.denied(5, "Bash", "rm old", DenialKind::Rule);
    f.denied(20, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    f.denied(21, "Bash", "mkdir probe_d4", DenialKind::Rule);
    f.denied(22, "Write", "notes.txt", DenialKind::User);
    f.call(23, "Bash", "ls", ToolState::Ok, None);
    f.call(24, "Bash", "false", ToolState::Failed, None);
    assert_eq!(
        f.current(),
        [
            ("mkdir probe_blocked_two".to_owned(), DenialKind::AutoMode),
            ("mkdir probe_d4".to_owned(), DenialKind::Rule),
            ("notes.txt".to_owned(), DenialKind::User),
        ]
    );
    // The same call fails again: still denied. It runs: no longer something to approve.
    f.call(30, "Bash", "mkdir probe_blocked_two", ToolState::Failed, None);
    assert_eq!(f.current().len(), 3);
    f.call(31, "Bash", "mkdir probe_blocked_two", ToolState::Ok, None);
    assert_eq!(f.current().len(), 2);
    // Denied again after it ran: current again.
    f.denied(32, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    assert_eq!(f.current().len(), 3);
    // The next prompt starts over.
    f.turn();
    assert!(f.current().is_empty());
    // A subagent's transcript in view: its calls are not the main conversation's.
    f.denied(200, "Bash", "mkdir x", DenialKind::Rule);
    assert_eq!(f.current().len(), 1);
    f.tab().pane.as_mut().unwrap().model.agent = Some("a1".into());
    assert!(f.current().is_empty());
}

#[test]
fn each_kind_says_who_refused_and_what_that_means() {
    assert_eq!(label(DenialKind::AutoMode), "✗ blocked by auto mode");
    assert_eq!(label(DenialKind::Rule), "✗ denied by a rule");
    assert_eq!(label(DenialKind::User), "✗ you said no");
    assert_eq!(why(&denial(DenialKind::AutoMode)), format!("Auto mode's classifier blocked it: {CLASSIFIER}"));
    assert_eq!(
        why(&denial(DenialKind::Rule)),
        "A permission rule denies it (set with --settings when Claude Code was started). It stays denied while that rule stands."
    );
    assert_eq!(why(&denial(DenialKind::User)), "You said no. Nothing needs approving.");
    let unknown = Denial { rule_source: Some("somewhereNew".into()), ..denial(DenialKind::Rule) };
    assert!(why(&unknown).contains("(from somewhereNew)"), "a source this version does not know is shown as it is");
}

#[test]
fn a_denial_opens_the_card_with_its_count_and_one_already_there_only_shows_the_count() {
    // Denied before the tab was opened: a strip with the count, to be opened by a click.
    let mut f = Live::new();
    f.denied(20, "Bash", "mkdir probe_d4", DenialKind::Rule);
    f.show(BLOCKED_IDLE);
    assert_eq!(f.pass(QUIET * 2), (Gate::Ready, None), "the card never blocks a send");
    assert_eq!(f.card().denials.len(), 1);
    assert!(!f.card().denials_watch.open);
    assert_eq!(f.paint(1000.0, 800.0), [Hit::Denials(true)]);
    assert!(f.card().rect.h < 50.0, "one line");
    assert!(f.click(Hit::Denials(true)));
    let hits = f.paint(1000.0, 800.0);
    assert!(hits.contains(&Hit::Review) && hits.contains(&Hit::AskRetry) && hits.contains(&Hit::Denials(false)));
    assert!(f.click(Hit::Denials(false)));
    assert_eq!(f.paint(1000.0, 800.0), [Hit::Denials(true)]);
    assert!(f.writes().is_empty());

    // Denied while the tab is open: the card opens by itself, and again for the next one.
    let mut f = Live::new();
    f.show(BLOCKED_IDLE);
    f.pass(QUIET * 2);
    assert!(f.card().denials.is_empty() && f.paint(1000.0, 800.0).is_empty());
    f.denied(20, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    f.pass(TICK);
    assert!(f.card().denials_watch.open);
    let hits = f.paint(1000.0, 800.0);
    assert!(hits.contains(&Hit::Review) && !hits.contains(&Hit::AskRetry), "auto mode's own list tells Claude");
    f.click(Hit::Denials(false));
    f.pass(TICK);
    assert!(!f.card().denials_watch.open, "folded stays folded");
    f.denied(21, "Write", "notes.txt", DenialKind::User);
    f.pass(TICK);
    assert!(f.card().denials_watch.open && f.card().denials.len() == 2);
    // The call runs after all: it leaves the count; the last one leaving takes the card along.
    f.call(30, "Bash", "mkdir probe_blocked_two", ToolState::Ok, None);
    f.call(31, "Write", "notes.txt", ToolState::Ok, None);
    f.pass(TICK);
    assert!(f.card().denials.is_empty() && f.paint(1000.0, 800.0).is_empty());
}

#[test]
fn ask_claude_to_retry_only_fills_the_message_box() {
    let mut f = Live::new();
    f.show(BLOCKED_IDLE);
    f.pass(QUIET * 2);
    f.denied(20, "Write", "notes.txt", DenialKind::User);
    f.denied(21, "Bash", "mkdir probe_d4", DenialKind::Rule);
    f.denied(22, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    f.pass(TICK);
    assert!(f.click(Hit::AskRetry));
    let sentence = "Please try again what was denied before, I allow it now: Write: notes.txt; Bash: mkdir probe_d4";
    assert_eq!(f.chat().editor.text, sentence, "the call auto mode blocked goes through its own list");
    f.pass(SECOND);
    assert!(f.writes().is_empty() && f.chat().queue.is_none(), "nothing is sent");
    // A draft is never replaced: the sentence waits for the box to be empty.
    f.chat().editor.set("my own draft".into());
    assert!(f.click(Hit::AskRetry));
    assert_eq!(f.chat().editor.text, "my own draft");
    assert!(f.chat().held_notice().is_some());
    f.chat().editor.set(String::new());
    f.pass(TICK);
    assert_eq!(f.chat().editor.text, sentence);
    assert!(f.writes().is_empty());
}

#[test]
fn review_runs_permissions_as_a_slash_command_and_the_clis_list_takes_the_card() {
    let mut f = Live::new();
    f.show(BLOCKED_IDLE);
    f.pass(QUIET * 2);
    f.denied(20, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    f.pass(TICK);
    assert!(f.click(Hit::Review));
    f.pass(TICK);
    assert_eq!(f.writes(), [b"/permissions".to_vec()], "the command's text first");
    // Enter only once the input row shows the command, as for any slash command.
    f.pass(TICK);
    assert!(f.writes().is_empty());
    f.show(BLOCKED_TYPED);
    f.pass(TICK);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // Claude Code opens its dialog on the list of blocked calls: that card comes first.
    f.show(BLOCKED_ONE);
    f.pass(QUIET);
    assert!(f.card().blocked_shown());
    let hits = f.paint(1000.0, 800.0);
    assert!(hits.contains(&Hit::Approve) && !hits.contains(&Hit::Review));
    assert!(f.writes().is_empty(), "nothing is chosen in the list without a click");

    // While a turn runs the command is refused like any other, and says so.
    let mut f = Live::new();
    f.show(BLOCKED_IDLE);
    f.pass(QUIET * 2);
    f.turn();
    f.denied(150, "Bash", "mkdir probe_blocked_two", DenialKind::AutoMode);
    f.show(crate::view::chat::question::screens::recorded::WORKING_TOOL);
    f.pass(TICK);
    assert_eq!(f.card().denials.len(), 1);
    assert!(f.click(Hit::Review));
    f.pass(SECOND * 4);
    assert!(f.writes().is_empty() && f.chat().error.is_some());
}
