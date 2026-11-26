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
    assert!(pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen));
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
fn text_for_the_composer_is_kept_until_it_lands_there() {
    // The composer holds a draft: the text waits, and lands once the composer is empty.
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.screen(&menu(3, None), now);
    f.chat().editor.set("my draft".into());
    f.chat().question.field.set_text("Let us talk first");
    press(f.tab(), now, Hit::ChatAbout);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    f.resolve();
    advance(f.tab(), now, true);
    assert_eq!(f.chat().editor.text, "my draft", "never over the user's draft");
    assert_eq!(f.chat().question.held.as_deref(), Some("Let us talk first"));
    assert_eq!(f.chat().held_notice(), Some(HELD));
    assert!(f.chat().question.field.text().is_empty(), "not left in the next card's field");
    f.chat().editor.set(String::new());
    advance(f.tab(), now, true);
    assert_eq!(f.chat().editor.text, "Let us talk first");
    assert_eq!(f.chat().question.held, None);

    // The prompt closes but the call stays pending past the deadline: the text still lands.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(3, None), now);
    f.chat().question.field.set_text("Let us talk first");
    press(f.tab(), now, Hit::ChatAbout);
    f.writes();
    f.screen("\x1b[2J\x1b[H\x1b[?2004h❯ ", now);
    advance(f.tab(), now + RESULT, true);
    assert!(f.chat().question.step.is_none());
    assert_eq!(f.chat().editor.text, "Let us talk first");
}

#[test]
fn a_card_that_goes_away_gives_the_keyboard_back() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    assert!(press(f.tab(), now, Hit::Field));
    assert!(f.chat().question.focus);
    // The prompt closes while the call stays pending: no card, so no hidden field takes keys.
    f.screen("\x1b[2J\x1b[H\x1b[?2004h❯ ", now);
    f.sync();
    assert!(!f.chat().question.focus);
    assert!(!key(f.tab(), now, &KeyName::Char('x'), Mods::default(), Some("x")));
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
        pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen);
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
        pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen)
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
fn a_queued_draft_lets_the_cards_answer_go_first_but_not_text_for_the_composer() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.chat().editor.set("next message".into());
    f.chat().queue_draft(0);
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)], "the queue waits for the turn, which waits for this answer");
    assert!(f.chat().locked(), "the draft stays queued");

    // "Chat about this" would put its text in the composer, which holds the queued draft.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(0, None), now);
    f.chat().editor.set("next message".into());
    f.chat().queue_draft(0);
    f.chat().question.field.set_text("Let's talk it through");
    assert!(press(f.tab(), now, Hit::ChatAbout));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(QUEUED_TEXT));
    assert_eq!(f.chat().question.field.text(), "Let's talk it through", "the text is kept");
}

/// A permission prompt whose "don't ask again" choice wraps onto three rows.
fn wrapped_prompt(command: &str) -> String {
    [
        "\x1b[2J\x1b[H────────────────────",
        " Bash command",
        "",
        &format!("   {command}"),
        "   Remove the build output",
        "",
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. Yes, and don't ask again for rm commands in",
        "      /home/someone/projects/a-rather-long-folder-name",
        "      /and-one-more-level",
        "   3. No, and tell Claude what to do differently (esc)",
        "",
        " Esc to cancel",
    ]
    .join("\r\n")
}

#[test]
fn a_permission_prompt_never_becomes_a_menu_card_even_when_its_choices_wrap() {
    let mut f = Fixture::tool("cargo test --workspace");
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model
        .items
        .insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: shell("toolu_0", "rm -rf target"), timestamp: None });
    let now = Instant::now();
    f.screen(&wrapped_prompt("rm -rf target"), now);
    assert!(crate::attention::screen_permission_prompt(&f.tab().term.screen), "wrapped choices are read");
    f.sync();
    assert!(f.chat().question.menu.is_none(), "never the generic menu card");
    assert_eq!(f.chat().question.answerable, None, "two calls pending: the terminal answers");
    assert_eq!(f.decide(now), Some(Reason::Permission));
    assert_ne!(f.chat().drawer, Drawer::Closed);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());

    // A screen that only looks like a permission prompt (its footer offers Tab to amend) is not
    // recognised, yet still never shows as a menu card.
    let mut f = Fixture::tool("cargo test --workspace");
    let odd = prompt("cargo test --workspace", 0, None).replace(" Do you want to proceed?", " Proceed?");
    f.screen(&odd, now);
    f.sync();
    assert!(f.chat().question.menu.is_none() && f.chat().question.shown.is_none());
    assert_eq!(f.decide(now), Some(Reason::Permission));
    assert_ne!(f.chat().drawer, Drawer::Closed);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());
}

