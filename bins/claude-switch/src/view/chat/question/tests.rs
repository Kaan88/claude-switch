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
    let (found, other, chat) = question_menu(&screen, &q, false).expect("the classic menu");
    assert_eq!((found.highlight(), other, chat), (Some(0), Some(2), Some(3)));

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
    let tab = f.tab();
    let pane = tab.pane.as_mut().unwrap();
    assert!(pane.chat.sync_question(&pane.model, &tab.term.screen));
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
        let tab = f.tab();
        let pane = tab.pane.as_mut().unwrap();
        pane.chat.sync_question(&pane.model, &tab.term.screen);
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

/// A permission prompt as the classic renderer draws it.
fn prompt(command: &str, highlight: usize, comment: Option<&str>) -> String {
    let choices = ["Yes", "Yes, and don't ask again for cargo test commands", "No"];
    let mut rows = vec![
        "\x1b[2J\x1b[H────────────────────".to_owned(),
        " Bash command".to_owned(),
        String::new(),
        format!("   {command}"),
        "   Run the tests".to_owned(),
        String::new(),
        " Do you want to proceed?".to_owned(),
    ];
    for (i, choice) in choices.iter().enumerate() {
        let mark = if i == highlight { "❯" } else { " " };
        // An open comment field shows on the decline row (a guess, to confirm live).
        let comment = comment.filter(|_| i == 2).map_or(String::new(), |c| format!(": {c}"));
        rows.push(format!(" {mark} {}. {choice}{comment}", i + 1));
    }
    rows.push(String::new());
    rows.push(" Esc to cancel · Tab to amend".to_owned());
    rows.join("\r\n")
}

fn shell(id: &str, command: &str) -> ItemKind {
    ItemKind::Tool {
        id: id.into(),
        name: "Bash".into(),
        summary: command.into(),
        file_path: None,
        state: ToolState::Running,
        duration_ms: None,
        pictures: vec![],
        subagent: None,
        card: Some(ToolCard::Shell { command: command.into(), result: vec![] }),
    }
}

impl Fixture {
    fn tool(command: &str) -> Self {
        let mut f = Self::new(vec![]);
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model
            .items
            .insert(ItemId { offset: AT, block: 0 }, Item { kind: shell("toolu_1", command), timestamp: None });
        f
    }

    fn sync(&mut self) -> bool {
        let tab = self.tab();
        let pane = tab.pane.as_mut().unwrap();
        pane.chat.sync_question(&pane.model, &tab.term.screen)
    }

    fn decide(&mut self, now: Instant) -> Option<Reason> {
        let tab = self.tab();
        let pane = tab.pane.as_mut().unwrap();
        let decision = pane.chat.decision(&pane.model.chat, &tab.term, "", now);
        pane.chat.react(decision.1);
        decision.1
    }
}

#[test]
fn a_permission_prompt_for_the_pending_call_is_answered_from_the_card() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT));
    assert!(f.chat().question.approval);
    assert_eq!(f.decide(now), Some(Reason::Approval));
    assert_eq!(f.chat().drawer, Drawer::Closed, "the card answers it: no drawer");
    let shown = f.chat().question.shown.clone().unwrap();
    assert_eq!(shown.question, "Do you want to proceed?");
    assert_eq!(shown.choices.len(), 3);
    assert!(shown.body.iter().any(|row| row == "cargo test --workspace"));
    let tab = f.tab();
    assert_eq!(waiting_label(&tab.pane.as_ref().unwrap().model, &tab.term.screen), Some(APPROVING));

    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)], "nothing is pre-selected: the arrows first");
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    f.screen(&prompt("cargo test --workspace", 1, None), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn a_prompt_for_another_call_or_with_other_choices_is_left_to_the_terminal() {
    // The screen names another command.
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("rm -rf target", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    assert_eq!(f.decide(now), Some(Reason::Permission));
    assert_ne!(f.chat().drawer, Drawer::Closed);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_SHOWN_TOOL));

    // Two calls are pending: the prompt may be for either.
    let mut f = Fixture::tool("cargo test --workspace");
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model
        .items
        .insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: shell("toolu_0", "cargo build"), timestamp: None });
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());

    // The choices change between the click and the next step: nothing more is written.
    let mut f = Fixture::tool("cargo test --workspace");
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    press(f.tab(), now, Hit::Option(2));
    assert_eq!(f.writes(), [down(2)]);
    f.screen(&prompt("cargo test --workspace", 2, None).replace("3. No", "3. Never"), now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(CHANGED));
}

