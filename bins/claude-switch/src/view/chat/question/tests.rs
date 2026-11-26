use super::*;
use crate::source::Source;
use crate::state::AppState;
use crate::tabs::NewTab;
use crate::term::Term;
use crate::view::chat::{Gate, QUIET, Reason};
use claude_switch_core::transcript::{Item, QuestionOption};

const AT: u64 = 100;

fn question(multi_select: bool) -> Question {
    Question {
        header: "Who builds".into(),
        question: "Who builds the parser?".into(),
        options: vec![
            QuestionOption { label: "Me".into(), description: "I write it".into() },
            QuestionOption { label: "You".into(), description: "Claude writes it".into() },
        ],
        multi_select,
    }
}

/// The menu as the classic renderer draws it: numbered rows, `❯` on the highlighted one.
fn menu(highlight: usize, typed: Option<&str>) -> String {
    let rows = ["Me", "You", typed.unwrap_or("Type something."), "Chat about this"];
    let mut out = String::from("\x1b[2J\x1b[H\x1b[?2004h ☐ Who builds\r\n\r\nWho builds the parser?\r\n\r\n");
    for (i, label) in rows.iter().enumerate() {
        let mark = if i == highlight { "❯" } else { " " };
        out.push_str(&format!("{mark} {}. {label}\r\n", i + 1));
        if i < 2 {
            out.push_str(&format!("     {}\r\n", ["I write it", "Claude writes it"][i]));
        }
    }
    out.push_str("\r\nEnter to select · ↑/↓ to navigate · Esc to cancel");
    out
}

struct Fixture {
    app: AppState,
    tab: u64,
}

impl Fixture {
    fn new(questions: Vec<Question>) -> Self {
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let root = std::env::temp_dir();
        let mut pane = app.new_session_pane(crate::view::model::Source::File(root.join("question.jsonl")), root);
        pane.chat.demo = true;
        pane.chat.enabled = true;
        pane.chat.identity_unverified = false;
        let kind = ItemKind::Tool {
            id: "toolu_1".into(),
            name: "AskUserQuestion".into(),
            summary: "AskUserQuestion".into(),
            file_path: None,
            state: ToolState::Running,
            duration_ms: None,
            pictures: vec![],
            subagent: None,
            card: Some(ToolCard::Questions(questions)),
        };
        pane.model.items.insert(ItemId { offset: AT, block: 0 }, Item { kind, timestamp: None });
        pane.model.chat.prompt = Some(10);
        pane.model.chat.latest = Some((AT, claude_switch_core::transcript::ChatPhase::Active));
        pane.model.chat.pending = Some(AT);
        let tab = app
            .tabs
            .open(NewTab { label: "Q".into(), color: [0; 3], session: None, account: None }, Term::empty((70, 24)));
        app.tabs.get_mut(tab).unwrap().pane = Some(pane);
        Self { app, tab }
    }

    fn tab(&mut self) -> &mut Tab {
        self.app.tabs.get_mut(self.tab).unwrap()
    }

    fn chat(&mut self) -> &mut Chat {
        &mut self.tab().pane.as_mut().unwrap().chat
    }

    fn screen(&mut self, text: &str, now: Instant) {
        self.tab().term.feed(text.as_bytes());
        self.chat().output(now, true, false);
    }

    fn writes(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.chat().writes)
    }

    fn resolve(&mut self) {
        let pane = self.tab().pane.as_mut().unwrap();
        pane.model.chat.pending = None;
        if let Some(Item { kind: ItemKind::Tool { state, .. }, .. }) = pane.model.items.values_mut().next() {
            *state = ToolState::Ok;
        }
    }
}

fn down(n: usize) -> Vec<u8> {
    mdshared_term::input::encode_key(mdshared_term::Key::Down, mdshared_term::Mods::NONE, &Default::default()).repeat(n)
}

#[test]
fn the_screen_must_show_the_same_question_and_every_option_in_order() {
    let q = question(false);
    let mut screen = Screen::new(70, 24, 0);
    screen.feed(menu(0, None).as_bytes());
    let found = shown(&screen, &q).expect("the classic menu");
    assert_eq!((found.highlight(), found.other, found.chat), (Some(0), Some(2), Some(3)));

    let other = |change: &dyn Fn(&mut Question)| {
        let mut q = question(false);
        change(&mut q);
        shown(&screen, &q)
    };
    assert!(other(&|q| q.question = "Who tests the parser?".into()).is_none());
    assert!(other(&|q| q.options[1].label = "Them".into()).is_none());
    assert!(other(&|q| q.options.swap(0, 1)).is_none());
    assert!(other(&|q| q.options.push(QuestionOption { label: "Both".into(), description: String::new() })).is_none());
    assert!(other(&|q| q.options.truncate(1)).is_none(), "an unknown extra entry is not accepted");

    let mut no_footer = Screen::new(70, 24, 0);
    no_footer.feed(menu(0, None).replace("Enter to select · ↑/↓ to navigate · Esc to cancel", "").as_bytes());
    assert!(shown(&no_footer, &q).is_none());
}