#[test]
fn a_prompt_must_show_the_whole_command_or_full_path_and_no_subagent_may_be_asking() {
    let now = Instant::now();
    // A short command inside a longer one is not the call's prompt.
    let mut f = Fixture::tool("make");
    f.screen(&prompt("make install", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    f.screen(&prompt("make", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT), "the whole command, on its own row");

    // A subagent that is running (or that this turn started, end unknown) may be the one asking.
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model.agents.push(claude_switch_core::transcript::Subagent {
        id: "a1".into(),
        finished: Some(false),
        ..Default::default()
    });
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model.agents[0].finished = Some(true);
    let mut agent = shell("toolu_a", "");
    if let ItemKind::Tool { name, state, card, .. } = &mut agent {
        *name = "Agent".into();
        *state = ToolState::Ok;
        *card = None;
    }
    pane.model.items.insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: agent, timestamp: None });
    f.sync();
    assert_eq!(f.chat().question.answerable, None, "an agent of this turn with no known end");
    if let Some(Item { kind: ItemKind::Tool { subagent, .. }, .. }) =
        f.tab().pane.as_mut().unwrap().model.items.get_mut(&ItemId { offset: AT - 1, block: 0 })
    {
        *subagent = Some("a1".into());
    }
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT), "its subagent finished");
}

#[test]
fn a_path_or_host_matches_only_as_a_whole_word() {
    assert!(has_word("Do you want to make this edit to src/main.rs?", "src/main.rs"));
    assert!(!has_word("Do you want to make this edit to other/src/main.rs?", "src/main.rs"));
    assert!(!has_word("Edit src/main.rs.bak", "src/main.rs"));
    assert!(has_word("Claude wants to fetch content from example.com", "example.com"));
    assert!(!has_word("fetch from notexample.com", "example.com"));
    let edit = |path: &str| ItemKind::Tool {
        id: "e".into(),
        name: "Edit".into(),
        summary: String::new(),
        file_path: Some(path.into()),
        state: ToolState::Running,
        duration_ms: None,
        pictures: vec![],
        subagent: None,
        card: None,
    };
    let root = std::env::temp_dir().join("proj");
    let file = root.join("src").join("main.rs");
    let Some(Needle::Word(forms)) = needles(&edit(&file.to_string_lossy()), &root).pop() else { panic!() };
    assert!(forms.contains(&"src/main.rs".to_owned()) && forms.contains(&file.to_string_lossy().into_owned()));
    assert!(!forms.contains(&"main.rs".to_owned()), "never the bare file name");
    assert!(has_rows(&["Bash command".into(), "cargo test".into(), "--workspace".into()], "cargo test --workspace"));
    assert!(!has_rows(&["sudo cargo test --workspace".into()], "cargo test --workspace"));
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

#[test]
fn a_comment_is_typed_only_into_the_field_the_screen_shows_never_into_the_menu() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.chat().question.field.set_text("2 is wrong");
    press(f.tab(), now, Hit::Deny);
    f.writes();
    f.screen(&prompt("cargo test --workspace", 2, None), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"	".to_vec()]);
    // Newer output (a repaint) with the same menu: no field opened, so nothing is typed.
    f.screen(&prompt("cargo test --workspace", 2, None), now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "digits would pick a choice in the menu");
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NO_COMMENT));

    // A question's free-text row shows no field of its own: a leading digit is never typed.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(0, None), now);
    f.chat().question.field.set_text("2 of them");
    press(f.tab(), now, Hit::Answer);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(DIGIT_FIRST));
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

/// The native /model picker (its rows to confirm in a live session).
fn picker(highlight: usize) -> String {
    let choices = ["Default (recommended)  Opus 5.5", "Sonnet  Sonnet 5.5", "Haiku  Haiku 5.5"];
    let mut rows =
        vec!["\x1b[2J\x1b[H Select model".to_owned(), " Switch between Claude models.".to_owned(), String::new()];
    for (i, choice) in choices.iter().enumerate() {
        let mark = if i == highlight { "❯" } else { " " };
        rows.push(format!(" {mark} {}. {choice}", i + 1));
    }
    rows.push(String::new());
    rows.push(" Enter to confirm · Esc to exit".to_owned());
    rows.join("\r\n")
}

