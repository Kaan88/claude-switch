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
            QuestionOption { label: "Me".into(), description: "I write it".into(), preview: String::new() },
            QuestionOption { label: "You".into(), description: "Claude writes it".into(), preview: String::new() },
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
            denial: None,
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
    assert!(
        other(&|q| q.options.push(QuestionOption {
            label: "Both".into(),
            description: String::new(),
            preview: String::new()
        }))
        .is_none()
    );
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
            QuestionOption { label: "Yes".into(), description: String::new(), preview: String::new() },
            QuestionOption { label: "No".into(), description: String::new(), preview: String::new() },
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
fn questions_worded_alike_stay_in_the_terminal_and_the_drawer_opens_as_before() {
    let mut f = Fixture::new(vec![question(false), question(false)]);
    let now = Instant::now();
    f.screen(&menu(0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None, "the screen could not tell them apart");
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(TERMINAL_ONLY));
}

#[test]
fn an_answerable_question_blocks_sending_without_opening_the_drawer() {
    for answerable in [true, false] {
        let questions = if answerable { vec![question(true)] } else { vec![question(false), question(false)] };
        let mut f = Fixture::new(questions);
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
        // Neither opens the drawer: the card answers it, or `terminal_card` shows the screen.
        assert_eq!(pane.chat.drawer, Drawer::Closed);
    }
}

/// A permission prompt as the classic renderer draws it, under its full-width rule (the test
/// terminal is 70 columns wide).
fn prompt(command: &str, highlight: usize, comment: Option<&str>) -> String {
    let choices = ["Yes", "Yes, and don't ask again for cargo test commands", "No"];
    let mut rows = vec![
        format!("\x1b[2J\x1b[H{}", "─".repeat(70)),
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
        denial: None,
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
fn a_prompt_for_another_call_is_shown_as_the_screen_has_it_never_as_the_pending_call() {
    // The screen names another command than the pending call: the card shows the screen's prompt,
    // not the transcript's call, and answers only what it has shown.
    let mut f = Fixture::tool("cargo test --workspace");
    let now = Instant::now();
    f.screen(&prompt("rm -rf target", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None, "not the pending call's card");
    assert!(f.chat().question.shown.is_none() && !f.chat().question.approval);
    let shown = f.chat().question.prompt.clone().expect("the prompt, read from the screen");
    assert!(shown.whole);
    assert_eq!(shown.rows, ["Bash command", "  rm -rf target", "  Run the tests"]);
    assert_eq!((shown.question.as_str(), shown.choices.len()), ("Do you want to proceed?", 3));
    assert_eq!(f.decide(now), Some(Reason::Approval));
    assert_eq!(f.chat().drawer, Drawer::Closed);
    // Not before the card has painted it, and not in the same moment.
    press(f.tab(), now, Hit::Option(0));
    assert_eq!((f.writes().len(), f.chat().question.error), (0, Some(prompt::READ_ALL)));
    f.chat().question.prompt_painted(now);
    press(f.tab(), now, Hit::Option(0));
    assert_eq!((f.writes().len(), f.chat().question.error), (0, Some(prompt::NOT_SEEN)));
    press(f.tab(), now + prompt::SEEN, Hit::Option(0));
    assert_eq!(f.writes(), [b"\r".to_vec()], "the highlight is on Yes already");

    // Two calls are pending: the prompt may be for either, so the pending call's card is not
    // the one shown. The screen's prompt is.
    let mut f = Fixture::tool("cargo test --workspace");
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model
        .items
        .insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: shell("toolu_0", "cargo build"), timestamp: None });
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, None);
    assert!(f.chat().question.prompt.is_some() && f.chat().question.shown.is_none());
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty(), "nothing before the card showed it");

    // A prompt whose top rule is not on the screen cannot be read whole: the terminal answers.
    let mut f = Fixture::tool("cargo test --workspace");
    let cut = prompt("rm -rf target", 0, None).replace(&"─".repeat(70), "");
    f.screen(&cut, now);
    f.sync();
    assert!(f.chat().question.prompt.as_ref().is_some_and(|p| !p.whole));
    assert_eq!(f.decide(now), Some(Reason::Permission));
    assert_ne!(f.chat().drawer, Drawer::Closed);
    f.chat().question.prompt_painted(now);
    press(f.tab(), now + prompt::SEEN, Hit::Option(0));
    assert_eq!((f.writes().len(), f.chat().question.error), (0, Some(prompt::TOP_HIDDEN)));
    press(f.tab(), now + prompt::SEEN, Hit::Escape);
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
        &format!("\x1b[2J\x1b[H{}", "─".repeat(70)),
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
    assert_eq!(f.chat().question.answerable, None, "two calls pending: not the pending call's card");
    let shown = f.chat().question.prompt.clone().expect("the prompt, read from the screen");
    assert_eq!(
        shown.choices[1],
        "Yes, and don't ask again for rm commands in /home/someone/projects/a-rather-long-folder-name /and-one-more-level"
    );
    assert_eq!(f.decide(now), Some(Reason::Approval));
    assert_eq!(f.chat().drawer, Drawer::Closed);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty(), "nothing before the card showed it");

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
        denial: None,
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
        denial: None,
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

impl Fixture {
    /// A screen recorded from Claude Code, as newer output.
    fn show(&mut self, fixture: &str, now: Instant) {
        self.tab().term.screen = screens::recorded::recorded(fixture);
        self.chat().output(now, true, false);
    }

    /// A background agent the transcript knows, not known to have finished.
    fn agent(&mut self, description: &str) {
        let pane = self.tab().pane.as_mut().unwrap();
        pane.model.agents.push(claude_switch_core::transcript::Subagent {
            id: format!("a-{description}"),
            agent_type: Some("general-purpose".into()),
            description: Some(description.into()),
            ..Default::default()
        });
    }
}

const PING: &str = "ping -n 40 127.0.0.1";

#[test]
fn recorded_background_screens_read_as_offered_and_moved() {
    use screens::recorded::*;
    let hint = recorded(BG_HINT);
    assert!(screens::background_hint(&hint, "Bash", PING));
    assert!(!screens::background_hint(&hint, "Bash", "ping -n 41 127.0.0.1"), "another command");
    assert!(!screens::background_hint(&hint, "PowerShell", PING), "another tool");
    assert!(!screens::backgrounded(&hint, "Bash", PING));
    let moved = recorded(BG_MOVED);
    assert!(screens::backgrounded(&moved, "Bash", PING));
    assert!(!screens::background_hint(&moved, "Bash", PING));
    // The hint row of the input box offers the agent list, but no shell command is offered there.
    assert!(!screens::background_hint(&recorded(AGENTS_IDLE), "Bash", PING));
}

#[test]
fn run_in_background_is_offered_only_under_its_calls_hint_and_ends_once_it_moved() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::tool(PING);
    f.show(REWIND_IDLE, now);
    f.sync();
    assert!(f.chat().question.background.is_none(), "no hint, no button");
    assert!(press(f.tab(), now, Hit::Background));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(background::NO_HINT));

    let mut f = Fixture::tool(PING);
    f.show(BG_HINT, now);
    f.sync();
    assert_eq!(f.chat().question.background.as_ref().map(|b| b.command.as_str()), Some(PING));
    assert!(press(f.tab(), now, Hit::Background));
    assert_eq!(f.writes(), [b"\x02".to_vec()]);
    press(f.tab(), now, Hit::Background);
    assert!(f.writes().is_empty(), "one at a time");
    assert_eq!(f.chat().question.error, Some(WAITING));
    advance(f.tab(), now, true);
    assert!(f.chat().question.background_step.is_some(), "the same screen proves nothing");
    f.show(BG_MOVED, now);
    advance(f.tab(), now, true);
    assert!(f.chat().question.background_step.is_none() && f.chat().question.error.is_none());

    // The hint stays: the CLI did not take it.
    let mut f = Fixture::tool(PING);
    f.show(BG_HINT, now);
    press(f.tab(), now, Hit::Background);
    f.show(BG_HINT, now);
    advance(f.tab(), now + STEP, true);
    assert_eq!(f.chat().question.error, Some(background::NOT_BACKGROUND));

    // The call ended meanwhile: nothing to wait for.
    let mut f = Fixture::tool(PING);
    f.show(BG_HINT, now);
    press(f.tab(), now, Hit::Background);
    f.resolve();
    advance(f.tab(), now, true);
    assert!(f.chat().question.background_step.is_none() && f.chat().question.error.is_none());

    // Two calls pending: the hint may be for either.
    let mut f = Fixture::tool(PING);
    let pane = f.tab().pane.as_mut().unwrap();
    pane.model
        .items
        .insert(ItemId { offset: AT - 1, block: 0 }, Item { kind: shell("toolu_0", "cargo build"), timestamp: None });
    f.show(BG_HINT, now);
    f.sync();
    assert!(f.chat().question.background.is_none());
}

