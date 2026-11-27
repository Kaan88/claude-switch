//! The card for a permission prompt read from the screen alone, on a tab gated like a real one and
//! fed the prompts recorded from Claude Code 2.1.332: a subagent's (the main transcript holds no
//! pending call then) and the two prompts of two calls made in one message.
use super::*;
use crate::view::chat::question::screens::recorded::{
    PERM_AGENT_BASH, PERM_AGENT_BASH_DOWN, PERM_AGENT_BASH_NO, PERM_AGENT_GONE, PERM_AGENT_WRITE, PERM_TWO_FIRST,
    PERM_TWO_SECOND, PLAN, recorded,
};
use crate::view::chat::submission::ready_tests::{Live, downs};
use crate::view::chat::{Gate, QUIET, Reason};

const TICK: Duration = Duration::from_millis(100);

impl Live {
    /// Lays Chat out in a window of this size and paints the card: what can be clicked.
    pub(in crate::view::chat::question) fn paint(&mut self, w: f32, h: f32) -> Vec<Hit> {
        use mdshared_text::Bitmap;
        use mdshared_ui::gfx::Gfx;
        let now = self.now;
        self.app.layout_chat(Rect::new(0.0, 0.0, w, h));
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(w as u32, h as u32);
        let mut hits = Vec::new();
        {
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            let pal = mdshared_ui::theme::DARK;
            p.fill(Rect::new(0.0, 0.0, w, h), pal.page);
            let pane = self.tab().pane.as_mut().unwrap();
            Draw { pal: &pal, hover: None, focused: true, now }.card(&mut p, &mut pane.chat, &pane.model, &mut hits);
        }
        if let Ok(dir) = std::env::var("CS_SHOT_DIR") {
            let name = format!("prompt_card_{}x{}_{}.png", w as u32, h as u32, hits.len());
            bmp.write_png(&std::path::Path::new(&dir).join(name)).unwrap();
        }
        let card = self.chat().question.rect;
        hits.into_iter()
            .filter(|(r, _)| r.bottom() <= card.bottom() + 0.5)
            .filter_map(|(_, target)| match target {
                Target::Chat(Control::Question(hit)) => Some(hit),
                _ => None,
            })
            .collect()
    }

    /// The card paints the prompt in a roomy window, and the moment a click needs passes.
    pub(in crate::view::chat::question) fn read_card(&mut self) -> Vec<Hit> {
        let hits = self.paint(1000.0, 900.0);
        self.pass(SEEN);
        hits
    }

    pub(in crate::view::chat::question) fn click(&mut self, hit: Hit) -> bool {
        let now = self.now;
        crate::view::chat::question::press(self.tab(), now, hit)
    }

    pub(in crate::view::chat::question) fn card(&mut self) -> &Card {
        &self.chat().question
    }
}

#[test]
fn a_subagents_prompt_shows_whole_in_the_card_and_is_answered_with_checked_keys() {
    let mut f = Live::new();
    f.turn();
    f.show(PERM_AGENT_BASH);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Approval)));
    // The main transcript holds no pending call: the subagent's call is in its own transcript.
    assert!(f.tab().pane.as_ref().unwrap().model.chat.pending.is_none());
    assert_eq!(f.chat().drawer, Drawer::Closed);
    assert_eq!(f.chat().prompt_waiting(), Some("Waiting for your approval"));
    let shown = f.card().prompt.clone().expect("the prompt as the screen has it");
    assert!(shown.whole && f.card().shown.is_none() && f.card().menu.is_none());
    assert_eq!(shown.rows[0], "Bash command · from the general-purpose agent");
    assert_eq!(shown.rows[4], "mkdir sub_probe_five");
    assert_eq!(shown.question, "Do you want to proceed?");
    assert_eq!(shown.choices.len(), 4);
    // A send says to answer first.
    f.send("later");
    assert!(f.writes().is_empty() && f.chat().queue.is_none());
    assert_eq!(f.chat().error, Some(Reason::Approval.label()));

    // Nothing the card has not shown: no key before it painted the prompt.
    assert!(f.click(Hit::Option(1)));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(READ_ALL)));
    let hits = f.read_card();
    for choice in 0..4 {
        assert!(hits.contains(&Hit::Option(choice)), "choice {choice} can be clicked");
    }
    assert!(hits.contains(&Hit::Escape) && hits.contains(&Hit::Open) && !hits.contains(&Hit::More));

    // The arrows first; Enter only once the screen shows the highlight on that choice.
    assert!(f.click(Hit::Option(1)));
    assert_eq!(f.writes(), [downs(1)]);
    f.pass(TICK);
    assert!(f.writes().is_empty());
    f.show(PERM_AGENT_BASH_DOWN);
    f.pass(TICK);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // The prompt goes: the answer is done, and so is the card.
    f.show(PERM_AGENT_GONE);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().prompt.is_none() && f.card().error.is_none());
    assert!(f.writes().is_empty());
}

