//! Readiness from the screen: a tab gated like a real one (not the demo), fed the screens recorded
//! from Claude Code 2.1.332. The transcript only hints: it has no completion for a resumed
//! session, its last record reads as a running turn after an interruption, and it holds no
//! `AskUserQuestion` call while that question waits.
use super::*;
use crate::source::Source;
use crate::tabs::NewTab;
use crate::term::Term;
use crate::view::chat::QUIET;
use crate::view::chat::question::screens::recorded::{
    ASK_OTHER, ASK_OTHER_FIRST, ASK_OTHER_LONG, ASK_OTHER_TYPED, FORM_OTHER, FORM_OTHER_FIRST, FORM_OTHER_TYPED,
};
use crate::view::chat::question::screens::recorded::{
    PLAIN_EMPTIED, PLAIN_IDLE, PLAIN_INTERRUPTED, PLAIN_NAMED, PLAIN_QUESTION, PLAIN_RESTORED, PLAIN_TRUST,
    PLAIN_TYPED, PLAIN_WORKING_TYPED, WORKING_TOOL, recorded,
};
use crate::view::chat::question::{self, Hit};
use std::ops::Not;

const SECOND: Duration = Duration::from_secs(1);

pub(in crate::view::chat) struct Live {
    pub(in crate::view::chat) app: AppState,
    tab: u64,
    /// The test's clock: past the real one, so that inputs noted with the real time have settled.
    pub(in crate::view::chat) now: Instant,
}

impl Live {
    pub(in crate::view::chat) fn new() -> Self {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let root = std::env::temp_dir();
        let mut pane = app.new_session_pane(crate::view::model::Source::File(root.join("ready.jsonl")), root);
        pane.chat.scripted = true;
        pane.chat.enabled = true;
        pane.chat.identity_unverified = false;
        pane.model.chat.prompt = Some(10);
        pane.model.chat.latest = Some((20, ChatPhase::EndTurn));
        let tab = app
            .tabs
            .open(NewTab { label: "Live".into(), color: [0; 3], session: None, account: None }, Term::empty((100, 40)));
        app.tabs.get_mut(tab).unwrap().pane = Some(pane);
        Self { app, tab, now: Instant::now() + SECOND }
    }

    pub(in crate::view::chat) fn tab(&mut self) -> &mut Tab {
        self.app.tabs.get_mut(self.tab).unwrap()
    }

    pub(in crate::view::chat) fn chat(&mut self) -> &mut Chat {
        &mut self.tab().pane.as_mut().unwrap().chat
    }

    /// The CLI drew this screen now.
    pub(in crate::view::chat) fn show(&mut self, fixture: &str) {
        let now = self.now;
        self.tab().term.screen = recorded(fixture);
        self.chat().output(now, true, false);
    }

    /// The CLI drew this screen now, read as the window reads output: only a change down to the
    /// input box is activity.
    pub(in crate::view::chat) fn draw(&mut self, fixture: &str) {
        let now = self.now;
        let screen = recorded(fixture);
        self.tab().term.screen = screen.clone();
        self.chat().output_on(now, true, false, &screen);
    }

    /// Time passes and the window's tick runs.
    pub(in crate::view::chat) fn pass(&mut self, time: Duration) -> (Gate, Option<Reason>) {
        self.now += time;
        let now = self.now;
        self.app.progress_chat_tabs(now, true);
        let tab = self.tab();
        let pane = tab.pane.as_ref().unwrap();
        pane.chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now)
    }

    /// A turn the transcript shows running: a newer prompt and an unfinished answer.
    pub(in crate::view::chat) fn turn(&mut self) {
        let pane = self.tab().pane.as_mut().unwrap();
        let at = pane.model.chat.prompt.unwrap_or(0) + 100;
        pane.model.chat.prompt = Some(at);
        pane.model.chat.latest = Some((at + 1, ChatPhase::Active));
        pane.chat.fresh = true;
    }

    pub(in crate::view::chat) fn send(&mut self, text: &str) {
        let now = self.now;
        self.chat().editor.set(text.into());
        AppState::chat_send(self.tab(), now, false);
    }

    pub(in crate::view::chat) fn writes(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.chat().writes)
    }
}

fn message(text: &str) -> Vec<Vec<u8>> {
    vec![format!("\x1b[200~{text}\x1b[201~").into_bytes(), b"\r".to_vec()]
}

#[test]
fn a_resumed_session_sends_once_its_idle_input_has_been_quiet() {
    let mut f = Live::new();
    // An old completion and no new prompt: the transcript proves nothing.
    assert!(!f.chat().fresh);
    f.show(PLAIN_IDLE);
    assert_eq!(f.pass(QUIET / 2), (Gate::Check, None), "the CLI may still be drawing");
    // A draft sent now is held, not refused, and goes out by itself.
    f.send("hello");
    assert!(f.chat().queue.is_some() && f.chat().error.is_none() && f.writes().is_empty());
    assert_eq!(f.pass(QUIET).0, Gate::Check, "its own keys are on their way");
    assert_eq!(f.writes(), message("hello"));
    assert!(f.chat().queue.is_none() && f.chat().editor.text.is_empty());
}