#[test]
fn a_ctrl_b_in_a_hidden_tab_still_ends() {
    use screens::recorded::*;
    let mut f = Fixture::tool(PING);
    let now = Instant::now();
    f.show(BG_HINT, now);
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
fn recorded_agent_lists_read_with_their_keys_and_states() {
    use screens::recorded::*;
    use screens::{AgentState, ListKeys};
    let list = |fixture: &str| screens::agent_list(&recorded(fixture));
    let idle = list(AGENTS_IDLE).expect("the list under the idle input");
    assert_eq!((idle.keys, idle.highlight), (ListKeys::Input, None));
    assert_eq!(idle.names(), [("main", ""), ("general-purpose", "First pinger"), ("general-purpose", "Second pinger")]);
    assert_eq!(idle.rows[1].state, AgentState::Running);
    assert!(crate::view::chat::submission::screen::clean_prompt(&recorded(AGENTS_IDLE)));
    let shown = |fixture: &str| list(fixture).map(|l| (l.keys, l.highlight));
    assert_eq!(shown(AGENTS_OPEN), Some((ListKeys::Select, Some(0))));
    assert_eq!(shown(AGENTS_FIRST), Some((ListKeys::Stop, Some(1))));
    assert_eq!(shown(AGENTS_SECOND), Some((ListKeys::Stop, Some(2))));
    assert_eq!(shown(AGENTS_STOPPED), Some((ListKeys::Clear, Some(2))));
    assert_eq!(list(AGENTS_STOPPED).unwrap().rows[2].state, AgentState::Stopped);
    assert!(!crate::view::chat::submission::screen::clean_prompt(&recorded(AGENTS_OPEN)), "the list has the keys");
    // After a stop the hint row offers "↓ to resume subagent": not the list Chat drives.
    assert_eq!(shown(AGENTS_CLOSED), None);
    assert!(crate::view::chat::submission::screen::clean_prompt(&recorded(AGENTS_CLOSED)));
    // Mid-turn, with a shell command running too.
    let shell = list(AGENTS_SHELL).expect("the list while a turn runs");
    assert_eq!((shell.keys, shell.rows.len()), (ListKeys::Input, 2));
    assert!(list(BG_HINT).is_none() && list(REWIND_IDLE).is_none());
}

#[test]
fn stop_agent_walks_the_list_one_checked_key_at_a_time() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    f.show(AGENTS_IDLE, now);
    f.sync();
    assert!(f.chat().question.agents.is_empty(), "the transcript does not know these agents");
    f.agent("First pinger");
    f.agent("Second pinger");
    f.sync();
    let offered: Vec<_> = f.chat().question.agents.iter().map(|a| (a.description.clone(), a.row)).collect();
    assert_eq!(offered, [("First pinger".to_owned(), 1), ("Second pinger".to_owned(), 2)]);
    assert!(f.chat().question.menu.is_none());

    assert!(press(f.tab(), now, Hit::StopAgent(1)));
    assert_eq!(f.writes(), [down(1)]);
    assert!(f.chat().waiting().is_some(), "the composer waits meanwhile");
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the same screen proves nothing");
    f.show(AGENTS_OPEN, now);
    advance(f.tab(), now, false);
    assert!(f.writes().is_empty(), "no write while the window does not allow one");
    assert!(f.chat().question.menu.is_none(), "the list is driven, not shown as a menu card");
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [down(1)]);
    f.show(AGENTS_FIRST, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [down(1)], "one row at a time, past the first agent");
    f.show(AGENTS_SECOND, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"x".to_vec()]);
    f.show(AGENTS_STOPPED, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.show(AGENTS_CLOSED, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    assert!(f.chat().question.agent_step.is_none() && f.chat().question.error.is_none());
    assert!(f.chat().waiting().is_none());
}

#[test]
fn stop_agent_stops_when_the_screen_shows_anything_else() {
    use screens::recorded::*;
    let now = Instant::now();
    let start = |f: &mut Fixture| {
        f.agent("First pinger");
        f.agent("Second pinger");
        f.show(AGENTS_IDLE, now);
        f.sync();
    };
    // The list did not open: no more keys, a notice and the drawer.
    let mut f = Fixture::idle();
    start(&mut f);
    press(f.tab(), now, Hit::StopAgent(0));
    f.writes();
    f.show(REWIND_IDLE, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    advance(f.tab(), now + STEP, true);
    assert_eq!(f.chat().question.error, Some(background::NO_LIST));
    assert_eq!(f.chat().drawer, Drawer::Manual);
    assert!(f.chat().question.agent_step.is_none());

    // The highlight landed elsewhere than the next row.
    let mut f = Fixture::idle();
    start(&mut f);
    press(f.tab(), now, Hit::StopAgent(0));
    f.writes();
    f.show(AGENTS_SECOND, now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty(), "never x on another agent");
    assert_eq!(f.chat().question.error, Some(background::NO_LIST));

    // x did not stop it.
    let mut f = Fixture::idle();
    start(&mut f);
    press(f.tab(), now, Hit::StopAgent(1));
    for screen in [AGENTS_OPEN, AGENTS_FIRST, AGENTS_SECOND] {
        f.show(screen, now);
        advance(f.tab(), now, true);
    }
    assert_eq!(f.writes().last(), Some(&b"x".to_vec()));
    f.show(AGENTS_SECOND, now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(background::NOT_STOPPED));

    // The agent the card offers is gone from the list by the click: nothing is written.
    let mut f = Fixture::idle();
    start(&mut f);
    f.tab().term.screen = recorded(AGENTS_CLOSED);
    assert!(press(f.tab(), now, Hit::StopAgent(1)));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(background::NO_AGENT));

    // An agent known to have finished, or one the transcript holds twice, is not offered.
    let mut f = Fixture::idle();
    start(&mut f);
    f.tab().pane.as_mut().unwrap().model.agents[0].finished = Some(true);
    f.agent("Second pinger");
    f.sync();
    assert!(f.chat().question.agents.is_empty());
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

    // Another call's prompt is the screen's own card: Esc declines what that card shows.
    let mut f = Fixture::tool("cargo test --workspace");
    f.screen(&prompt("rm -rf target", 0, None), now);
    assert!(press(f.tab(), now, Hit::Escape));
    assert_eq!(f.writes(), [ESC.to_vec()]);
    assert!(
        matches!(f.chat().question.step.as_ref().map(|s| &s.subject), Some(Subject::Prompt(p)) if p.rows[1] == "  rm -rf target")
    );
    // A question the screen does not show: nothing.
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
    let shell = background::Shell { item: ItemId { offset: AT, block: 0 }, tool: "Bash".into(), command: "x".into() };
    f.chat().question.background_step = Some((shell, now + STEP, 0));
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(WAITING));
    f.chat().question.background_step = None;
    f.sync();
    assert_eq!(f.chat().question.error, None, "the queued draft is no key on its way");
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn a_question_worded_like_a_permission_prompt_never_opens_the_drawer_for_a_tick() {
    // "Do you want to …" is how Claude often asks. The gate must know the question card answers
    // this menu before it judges the screen, in the same tick the call and the menu appear.
    let mut q = question(false);
    q.question = "Do you want to build the parser?".into();
    let mut f = Fixture::new(vec![q]);
    let now = Instant::now();
    f.app.tabs.active = Some(f.tab);
    f.screen(&menu(0, None).replace("Who builds the parser?", "Do you want to build the parser?"), now);
    assert!(screens::permission_like(&f.tab().term.screen), "the look-alike rule alone would refuse it");
    f.app.progress_chat_tabs(now, true);
    assert_eq!(f.chat().drawer, Drawer::Closed);
    assert_ne!(f.chat().reason, Some(Reason::Permission));
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

fn fruits() -> Question {
    let option =
        |label: &str| QuestionOption { label: label.into(), description: String::new(), preview: String::new() };
    Question {
        header: "Fruits".into(),
        question: "Which fruits do you like?".into(),
        options: vec![option("Apple"), option("Pear"), option("Plum")],
        multi_select: true,
    }
}

fn size() -> Question {
    let option =
        |label: &str| QuestionOption { label: label.into(), description: String::new(), preview: String::new() };
    Question {
        header: "Size".into(),
        question: "Which size?".into(),
        options: vec![option("Small"), option("Large")],
        multi_select: false,
    }
}

fn speed() -> Question {
    let option =
        |label: &str| QuestionOption { label: label.into(), description: String::new(), preview: String::new() };
    let options = vec![option("Slow"), option("Fast")];
    Question { header: "Speed".into(), question: "Which speed?".into(), options, multi_select: false }
}

/// Question `at` of a form as Claude Code 2.1.332 draws it (see the recorded `form_*` screens):
/// the tab row, the question, its entries with boxes for a multi-select, the unnumbered Next or
/// Submit row, a rule, "Chat about this". `cursor` counts the rows the arrow keys walk.
fn form_screen(questions: &[Question], at: usize, cursor: usize, marks: &[bool], typed: Option<&str>) -> String {
    let q = &questions[at];
    let rule = "─".repeat(40);
    let tabs: Vec<String> = questions.iter().map(|q| format!("☐ {}", q.header)).collect();
    let mut rows = vec![format!("\x1b[2J\x1b[H\x1b[?2004h{rule}"), format!("←  {}  ✔ Submit  →", tabs.join("  "))];
    rows.extend([String::new(), q.question.clone(), String::new()]);
    let mark = |i: usize| if i == cursor { "❯" } else { " " };
    let n = q.options.len();
    for (i, option) in q.options.iter().enumerate() {
        let tick = if q.multi_select { if marks[i] { "[✔] " } else { "[ ] " } } else { "" };
        rows.push(format!("{} {}. {tick}{}", mark(i), i + 1, option.label));
    }
    let other = match (q.multi_select, typed) {
        (true, Some(text)) => format!("[✔] {text}"),
        (true, None) => "[ ] Type something".to_owned(),
        (false, Some(text)) => text.to_owned(),
        (false, None) => "Type something.".to_owned(),
    };
    rows.push(format!("{} {}. {other}", mark(n), n + 1));
    let mut chat = n + 1;
    if q.multi_select {
        let label = if at + 1 < questions.len() { "Next" } else { "Submit" };
        rows.push(format!("{}    {label}", mark(n + 1)));
        chat += 1;
    }
    rows.push(rule);
    rows.push(format!("{} {}. Chat about this", mark(chat), n + 2));
    rows.push(String::new());
    rows.push("Enter to select · Tab/Arrow keys to navigate · Esc to cancel".to_owned());
    rows.join("\r\n")
}

/// The review after a form's last question, as recorded: no footer.
fn review_screen(questions: &[Question], answers: &[&str], highlight: usize) -> String {
    let tabs: Vec<String> = questions.iter().map(|q| format!("☒ {}", q.header)).collect();
    let mut rows = vec![
        format!("\x1b[2J\x1b[H{}", "─".repeat(40)),
        format!("←  {}  ✔ Submit  →", tabs.join("  ")),
        String::new(),
        "Review your answers".to_owned(),
        String::new(),
    ];
    for (q, answer) in questions.iter().zip(answers) {
        rows.push(format!(" ● {}", q.question));
        rows.push(format!("   → {answer}"));
    }
    rows.push(String::new());
    for (i, entry) in ["Submit answers", "Cancel"].iter().enumerate() {
        rows.push(format!("{} {}. {entry}", if i == highlight { "❯" } else { " " }, i + 1));
    }
    rows.join("\r\n")
}

impl Fixture {
    /// Feeds the screen, advances once, and returns what was written.
    fn step(&mut self, screen: &str, now: Instant) -> Vec<Vec<u8>> {
        self.screen(screen, now);
        advance(self.tab(), now, true);
        self.writes()
    }
}

#[test]
fn a_form_is_answered_question_by_question_and_submitted_only_once_the_review_matches() {
    let qs = [fruits(), size()];
    let mut f = Fixture::new(qs.to_vec());
    let now = Instant::now();
    let (none, f_t_t) = ([false; 3], [false, true, true]);
    f.screen(&form_screen(&qs, 0, 0, &none, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT), "a form is answered from the card");

    // Picks stay in the card: nothing is written before Submit.
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert!(press(f.tab(), now, Hit::Option(2)));
    f.chat().question.field.set_text("Kiwi");
    assert!(press(f.tab(), now, Hit::Submit));
    assert_eq!(f.chat().question.error, Some(UNANSWERED), "the size is not answered yet");
    assert_eq!(f.chat().question.form.page, 1, "the card shows that question");
    assert!(f.chat().question.field.text().is_empty());
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert!(press(f.tab(), now, Hit::Back));
    assert_eq!(f.chat().question.field.text(), "Kiwi", "each question keeps its text");
    assert!(press(f.tab(), now, Hit::Next));
    assert!(f.writes().is_empty());

    assert!(press(f.tab(), now, Hit::Submit));
    assert_eq!(f.writes(), [down(1)], "to Pear; nothing is pre-selected");
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the screen has not moved yet");
    assert_eq!(f.step(&form_screen(&qs, 0, 1, &none, None), now), [b"\r".to_vec()], "tick Pear");
    assert!(f.step(&form_screen(&qs, 0, 1, &none, None), now).is_empty(), "a repaint without the tick: no more keys");
    assert_eq!(f.step(&form_screen(&qs, 0, 1, &[false, true, false], None), now), [down(1)]);
    assert_eq!(f.step(&form_screen(&qs, 0, 2, &[false, true, false], None), now), [b"\r".to_vec()]);
    assert_eq!(f.step(&form_screen(&qs, 0, 2, &f_t_t, None), now), [down(1)]);
    assert_eq!(f.step(&form_screen(&qs, 0, 3, &f_t_t, None), now), [b"Kiwi".to_vec()]);
    assert!(f.step(&form_screen(&qs, 0, 3, &f_t_t, None), now).is_empty(), "not typed yet");
    assert_eq!(f.step(&form_screen(&qs, 0, 3, &f_t_t, Some("Kiwi")), now), [down(1)], "to Next");
    assert_eq!(f.step(&form_screen(&qs, 0, 4, &f_t_t, Some("Kiwi")), now), [b"\r".to_vec()]);
    assert!(f.step(&form_screen(&qs, 0, 4, &f_t_t, Some("Kiwi")), now).is_empty(), "the next question first");
    assert_eq!(f.step(&form_screen(&qs, 1, 0, &[], None), now), [down(1)]);
    assert_eq!(f.step(&form_screen(&qs, 1, 1, &[], None), now), [b"\r".to_vec()]);
    let review = review_screen(&qs, &["Pear, Plum, Kiwi", "Large"], 0);
    assert_eq!(f.step(&review, now), [b"\r".to_vec()], "Submit answers");
    assert!(f.chat().question.step.is_some(), "waits for the call to resolve");
    f.resolve();
    assert!(advance(f.tab(), now, true));
    assert!(f.chat().question.step.is_none() && f.chat().question.error.is_none());
}

#[test]
fn a_review_that_differs_from_the_cards_answers_is_never_submitted() {
    let qs = [size(), speed()];
    let mut f = Fixture::new(qs.to_vec());
    let now = Instant::now();
    f.screen(&form_screen(&qs, 0, 0, &[], None), now);
    press(f.tab(), now, Hit::Option(1));
    press(f.tab(), now, Hit::Next);
    press(f.tab(), now, Hit::Option(0));
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.writes(), [down(1)]);
    assert_eq!(f.step(&form_screen(&qs, 0, 1, &[], None), now), [b"\r".to_vec()]);
    assert_eq!(f.step(&form_screen(&qs, 1, 0, &[], None), now), [b"\r".to_vec()], "Slow is highlighted already");
    // The terminal recorded Small: no Enter on "Submit answers", ever.
    assert!(f.step(&review_screen(&qs, &["Small", "Slow"], 0), now).is_empty());
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(REVIEW_DIFFERS));
    assert_eq!(f.chat().drawer, Drawer::Manual);

    // The highlight on Cancel is moved up first, and Enter follows only once it shows there.
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 1, &[], None), now);
    press(f.tab(), now, Hit::Option(1));
    press(f.tab(), now, Hit::Next);
    press(f.tab(), now, Hit::Option(0));
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    assert_eq!(f.step(&form_screen(&qs, 1, 0, &[], None), now), [b"\r".to_vec()]);
    let up = mdshared_term::input::encode_key(mdshared_term::Key::Up, mdshared_term::Mods::NONE, &Default::default());
    assert_eq!(f.step(&review_screen(&qs, &["Large", "Slow"], 1), now), [up]);
    assert_eq!(f.step(&review_screen(&qs, &["Large", "Slow"], 0), now), [b"\r".to_vec()]);
}