impl Fixture {
    /// Nothing pending: a menu the user opened with a command.
    fn idle() -> Self {
        let mut f = Self::new(vec![]);
        let pane = f.tab().pane.as_mut().unwrap();
        pane.model.items.clear();
        pane.model.chat.pending = None;
        pane.model.chat.latest = Some((AT, claude_switch_core::transcript::ChatPhase::EndTurn));
        f
    }
}

#[test]
fn a_cli_menu_is_answered_from_its_card_and_keeps_the_drawer_closed() {
    let mut f = Fixture::idle();
    let now = Instant::now();
    f.screen(&picker(0), now);
    f.sync();
    let menu = f.chat().question.menu.clone().expect("a menu card");
    assert_eq!(menu.title, "Switch between Claude models.");
    assert_eq!(menu.choices.len(), 3);
    assert!(menu.esc);
    assert_eq!(f.decide(now), Some(Reason::Menu));
    assert_eq!(f.chat().drawer, Drawer::Closed);

    assert!(press(f.tab(), now, Hit::Option(2)));
    assert_eq!(f.writes(), [down(2)]);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "not moved yet");
    f.screen(&picker(2), now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // The menu closed: the step ends at once, without waiting for the deadline.
    f.screen("\x1b[2J\x1b[H\x1b[?2004h❯ ", now);
    assert!(advance(f.tab(), now, true));
    assert!(f.chat().question.step.is_none() && f.chat().question.menu.is_none());
    assert!(f.chat().question.error.is_none());

    // Other choices by the time the highlight moved: nothing more is written.
    let mut f = Fixture::idle();
    f.screen(&picker(0), now);
    press(f.tab(), now, Hit::Option(1));
    f.writes();
    f.screen(&picker(1).replace("Sonnet  Sonnet 5.5", "Sonnet  Sonnet 4.6"), now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(CHANGED));
}

/// A settings menu whose rows toggle and stay open (its look to confirm in a live session).
fn toggles(highlight: usize, verbose: bool) -> String {
    let state = |on: bool| if on { "✔ on" } else { "off" };
    let choices = [format!("Verbose output  {}", state(verbose)), format!("Auto-compact  {}", state(true))];
    let mut rows = vec!["\x1b[2J\x1b[H Settings".to_owned(), String::new()];
    for (i, choice) in choices.iter().enumerate() {
        let mark = if i == highlight { "❯" } else { " " };
        rows.push(format!(" {mark} {}. {choice}", i + 1));
    }
    rows.push(String::new());
    rows.push(" Enter to toggle · Esc to close".to_owned());
    rows.join("\r\n")
}

#[test]
fn a_toggle_that_keeps_its_menu_open_is_done_once_its_row_changed() {
    let mut f = Fixture::idle();
    let now = Instant::now();
    f.screen(&toggles(0, false), now);
    f.sync();
    assert!(f.chat().question.menu.is_some());
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    f.screen(&toggles(0, true), now);
    assert!(advance(f.tab(), now, true));
    assert!(f.chat().question.step.is_none());
    assert_eq!(f.chat().question.error, None);
    assert_eq!(f.chat().drawer, Drawer::Closed);
    f.sync();
    assert!(f.chat().question.menu.as_ref().unwrap().choices[0].ends_with("✔ on"), "the card shows the new state");

    // Nothing changed by the deadline: that is still a failure.
    assert!(press(f.tab(), now, Hit::Option(0)));
    f.writes();
    f.screen(&toggles(0, true), now);
    advance(f.tab(), now + RESULT, true);
    assert_eq!(f.chat().question.error, Some(NOT_CLOSED));
}

#[test]
fn a_ctrl_b_in_a_hidden_tab_still_ends() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&running(true), now);
    f.sync();
    assert!(press(f.tab(), now, Hit::Background));
    assert!(f.chat().waiting().is_some(), "chat work, so a hidden tab progresses too");
    let id = f.tab;
    assert!(f.app.tabs.activate(crate::tabs::SESSIONS));
    f.app.progress_chat_tabs(now + STEP, true);
    let chat = &f.app.tabs.get(id).unwrap().pane.as_ref().unwrap().chat;
    assert!(chat.question.background_step.is_none());
}