#[test]
fn a_named_sessions_idle_input_is_ready_and_a_message_goes_at_once() {
    // Claude Code writes a named session's name into the rule over its input (2.1.332). Read as
    // earlier text, it hid the input: every message was held and never went (owner, 2026-11-27).
    let screen = recorded(PLAIN_NAMED);
    assert_eq!(screen::prompt(&screen).as_deref(), Some(""));
    assert!(!screen::menu(&screen) && !screen::working(&screen));
    let mut f = Live::new();
    f.show(PLAIN_NAMED);
    assert_eq!(f.pass(SECOND), (Gate::Ready, None));
    f.send("hello");
    assert_eq!(f.writes(), message("hello"));
}

#[test]
fn a_new_folders_trust_dialog_is_answered_from_its_card() {
    let mut f = Live::new();
    {
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.chat.prompt = None;
        pane.model.chat.latest = None;
        pane.chat.awaiting_transcript = true;
    }
    f.show(PLAIN_TRUST);
    assert_eq!(f.pass(SECOND), (Gate::Check, Some(Reason::Menu)));
    let shown = f.chat().question.menu.clone().expect("a card");
    assert!(format!("{shown:?}").contains(r#"choices: ["No, exit", "Yes, I trust this folder"]"#), "{shown:?}");
    assert_eq!(f.chat().drawer, Drawer::Closed, "no terminal needed");
    // "Yes": the highlight moves down first; Enter only once the screen shows it there.
    let now = f.now;
    assert!(question::press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [b"\x1b[B".to_vec()]);
    f.pass(SECOND);
    assert!(f.writes().is_empty(), "the screen still shows No highlighted");
    let moved: Vec<String> = PLAIN_TRUST
        .lines()
        .map(|line| match line {
            l if l.contains("exit") => "   No, exit".to_owned(),
            l if l.contains("Yes, I trust") => " \u{276f} Yes, I trust this folder".to_owned(),
            l => l.to_owned(),
        })
        .collect();
    let moved = moved.join("\n");
    assert_ne!(moved, PLAIN_TRUST);
    f.show(&moved);
    f.pass(SECOND);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn only_a_rule_with_a_name_between_spaces_is_the_named_rule() {
    assert!(screen::named_rule("──────────────── Fix the flaky tile cache test ─"));
    assert!(screen::named_rule("  ━━━━━━━━ notes ━━  "));
    for other in [
        "────────────────────────",
        "──────── Fix the flaky tile cache test",
        "Fix the flaky tile cache test ─",
        "── a ─",
        "────────Fix─",
        "────────  ─",
    ] {
        assert!(!screen::named_rule(other), "{other}");
    }
}

#[test]
fn a_message_just_sent_is_not_followed_by_another_into_the_same_screen() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    assert_eq!(f.pass(SECOND), (Gate::Ready, None));
    f.send("first");
    assert_eq!(f.writes(), message("first"));
    // The screen has not shown the message yet: it still reads idle and quiet.
    assert_eq!(f.pass(SECOND).0, Gate::Check);
    f.send("second");
    assert!(f.chat().queue.is_some() && f.writes().is_empty());
    // The turn shows, runs and ends: only then does the second one go.
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    assert!(f.writes().is_empty());
    f.show(PLAIN_IDLE);
    assert_eq!(f.pass(QUIET / 2).0, Gate::Running, "not quiet yet, and the transcript still shows the turn");
    assert!(f.writes().is_empty());
    f.pass(QUIET);
    assert_eq!(f.writes(), message("second"));
}

#[test]
fn an_interrupted_turn_leaves_chat_ready_not_running() {
    let mut f = Live::new();
    // The transcript's last record after Esc is "[Request interrupted by user…]": a prompt with
    // no answer, which reads as a turn still running.
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    f.show(PLAIN_INTERRUPTED);
    assert_eq!(f.pass(SECOND), (Gate::Ready, None));
    f.send("go on");
    assert_eq!(f.writes(), message("go on"));
}

#[test]
fn a_bell_asks_for_nothing_once_the_input_is_idle() {
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    let now = f.now;
    f.chat().output(now, true, true);
    assert_eq!(f.pass(SECOND), (Gate::Running, None), "under the working line a bell asks for nothing");
    f.show(PLAIN_IDLE);
    assert_eq!(f.pass(SECOND), (Gate::Ready, None));
    assert!(!f.chat().bell && f.chat().drawer == Drawer::Closed);
    // It does not come back with the next turn.
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
}

#[test]
fn text_on_the_cli_input_is_never_an_idle_prompt_and_a_draft_waits_until_it_is_gone() {
    let mut f = Live::new();
    f.show(PLAIN_TYPED);
    assert_eq!(f.pass(SECOND), (Gate::Check, None));
    f.send("mine");
    // Held, never typed behind someone's text (it would be appended to it) and never clearing it.
    assert!(f.chat().queue.is_some() && f.chat().error.is_none());
    assert_eq!(f.pass(SECOND).0, Gate::Check);
    assert!(f.writes().is_empty(), "the typed text is left alone");
    // The text went without keys from this app (Claude Code redrew its input): the draft goes by
    // itself. Typing in the terminal would instead cancel the queue and keep the draft.
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    assert_eq!(f.writes(), message("mine"));
    assert!(f.chat().queue.is_none());
}

/// What `PLAIN_WORKING_TYPED` shows on the input while a turn runs.
const TYPED_WHILE_BUSY: &str = "hello while busy";

fn paste(text: &str) -> Vec<Vec<u8>> {
    vec![format!("\x1b[200~{text}\x1b[201~").into_bytes()]
}

#[test]
fn a_send_while_a_turn_runs_goes_into_the_empty_input_and_enter_waits_for_its_echo() {
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    // As Send now: pasted into Claude Code's empty input, which hands it to the running turn.
    f.send(TYPED_WHILE_BUSY);
    assert_eq!(f.writes(), paste(TYPED_WHILE_BUSY), "no Enter before the input shows the text");
    assert!(f.chat().locked());
    // Nothing new on the screen: still no Enter.
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty());
    // The input shows it: Enter, and the box is free for the next message.
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    let chat = f.chat();
    assert!(chat.queue.is_none() && chat.editor.text.is_empty() && !chat.locked() && chat.error.is_none());
}

#[test]
fn enter_never_goes_into_a_permission_prompt_drawn_after_the_paste() {
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.send(TYPED_WHILE_BUSY);
    assert_eq!(f.writes(), paste(TYPED_WHILE_BUSY));
    // A permission prompt is drawn before the text shows: its default is Yes, Enter must not go.
    f.show(crate::view::chat::question::screens::recorded::PERM_BASH);
    for _ in 0..8 {
        f.pass(SECOND);
    }
    assert!(f.writes().iter().all(|w| w.as_slice() != b"\r"), "no Enter into the prompt");
    assert!(f.chat().error.is_some(), "it says the message was not submitted");
}

#[test]
fn a_plain_send_during_a_turn_writes_nothing_while_something_else_is_up() {
    use crate::view::chat::question::screens::recorded::{MENU_MODEL, PERM_BASH};
    type Up = fn(&mut Live);
    let cases: [(&str, Up); 4] = [
        ("a permission question", |f| f.show(PERM_BASH)),
        ("a menu", |f| f.show(MENU_MODEL)),
        ("a pending tool", |f| {
            f.show(WORKING_TOOL);
            f.tab().pane.as_mut().unwrap().model.chat.pending = Some(25);
        }),
        ("the alternate screen", |f| {
            f.show(WORKING_TOOL);
            f.tab().term.feed(b"\x1b[?1049h");
        }),
    ];
    for (what, up) in cases {
        let mut f = Live::new();
        f.turn();
        up(&mut f);
        f.pass(SECOND);
        f.send(TYPED_WHILE_BUSY);
        for _ in 0..3 {
            f.pass(SECOND);
        }
        assert!(f.writes().is_empty(), "{what}");
        assert_eq!(f.chat().editor.text, TYPED_WHILE_BUSY, "{what}: the draft is kept");
    }
}

#[test]
fn a_call_the_turn_records_during_the_echo_wait_does_not_stop_the_message() {
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.send(TYPED_WHILE_BUSY);
    assert_eq!(f.writes(), paste(TYPED_WHILE_BUSY));
    // The running turn records a call meanwhile: still its turn, and the message still goes.
    f.tab().pane.as_mut().unwrap().model.chat.pending = Some(990);
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    assert!(f.chat().error.is_none() && f.chat().staging.recovery.is_none());
    // It went into the turn: the turn stays the transcript's (Stop is still offered).
    assert!(f.chat().fresh);
}

#[test]
fn a_long_line_into_a_running_turn_is_entered_once_claude_code_shows_its_paste_marker() {
    use crate::view::chat::question::screens::recorded::PLAIN_PASTED_MARKER;
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    let long = "word ".repeat(300).trim_end().to_owned();
    f.send(&long);
    assert_eq!(f.writes(), paste(&long));
    // Claude Code shows a long paste as its marker, not as the text.
    f.show(PLAIN_PASTED_MARKER);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    assert!(f.chat().error.is_none());
}

#[test]
fn send_now_released_after_the_turn_ended_is_an_ordinary_send() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    assert_eq!(f.writes(), message(RESTORED));
    f.turn();
    // Held behind the turn (the input holds text), then Send now.
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND);
    f.send(TYPED_WHILE_BUSY);
    let now = f.now;
    AppState::chat_send(f.tab(), now, true);
    assert!(f.chat().queue.as_ref().is_some_and(Queue::sends_now));
    // The turn ends before the input reads empty: the draft goes into an idle input, which is no
    // running turn. Enter at once, as any send, and it starts a turn of its own: the one before
    // is done with (nothing of it to take back) and the old completion is not taken for its.
    let pane = f.tab().pane.as_mut().unwrap();
    let at = pane.model.chat.latest.unwrap().0 + 10;
    pane.model.chat.latest = Some((at, ChatPhase::EndTurn));
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.pass(SECOND);
    assert_eq!(f.writes(), message(TYPED_WHILE_BUSY));
    let chat = f.chat();
    assert_eq!(chat.staging.sent.len(), 1);
    assert!(!chat.fresh, "the old end_turn is not read as this message's");
}