#[test]
fn a_form_stops_when_the_terminal_does_not_follow() {
    let qs = [fruits(), size()];
    let now = Instant::now();
    let answered = |f: &mut Fixture| {
        press(f.tab(), now, Hit::Option(0));
        press(f.tab(), now, Hit::Next);
        press(f.tab(), now, Hit::Option(0));
    };

    // A box that does not tick: no second Enter, then a stop that says why.
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 0, &[false; 3], None), now);
    answered(&mut f);
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.writes(), [b"\r".to_vec()], "Apple is highlighted: tick it");
    assert!(f.step(&form_screen(&qs, 0, 0, &[false; 3], None), now).is_empty());
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_TICKED));

    // The terminal shows the second question: nothing is sent.
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 1, 0, &[], None), now);
    answered(&mut f);
    press(f.tab(), now, Hit::Submit);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(NOT_SHOWN));

    // Text typed in the terminal is never cleared or sent on.
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 0, &[true, false, false], Some("Fig")), now);
    answered(&mut f);
    press(f.tab(), now, Hit::Submit);
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().question.error, Some(OTHER_TAKEN));

    // Enter on Next, and the next question never comes.
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 4, &[true, false, false], None), now);
    answered(&mut f);
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    advance(f.tab(), now + STEP, true);
    assert_eq!(f.chat().question.error, Some(NO_NEXT));
}

#[test]
fn a_forms_answers_are_checked_in_the_card_before_any_key() {
    let qs = [size(), fruits()];
    let now = Instant::now();
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 0, &[], None), now);
    press(f.tab(), now, Hit::Option(0));
    f.chat().question.field.set_text("Medium");
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.chat().question.error, Some(BOTH), "a single choice takes one answer");
    press(f.tab(), now, Hit::Option(0));
    assert_eq!(f.chat().question.form.chosen[0], [false, false], "a second click clears the choice");
    f.chat().question.field.set_text("3 sizes");
    press(f.tab(), now, Hit::Next);
    press(f.tab(), now, Hit::Option(1));
    press(f.tab(), now, Hit::Option(2));
    assert_eq!(f.chat().question.form.chosen[1], [false, true, true], "several boxes");
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.chat().question.error, Some(DIGIT_FIRST));
    assert_eq!(f.chat().question.form.page, 0);
    assert!(f.writes().is_empty());
    assert!(!press(f.tab(), now, Hit::Back), "no question before the first");
}

#[test]
fn one_multi_select_question_goes_through_its_submit_row_and_the_review() {
    let qs = [fruits()];
    let mut f = Fixture::new(qs.to_vec());
    let now = Instant::now();
    let pear = [false, true, false];
    f.screen(&form_screen(&qs, 0, 1, &pear, None), now);
    press(f.tab(), now, Hit::Option(1));
    press(f.tab(), now, Hit::Submit);
    assert_eq!(f.writes(), [down(3)], "Pear is ticked already: to Submit");
    assert_eq!(f.step(&form_screen(&qs, 0, 4, &pear, None), now), [b"\r".to_vec()]);
    assert_eq!(f.step(&review_screen(&qs, &["Pear"], 0), now), [b"\r".to_vec()]);
}

#[test]
fn esc_and_chat_about_this_act_on_the_forms_question_the_screen_shows() {
    let qs = [fruits(), size()];
    let now = Instant::now();
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 1, 0, &[], None), now);
    assert!(press(f.tab(), now, Hit::Escape));
    assert_eq!(f.writes(), [ESC.to_vec()]);

    // The multi-select's Submit row sits between "Type something" and "Chat about this".
    let mut f = Fixture::new(qs.to_vec());
    f.screen(&form_screen(&qs, 0, 0, &[false; 3], None), now);
    f.chat().question.field.set_text("Let me explain");
    press(f.tab(), now, Hit::ChatAbout);
    assert_eq!(f.writes(), [down(5)]);
    assert_eq!(f.step(&form_screen(&qs, 0, 5, &[false; 3], None), now), [b"\r".to_vec()]);
    f.resolve();
    advance(f.tab(), now, true);
    assert_eq!(f.chat().editor.text, "Let me explain");
}