#[test]
fn a_menu_without_numbered_rows_is_shown_read_only_and_an_old_one_is_ignored() {
    let mut f = Fixture::idle();
    let now = Instant::now();
    f.screen("\x1b[2J\x1b[H Settings\r\n  Theme  Dark\r\n  Verbose  false\r\n\r\n Esc to cancel", now);
    f.sync();
    let menu = f.chat().question.menu.clone().expect("a read-only card");
    assert!(menu.choices.is_empty());
    assert_eq!(menu.rows, ["Settings", "Theme Dark", "Verbose false"]);
    assert!(!press(f.tab(), now, Hit::Option(0)));
    assert!(f.writes().is_empty());

    let mut f = Fixture::idle();
    f.screen(&format!("{}\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n\x1b[?2004h❯ ", picker(0)), now);
    f.sync();
    assert!(f.chat().question.menu.is_none(), "a menu far above the input is old output");
}

fn running(hint: bool) -> String {
    let hint = if hint { "  ctrl+b to run in background" } else { "  esc to interrupt" };
    format!("\x1b[2J\x1b[H● Bash(npm test)\r\n  ⎿  Running…\r\n\r\n✻ Testing… (12s)\r\n{hint}")
}

#[test]
fn run_in_background_is_offered_only_with_the_clis_hint_and_waits_for_it_to_go() {
    let mut f = Fixture::tool("npm test");
    let now = Instant::now();
    f.screen(&running(false), now);
    f.sync();
    assert!(f.chat().question.background.is_none(), "no hint, no button");
    assert!(press(f.tab(), now, Hit::Background));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NO_HINT));

    let mut f = Fixture::tool("npm test");
    f.screen(&running(true), now);
    f.sync();
    assert_eq!(f.chat().question.background.as_ref().map(|b| b.1.as_str()), Some("npm test"));
    assert!(press(f.tab(), now, Hit::Background));
    assert_eq!(f.writes(), [CTRL_B.to_vec()]);
    press(f.tab(), now, Hit::Background);
    assert!(f.writes().is_empty(), "one at a time");
    assert_eq!(f.chat().question.error, Some(WAITING));
    advance(f.tab(), now, true);
    assert!(f.chat().question.background_step.is_some(), "the same screen proves nothing");
    f.screen(&running(false), now);
    advance(f.tab(), now, true);
    assert!(f.chat().question.background_step.is_none() && f.chat().question.error.is_none());

    // The hint stays: the CLI did not take it.
    let mut f = Fixture::tool("npm test");
    f.screen(&running(true), now);
    press(f.tab(), now, Hit::Background);
    f.screen(&running(true), now);
    advance(f.tab(), now + STEP, true);
    assert_eq!(f.chat().question.error, Some(NOT_BACKGROUND));

    // Two calls pending: the hint may be for either.
    let mut f = Fixture::tool("npm test");
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model
        .items
        .insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: shell("toolu_0", "cargo build"), timestamp: None });
    f.screen(&running(true), now);
    f.sync();
    assert!(f.chat().question.background.is_none());
}

#[test]
fn esc_leaves_a_menu_or_declines_a_prompt_only_while_the_screen_shows_it() {
    let now = Instant::now();
    // A menu, readable or not: Esc, then done once the menu is gone.
    for screen in [picker(0), "\x1b[2J\x1b[H Settings\r\n  Theme  Dark\r\n\r\n Esc to cancel".to_owned()] {
        let mut f = Fixture::idle();
        f.screen(&screen, now);
        assert!(press(f.tab(), now, Hit::Escape));
        assert_eq!(f.writes(), [ESC.to_vec()]);
        assert!(!press(f.tab(), now, Hit::Escape), "one step at a time");
        f.screen("\x1b[2J\x1b[H\x1b[?2004h❯ ", now);
        advance(f.tab(), now, true);
        assert!(f.chat().question.step.is_none() && f.chat().question.error.is_none());
    }

    // A footer without Esc: nothing is written.
    let mut f = Fixture::idle();
    f.screen(&picker(0).replace("Enter to confirm · Esc to exit", "Enter to select"), now);
    press(f.tab(), now, Hit::Escape);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NO_ESC));

    // The pending call's permission prompt: Esc declines it.
    let mut f = Fixture::tool("cargo test --workspace");
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    assert!(press(f.tab(), now, Hit::Escape));
    assert_eq!(f.writes(), [ESC.to_vec()]);

    // Another call's prompt, or a question the screen does not show: nothing.
    let mut f = Fixture::tool("cargo test --workspace");
    f.screen(&prompt("rm -rf target", 0, None), now);
    press(f.tab(), now, Hit::Escape);
    assert!(f.writes().is_empty());
    let mut f = Fixture::new(vec![question(false)]);
    f.screen("\x1b[?2004h❯ ", now);
    press(f.tab(), now, Hit::Escape);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_SHOWN));
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(1, None), now);
    press(f.tab(), now, Hit::Escape);
    assert_eq!(f.writes(), [ESC.to_vec()]);
}