#[test]
fn send_now_into_an_input_that_does_not_read_empty_goes_once_it_does() {
    let mut f = Live::new();
    f.turn();
    // The CLI's input holds text while the turn runs: Send holds the draft behind the turn.
    f.show(PLAIN_WORKING_TYPED);
    assert_eq!(f.pass(SECOND).0, Gate::Running);
    f.send(TYPED_WHILE_BUSY);
    assert!(f.chat().queue.is_some() && f.writes().is_empty(), "queued behind the turn");
    // Send now while it still holds text: no refusal, the request is kept.
    let now = f.now;
    AppState::chat_send(f.tab(), now, true);
    assert!(f.chat().error.is_none() && f.chat().queue.as_ref().is_some_and(Queue::sends_now));
    assert_eq!(f.chat().notice(&Default::default()), Some("Sends as soon as Claude Code's input reads empty."));
    assert!(f.writes().is_empty());
    // The input reads empty again while the turn still runs: the draft goes into it, and Enter
    // once the input shows it.
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND).0, Gate::Running);
    assert_eq!(f.writes(), paste(TYPED_WHILE_BUSY));
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    assert!(f.chat().queue.is_none() && f.chat().editor.text.is_empty());
}

#[test]
fn a_question_the_transcript_does_not_hold_yet_shows_as_a_card_and_as_waiting() {
    let mut f = Live::new();
    // The turn runs, with no pending call on record (Claude Code writes the question's call
    // only once it is answered).
    f.turn();
    f.show(PLAIN_QUESTION);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Menu)));
    let chat = f.chat();
    assert!(chat.question.menu.is_some() && chat.drawer == Drawer::Closed);
    assert_eq!(chat.menu_waiting(), Some("Waiting for your answer"));
    // A send says to answer first; nothing is typed into the question.
    f.send("later");
    assert!(f.writes().is_empty() && f.chat().queue.is_none());
    assert_eq!(f.chat().error, Some(Reason::Menu.label()));
    // A click on the second choice moves the highlight there first.
    let now = f.now;
    assert!(question::press(f.tab(), now, Hit::Option(1)));
    let down =
        mdshared_term::input::encode_key(mdshared_term::Key::Down, mdshared_term::Mods::NONE, &Default::default());
    assert_eq!(f.writes(), [down]);
}