#[test]
fn recorded_form_screens_lead_to_the_expected_keys() {
    use screens::recorded::*;
    let screen = |fixture: &str| recorded(fixture);
    let qs = vec![fruits(), size()];
    let answers = vec![Answer { picks: vec![0, 2], text: Some("Kiwi".into()) }, Answer { picks: vec![1], text: None }];
    let run = |expect: Expect| FormRun { questions: qs.clone(), answers: answers.clone(), expect, why: "" };
    let next =
        |expect: Expect, fixture: &str| form_next(&run(expect), &screen(fixture)).map(|(key, expect, _)| (key, expect));
    let modes = Default::default();
    let down = |n: usize| {
        mdshared_term::input::encode_key(mdshared_term::Key::Down, mdshared_term::Mods::NONE, &modes).repeat(n)
    };
    let other = "[ ] Type something".to_owned();
    let shows = |cursor: usize, marks: [bool; 3], other: &str| Expect::Shows {
        at: 0,
        cursor,
        marks: marks.to_vec(),
        other: other.into(),
    };

    let shown = form_question(&screen(FORM_MULTI), &qs, 0).expect("the first question");
    assert_eq!((shown.other, shown.submit, shown.chat, shown.marks.as_slice()), (Some(3), Some(4), 5, &[false; 3][..]));
    assert!(form_question(&screen(FORM_MULTI), &qs, 1).is_none());
    assert!(form_question(&screen(FORM_SECOND), &qs, 0).is_none(), "the second question shows");
    assert!(form_question(&screen(FORM_SECOND), &qs, 1).is_some());
    assert!(form_question(&screen(ASK_MULTI), &qs, 0).is_none(), "another form: its tabs differ");

    assert_eq!(next(Expect::Opened(0), FORM_MULTI), Ok((b"\r".to_vec(), shows(0, [true, false, false], &other))));
    assert_eq!(
        next(shows(0, [true, false, false], &other), FORM_MULTI_APPLE),
        Ok((down(2), shows(2, [true, false, false], &other)))
    );
    assert_eq!(next(shows(0, [true, false, false], &other), FORM_MULTI), Err(""), "Apple not ticked yet");
    assert_eq!(
        next(shows(2, [true, false, true], &other), FORM_MULTI_PLUM),
        Ok((down(1), shows(3, [true, false, true], &other)))
    );
    let typed = Expect::Typed { at: 0, cursor: 3, marks: vec![true, false, true], text: "Kiwi".into() };
    assert_eq!(next(shows(3, [true, false, true], &other), FORM_MULTI_TYPE), Ok((b"Kiwi".to_vec(), typed.clone())));
    assert_eq!(next(typed.clone(), FORM_MULTI_TYPE), Err(""), "the text is not there yet");
    assert_eq!(next(typed, FORM_MULTI_TYPED), Ok((down(1), shows(4, [true, false, true], "[✔] Kiwi"))));
    assert_eq!(
        next(shows(4, [true, false, true], "[✔] Kiwi"), FORM_MULTI_NEXT),
        Ok((b"\r".to_vec(), Expect::Opened(1)))
    );
    let second = Expect::Shows { at: 1, cursor: 1, marks: vec![], other: "Type something.".into() };
    assert_eq!(next(Expect::Opened(1), FORM_SECOND), Ok((down(1), second)));
    assert_eq!(next(Expect::Review(None), FORM_REVIEW), Ok((b"\r".to_vec(), Expect::Submitted)));
    assert_eq!(next(Expect::Review(None), FORM_SECOND), Err(""));

    // Two single choices (Small, Slow), then their review.
    let qs = vec![size(), speed()];
    let answers = vec![Answer { picks: vec![0], text: None }, Answer { picks: vec![0], text: None }];
    let run = |expect: Expect| FormRun { questions: qs.clone(), answers: answers.clone(), expect, why: "" };
    assert_eq!(form_next(&run(Expect::Opened(0)), &screen(ASK_TWO)).map(|n| n.1), Ok(Expect::Opened(1)));
    assert_eq!(form_next(&run(Expect::Review(None)), &screen(ASK_REVIEW)).map(|n| n.1), Ok(Expect::Submitted));
    let wrong = vec![Answer { picks: vec![1], text: None }, Answer { picks: vec![0], text: None }];
    let run = FormRun { questions: qs.clone(), answers: wrong, expect: Expect::Review(None), why: "" };
    assert_eq!(form_next(&run, &screen(ASK_REVIEW)).map(|n| n.1), Err(REVIEW_DIFFERS));

    // One multi-select question: its Submit row leads to the review.
    let qs = vec![fruits()];
    let answers = vec![Answer { picks: vec![1], text: None }];
    let run = |expect: Expect| FormRun { questions: qs.clone(), answers: answers.clone(), expect, why: "" };
    assert_eq!(form_next(&run(Expect::Opened(0)), &screen(MULTI_SUBMIT)).map(|n| n.1), Ok(Expect::Review(None)));
    assert_eq!(form_next(&run(Expect::Review(None)), &screen(MULTI_REVIEW)).map(|n| n.1), Ok(Expect::Submitted));
}

#[test]
fn enter_in_a_forms_field_turns_the_page_and_submits_on_the_last_question() {
    let qs = [size(), speed()];
    let mut f = Fixture::new(qs.to_vec());
    let now = Instant::now();
    f.screen(&form_screen(&qs, 0, 1, &[], None), now);
    press(f.tab(), now, Hit::Option(1));
    f.chat().question.rect = Rect::new(0.0, 0.0, 400.0, 300.0);
    assert!(press(f.tab(), now, Hit::Field));
    assert!(key(f.tab(), now, &KeyName::Enter, Mods::default(), None));
    assert_eq!(f.chat().question.form.page, 1, "Enter turns to the next question");
    assert!(f.writes().is_empty());
    // After the last question Enter turns to the Submit step, which has no field.
    assert!(key(f.tab(), now, &KeyName::Enter, Mods::default(), None));
    assert_eq!(f.chat().question.form.page, 2);
    assert!(!f.chat().question.focus && f.writes().is_empty());
    // Submit there checks first: the second question has no answer, and is shown.
    assert!(press(f.tab(), now, Hit::Submit));
    assert_eq!(f.chat().question.error, Some(UNANSWERED));
    assert_eq!(f.chat().question.form.page, 1);
    assert!(f.writes().is_empty());
    press(f.tab(), now, Hit::Option(0));
    assert!(press(f.tab(), now, Hit::Page(2)));
    assert_eq!(f.chat().question.form.page, 2);
    assert!(press(f.tab(), now, Hit::Submit));
    assert_eq!(f.writes(), [b"\r".to_vec()], "Large is highlighted: chosen");
    assert!(f.chat().question.step.is_some());
}

#[test]
fn a_forms_tabs_name_its_questions_and_submit_and_mark_what_the_terminal_shows() {
    let qs = [size(), speed()];
    let mut f = Fixture::new(qs.to_vec());
    let now = Instant::now();
    f.screen(&form_screen(&qs, 0, 0, &[], None), now);
    let tab = f.tab();
    let pane = tab.pane.as_mut().unwrap();
    pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen);
    assert_eq!(f.chat().question.form_shown, Some(0), "the terminal shows the first question");
    press(f.tab(), now, Hit::Option(1));
    let tabs = |f: &mut Fixture| {
        let pane = f.tab().pane.as_mut().unwrap();
        let parts = cards::parts(&pane.chat, &pane.model, 600.0).expect("the form's card");
        parts
            .into_iter()
            .find_map(|p| match p {
                cards::Part::Tabs(tabs) => Some(tabs),
                _ => None,
            })
            .expect("its tabs")
    };
    let row = tabs(&mut f);
    let names: Vec<(&str, bool, bool, bool)> =
        row.iter().map(|t| (t.label.as_str(), t.answered, t.current, t.terminal)).collect();
    assert_eq!(
        names,
        [
            (qs[0].header.as_str(), true, true, true),
            (qs[1].header.as_str(), false, false, false),
            ("Submit", false, false, false)
        ]
    );
    // A tab turns the card's page, and nothing is written.
    assert!(press(f.tab(), now, Hit::Page(2)));
    assert!(f.writes().is_empty());
    let pane = f.tab().pane.as_mut().unwrap();
    let parts = cards::parts(&pane.chat, &pane.model, 600.0).unwrap();
    let answers = parts.iter().find_map(|p| match p {
        cards::Part::Answers(rows) => Some(rows.clone()),
        _ => None,
    });
    let answers = answers.expect("the Submit step lists the answers");
    assert_eq!(answers[0], (collapse(&qs[0].question), collapse(&qs[0].options[1].label), true));
    assert!(!answers[1].2, "the second has none yet");
    assert!(parts.iter().any(
        |p| matches!(p, cards::Part::Buttons(b) if b.iter().any(|(_, h, primary)| *h == Hit::Submit && *primary))
    ));
}

#[test]
fn a_panel_card_replaces_the_composers_prompt_notices() {
    let mut f = Fixture::idle();
    let now = Instant::now();
    f.screen("\x1b[2J\x1b[H Usage\r\n  ✻ Scanning local sessions…\r\n  0% used\r\n\r\n Esc to close", now);
    f.sync();
    let menu = f.chat().question.menu.clone().expect("a panel card");
    assert!(menu.panel() && menu.esc);
    assert!(crate::view::working::spinner_row("✻ Scanning local sessions…"));
    assert!(!crate::view::working::spinner_row("Resets Dec 2, 3am"));
    f.chat().question.rect = Rect::new(0.0, 0.0, 400.0, 200.0);
    f.chat().error = Some(crate::view::chat::submission::NO_QUIET_PROMPT);
    let info = f.tab().pane.as_ref().unwrap().model.chat.clone();
    assert_eq!(f.chat().notice(&info), Some(crate::view::chat::composer::PANEL_UP));
    // Close is Esc, under the same checks as any menu.
    assert!(press(f.tab(), now, Hit::Escape));
    assert_eq!(f.writes(), [ESC.to_vec()]);
}