#[test]
fn declining_with_a_comment_opens_the_prompts_field_and_types_before_enter() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.chat().question.field.set_text("Run only the core tests");
    press(f.tab(), now, Hit::Deny);
    assert_eq!(f.writes(), [down(2)]);
    f.screen(&prompt("cargo test --workspace", 2, None), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\t".to_vec()], "Tab opens the comment");
    f.screen(&prompt("cargo test --workspace", 2, Some("")), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"Run only the core tests".to_vec()]);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the text is not on the screen yet");
    f.screen(&prompt("cargo test --workspace", 2, Some("Run only the core tests")), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);

    // Without a comment, the decline is a plain choice.
    let mut f = Fixture::tool("cargo test --workspace");
    f.screen(&prompt("cargo test --workspace", 2, None), now);
    press(f.tab(), now, Hit::Deny);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

/// The plan approval menu (its rows to confirm in a live session).
fn plan_menu(highlight: usize) -> String {
    let choices = ["Yes, and use auto mode", "Yes, manually approve edits", "No, keep planning"];
    let mut rows = vec!["\x1b[2J\x1b[H Ready to code?".to_owned(), String::new(), " Plan: build the parser".to_owned()];
    rows.push(String::new());
    rows.push(" Would you like to proceed?".to_owned());
    for (i, choice) in choices.iter().enumerate() {
        let mark = if i == highlight { "❯" } else { " " };
        rows.push(format!(" {mark} {}. {choice}", i + 1));
    }
    rows.push(String::new());
    rows.push(" Esc to cancel".to_owned());
    rows.join("\r\n")
}

#[test]
fn a_plan_is_approved_from_the_card_only_while_its_menu_shows() {
    let mut f = Fixture::new(vec![]);
    let kind = ItemKind::Tool {
        id: "toolu_1".into(),
        name: "ExitPlanMode".into(),
        summary: "ExitPlanMode".into(),
        file_path: None,
        state: ToolState::Running,
        duration_ms: None,
        pictures: vec![],
        subagent: None,
        card: Some(ToolCard::Plan("# Build the parser\n\n1. Tokens\n2. Tree".into())),
    };
    f.tab().pane.as_mut().unwrap().model.items.insert(ItemId { offset: AT, block: 0 }, Item { kind, timestamp: None });
    let now = Instant::now();

    // Not shown yet: nothing to answer, no buttons.
    f.screen("\x1b[?2004h❯ ", now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    assert!(f.chat().question.shown.is_none());

    f.screen(&plan_menu(0), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT));
    let shown = f.chat().question.shown.clone().unwrap();
    assert_eq!(shown.choices, ["Yes, and use auto mode", "Yes, manually approve edits", "No, keep planning"]);
    assert_eq!(shown.deny, Some(2));
    assert_eq!(f.decide(now), Some(Reason::Approval));
    let tab = f.tab();
    assert_eq!(waiting_label(&tab.pane.as_ref().unwrap().model, &tab.term.screen), Some(APPROVING));

    press(f.tab(), now, Hit::Option(1));
    assert_eq!(f.writes(), [down(1)]);
    f.screen(&plan_menu(1), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);

    // Keep planning with a note: no Tab field offered, so the note goes to the composer.
    let mut f2 = Fixture::new(vec![]);
    let kind = f.tab().pane.as_ref().unwrap().model.items.values().next().unwrap().kind.clone();
    f2.tab().pane.as_mut().unwrap().model.items.insert(ItemId { offset: AT, block: 0 }, Item { kind, timestamp: None });
    f2.screen(&plan_menu(2), now);
    f2.chat().question.field.set_text("Add error recovery first");
    press(f2.tab(), now, Hit::Deny);
    assert_eq!(f2.writes(), [b"\r".to_vec()]);
    f2.resolve();
    advance(f2.tab(), now + RESULT, true);
    assert_eq!(f2.chat().editor.text, "Add error recovery first");
}

#[test]
fn a_typed_row_that_loses_its_number_never_panics_and_stops_before_enter() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen(&menu(0, None), now);
    f.chat().question.field.set_text("Both of us");
    press(f.tab(), now, Hit::Answer);
    assert_eq!(f.writes(), [down(2)]);
    f.screen(&menu(2, None), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"Both of us".to_vec()]);
    // The typed row is drawn without its number and there is no "Chat about this": the menu has
    // only the two options, fewer entries than the typed row's index.
    let screen = "\x1b[2J\x1b[H\x1b[?2004h ☐ Who builds\r\n\r\nWho builds the parser?\r\n\r\n  1. Me\r\n     I write it\r\n  \
                  2. You\r\n     Claude writes it\r\n❯ Both of us\r\n\r\nEnter to select · ↑/↓ to navigate · Esc to cancel";
    f.screen(screen, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty(), "no Enter");
    assert_eq!(f.chat().question.error, Some(NOT_TYPED));
}