#[test]
fn a_silent_turn_with_nothing_chat_can_read_opens_the_terminal() {
    let mut f = Live::new();
    f.turn();
    let now = f.now;
    let tab = f.tab();
    tab.term.screen = mdshared_term::Screen::new(100, 40, 0);
    tab.term.screen.feed(b"\x1b[?2004hOverwrite the file? [y/N] ");
    f.chat().output(now, true, false);
    // A short silence is only a slow turn.
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    assert_eq!(f.pass(SECOND * 4), (Gate::Check, Some(Reason::Tool)));
    assert_eq!(f.chat().drawer, Drawer::Auto(Reason::Tool));
    // Under the working line the same silence is a slow tool.
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND * 6), (Gate::Running, None));
}

/// What `PLAIN_RESTORED` was recorded with: sent, then interrupted before any answer.
const RESTORED: &str = "Count from 1 to 50 slowly, one number per line. No tools.";

#[test]
fn a_message_the_cli_puts_back_after_an_early_stop_returns_to_the_message_box() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    assert_eq!(f.writes(), message(RESTORED));
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    // Esc before any answer: Claude Code drops the turn and puts the prompt back on its input.
    // The transcript's last record is still that prompt.
    f.show(PLAIN_RESTORED);
    assert_eq!(f.pass(SECOND).0, Gate::Check, "no turn runs, whatever the transcript says");
    assert!(f.chat().staging.takeback.is_some() && f.writes().is_empty());
    // One Ctrl+U, and no second one before the screen shows the first.
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\x15".to_vec()]);
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty());
    f.show(PLAIN_EMPTIED);
    assert_eq!(f.pass(SECOND), (Gate::Ready, None));
    let chat = f.chat();
    assert_eq!(chat.editor.text, RESTORED);
    assert_eq!(chat.error, Some(TAKEN_BACK));
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_empty() && !chat.locked());
    assert!(f.writes().is_empty());
}

#[test]
fn the_first_message_is_taken_back_after_a_stop_even_when_others_went_in_during_its_turn() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    assert_eq!(f.writes(), message(RESTORED));
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    // A second message goes in while the turn runs.
    f.send(TYPED_WHILE_BUSY);
    f.show(PLAIN_WORKING_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), message(TYPED_WHILE_BUSY));
    assert_eq!(f.chat().staging.sent.len(), 2);
    // Stop before any answer: Claude Code puts the first prompt back (it sends the waiting ones
    // as the next turn: recorded from 2.1.332). It is still taken back.
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    let now = f.now;
    AppState::chat_stop(f.tab(), now);
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.show(PLAIN_RESTORED);
    f.pass(SECOND);
    assert!(f.chat().staging.takeback.is_some());
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\x15".to_vec()]);
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND);
    let chat = f.chat();
    assert_eq!((chat.editor.text.as_str(), chat.error), (RESTORED, Some(TAKEN_BACK)));
    assert!(chat.staging.sent.is_empty());
}

#[test]
fn text_left_on_the_input_after_a_stop_that_chat_did_not_send_is_said_not_held_silently() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    f.writes();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    let now = f.now;
    AppState::chat_stop(f.tab(), now);
    f.writes();
    // After the stop the input holds something else: nothing is cleared, and it is said.
    f.show(PLAIN_TYPED);
    for _ in 0..3 {
        f.pass(SECOND);
    }
    assert!(f.writes().is_empty(), "nothing is cleared");
    assert_eq!(f.chat().error, Some(TAKEBACK_OTHER));
    assert!(f.chat().staging.sent.is_empty() && f.chat().staging.takeback.is_none());
    // Without a stop, the same screen is no business of the take-back.
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    f.writes();
    f.show(PLAIN_TYPED);
    for _ in 0..3 {
        f.pass(SECOND);
    }
    assert!(f.writes().is_empty() && f.chat().error.is_none());
}

