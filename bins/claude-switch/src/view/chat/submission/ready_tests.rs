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
    PLAIN_EMPTIED, PLAIN_IDLE, PLAIN_INTERRUPTED, PLAIN_QUESTION, PLAIN_RESTORED, PLAIN_TYPED, WORKING_TOOL, recorded,
};
use crate::view::chat::question::{self, Hit};

const SECOND: Duration = Duration::from_secs(1);

struct Live {
    app: AppState,
    tab: u64,
    /// The test's clock: past the real one, so that inputs noted with the real time have settled.
    now: Instant,
}

impl Live {
    fn new() -> Self {
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

    fn tab(&mut self) -> &mut Tab {
        self.app.tabs.get_mut(self.tab).unwrap()
    }

    fn chat(&mut self) -> &mut Chat {
        &mut self.tab().pane.as_mut().unwrap().chat
    }

    /// The CLI drew this screen now.
    fn show(&mut self, fixture: &str) {
        let now = self.now;
        self.tab().term.screen = recorded(fixture);
        self.chat().output(now, true, false);
    }

    /// Time passes and the window's tick runs.
    fn pass(&mut self, time: Duration) -> (Gate, Option<Reason>) {
        self.now += time;
        let now = self.now;
        self.app.progress_chat_tabs(now, true);
        let tab = self.tab();
        let pane = tab.pane.as_ref().unwrap();
        pane.chat.decision(&pane.model.chat, &tab.term, &tab.program_title, now)
    }

    /// A turn the transcript shows running: a newer prompt and an unfinished answer.
    fn turn(&mut self) {
        let pane = self.tab().pane.as_mut().unwrap();
        let at = pane.model.chat.prompt.unwrap_or(0) + 100;
        pane.model.chat.prompt = Some(at);
        pane.model.chat.latest = Some((at + 1, ChatPhase::Active));
        pane.chat.fresh = true;
    }

    fn send(&mut self, text: &str) {
        let now = self.now;
        self.chat().editor.set(text.into());
        AppState::chat_send(self.tab(), now, false);
    }

    fn writes(&mut self) -> Vec<Vec<u8>> {
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
fn text_on_the_cli_input_is_never_an_idle_prompt() {
    let mut f = Live::new();
    f.show(PLAIN_TYPED);
    assert_eq!(f.pass(SECOND), (Gate::Check, None));
    f.send("mine");
    // Not queued behind someone's typed text: it would be appended to it.
    assert!(f.chat().queue.is_none() && f.writes().is_empty());
    assert!(f.chat().error.is_some_and(|e| e.contains("already contains text")));
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