#[test]
fn a_screen_that_looks_like_a_permission_question_is_never_answered() {
    let q = Question {
        header: String::new(),
        question: "Do you want to proceed?".into(),
        options: vec![
            QuestionOption { label: "Yes".into(), description: String::new() },
            QuestionOption { label: "No".into(), description: String::new() },
        ],
        multi_select: false,
    };
    let mut screen = Screen::new(70, 12, 0);
    screen.feed("Do you want to proceed?\r\n❯ 1. Yes\r\n  2. No\r\nEsc to cancel".as_bytes());
    assert!(crate::attention::screen_permission_prompt(&screen));
    assert!(shown(&screen, &q).is_none());
}

#[test]
fn an_option_moves_the_highlight_and_enter_follows_only_once_the_screen_shows_it() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen(&menu(0, None), now);
    assert!(f.tab().pane.as_mut().map(|p| p.chat.sync_question(&p.model)).unwrap());
    assert_eq!(f.chat().question.answerable, Some(AT));
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)], "arrows only; no Enter yet");
    assert!(f.chat().waiting().is_some(), "the composer waits meanwhile");

    // The screen has not moved yet: nothing more is written.
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    f.screen(&menu(1, None), now);
    advance(f.tab(), now, false);
    assert!(f.writes().is_empty(), "no write while the window does not allow one");
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);

    f.resolve();
    assert!(advance(f.tab(), now, true));
    assert!(f.chat().question.step.is_none() && f.chat().question.error.is_none());
    assert_eq!(f.chat().question.answerable, None);
}

#[test]
fn a_question_the_screen_does_not_show_fails_closed_and_opens_the_drawer() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen("\x1b[?2004h❯ ", now);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_SHOWN));
    assert_eq!(f.chat().drawer, Drawer::Manual);
    assert_eq!(f.chat().question.answerable, None, "the card no longer keeps the drawer closed");

    // A menu that never shows the chosen row stops at the deadline, before Enter.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(0, None), now);
    press(f.tab(), now, Hit::Option(1));
    f.writes();
    f.screen(&menu(0, None), now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_MOVED));
}

#[test]
fn a_typed_answer_is_entered_only_after_the_terminal_shows_it() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen(&menu(0, None), now);
    press(f.tab(), now, Hit::Answer);
    assert_eq!(f.chat().question.error, Some(BAD_TEXT), "an empty answer is not sent");
    f.chat().question.field.set_text("Both of us");
    press(f.tab(), now, Hit::Answer);
    assert_eq!(f.writes(), [down(2)]);
    f.screen(&menu(2, None), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"Both of us".to_vec()]);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the text is not on the screen yet");
    f.screen(&menu(2, Some("Both of us")), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn chat_about_this_leaves_the_question_and_keeps_the_text_for_the_composer() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen(&menu(3, None), now);
    f.chat().question.field.set_text("Let us talk first");
    press(f.tab(), now, Hit::ChatAbout);
    assert_eq!(f.writes(), [b"\r".to_vec()], "already highlighted: Enter only");
    f.resolve();
    advance(f.tab(), now, true);
    assert_eq!(f.chat().editor.text, "Let us talk first");
}

#[test]
fn several_questions_or_answers_stay_in_the_terminal_and_the_drawer_opens_as_before() {
    for questions in [vec![question(true)], vec![question(false), question(false)]] {
        let mut f = Fixture::new(questions);
        let now = Instant::now();
        f.screen(&menu(0, None), now);
        press(f.tab(), now, Hit::Option(0));
        assert!(f.writes().is_empty());
        assert_eq!(f.chat().question.error, Some(TERMINAL_ONLY));
    }
}

#[test]
fn an_answerable_question_blocks_sending_without_opening_the_drawer() {
    for answerable in [true, false] {
        let mut f = Fixture::new(vec![question(!answerable)]);
        let now = Instant::now();
        f.screen(&menu(0, None), now - QUIET);
        let pane = f.tab().pane.as_mut().unwrap();
        pane.chat.sync_question(&pane.model);
        pane.chat.observe(&pane.model.chat, now - super::super::TOOL_WAIT);
        let tab = f.tab();
        let pane = tab.pane.as_mut().unwrap();
        let decision = pane.chat.decision(&pane.model.chat, &tab.term, "", now);
        let reason = if answerable { Reason::Question } else { Reason::Tool };
        assert_eq!(decision, (Gate::Check, Some(reason)));
        pane.chat.react(decision.1);
        assert_eq!(pane.chat.drawer == Drawer::Closed, answerable);
    }
}