impl Fixture {
    /// Prompts the user typed, in order, at offsets 10, 20, … and the turn after the last ended.
    fn prompts(&mut self, texts: &[&str]) -> Vec<ItemId> {
        let pane = self.tab().pane.as_mut().unwrap();
        let mut ids = Vec::new();
        for (i, text) in texts.iter().enumerate() {
            let id = ItemId { offset: 10 * (i as u64 + 1), block: 0 };
            let kind = ItemKind::Prompt { text: (*text).into(), pictures: vec![], synthetic: false, queued: false };
            pane.model.items.insert(id, Item { kind, timestamp: None });
            ids.push(id);
        }
        pane.model.chat.prompt = ids.last().map(|id| id.offset);
        ids
    }

    fn rewind(&mut self) -> Option<rewind::Rewind> {
        self.chat().question.rewind.clone()
    }
}

fn up() -> Vec<u8> {
    mdshared_term::input::encode_key(mdshared_term::Key::Up, mdshared_term::Mods::NONE, &Default::default())
}

const ALPHA: &str = "Reply with just the word alpha.";
const BRAVO: &str = "Reply with just the word bravo.";
const CHARLIE: &str = "Reply with just the word charlie.";

#[test]
fn recorded_rewind_screens_read_as_the_list_and_the_choices() {
    use screens::recorded::*;
    let list = |fixture: &str| screens::rewind_list(&recorded(fixture));
    let open = list(REWIND_OPEN).expect("the list");
    assert_eq!(open.entries, [ALPHA, BRAVO, CHARLIE, "(current)"]);
    assert_eq!(open.highlight, 3);
    assert_eq!(list(REWIND_UP2).map(|l| l.highlight), Some(1));
    assert!(list(REWIND_IDLE).is_none() && list(REWIND_ACTIONS).is_none());

    let actions = screens::rewind_actions(&recorded(REWIND_ACTIONS)).expect("the choices");
    assert_eq!(actions.quoted, BRAVO);
    assert_eq!(actions.choices, ["Restore conversation", "Summarize from here", "Summarize up to here", "Never mind"]);
    assert_eq!(actions.highlight, Some(0));
    assert_eq!(actions.body, ["The conversation goes back to before this message.", "The code will be unchanged."]);
    let code = screens::rewind_actions(&recorded(REWIND_ACTIONS_CODE)).expect("the choices with code");
    assert_eq!(code.choices.len(), 6);
    assert_eq!(code.choices[0], "Restore code and conversation");
    assert!(code.body.iter().any(|row| row.starts_with("⚠ Restored files go back whole")));
    let down = screens::rewind_actions(&recorded(REWIND_ACTIONS_CODE_DOWN)).unwrap();
    assert_eq!((down.highlight, down.choices), (Some(1), code.choices));
    assert!(screens::rewind_actions(&recorded(REWIND_OPEN)).is_none());

    // A prompt of several lines shows its first line and "…"; a long one is cut with " …".
    let cut = list(REWIND_CUT_UP1).expect("the list");
    let echo = "Reply with just the word echo.\nSecond line of the prompt.";
    assert_eq!(cut.highlight, 1);
    assert!(screens::rewind_row_is(&cut.entries[1], echo));
    assert!(!screens::rewind_row_is(&cut.entries[1], "Reply with just the word delta."));
    let long = "Reply with just the word delta. This sentence is only padding so that the prompt is long enough \
                to be cut by the rewind list, more padding words here.";
    assert!(screens::rewind_row_is(&list(REWIND_CUT_UP2).unwrap().entries[0], long));
    assert!(!screens::rewind_row_is(&cut.entries[0], "Reply with just the word delta."));
    assert!(screens::rewind_row_is(BRAVO, BRAVO) && !screens::rewind_row_is(BRAVO, echo));
}

#[test]
fn rewind_walks_the_list_to_the_prompt_and_chooses_only_on_a_click() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE]);
    f.show(REWIND_IDLE, now);
    let now = now + QUIET;
    rewind::start(f.tab(), now, ids[1]);
    assert_eq!(f.writes(), [ESC.to_vec()]);
    assert!(f.chat().waiting().is_some(), "the composer waits meanwhile");
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the second Esc waits for its gap");
    let now = now + STEP / 10;
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [ESC.to_vec()], "the same idle prompt: the second Esc");
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the same screen proves nothing");
    f.show(REWIND_OPEN, now);
    assert!(f.chat().question.menu.is_none(), "the list is driven, not shown as a menu card");
    advance(f.tab(), now, false);
    assert!(f.writes().is_empty(), "no write while the window does not allow one");
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [up()]);
    f.show(REWIND_UP1, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [up()], "charlie is not the prompt: up again");
    f.show(REWIND_UP2, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "nothing is chosen without a click");
    let shown = f.rewind().and_then(|r| r.actions).expect("the choices in the card");
    assert_eq!(shown.choices.len(), 4);
    assert!(f.chat().waiting().is_none(), "the card waits for the user, not for keys");

    assert!(press(f.tab(), now, Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()], "the highlight is already there");
    f.show(REWIND_CLOSED, now);
    advance(f.tab(), now, true);
    assert!(f.rewind().is_none());
    assert_eq!(f.chat().error, Some(rewind::REWOUND));
}

#[test]
fn a_message_sent_while_a_turn_ran_is_in_the_rewind_list_as_any_prompt() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE]);
    // Charlie went in while bravo's turn ran: Claude Code's rewind list shows it all the same
    // (recorded from 2.1.332, `recorder compat`'s `queued` script).
    let kind = ItemKind::Prompt { text: CHARLIE.into(), pictures: vec![], synthetic: false, queued: true };
    f.tab().pane.as_mut().unwrap().model.items.insert(ids[2], Item { kind, timestamp: None });
    f.show(REWIND_IDLE, now);
    let now = now + QUIET;
    rewind::start(f.tab(), now, ids[2]);
    assert_eq!(f.writes(), [ESC.to_vec()], "it can be rewound to");
    f.chat().question.rewind = None;
    // An earlier prompt is found with it under it in the list.
    rewind::start(f.tab(), now, ids[1]);
    assert_eq!(f.writes(), [ESC.to_vec()]);
    let now = now + STEP / 10;
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [ESC.to_vec()]);
    f.show(REWIND_OPEN, now);
    advance(f.tab(), now, true);
    f.show(REWIND_UP1, now);
    advance(f.tab(), now, true);
    f.show(REWIND_UP2, now);
    f.writes();
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()], "bravo, with charlie under it");
}

#[test]
fn a_rewind_choice_moves_one_row_at_a_time_and_cancel_goes_back_by_esc() {
    use screens::recorded::*;
    let now = Instant::now();
    let write = "Use the Write tool to create the file note.txt containing the word one.";
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(write));
    f.show(REWIND_ACTIONS_CODE, now);
    advance(f.tab(), now, true);
    assert!(f.rewind().is_some_and(|r| r.choosing()));
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)]);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    f.show(REWIND_ACTIONS_CODE_DOWN, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // Claude Code puts the prompt back on its input.
    f.show(REWIND_DONE, now);
    advance(f.tab(), now, true);
    assert!(f.rewind().is_none());

    // Cancel: Esc back to the list, Esc back to the prompt.
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(BRAVO));
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    assert!(press(f.tab(), now, Hit::Escape));
    assert_eq!(f.writes(), [ESC.to_vec()]);
    f.show(REWIND_UP2, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [ESC.to_vec()]);
    f.show(REWIND_CLOSED, now);
    advance(f.tab(), now, true);
    assert!(f.rewind().is_none());
    assert_eq!(f.chat().error, None, "a cancel is not a rewind");

    // The choices changed under the card: nothing is written.
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(BRAVO));
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    f.tab().term.screen = recorded(REWIND_ACTIONS_CODE);
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::ACTIONS_CHANGED));

    // The choices quote another prompt.
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(ALPHA));
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::NO_ACTIONS));
}

#[test]
fn a_rewind_stops_before_any_key_the_screen_does_not_prove() {
    use screens::recorded::*;
    let now = Instant::now();
    // Not at an idle prompt: a running command.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE]);
    f.show(BG_HINT, now);
    rewind::start(f.tab(), now + QUIET, ids[1]);
    assert!(f.writes().is_empty());
    let failed = f.rewind().expect("the card says why");
    assert!(failed.failed() && failed.error == Some(rewind::NOT_IDLE));
    assert!(press(f.tab(), now, Hit::Close));
    assert!(f.rewind().is_none());

    // The list never moves up to the prompt: no Enter.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE]);
    f.show(REWIND_IDLE, now);
    let now = now + QUIET;
    rewind::start(f.tab(), now, ids[0]);
    advance(f.tab(), now + STEP / 10, true);
    f.writes();
    f.show(REWIND_OPEN, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [up()]);
    f.show(REWIND_OPEN, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty(), "the highlight did not move");
    advance(f.tab(), now + STEP, true);
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::NOT_LISTED));
    assert_eq!(f.chat().drawer, Drawer::Manual);

    // A later prompt reads the same: the row is ambiguous.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[1]);
    advance(f.tab(), now + QUIET + STEP / 10, true);
    for screen in [REWIND_OPEN, REWIND_UP1, REWIND_UP2] {
        f.show(screen, now);
        advance(f.tab(), now, true);
    }
    assert_eq!(f.writes(), [ESC.to_vec(), ESC.to_vec(), up(), up(), ESC.to_vec()], "no Enter; Esc closes the list");
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::AMBIGUOUS));

    // The list does not open after Esc Esc.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[0]);
    advance(f.tab(), now + QUIET + STEP / 10, true);
    f.show(REWIND_IDLE, now);
    advance(f.tab(), now + QUIET + STEP * 2, true);
    assert_eq!(f.writes(), [ESC.to_vec(), ESC.to_vec()]);
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::NOT_OPENED));

    // Only a prompt the user typed.
    let mut f = Fixture::idle();
    f.prompts(&[ALPHA]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ItemId { offset: 99, block: 0 });
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::NOT_PROMPT));
}