#[test]
fn a_put_back_message_stays_on_the_cli_input_while_the_message_box_holds_a_draft() {
    // A second message held behind the first, then Stop before Claude answers.
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    assert_eq!(f.writes(), message(RESTORED));
    f.turn();
    // Held, as the CLI's input does not read empty while it is sent (into an empty input it
    // would go into the running turn at once).
    f.show(PLAIN_WORKING_TYPED);
    assert_eq!(f.pass(SECOND).0, Gate::Running);
    f.send("and then this");
    assert!(f.chat().queue.is_some() && f.writes().is_empty());
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    assert!(f.chat().queue.is_some() && f.writes().is_empty());
    let now = f.now;
    AppState::chat_stop(f.tab(), now);
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.show(PLAIN_RESTORED);
    for _ in 0..4 {
        f.pass(SECOND);
    }
    // No Ctrl+U: the first message is still on the CLI input, the second still in the box and
    // not sent into it.
    assert!(f.writes().is_empty());
    let chat = f.chat();
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_empty());
    assert_eq!(chat.editor.text, "and then this");
    assert_eq!(chat.error, Some(TAKEBACK_HELD));
    // The same when the user has only started typing.
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    f.writes();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.chat().editor.set("half a sen".into());
    f.show(PLAIN_RESTORED);
    f.pass(SECOND);
    f.pass(SECOND);
    assert!(f.writes().is_empty());
    let chat = f.chat();
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_empty());
    assert_eq!(chat.editor.text, "half a sen");
    assert_eq!(chat.error, Some(TAKEBACK_HELD));
}

/// A take-back up to its first Ctrl+U, which the screen has not answered yet.
fn one_ctrl_u() -> Live {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    f.writes();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.show(PLAIN_RESTORED);
    f.pass(SECOND);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\x15".to_vec()]);
    f
}

#[test]
fn while_output_keeps_coming_an_empty_input_counts_once_two_newer_screens_read_empty() {
    let mut f = one_ctrl_u();
    // The CLI draws the emptied input, and keeps drawing: no quiet.
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    assert!(f.chat().staging.takeback.is_some(), "one reading may be a redraw caught half way");
    // The same screen, with nothing drawn since, is not a second reading.
    f.pass(SECOND / 10);
    assert!(f.chat().staging.takeback.is_some());
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    let chat = f.chat();
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_empty() && !chat.locked());
    assert_eq!(chat.editor.text, RESTORED);
    assert_eq!(chat.error, Some(TAKEN_BACK));
    assert!(f.writes().is_empty());
}

#[test]
fn an_empty_reading_between_two_that_still_show_the_message_does_not_end_the_take_back() {
    let mut f = one_ctrl_u();
    // A turn starts by itself and redraws: the input reads empty half way through the redraw,
    // then shows the message again.
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    f.show(PLAIN_RESTORED);
    f.pass(SECOND / 10);
    assert!(f.chat().staging.takeback.is_some() && f.chat().editor.text.is_empty());
    // Newer output, so the next Ctrl+U goes; a single empty reading after it counts for nothing
    // yet, as the one before it was not empty.
    assert_eq!(f.writes(), [b"\x15".to_vec()]);
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    assert!(f.chat().staging.takeback.is_some());
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    assert_eq!(f.chat().editor.text, RESTORED);
    assert_eq!(f.chat().error, Some(TAKEN_BACK));
}

#[test]
fn a_working_line_over_an_empty_input_is_one_reading_not_the_end() {
    let mut f = one_ctrl_u();
    // A turn starts by itself: its working line over the input, which reads empty.
    assert!(clean_prompt(&recorded(WORKING_TOOL)), "the fixture's input reads empty");
    f.show(WORKING_TOOL);
    f.pass(SECOND / 10);
    assert!(f.chat().staging.takeback.is_some() && f.chat().editor.text.is_empty());
    f.show(WORKING_TOOL);
    f.pass(SECOND / 10);
    assert_eq!(f.chat().editor.text, RESTORED);
    assert_eq!(f.chat().error, Some(TAKEN_BACK));
}

#[test]
fn an_empty_input_never_confirmed_gives_the_message_back_at_the_deadline_with_a_warning() {
    let mut f = one_ctrl_u();
    f.show(PLAIN_EMPTIED);
    f.pass(SECOND / 10);
    // Nothing newer is drawn, and the screen is never quiet: the clock keeps output recent.
    let mut ticks = 0;
    while f.chat().staging.takeback.is_some() {
        let now = f.now;
        // Output just now (no quiet), but no newer screen.
        f.chat().last_output = Some(now);
        f.pass(SECOND / 10);
        let now = f.now;
        assert!(stage_deadline(f.chat(), now).is_none_or(|due| due > now), "a past deadline re-armed");
        ticks += 1;
        assert!(ticks <= 25, "the take-back has no deadline");
    }
    let chat = f.chat();
    assert_eq!(chat.editor.text, RESTORED, "not cleared and dropped: no Enter went, so it was not sent");
    assert_eq!(chat.error, Some(TAKEBACK_UNSURE));
    assert!(chat.staging.sent.is_empty() && !chat.locked() && f.writes().is_empty());
}

#[test]
fn text_the_user_typed_in_the_terminal_is_never_cleared() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send(RESTORED);
    f.writes();
    // The same words, but the terminal got input of its own in between.
    let pane = f.tab().pane.as_mut().unwrap();
    pane.note_terminal_input();
    f.show(PLAIN_RESTORED);
    f.pass(SECOND);
    f.pass(SECOND);
    assert!(f.chat().staging.takeback.is_none() && f.writes().is_empty());
    // Nor is other text after a send.
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.send("something else");
    f.writes();
    f.show(PLAIN_RESTORED);
    f.pass(SECOND);
    f.pass(SECOND);
    assert!(f.chat().staging.takeback.is_none() && f.writes().is_empty());
}