#[test]
fn a_prompt_whose_rows_change_between_the_click_and_the_key_gets_no_enter() {
    let mut f = Live::new();
    f.turn();
    f.show(PERM_AGENT_BASH);
    f.pass(QUIET);
    f.read_card();
    assert!(f.click(Hit::Option(3)));
    assert_eq!(f.writes(), [downs(3)]);
    // The highlight arrives on "No", but the prompt now names another folder.
    let other = PERM_AGENT_BASH_NO.replace("sub_probe_five", "sub_probe_six");
    f.show(&other);
    f.pass(TICK);
    assert!(f.writes().is_empty(), "not the prompt the card showed");
    f.pass(STEP);
    assert!(f.writes().is_empty());
    assert_eq!(f.card().error, Some(CHANGED));
    assert!(f.card().step.is_none());
    // The card now shows what the screen has, and answers that only after it was shown.
    assert_eq!(f.card().prompt.as_ref().unwrap().rows[4], "mkdir sub_probe_six");
    assert!(f.click(Hit::Option(3)));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(READ_ALL)));

    // Another prompt altogether takes its place before the click's first key: nothing is sent.
    let mut f = Live::new();
    f.turn();
    f.show(PERM_AGENT_BASH);
    f.pass(QUIET);
    f.read_card();
    f.tab().term.screen = recorded(PERM_AGENT_WRITE);
    assert!(f.click(Hit::Option(0)));
    assert!(f.writes().is_empty());
    assert_eq!(f.card().error, Some(READ_ALL), "the new prompt has not been shown yet");
}

#[test]
fn two_calls_of_one_message_are_answered_one_prompt_at_a_time() {
    let mut f = Live::new();
    f.turn();
    f.show(PERM_TWO_FIRST);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Approval)));
    assert_eq!(f.card().prompt.as_ref().unwrap().rows[0], "Bash command", "the prompt names no source");
    assert_eq!(f.card().prompt.as_ref().unwrap().rows[4], "mkdir par_a");
    f.read_card();
    // Yes is highlighted already: Enter at once.
    assert!(f.click(Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    // The second call's prompt takes its place: the first answer is done.
    f.show(PERM_TWO_SECOND);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().error.is_none());
    assert_eq!(f.card().prompt.as_ref().unwrap().rows[4], "mkdir par_b");
    // A second click meant for the first prompt must not answer this one: it has only just
    // been painted.
    f.paint(1000.0, 900.0);
    assert!(f.click(Hit::Option(0)));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(NOT_SEEN)));
    f.pass(SEEN);
    assert!(f.click(Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);
}

#[test]
fn esc_declines_only_while_the_screen_shows_the_same_prompt() {
    let mut f = Live::new();
    f.turn();
    f.show(PERM_AGENT_BASH);
    f.pass(QUIET);
    // Declining needs no reading to the end.
    assert!(f.click(Hit::Escape));
    assert_eq!(f.writes(), [b"\x1b".to_vec()]);
    f.show(PERM_AGENT_GONE);
    f.pass(TICK);
    assert!(f.card().step.is_none() && f.card().error.is_none());

    let mut f = Live::new();
    f.turn();
    f.show(PERM_AGENT_BASH);
    f.pass(QUIET);
    f.tab().term.screen = recorded(PERM_AGENT_GONE);
    f.click(Hit::Escape);
    assert!(f.writes().is_empty(), "no prompt on the screen: no Esc");
}