/// Settings › Chat › "Answer menus from the chat" off: no card for a CLI menu or a permission
/// prompt, nothing written on a click, and the prompt goes to the terminal as before item 42.
#[test]
fn with_menus_off_menus_and_permission_prompts_stay_in_the_terminal() {
    let now = Instant::now();
    let mut f = Fixture::idle();
    f.chat().menus_off = true;
    f.screen(&toggles(0, false), now);
    f.sync();
    assert!(f.chat().question.menu.is_none());
    assert!(!press(f.tab(), now, Hit::Option(0)) || f.writes().is_empty());
    assert!(f.writes().is_empty());

    let mut f = Fixture::tool("cargo test --workspace");
    f.chat().menus_off = true;
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.sync();
    assert!(f.chat().question.shown.is_none() && f.chat().question.menu.is_none());
    assert!(f.chat().question.prompt.is_none(), "nor the card read from the screen");
    assert_eq!(f.chat().question.answerable, None);
    assert!(!f.chat().question.approval);
    assert_eq!(f.decide(now), Some(Reason::Permission));
    assert_ne!(f.chat().drawer, Drawer::Closed);
    f.chat().menus_off = false;
    f.sync();
    assert!(f.chat().question.approval, "on again: the card is back");
}

#[test]
fn rewind_from_the_menu_starts_only_in_the_shown_tab_with_chat_on() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    let tab = f.tab;
    f.chat().enabled = false;
    assert!(!f.app.rewind_from_menu(tab, ids[1], now + QUIET), "Chat was switched off after the menu opened");
    assert!(f.writes().is_empty() && f.rewind().is_none());
    assert!(!f.app.rewind_from_menu(tab + 1, ids[1], now + QUIET), "another tab");
    f.chat().enabled = true;
    assert!(f.app.rewind_from_menu(tab, ids[1], now + QUIET));
    assert_eq!(f.writes(), [ESC.to_vec()]);
}

/// A recorded screen with one text replaced, as the CLI would draw another prompt's choices.
fn recorded_with(fixture: &str, from: &str, to: &str) -> Screen {
    assert!(fixture.contains(from));
    screens::recorded::recorded(&fixture.replace(from, to))
}

#[test]
fn a_rewind_choice_needs_the_choices_for_the_chosen_prompt_on_every_key() {
    use screens::recorded::*;
    let now = Instant::now();
    // The terminal moved to alpha's choices, which read the same: no Enter.
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(BRAVO));
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    f.tab().term.screen = recorded_with(REWIND_ACTIONS, BRAVO, ALPHA);
    assert!(press(f.tab(), now, Hit::Option(0)));
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::ACTIONS_CHANGED));

    // The same while the highlight moves: the arrow went, Enter does not.
    let write = "Use the Write tool to create the file note.txt containing the word one.";
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(write));
    f.show(REWIND_ACTIONS_CODE, now);
    advance(f.tab(), now, true);
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)]);
    f.tab().term.screen = recorded_with(REWIND_ACTIONS_CODE_DOWN, write, "Use the Write tool to create other.txt");
    f.chat().output(now, true, false);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::ACTIONS_CHANGED));
}

#[test]
fn the_rows_under_the_chosen_prompt_must_be_the_later_prompts() {
    use screens::recorded::*;
    let now = Instant::now();
    let walk = |prompts: &[&str], chosen: usize| {
        let mut f = Fixture::idle();
        let ids = f.prompts(prompts);
        f.show(REWIND_IDLE, now);
        rewind::start(f.tab(), now + QUIET, ids[chosen]);
        advance(f.tab(), now + QUIET + STEP / 10, true);
        for screen in [REWIND_OPEN, REWIND_UP1, REWIND_UP2] {
            f.show(screen, now);
            advance(f.tab(), now, true);
        }
        f
    };
    // A later prompt the list leaves out: the bravo row may be another, earlier bravo.
    let mut f = walk(&[ALPHA, BRAVO, "Reply with just the word delta.", CHARLIE], 1);
    assert_eq!(f.writes(), [ESC.to_vec(), ESC.to_vec(), up(), up(), ESC.to_vec()], "no Enter; Esc closes the list");
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::MISPLACED));
    // A slash command between them is not in the list and does not count.
    let mut f = walk(&[ALPHA, BRAVO, "/context", CHARLIE], 1);
    assert_eq!(f.writes().last(), Some(&b"\r".to_vec()));
    assert!(f.rewind().is_some_and(|r| r.error.is_none()));
    assert!(!rewind::listed("/context") && !rewind::listed("<command-name>/x</command-name>"));
    assert!(rewind::listed(BRAVO));
}

#[test]
fn terminal_input_stops_every_card_key_sequence_before_its_next_key() {
    use screens::recorded::*;
    let now = Instant::now();
    let typed = |f: &mut Fixture| f.tab().pane.as_mut().unwrap().note_terminal_input();

    // An answer: the arrow went, Enter does not follow once the user typed in the terminal.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(0, None), now);
    press(f.tab(), now, Hit::Option(1));
    assert_eq!(f.writes(), [down(1)]);
    typed(&mut f);
    f.screen(&menu(1, None), now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    assert!(f.chat().question.step.is_none());
    assert_eq!(f.chat().question.error, Some(INTERRUPTED));
    assert!(f.chat().waiting().is_none(), "the composer is free again");

    // Stop agent: no ↓ after the list opened.
    let mut f = Fixture::idle();
    f.agent("First pinger");
    f.agent("Second pinger");
    f.show(AGENTS_IDLE, now);
    f.sync();
    press(f.tab(), now, Hit::StopAgent(1));
    f.writes();
    typed(&mut f);
    assert_eq!(f.chat().question.error, Some(INTERRUPTED));
    f.show(AGENTS_OPEN, now);
    advance(f.tab(), now, true);
    assert!(f.writes().is_empty());
    assert!(f.chat().question.agent_step.is_none());

    // A rewind: no second Esc.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[0]);
    f.writes();
    typed(&mut f);
    advance(f.tab(), now + QUIET + STEP / 10, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(INTERRUPTED));
    assert!(f.rewind().is_some_and(|r| r.failed()));
}

#[test]
fn leaving_chat_stops_a_rewind_and_a_held_key_still_times_out() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[0]);
    assert!(f.app.set_shown_tab_chat(false));
    assert_eq!(f.rewind().and_then(|r| r.error), Some(INTERRUPTED));
    f.chat().enabled = true;
    advance(f.tab(), now + QUIET + STEP / 10, true);
    assert_eq!(f.writes(), [ESC.to_vec()], "only the first Esc ever went");

    // Writes held back (a dialog over the window): the Up waits, and the deadline still holds.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    let now = now + QUIET;
    rewind::start(f.tab(), now, ids[0]);
    advance(f.tab(), now + STEP / 10, true);
    f.writes();
    f.show(REWIND_OPEN, now);
    advance(f.tab(), now + STEP / 2, false);
    assert!(f.writes().is_empty() && f.rewind().is_some_and(|r| r.in_flight()));
    advance(f.tab(), now + STEP * 2, false);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::HELD_BACK));
    assert!(f.chat().waiting().is_none(), "the composer is free again");

    // The second Esc held back past the CLI's double-press window: stopped, not sent late.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    let now = now + QUIET;
    rewind::start(f.tab(), now, ids[0]);
    assert_eq!(f.writes(), [ESC.to_vec()]);
    advance(f.tab(), now + STEP, false);
    advance(f.tab(), now + STEP * 3, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::HELD_BACK));
}

#[test]
fn a_rewind_that_stops_on_its_list_closes_it_or_says_it_is_open() {
    use screens::recorded::*;
    let now = Instant::now();
    let ambiguous = |f: &mut Fixture, may_write_last: bool| {
        let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE, BRAVO]);
        f.show(REWIND_IDLE, now);
        rewind::start(f.tab(), now + QUIET, ids[1]);
        advance(f.tab(), now + QUIET + STEP / 10, true);
        for screen in [REWIND_OPEN, REWIND_UP1] {
            f.show(screen, now);
            advance(f.tab(), now, true);
        }
        f.show(REWIND_UP2, now);
        advance(f.tab(), now, may_write_last);
    };
    // Stopped on the list: one Esc, and the card says so once the prompt is back.
    let mut f = Fixture::idle();
    ambiguous(&mut f, true);
    assert_eq!(f.writes().last(), Some(&ESC.to_vec()));
    assert_eq!(f.chat().drawer, Drawer::Manual);
    f.show(REWIND_CLOSED, now);
    advance(f.tab(), now, true);
    let r = f.rewind().unwrap();
    assert!(r.failed());
    assert_eq!((r.error, r.note), (Some(rewind::AMBIGUOUS), Some(rewind::CLOSED)));

    // The list stays: it says that it is still open, and sends nothing more.
    let mut f = Fixture::idle();
    ambiguous(&mut f, true);
    f.writes();
    f.show(REWIND_UP2, now);
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.note), Some(rewind::LEFT_OPEN));

    // No key allowed when it stops: no Esc, and the card says the list is open.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[0]);
    advance(f.tab(), now + QUIET + STEP / 10, true);
    f.show(REWIND_OPEN, now);
    advance(f.tab(), now + QUIET + STEP * 3, false);
    assert_eq!(f.writes(), [ESC.to_vec(), ESC.to_vec()]);
    let r = f.rewind().unwrap();
    assert_eq!((r.error, r.note), (Some(rewind::HELD_BACK), Some(rewind::LEFT_OPEN)));

    // A slash command is never rewound to: the list does not show them.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, "/context"]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[1]);
    assert!(f.writes().is_empty());
    assert_eq!(f.rewind().and_then(|r| r.error), Some(rewind::NOT_PROMPT));
}