#[test]
fn a_slash_command_waits_a_moment_for_the_prompt_to_go_quiet() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    let now = f.now;
    // Right after the CLI drew its prompt: held, not refused.
    AppState::submit_slash(f.tab(), now, "/context", false);
    assert!(f.chat().staging.command.is_some() && f.chat().error.is_none() && f.writes().is_empty());
    f.pass(SECOND);
    assert_eq!(f.writes(), [b"/context".to_vec()]);
    assert!(f.chat().staging.command.is_none());
    // A prompt that holds text does not pass: the command is refused once the wait is over.
    let mut f = Live::new();
    f.show(PLAIN_TYPED);
    f.pass(SECOND);
    let now = f.now;
    AppState::submit_slash(f.tab(), now, "/context", false);
    f.pass(SECOND);
    assert!(f.chat().staging.command.is_some());
    f.pass(COMMAND_WAIT);
    assert_eq!(f.chat().error, Some(NO_QUIET_PROMPT));
    assert!(f.chat().staging.command.is_none() && f.writes().is_empty());
}

pub(in crate::view::chat) fn downs(n: usize) -> Vec<u8> {
    mdshared_term::input::encode_key(mdshared_term::Key::Down, mdshared_term::Mods::NONE, &Default::default()).repeat(n)
}

impl Live {
    /// The user types into the card's field and presses Enter there.
    fn answer(&mut self, text: &str) {
        let now = self.now;
        self.chat().question.field.insert(text);
        assert!(question::press(self.tab(), now, Hit::Answer));
    }
}

#[test]
fn a_typed_answer_goes_on_the_type_something_row_and_enter_only_once_it_reads_exactly() {
    let mut f = Live::new();
    f.turn();
    f.show(ASK_OTHER_FIRST);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Menu)));
    // The highlight first: three rows down, and no text before the screen shows it there.
    f.answer("my own answer");
    assert_eq!(f.writes(), [downs(3)]);
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty());
    f.show(ASK_OTHER);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"my own answer".to_vec()]);
    // Other text on the row (here: more than was typed) is not the answer: no Enter.
    f.show(ASK_OTHER_LONG);
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty() && f.chat().question.probe().0);
    f.show(ASK_OTHER_TYPED);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn a_long_typed_answer_is_read_across_the_rows_it_wraps_onto() {
    const LONG: &str = "my own answer and it goes on with many more words so that the text is longer than one row \
        of the terminal and has to wrap onto the next row of the screen";
    let mut f = Live::new();
    f.turn();
    f.show(ASK_OTHER_FIRST);
    f.pass(QUIET);
    f.answer(LONG);
    f.writes();
    f.show(ASK_OTHER);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [LONG.as_bytes().to_vec()]);
    // Only part of it shown yet: wait.
    f.show(ASK_OTHER_TYPED);
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty());
    f.show(ASK_OTHER_LONG);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // A typed answer that never shows stops with a notice and no Enter.
    let mut f = Live::new();
    f.turn();
    f.show(ASK_OTHER_FIRST);
    f.pass(QUIET);
    f.answer("something else");
    f.show(ASK_OTHER);
    f.pass(SECOND / 10);
    f.show(ASK_OTHER_TYPED);
    f.pass(SECOND * 3);
    assert!(f.chat().question.probe().0.not() && f.chat().question.probe().1.is_some());
    assert!(!f.writes().iter().any(|w| w == b"\r"));
    // A digit first could pick a choice: refused before any key.
    let mut f = Live::new();
    f.turn();
    f.show(ASK_OTHER_FIRST);
    f.pass(QUIET);
    f.answer("2 of them");
    assert!(f.writes().is_empty() && f.chat().question.probe().1.is_some());
}

#[test]
fn a_forms_typed_answer_ticks_its_box_and_is_left_for_next_or_submit() {
    let mut f = Live::new();
    f.turn();
    f.show(FORM_OTHER_FIRST);
    f.pass(QUIET);
    f.answer("kiwi");
    assert_eq!(f.writes(), [downs(3)]);
    f.show(FORM_OTHER);
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"kiwi".to_vec()]);
    f.show(FORM_OTHER_TYPED);
    f.pass(SECOND / 10);
    // No Enter: on a multi-select's row it would clear the box again. The card now offers the
    // page with the answer ticked, and Next as a choice.
    assert!(f.writes().is_empty());
    let chat = f.chat();
    assert!(chat.question.probe().0.not() && chat.question.probe().1.is_none());
    assert!(chat.question.field.text().is_empty());
    let now = f.now;
    assert!(question::press(f.tab(), now, Hit::Option(4)));
    assert_eq!(f.writes(), [downs(1)]);
}

const RULE: &str = "────────────────────";
const NAMED: &str = "──────────────────── Test session ─";

/// Background agents listed under the input, their row redrawn with `timer`; `above` over the
/// input's rule.
fn agents_below(above: &str, timer: &str) -> String {
    format!(
        "cursor 2 2\n{above}\n{NAMED}\n❯ \x1b[2msuggested text\x1b[0m\n{RULE}\n  ⏵⏵ auto mode on · 2 background tasks · ↓ to manage\n  ● main\n  ◯ general-purpose  Reading a.rs   {timer} · ↓ 1.0k tokens\n  ↓ 1 more"
    )
}

