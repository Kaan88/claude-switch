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
    PLAIN_EMPTIED, PLAIN_IDLE, PLAIN_INTERRUPTED, PLAIN_NAMED, PLAIN_QUESTION, PLAIN_RESTORED, PLAIN_TYPED,
    PLAIN_WORKING_TYPED, WORKING_TOOL, recorded,
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

#[test]
fn send_now_into_an_input_that_does_not_read_empty_goes_once_it_does() {
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    f.send("next");
    assert!(f.chat().queue.is_some() && f.writes().is_empty(), "queued behind the turn");
    // Send now while the CLI's input holds text: no refusal, the request is kept.
    f.show(PLAIN_WORKING_TYPED);
    assert_eq!(f.pass(SECOND).0, Gate::Running);
    let now = f.now;
    AppState::chat_send(f.tab(), now, true);
    assert!(f.chat().error.is_none() && f.chat().queue.as_ref().is_some_and(Queue::sends_now));
    assert_eq!(f.chat().notice(&Default::default()), Some("Sends as soon as Claude Code's input reads empty."));
    assert!(f.writes().is_empty());
    // The input reads empty again while the turn still runs: the draft goes into it.
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND).0, Gate::Running);
    assert_eq!(f.writes(), message("next"));
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
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_none() && !chat.locked());
    assert!(f.writes().is_empty());
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
    f.show(WORKING_TOOL);
    assert_eq!(f.pass(SECOND), (Gate::Running, None));
    f.send("and then this");
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
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_none());
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
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_none());
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
    assert!(chat.staging.takeback.is_none() && chat.staging.sent.is_none() && !chat.locked());
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
    assert!(chat.staging.sent.is_none() && !chat.locked() && f.writes().is_empty());
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