#[test]
fn a_question_comes_before_a_stopped_rewinds_card() {
    let now = Instant::now();
    let mut f = Fixture::new(vec![question(false)]);
    let mut stopped = rewind::Rewind::entering(BRAVO);
    stopped.interrupt(INTERRUPTED);
    f.chat().question.rewind = Some(stopped);
    f.screen(&menu(0, None), now);
    f.sync();
    let pane = f.tab().pane.as_mut().unwrap();
    assert!(!rewind_shown(&pane.chat.question, &pane.model), "the question takes the card");
    assert!(press(f.tab(), now, Hit::Option(1)));
    assert_eq!(f.writes(), [down(1)], "the click answers the question");
    assert!(f.rewind().is_some(), "the stopped rewind waits for its turn");

    // Once nothing else needs the card, it shows again and Close puts it away.
    let mut f = Fixture::idle();
    let mut stopped = rewind::Rewind::entering(BRAVO);
    stopped.interrupt(INTERRUPTED);
    f.chat().question.rewind = Some(stopped);
    let pane = f.tab().pane.as_mut().unwrap();
    assert!(rewind_shown(&pane.chat.question, &pane.model));
    assert!(press(f.tab(), now, Hit::Close));
    assert!(f.rewind().is_none());

    // A rewind still running keeps the card even over a question.
    let mut f = Fixture::new(vec![question(false)]);
    f.chat().question.rewind = Some(rewind::Rewind::entering(BRAVO));
    let pane = f.tab().pane.as_mut().unwrap();
    assert!(rewind_shown(&pane.chat.question, &pane.model));
}

#[test]
fn an_interrupted_side_step_leaves_a_notice_not_a_stuck_card() {
    use screens::recorded::*;
    let now = Instant::now();
    let typed = |f: &mut Fixture| f.tab().pane.as_mut().unwrap().note_terminal_input();
    // A Stop agent interrupted while a main-turn call is pending without a prompt yet: when the
    // call's permission prompt shows, the card answers it.
    let mut f = Fixture::tool("cargo test --workspace");
    f.agent("First pinger");
    f.agent("Second pinger");
    f.show(AGENTS_IDLE, now);
    f.sync();
    assert!(!f.chat().question.agents.is_empty());
    press(f.tab(), now, Hit::StopAgent(0));
    assert_eq!(f.writes().len(), 1);
    typed(&mut f);
    assert!(f.chat().question.agent_step.is_none());
    assert_eq!(f.chat().question.error, Some(INTERRUPTED));
    assert!(!f.chat().question.failed, "only a notice");
    // The agents end; the pending call then asks.
    f.tab().pane.as_mut().unwrap().model.agents.iter_mut().for_each(|a| a.finished = Some(true));
    f.screen(&prompt("cargo test --workspace", 0, None), now);
    f.sync();
    assert_eq!(f.chat().question.answerable, Some(AT), "the prompt is answered from the card");
    assert_eq!(f.chat().question.error, None, "the notice was about the agent list");

    // An answer whose Enter went: sent, so no "stopped" notice, and the step stays so a second
    // click on the still-painted prompt writes nothing.
    let mut f = Fixture::new(vec![question(false)]);
    f.screen(&menu(0, None), now);
    press(f.tab(), now, Hit::Option(0));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    typed(&mut f);
    assert!(f.chat().question.step.as_ref().is_some_and(|s| s.stage == Stage::Sent));
    assert_eq!(f.chat().question.error, None);
    assert!(!f.chat().question.failed);
    press(f.tab(), now, Hit::Option(0));
    assert!(f.writes().is_empty(), "no second Enter while the first is on its way");

    // An interrupted rewind keeps its notice on its own card, not as the next menu card's status.
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[0]);
    typed(&mut f);
    assert_eq!(f.rewind().and_then(|r| r.error), Some(INTERRUPTED));
    assert_eq!(f.chat().question.error, None);
    f.chat().question.rewind = None;
    f.show(MENU_MODEL, now);
    f.sync();
    assert!(f.chat().question.menu.is_some());
    assert_eq!(f.chat().question.error, None);
}

#[test]
fn hover_and_wheel_reports_leave_a_card_key_sequence_alone_but_a_click_stops_it() {
    use mdshared_ui::host::Button;
    use screens::recorded::*;
    let now = Instant::now();
    let start = |f: &mut Fixture| {
        f.agent("First pinger");
        f.agent("Second pinger");
        f.show(AGENTS_IDLE, now);
        f.sync();
        press(f.tab(), now, Hit::StopAgent(1));
        f.writes();
        // The program asks for every mouse move, as Claude Code's full-screen view does.
        f.tab().term.feed(b"\x1b[?1003h\x1b[?1006h");
        let area = mdshared_ui::gfx::Rect::new(0.0, 0.0, 800.0, 400.0);
        f.app.layout_terminals(area, area, 1.0);
    };
    let plain = Mods::default();
    let mut f = Fixture::idle();
    start(&mut f);
    f.app.term_move(20.0, 20.0, plain);
    f.app.term_move(60.0, 40.0, plain);
    assert!(f.tab().term.last_cell.is_some(), "the moves were reported to the program");
    f.app.mouse = (60.0, 40.0);
    f.app.term_wheel(-120.0, plain);
    assert!(f.chat().question.agent_step.is_some(), "hover and wheel reports go to the program only");
    assert_eq!(f.chat().question.error, None);

    let mut f = Fixture::idle();
    start(&mut f);
    f.app.term_press(20.0, 20.0, Button::Left, 1, plain);
    assert!(f.chat().question.agent_step.is_none(), "a click may pick something in the CLI");
    assert_eq!(f.chat().question.error, Some(INTERRUPTED));
}

#[test]
fn a_rewind_left_open_keeps_its_card_over_the_list_until_the_list_is_gone() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO, CHARLIE, BRAVO]);
    f.show(REWIND_IDLE, now);
    rewind::start(f.tab(), now + QUIET, ids[1]);
    advance(f.tab(), now + QUIET + STEP / 10, true);
    for screen in [REWIND_OPEN, REWIND_UP1, REWIND_UP2] {
        f.show(screen, now);
        advance(f.tab(), now, true);
    }
    // Ambiguous; the closing Esc did not close the list.
    f.show(REWIND_UP2, now);
    advance(f.tab(), now + STEP, true);
    let r = f.rewind().unwrap();
    assert_eq!((r.error, r.note), (Some(rewind::AMBIGUOUS), Some(rewind::LEFT_OPEN)));
    f.sync();
    assert!(f.chat().question.menu.is_none(), "the list is not offered as a panel card");
    let pane = f.tab().pane.as_mut().unwrap();
    assert!(rewind_shown(&pane.chat.question, &pane.model), "the rewind's card says why");
    // The user closes it in the terminal: the card goes with it.
    f.show(REWIND_CLOSED, now);
    f.sync();
    assert!(f.rewind().is_none());
}

#[test]
fn a_cancel_that_stops_sends_no_esc_on_top_of_its_own() {
    use screens::recorded::*;
    let now = Instant::now();
    let mut f = Fixture::idle();
    f.chat().question.rewind = Some(rewind::Rewind::entering(BRAVO));
    f.show(REWIND_ACTIONS, now);
    advance(f.tab(), now, true);
    assert!(press(f.tab(), now, Hit::Escape));
    f.show(REWIND_UP2, now);
    advance(f.tab(), now, true);
    assert_eq!(f.writes(), [ESC.to_vec(), ESC.to_vec()]);
    // The CLI does not repaint: the list still shows when the cancel times out.
    advance(f.tab(), now + STEP, true);
    assert!(f.writes().is_empty(), "no third Esc");
    let r = f.rewind().unwrap();
    assert_eq!((r.error, r.note), (Some(rewind::NOT_CANCELLED), Some(rewind::LEFT_OPEN)));
}

#[test]
fn open_terminal_on_a_card_gives_the_terminal_the_keyboard() {
    let mut f = Fixture::new(vec![question(false)]);
    let now = Instant::now();
    f.sync();
    assert!(f.app.chat_takes_input());
    assert!(press(f.tab(), now, Hit::Open));
    assert!(f.chat().drawer != Drawer::Closed && f.chat().drawer_focus);
    assert!(!f.app.chat_takes_input(), "keys go to the terminal");
    // A card's answer that could not go hands the question to the terminal, with the keyboard.
    let mut chat = Chat::default();
    chat.question_failed("The question changed.");
    assert!(chat.drawer == Drawer::Manual && chat.drawer_focus);
    // Not while a selection in the conversation waits for Ctrl+C (or the box holds text).
    for keep in [true, false] {
        let mut chat = Chat::default();
        if keep {
            chat.set_selection(true);
        } else {
            chat.editor.set("draft".into());
        }
        chat.question_failed("The question changed.");
        assert!(chat.drawer == Drawer::Manual && !chat.drawer_focus);
    }
}

#[test]
fn rewind_is_refused_in_a_subagents_transcript() {
    let mut f = Fixture::idle();
    let ids = f.prompts(&[ALPHA, BRAVO]);
    f.tab().pane.as_mut().unwrap().model.agent = Some("agent-1".into());
    let tab = f.tab;
    f.app.tabs.active = Some(tab);
    assert!(f.app.rewind_from_menu(tab, ids[0], Instant::now()));
    assert!(f.writes().is_empty(), "nothing is written");
    assert_eq!(f.chat().error, Some(rewind::IN_AGENT));
    assert!(f.rewind().is_none());
}

#[test]
fn a_read_outside_the_project_is_a_card_with_or_without_its_call_in_the_transcript() {
    use screens::recorded::PERM_READ_OUTSIDE;
    for known in [true, false] {
        let mut f = Fixture::idle();
        let now = Instant::now();
        if known {
            // The transcript holds the pending Read call (a handover tab that read it already).
            let path = r"C:\Users\user1\AppData\Local\Temp\cs-hand\data\handover\one\h.md";
            let kind = ItemKind::Tool {
                id: "toolu_read".into(),
                name: "Read".into(),
                summary: path.into(),
                file_path: Some(path.into()),
                state: ToolState::Running,
                duration_ms: None,
                pictures: vec![],
                subagent: None,
                denial: None,
                card: None,
            };
            let pane = f.tab().pane.as_mut().unwrap();
            pane.model.items.insert(ItemId { offset: 900, block: 0 }, Item { kind, timestamp: None });
            pane.model.chat.pending = Some(900);
            pane.model.chat.latest = Some((900, claude_switch_core::transcript::ChatPhase::Active));
        }
        f.tab().term.screen = screens::recorded::recorded(PERM_READ_OUTSIDE);
        f.chat().output(now, true, false);
        f.sync();
        let card = &f.chat().question;
        // With its call: the call's card; without (a new session whose transcript Chat has not read
        // yet): the prompt read whole from the screen. Either answers it here.
        assert!(if known { card.approval && card.answerable == Some(900) } else { card.prompt_answers() }, "{known}");
        assert_eq!(f.decide(now + QUIET), Some(Reason::Approval), "{known}");
    }
}