#[test]
fn background_agents_redrawn_under_the_input_do_not_keep_a_command_from_going() {
    // Their timers redraw every second while the CLI is at rest: counted as activity, the screen
    // was never quiet and every command was refused (owner, 2026-11-28).
    let waiting = "✻ Waiting for 2 background agents to finish";
    let mut f = Live::new();
    f.draw(&agents_below(waiting, "1m 02s"));
    let now = f.now;
    AppState::submit_slash(f.tab(), now, "/context", false);
    assert!(f.chat().error.is_none() && f.writes().is_empty());
    for second in 3..7 {
        f.pass(QUIET * 2 / 3);
        f.draw(&agents_below(waiting, &format!("1m 0{second}s")));
    }
    assert_eq!(f.writes(), [b"/context".to_vec()]);
    assert!(f.chat().staging.command.is_none() && f.chat().error.is_none());

    // A row above the input that keeps changing is still activity: never quiet, refused.
    let mut f = Live::new();
    f.draw(&agents_below("✻ Thinking (1s)", "1m 02s"));
    let now = f.now;
    AppState::submit_slash(f.tab(), now, "/context", false);
    for second in 2..10 {
        f.pass(QUIET * 2 / 3);
        f.draw(&agents_below(&format!("✻ Thinking ({second}s)"), "1m 02s"));
    }
    assert_eq!(f.chat().error, Some(NO_QUIET_PROMPT));
    assert!(f.chat().staging.command.is_none() && f.writes().is_empty());
}

#[test]
fn only_the_rows_down_to_the_input_box_count_as_activity() {
    let digest = |fixture: &str| screen::activity_digest(&recorded(fixture));
    let base = digest(&agents_below("Done.", "1m 02s"));
    assert_eq!(base, digest(&agents_below("Done.", "1m 03s")), "under the box");
    assert_ne!(base, digest(&agents_below("Done!", "1m 02s")), "over it");
    // A menu has no input box: an earlier prompt's mark in the history does not cut it short.
    let menu = |highlight: usize| {
        let row = |n: usize, text: &str| format!("{} {n}. {text}", if n == highlight { "❯" } else { " " });
        format!("cursor 0 0\n❯ hello\n\n{RULE}\n Do you want to proceed?\n{}\n{}", row(1, "Yes"), row(2, "No"))
    };
    assert_ne!(digest(&menu(1)), digest(&menu(2)));
}

#[test]
fn a_menus_highlighted_choice_is_no_input_mark() {
    let screen = recorded(&format!("cursor 0 0\n❯ 1. Yes\n  2. No\n❯ 12. Twelve\n❯ hello\n❯ 1.5 is a number\n{RULE}"));
    assert!(!screen::marks_input(&screen, 0) && !screen::marks_input(&screen, 2));
    assert!(screen::marks_input(&screen, 3) && screen::marks_input(&screen, 4));
    // Right under the input box's rule, plain or named, `❯ 1. …` is a message on the input.
    let typed = recorded(&format!("cursor 1 10\n{RULE}\n❯ 1. fix a\n{RULE}"));
    assert!(screen::marks_input(&typed, 1));
    let named = recorded(&format!("cursor 1 10\n{NAMED}\n❯ 1. fix a\n{RULE}"));
    assert!(screen::marks_input(&named, 1));
}

#[test]
fn a_message_starting_with_a_number_reads_as_the_input() {
    // Several lines, sent as typed: the echo check and the take-back read them.
    let lines = recorded(&format!("cursor 2 10\n{RULE}\n❯ 1. fix a\n  2. fix b\n{RULE}\n  ⏵⏵ auto mode on"));
    assert!(screen::shows_lines(&lines, "1. fix a\n2. fix b"));
    assert!(screen::shows_some_input(&lines));
    // One line.
    let single = recorded(&format!("cursor 1 8\n{NAMED}\n❯ 1. x\n{RULE}\n  ⏵⏵ auto mode on"));
    assert!(screen::shows_some_input(&single));
    assert_eq!(screen::prompt(&single).as_deref(), Some("1. x"));
    // The activity digest ends at the input box's bottom rule, as for any input.
    let below =
        |hint: &str| screen::activity_digest(&recorded(&format!("cursor 1 8\n{RULE}\n❯ 1. x\n{RULE}\n  {hint}")));
    assert_eq!(below("1m 02s"), below("1m 03s"));
    // The menu the exclusion is for: its highlighted choice is under its question, no input.
    let first = format!(
        "cursor 4 0\n❯ hello\n\n{RULE}\n Do you want to proceed?\n❯ 1. Yes\n  2. No\n\n Esc to cancel · Tab to amend"
    );
    let menu = recorded(&first);
    assert!(!screen::marks_input(&menu, 4));
    assert!(!screen::shows_some_input(&menu));
    // A move of the highlight is activity: the menu counts whole.
    let second = recorded(&first.replace("❯ 1. Yes\n  2.", "  1. Yes\n❯ 2."));
    assert_ne!(screen::activity_digest(&menu), screen::activity_digest(&second));
}