#[test]
fn words_in_the_conversation_above_a_menu_do_not_make_it_a_permission_prompt() {
    // Claude's last message asked something; then the user opened a picker. The CLI draws its
    // menus under a rule, so only the rows below the nearest rule are the menu's own.
    let mut f = Fixture::idle();
    let now = Instant::now();
    let above = "\x1b[2J\x1b[H● Do you want to keep the old parser? Would you like to see the diff?\r\n\r\n";
    let rule = "────────────────────\r\n";
    f.screen(&format!("{above}{rule}{}", picker(0).replace("\x1b[2J\x1b[H", "")), now);
    assert!(!screens::permission_like(&f.tab().term.screen));
    f.sync();
    assert_eq!(f.chat().question.menu.as_ref().map(|m| m.choices.len()), Some(3), "the picker's card");
    assert_eq!(f.decide(now), Some(Reason::Menu));
    assert_eq!(f.chat().drawer, Drawer::Closed);

    // The prompt's own rows still count, with or without a rule above them.
    for rule in [rule, ""] {
        let odd = prompt("cargo test --workspace", 0, None).replace(" Do you want to proceed?", " Proceed?");
        let mut f = Fixture::tool("cargo test --workspace");
        f.screen(&format!("{above}{rule}{}", odd.replace("\x1b[2J\x1b[H", "")), now);
        assert!(screens::permission_like(&f.tab().term.screen), "Tab to amend is in its footer");
        let asks = picker(0).replace(" Switch between Claude models.", " Do you want to switch models?");
        let mut f = Fixture::idle();
        f.screen(&format!("\x1b[2J\x1b[H{rule}{}", asks.replace("\x1b[2J\x1b[H", "")), now);
        assert!(screens::permission_like(&f.tab().term.screen), "the question is the menu's own row");
    }
}

#[test]
fn a_queued_draft_stays_queued_while_a_cards_keys_are_on_their_way() {
    // The turn ended and the menu closed, but the card's answer is not known to have landed yet
    // (no window focus, so nothing newer was read). The queue must neither send past it nor be
    // dropped without a word.
    let mut f = Fixture::idle();
    let now = Instant::now();
    f.chat().demo_progress = true;
    f.screen(&picker(0), now);
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    f.tab().term.feed("\x1b[2J\x1b[H\x1b[?2004h❯ ".as_bytes());
    f.chat().editor.set("next message".into());
    f.chat().queue_draft(0);
    AppState::advance_submission(f.tab(), now, false);
    AppState::advance_submission(f.tab(), now, true);
    assert!(f.chat().question.step.is_some(), "the answer is still in flight");
    assert!(f.writes().is_empty());
    assert!(f.chat().queue.is_some(), "still queued");

    // Once the screen shows the menu gone, the step ends and the draft goes.
    f.screen("\x1b[2J\x1b[H\x1b[?2004h❯ ", now);
    AppState::advance_submission(f.tab(), now, true);
    AppState::advance_submission(f.tab(), now, true);
    assert!(f.chat().question.step.is_none());
    assert!(f.chat().queue.is_none() && f.chat().editor.text.is_empty(), "sent");
}

#[test]
fn the_busy_notice_goes_once_the_other_keys_have_gone_even_with_a_draft_queued() {
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.chat().editor.set("next message".into());
    f.chat().queue_draft(0);
    f.sync();
    f.chat().question.background_step = Some((ItemId { offset: AT, block: 0 }, now + STEP, 0));
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(WAITING));
    f.chat().question.background_step = None;
    f.sync();
    assert_eq!(f.chat().question.error, None, "the queued draft is no key on its way");
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}