#[test]
fn with_answer_menus_off_the_prompt_stays_in_the_terminal() {
    let mut f = Live::new();
    f.app.config.chat.answer_menus = false;
    f.turn();
    f.show(PERM_AGENT_BASH);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Permission)));
    assert!(f.card().prompt.is_none());
    assert_eq!(f.chat().drawer, Drawer::Auto(Reason::Permission));
    assert_eq!(f.chat().prompt_waiting(), None);
    f.click(Hit::Option(0));
    assert!(f.writes().is_empty());
}

#[test]
fn the_plan_approval_is_not_taken_for_a_permission_prompt() {
    let mut f = Live::new();
    f.turn();
    f.show(PLAN);
    f.pass(QUIET);
    assert!(f.card().prompt.is_none(), "its menu has no No choice: the plan's own card answers it");
}

/// A prompt with a command of `lines` lines, as the CLI draws it at 100 columns.
fn tall(lines: usize) -> String {
    let mut rows = vec!["─".repeat(100), " Bash command".to_owned(), " Run a long script".to_owned(), "╌".repeat(100)];
    rows.extend((1..=lines).map(|n| format!(" echo step {n}")));
    rows.push("╌".repeat(100));
    rows.extend(
        [" Do you want to proceed?", " ❯ 1. Yes", "   2. No", "", " Esc to cancel · Tab to amend"].map(str::to_owned),
    );
    rows.join("\r\n")
}

#[test]
fn a_prompt_longer_than_the_card_takes_a_choice_only_once_it_was_scrolled_to_its_end() {
    let mut f = Live::new();
    f.turn();
    let now = f.now;
    let tab = f.tab();
    tab.term.screen = mdshared_term::Screen::new(100, 40, 0);
    tab.term.screen.feed(format!("\x1b[?2004h{}", tall(24)).as_bytes());
    f.chat().output(now, true, false);
    assert_eq!(f.pass(QUIET), (Gate::Check, Some(Reason::Approval)));
    assert_eq!(f.card().prompt.as_ref().unwrap().rows.len(), 28);
    // A small window: the card has no room for 28 rows.
    let hits = f.paint(900.0, 520.0);
    assert!(hits.contains(&Hit::More) && hits.contains(&Hit::Open) && hits.contains(&Hit::Escape));
    assert!(!hits.iter().any(|hit| matches!(hit, Hit::Option(_))), "no choice before the end was shown");
    f.pass(SEEN);
    assert!(f.click(Hit::Option(0)));
    assert_eq!((f.writes().len(), f.card().error), (0, Some(READ_ALL)));
    // More, or the wheel, scrolls on; once every row has been in view the choices can be clicked.
    let mut clicks = 0;
    while f.paint(900.0, 520.0).contains(&Hit::More) {
        assert!(f.click(Hit::More));
        clicks += 1;
        assert!(clicks < 30, "More ends");
    }
    assert!(clicks > 0 && f.writes().is_empty());
    let hits = f.paint(900.0, 520.0);
    assert!(hits.contains(&Hit::Option(0)) && hits.contains(&Hit::Option(1)));
    assert!(f.click(Hit::Option(0)));
    assert_eq!(f.writes(), [b"\r".to_vec()]);

    // The wheel does the same, only over the card.
    let mut f = Live::new();
    f.turn();
    let now = f.now;
    let tab = f.tab();
    tab.term.screen = mdshared_term::Screen::new(100, 40, 0);
    tab.term.screen.feed(format!("\x1b[?2004h{}", tall(24)).as_bytes());
    f.chat().output(now, true, false);
    f.pass(QUIET);
    f.paint(900.0, 520.0);
    let card = f.card().rect;
    assert!(!f.chat().question_wheel(card.x - 5.0, card.y - 5.0, -1.0), "not outside the card");
    for _ in 0..12 {
        assert!(f.chat().question_wheel(card.cx(), card.cy(), -1.0));
        f.paint(900.0, 520.0);
    }
    assert!(f.paint(900.0, 520.0).contains(&Hit::Option(0)));
}