#[test]
fn a_command_after_slash_model_goes_once_the_idle_screen_is_quiet() {
    // The turn before `/model` had no `end_turn` record, so the command's own record stays the
    // newest and Active: the idle screen is what tells it is over.
    let idle = format!(
        "cursor 3 2\n❯ /model opus\n  ⎿  Set model to Opus and saved as your default\n{NAMED}\n❯ \n{RULE}\n  ⏵⏵ auto mode on (shift+tab to cycle)"
    );
    let mut f = Live::new();
    {
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.chat.prompt = Some(500);
        pane.model.chat.latest = Some((500, ChatPhase::Active));
        pane.chat.fresh = true;
    }
    f.draw(&idle);
    let now = f.now;
    assert_eq!(blocked(f.tab(), now, Intent::Slash), Some(NO_QUIET_PROMPT), "not quiet yet: waited for");
    f.pass(SECOND);
    let now = f.now;
    assert_eq!(blocked(f.tab(), now, Intent::Slash), None);
    AppState::submit_slash(f.tab(), now, "/context", false);
    assert_eq!(f.writes(), [b"/context".to_vec()]);
    // A chip command right after `/model`, within the quiet time: held, then run by itself.
    let mut f = Live::new();
    {
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.chat.prompt = Some(500);
        pane.model.chat.latest = Some((500, ChatPhase::Active));
        pane.chat.fresh = true;
    }
    f.draw(&idle);
    let now = f.now;
    AppState::submit_slash(f.tab(), now, "/context", false);
    assert!(f.chat().error.is_none() && f.writes().is_empty());
    assert!(f.chat().staging.command.is_some(), "held by the command's wait");
    f.pass(SECOND);
    assert_eq!(f.writes(), [b"/context".to_vec()]);
    assert!(f.chat().staging.command.is_none() && f.chat().error.is_none());
    // Held the same way, but a turn shows meanwhile: refused as running, nothing written.
    let mut f = Live::new();
    {
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.chat.prompt = Some(500);
        pane.model.chat.latest = Some((500, ChatPhase::Active));
        pane.chat.fresh = true;
    }
    f.draw(&idle);
    let now = f.now;
    AppState::submit_slash(f.tab(), now, "/context", false);
    assert!(f.chat().staging.command.is_some());
    f.draw(&idle.replace("  ⎿  Set model", "✻ Working… (1s)\n  ⎿  Set model").replacen("cursor 3 2", "cursor 4 2", 1));
    f.pass(QUIET / 3);
    assert!(f.writes().is_empty() && f.chat().staging.command.is_none());
    assert_eq!(f.chat().error, Some(BUSY_CMD));
    // The same transcript with a working line: the turn runs.
    let mut working = idle.replace("  ⎿  Set model", "✻ Working… (3s)\n  ⎿  Set model");
    working = working.replacen("cursor 3 2", "cursor 4 2", 1);
    let mut f = Live::new();
    {
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.chat.prompt = Some(500);
        pane.model.chat.latest = Some((500, ChatPhase::Active));
        pane.chat.fresh = true;
    }
    f.draw(&working);
    f.pass(SECOND);
    let now = f.now;
    assert_eq!(blocked(f.tab(), now, Intent::Slash), Some(BUSY_CMD));
}

#[test]
fn a_review_step_over_blank_rows_and_a_named_rule_shows_as_a_card() {
    // Claude Code left five blank rows and the input's named rule under "2. Cancel": the last
    // row was taken for the review's last entry, and Chat sent the user to the terminal.
    let review = format!(
        "cursor 9 2\n{RULE}\n←  ☒ Size  ✔ Submit  →\n\nReview your answers\n\n │ ● Which size should the test use? A long question\n │   that wraps onto a second row.\n   → Small\n\n❯ 1. Submit answers\n  2. Cancel\n\n\n\n\n\n{NAMED}"
    );
    let mut f = Live::new();
    f.turn();
    f.draw(&review);
    // The transcript holds no call while the question waits: a card for the screen's menu.
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Menu)));
    let shown = f.chat().question.menu.clone().expect("a card");
    assert!(format!("{shown:?}").contains(r#"["Submit answers", "Cancel"]"#), "{shown:?}");
    assert_eq!(f.chat().drawer, Drawer::Closed, "no terminal needed");
    f.send("later");
    assert!(f.writes().is_empty() && f.chat().queue.is_none());
}

#[test]
fn a_numbered_message_makes_no_card_in_the_input_or_once_sent() {
    // The owner's message, a numbered list of three lines: Chat read a menu from its own lines,
    // showed a card for it and refused every send.
    let footer_like = "  Enter to select · ↑/↓ to navigate · Esc to cancel";
    let draft =
        format!("cursor 3 12\n❯ earlier\n{RULE}\n❯ 1. say pong\n  2. nothing else\n  3. done\n{RULE}\n{footer_like}");
    let mut f = Live::new();
    f.draw(&draft);
    let (_, reason) = f.pass(QUIET);
    assert_ne!(reason, Some(Reason::Menu));
    assert!(f.chat().question.menu.is_none(), "no card");
    let now = f.now;
    assert_ne!(blocked(f.tab(), now, Intent::Slash), Some(MENU_OPEN));
    // Sent: the message in the conversation, the turn running, the empty input.
    let sent = format!(
        "cursor 6 2\n❯ 1. say pong\n  2. nothing else\n  3. done\n\n✻ Working… (1s)\n{RULE}\n❯ \n{RULE}\n{footer_like}"
    );
    let mut f = Live::new();
    f.turn();
    f.draw(&sent);
    let (_, reason) = f.pass(QUIET);
    assert_ne!(reason, Some(Reason::Menu));
    assert!(f.chat().question.menu.is_none(), "no card");
}