/// The question of the recorded `preview_*` screens (Claude Code 2.1.332, `recorder compat`
/// script `preview`), as the transcript holds it.
fn layout_question() -> Question {
    let option = |label: &str, preview: &str| QuestionOption {
        label: label.into(),
        description: String::new(),
        preview: preview.into(),
    };
    Question {
        header: "Layout".into(),
        question: "Which layout do you prefer?".into(),
        options: vec![
            option("Boxed", "+-----+\n| Map |\n+-----+"),
            option("Tree", "cache/\n  12/\n  13/"),
            option("Plain", ""),
        ],
        multi_select: false,
    }
}

/// The form of the recorded `preview_form*` screens (script `preview-form`).
fn preview_form() -> Vec<Question> {
    let option = |label: &str| QuestionOption { label: label.into(), ..Default::default() };
    let mut layout = layout_question();
    layout.options.truncate(2);
    vec![
        layout,
        Question {
            header: "Extras".into(),
            question: "Which extras should it keep?".into(),
            options: vec![option("Logs"), option("Metrics"), option("Traces")],
            multi_select: true,
        },
        Question {
            header: "Name".into(),
            question: "How long a name?".into(),
            options: vec![option("Short"), option("Long")],
            multi_select: false,
        },
    ]
}

#[test]
fn a_question_with_previews_reads_from_the_recorded_screen_without_its_box() {
    use screens::recorded::{PREVIEW_ASKED, PREVIEW_NONE, recorded};
    let screen = recorded(PREVIEW_ASKED);
    let (col, rows) = screens::side_box(&screen).expect("the box beside the choices");
    assert_eq!(col, 34);
    let view = screens::question_view(&screen);
    let notes = (0..view.rows()).any(|r| view.row_text(r).contains("Notes:"));
    assert!(!notes && rows.len() >= 5, "the notes row goes with the box");
    // The cut keeps the modes the arrow keys are written for and the cursor.
    assert_eq!(view.modes(), screen.modes());
    assert_eq!((view.cursor().row, view.cursor().col), (screen.cursor().row, screen.cursor().col));
    // No "Type something" with previews; "Chat about this" carries no number but is walked to.
    let (menu, other, chat) = question_menu(&screen, &layout_question(), false).expect("the question");
    assert_eq!(menu.texts(), ["Boxed", "Tree", "Plain", "Chat about this"]);
    assert_eq!((menu.highlight(), other, chat), (Some(0), None, Some(3)));
    // The menu card read from the screen alone: the question, its choices, no field.
    let shown = menu_shown(&screen).expect("the menu card");
    assert!(shown.question && shown.other.is_none());
    assert_eq!((shown.title.as_str(), shown.header.as_str()), ("Which layout do you prefer?", "Layout"));
    assert_eq!(shown.choices, ["Boxed", "Tree", "Plain", "Chat about this"]);
    assert!(shown.notes.iter().all(String::is_empty), "{:?}", shown.notes);
    // The option without a preview: "No preview available" in a shorter box.
    let none = recorded(PREVIEW_NONE);
    let (menu, ..) = question_menu(&none, &layout_question(), false).expect("the question");
    assert_eq!(menu.highlight(), Some(2));
    // A label that differs fails closed.
    let mut other = layout_question();
    other.options[1].label = "Trees".into();
    assert!(question_menu(&screen, &other, false).is_none());
}

#[test]
fn a_box_near_the_left_edge_or_without_a_menu_footer_is_no_preview_box() {
    let footer = "Enter to select · ↑/↓ to navigate · Esc to cancel";
    let draw = |indent: &str, footer: &str| {
        let mut screen = Screen::new(80, 12, 0);
        let rows = [
            format!("  1. A     {indent}┌──────────┐"),
            format!("  2. B     {indent}│ text     │"),
            format!("           {indent}└──────────┘"),
            String::new(),
            footer.to_owned(),
        ];
        screen.feed(rows.join("\r\n").as_bytes());
        screen
    };
    assert!(screens::side_box(&draw("    ", footer)).is_some());
    let mut near = Screen::new(80, 12, 0);
    near.feed(format!("┌──────────┐\r\n│ text     │\r\n└──────────┘\r\n{footer}").as_bytes());
    assert!(screens::side_box(&near).is_none());
    assert!(screens::side_box(&draw("    ", "")).is_none());
}

#[test]
fn a_form_with_previews_is_driven_over_the_recorded_pages_and_review() {
    use screens::recorded::{PREVIEW_FORM, PREVIEW_FORM_EXTRAS, PREVIEW_FORM_REVIEW, PREVIEW_FORM_TREE, recorded};
    let qs = preview_form();
    let first = recorded(PREVIEW_FORM);
    let shown = form_question(&first, &qs, 0).expect("the first page, without its box");
    assert_eq!((shown.other, shown.chat, shown.menu.highlight()), (None, 2, Some(0)));
    assert!(form_question(&first, &qs, 1).is_none());
    let tree = recorded(PREVIEW_FORM_TREE);
    assert_eq!(form_question(&tree, &qs, 0).and_then(|s| s.menu.highlight()), Some(1));
    let extras = recorded(PREVIEW_FORM_EXTRAS);
    let shown = form_question(&extras, &qs, 1).expect("the multi-select page");
    assert_eq!((shown.other, shown.submit, shown.marks.as_slice()), (Some(3), Some(4), &[false, false, false][..]));
    // Tree, then Logs with "alerts", then Short: the first key moves to Tree, then Enter.
    let answers = vec![
        Answer { picks: vec![1], text: None },
        Answer { picks: vec![0], text: Some("alerts".into()) },
        Answer { picks: vec![0], text: None },
    ];
    let mut run = FormRun { questions: qs.clone(), answers, expect: Expect::Opened(0), why: NOT_SHOWN };
    let (key, expect, _) = form_next(&run, &first).expect("a key");
    assert_eq!(key, down(1));
    run.expect = expect;
    let (key, expect, _) = form_next(&run, &tree).expect("Enter on Tree");
    assert_eq!((key, expect), (b"\r".to_vec(), Expect::Opened(1)));
    // A typed answer on a page without "Type something" stops before any key.
    let mut typed = run.clone();
    typed.answers[0] = Answer { picks: vec![], text: Some("mine".into()) };
    typed.expect = Expect::Opened(0);
    assert_eq!(form_next(&typed, &first).err(), Some(NO_FIELD));
    // The review shows every answer as the card holds them: Enter on "Submit answers".
    run.expect = Expect::Review(None);
    let (key, expect, _) = form_next(&run, &recorded(PREVIEW_FORM_REVIEW)).expect("Submit answers");
    assert_eq!((key, expect), (b"\r".to_vec(), Expect::Submitted));
}

#[test]
fn a_question_with_previews_shows_the_one_under_the_pointer_or_the_keys_and_copies_plain_text() {
    let mut f = Fixture::new(vec![layout_question()]);
    let now = Instant::now();
    f.screen("\x1b[2J", now);
    let tab = f.tab();
    let pane = tab.pane.as_mut().unwrap();
    pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen);
    let lay = |f: &mut Fixture, width: f32| {
        let pane = f.tab().pane.as_mut().unwrap();
        let mut parts = cards::parts(&pane.chat, &pane.model, width).expect("the question's card");
        let options = layout_question().options;
        preview::lay(&mut pane.chat.question, &mut parts, width, 1.0, &options);
        parts
    };
    // Wide: beside the choices, as tall as the tallest preview.
    let parts = lay(&mut f, 900.0);
    let side = parts.iter().find_map(|p| match p {
        cards::Part::Choices { side, .. } => Some(*side),
        _ => None,
    });
    let size = side.flatten().expect("the preview beside the list");
    assert_eq!(size.rows, 3);
    assert!(!parts.iter().any(|p| matches!(p, cards::Part::Preview(_))));
    // Narrow: under them.
    let parts = lay(&mut f, 420.0);
    assert!(parts.iter().any(|p| matches!(p, cards::Part::Preview(s) if s.w == 420.0 && s.rows == 3)));
    let previews = &mut f.chat().question.previews;
    assert_eq!(previews.copy_text(false).as_deref(), Some(layout_question().options[0].preview.as_str()));
    // ↓ shows the next option's (while the card is in view); the pointer over the third shows
    // it has none.
    f.chat().question.rect = Rect::new(0.0, 0.0, 420.0, 400.0);
    assert!(key(f.tab(), now, &KeyName::Down, Mods::default(), None));
    assert_eq!(
        f.chat().question.previews.copy_text(false).as_deref(),
        Some(layout_question().options[1].preview.as_str())
    );
    assert!(f.writes().is_empty(), "the keys only move the preview");
    f.chat().question.previews.hover(2);
    assert_eq!(f.chat().question.previews.copy_text(false), None);
    // Nothing to copy without a selection; the press goes nowhere near the terminal.
    f.chat().question.previews.hover(0);
    assert_eq!(f.chat().question.previews.copy_text(true), None);
    assert!(!press(f.tab(), now, Hit::Preview) && !press(f.tab(), now, Hit::CopyPreview));
    assert!(f.writes().is_empty());
    // A question without previews has no preview part.
    let mut plain = Fixture::new(vec![question(false)]);
    let pane = plain.tab().pane.as_mut().unwrap();
    let mut parts = cards::parts(&pane.chat, &pane.model, 700.0).unwrap();
    preview::lay(&mut pane.chat.question, &mut parts, 700.0, 1.0, &question(false).options);
    assert!(parts.iter().all(|p| !matches!(p, cards::Part::Preview(_) | cards::Part::Choices { side: Some(_), .. })));
}
