//! Screens recorded from Claude Code 2.1.332 (Haiku, 100 x 40 cells, classic renderer) on
//! 2026-11-27, with personal paths and names replaced. Each fixture is the screen's rows and where
//! the CLI left the cursor. When a Claude Code update changes one of these screens, record it again
//! the same way and keep the old one if both versions are in use.
use super::*;

/// The fixture's rows on its screen (100 x 40 unless it says otherwise), with the cursor where the
/// CLI left it. After the cursor line a fixture may say `@size cols rows`, `@wrapped row…` (rows
/// the terminal marked as wrapping onto the next) and `@modes [paste] [alt] [cursor]`; a fixture
/// without `@modes` has bracketed paste on, as Claude Code sets it.
pub(in crate::view::chat) fn recorded(fixture: &str) -> Screen {
    let mut lines = fixture.lines().peekable();
    let cursor = lines.next().and_then(|l| l.strip_prefix("cursor ")).expect("cursor line");
    let (row, col) = cursor.split_once(' ').expect("row and column");
    let (row, col): (usize, usize) = (row.parse().unwrap(), col.parse().unwrap());
    let (mut cols, mut rows, mut wrapped, mut modes) = (100, 40, Vec::new(), "paste");
    while let Some(head) = lines.peek().and_then(|l| l.strip_prefix('@')) {
        let numbers = |s: &str| s.split(' ').map(|n| n.parse::<usize>().unwrap()).collect::<Vec<_>>();
        if let Some(size) = head.strip_prefix("size ") {
            (cols, rows) = (numbers(size)[0], numbers(size)[1]);
        } else if let Some(rows) = head.strip_prefix("wrapped ") {
            wrapped = numbers(rows);
        } else if let Some(set) = head.strip_prefix("modes") {
            modes = set;
        } else {
            break;
        }
        lines.next();
    }
    let mut screen = Screen::new(cols, rows, 0);
    for (i, line) in lines.enumerate() {
        if wrapped.contains(&i) {
            // Two characters from the last column: the second wraps and marks the row. The
            // first is then blanked; a character there does not end the mark.
            screen.feed(format!("\x1b[{row};{cols}Hxx\x1b[2K\x1b[{row};{cols}H ", row = i + 1).as_bytes());
        }
        screen.feed(format!("\x1b[{};1H{line}", i + 1).as_bytes());
    }
    if modes.contains("paste") {
        screen.feed(b"\x1b[?2004h");
    }
    if modes.contains("alt") {
        screen.feed(b"\x1b[?1049h");
    }
    screen.feed(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    screen
}

pub(in crate::view::chat) const ASK_SINGLE: &str = include_str!("../fixtures/ask_single.txt");
pub(in crate::view::chat) const ASK_SINGLE_OTHER: &str = include_str!("../fixtures/ask_single_other.txt");
pub(in crate::view::chat) const ASK_MULTI: &str = include_str!("../fixtures/ask_multi.txt");
pub(in crate::view::chat) const ASK_TWO: &str = include_str!("../fixtures/ask_two.txt");
pub(in crate::view::chat) const ASK_REVIEW: &str = include_str!("../fixtures/ask_review.txt");
pub(in crate::view::chat) const FORM_MULTI: &str = include_str!("../fixtures/form_multi.txt");
pub(in crate::view::chat) const FORM_MULTI_APPLE: &str = include_str!("../fixtures/form_multi_apple.txt");
pub(in crate::view::chat) const FORM_MULTI_PLUM: &str = include_str!("../fixtures/form_multi_plum.txt");
pub(in crate::view::chat) const FORM_MULTI_TYPE: &str = include_str!("../fixtures/form_multi_type.txt");
pub(in crate::view::chat) const FORM_MULTI_TYPED: &str = include_str!("../fixtures/form_multi_typed.txt");
pub(in crate::view::chat) const FORM_MULTI_NEXT: &str = include_str!("../fixtures/form_multi_next.txt");
pub(in crate::view::chat) const FORM_SECOND: &str = include_str!("../fixtures/form_second.txt");
pub(in crate::view::chat) const FORM_REVIEW: &str = include_str!("../fixtures/form_review.txt");
pub(in crate::view::chat) const MULTI_SUBMIT: &str = include_str!("../fixtures/multi_submit.txt");
pub(in crate::view::chat) const MULTI_REVIEW: &str = include_str!("../fixtures/multi_review.txt");
pub(in crate::view::chat) const PERM_BASH: &str = include_str!("../fixtures/perm_bash.txt");
/// Read of a file outside the project folder (Claude Code 2.1.332, a handover's file in manual
/// mode): Yes, Yes for that folder this session, No.
pub(in crate::view::chat) const PERM_READ_OUTSIDE: &str = include_str!("../fixtures/perm_read_outside.ansi");
pub(in crate::view::chat) const PERM_WRITE: &str = include_str!("../fixtures/perm_write.txt");
pub(in crate::view::chat) const PERM_FETCH_CLIPPED: &str = include_str!("../fixtures/perm_fetch_clipped.txt");
pub(in crate::view::chat) const IDLE_NBSP: &str = include_str!("../fixtures/idle_nbsp.txt");
pub(in crate::view::chat) const PLAN: &str = include_str!("../fixtures/plan.txt");
pub(in crate::view::chat) const MENU_MODEL: &str = include_str!("../fixtures/menu_model.txt");
pub(in crate::view::chat) const MENU_EFFORT: &str = include_str!("../fixtures/menu_effort.txt");
pub(in crate::view::chat) const PANEL_USAGE: &str = include_str!("../fixtures/panel_usage.txt");
pub(in crate::view::chat) const MENU_CONFIG: &str = include_str!("../fixtures/menu_config.txt");
pub(in crate::view::chat) const BG_HINT: &str = include_str!("../fixtures/bg_hint.txt");
pub(in crate::view::chat) const BG_MOVED: &str = include_str!("../fixtures/bg_moved.txt");
pub(in crate::view::chat) const AGENTS_IDLE: &str = include_str!("../fixtures/agents_idle.txt");
pub(in crate::view::chat) const AGENTS_OPEN: &str = include_str!("../fixtures/agents_open.txt");
pub(in crate::view::chat) const AGENTS_FIRST: &str = include_str!("../fixtures/agents_first.txt");
pub(in crate::view::chat) const AGENTS_SECOND: &str = include_str!("../fixtures/agents_second.txt");
pub(in crate::view::chat) const AGENTS_STOPPED: &str = include_str!("../fixtures/agents_stopped.txt");
pub(in crate::view::chat) const AGENTS_CLOSED: &str = include_str!("../fixtures/agents_closed.txt");
pub(in crate::view::chat) const AGENTS_SHELL: &str = include_str!("../fixtures/agents_shell.txt");
pub(in crate::view::chat) const REWIND_IDLE: &str = include_str!("../fixtures/rewind_idle.txt");
pub(in crate::view::chat) const REWIND_OPEN: &str = include_str!("../fixtures/rewind_open.txt");
pub(in crate::view::chat) const REWIND_UP1: &str = include_str!("../fixtures/rewind_up1.txt");
pub(in crate::view::chat) const REWIND_UP2: &str = include_str!("../fixtures/rewind_up2.txt");
pub(in crate::view::chat) const REWIND_ACTIONS: &str = include_str!("../fixtures/rewind_actions.txt");
pub(in crate::view::chat) const REWIND_CLOSED: &str = include_str!("../fixtures/rewind_closed.txt");
pub(in crate::view::chat) const REWIND_ACTIONS_CODE: &str = include_str!("../fixtures/rewind_actions_code.txt");
pub(in crate::view::chat) const REWIND_ACTIONS_CODE_DOWN: &str =
    include_str!("../fixtures/rewind_actions_code_down.txt");
pub(in crate::view::chat) const REWIND_DONE: &str = include_str!("../fixtures/rewind_done.txt");
pub(in crate::view::chat) const REWIND_CUT_UP1: &str = include_str!("../fixtures/rewind_cut_up1.txt");
pub(in crate::view::chat) const REWIND_CUT_UP2: &str = include_str!("../fixtures/rewind_cut_up2.txt");
pub(in crate::view::chat) const PLAIN_IDLE: &str = include_str!("../fixtures/plain_idle.ansi");
/// A named session's idle input: the name sits in the rule above it (150 x 40, auto mode).
pub(in crate::view::chat) const PLAIN_NAMED: &str = include_str!("../fixtures/plain_named.ansi");
/// The folder trust dialog of a new folder (120 x 40, the path replaced).
pub(in crate::view::chat) const PLAIN_TRUST: &str = include_str!("../fixtures/plain_trust.ansi");
pub(in crate::view::chat) const PLAIN_START: &str = include_str!("../fixtures/plain_start.ansi");
pub(in crate::view::chat) const PLAIN_TYPED: &str = include_str!("../fixtures/plain_typed.ansi");
pub(in crate::view::chat) const PLAIN_QUESTION: &str = include_str!("../fixtures/plain_question.ansi");
pub(in crate::view::chat) const PLAIN_PERM_BASH: &str = include_str!("../fixtures/plain_perm_bash.ansi");
pub(in crate::view::chat) const PLAIN_FORM: &str = include_str!("../fixtures/plain_form.ansi");
pub(in crate::view::chat) const PLAIN_SUGGEST: &str = include_str!("../fixtures/plain_suggest.ansi");
pub(in crate::view::chat) const WORKING_TOOL: &str = include_str!("../fixtures/working_tool.ansi");
pub(in crate::view::chat) const PLAIN_WORKING_TYPED: &str = include_str!("../fixtures/plain_working_typed.ansi");
pub(in crate::view::chat) const PLAIN_QUEUED: &str = include_str!("../fixtures/plain_queued.ansi");
pub(in crate::view::chat) const DECLINED_WRAPPED: &str = include_str!("../fixtures/declined_wrapped.ansi");
pub(in crate::view::chat) const STATUS_FOOTER: &str = include_str!("../fixtures/status_footer.ansi");
pub(in crate::view::chat) const PLAIN_INTERRUPTED: &str = include_str!("../fixtures/plain_interrupted.ansi");
pub(in crate::view::chat) const PLAIN_BG_SHELL: &str = include_str!("../fixtures/plain_bg_shell.ansi");
pub(in crate::view::chat) const NARROW_WORKING: &str = include_str!("../fixtures/narrow_working.ansi");
pub(in crate::view::chat) const PLAIN_PASTED_MARKER: &str = include_str!("../fixtures/plain_pasted_marker.ansi");
pub(in crate::view::chat) const PLAIN_PASTED_LINES: &str = include_str!("../fixtures/plain_pasted_lines.ansi");
pub(in crate::view::chat) const PLAIN_LONG: &str = include_str!("../fixtures/plain_long.ansi");
pub(in crate::view::chat) const PLAIN_COMPACTING: &str = include_str!("../fixtures/plain_compacting.ansi");
pub(in crate::view::chat) const PLAIN_PLAN: &str = include_str!("../fixtures/plain_plan.ansi");
pub(in crate::view::chat) const PLAIN_RESTORED: &str = include_str!("../fixtures/plain_restored.ansi");
pub(in crate::view::chat) const PLAIN_EMPTIED: &str = include_str!("../fixtures/plain_emptied.ansi");
pub(in crate::view::chat) const ASK_OTHER_FIRST: &str = include_str!("../fixtures/ask_other_first.ansi");
pub(in crate::view::chat) const ASK_OTHER: &str = include_str!("../fixtures/ask_other.ansi");
pub(in crate::view::chat) const ASK_OTHER_TYPED: &str = include_str!("../fixtures/ask_other_typed.ansi");
pub(in crate::view::chat) const ASK_OTHER_LONG: &str = include_str!("../fixtures/ask_other_long.ansi");
pub(in crate::view::chat) const FORM_OTHER_FIRST: &str = include_str!("../fixtures/form_other_first.ansi");
pub(in crate::view::chat) const FORM_OTHER: &str = include_str!("../fixtures/form_other.ansi");
pub(in crate::view::chat) const FORM_OTHER_TYPED: &str = include_str!("../fixtures/form_other_typed.ansi");
pub(in crate::view::chat) const PLAIN_ASK_OTHER_TYPED: &str = include_str!("../fixtures/plain_ask_other_typed.ansi");
pub(in crate::view::chat) const PLAIN_FORM_OTHER_TYPED: &str = include_str!("../fixtures/plain_form_other_typed.ansi");
pub(in crate::view::chat) const PROMPT_IDLE: &str = include_str!("../fixtures/prompt_idle.ansi");
pub(in crate::view::chat) const PROMPT_SUGGEST: &str = include_str!("../fixtures/prompt_suggest.ansi");
pub(in crate::view::chat) const PROMPT_PASTED: &str = include_str!("../fixtures/prompt_pasted.ansi");
pub(in crate::view::chat) const PROMPT_CLEARED: &str = include_str!("../fixtures/prompt_cleared.ansi");
// Permission prompts no pending call of the main transcript covers (110 x 45, manual mode): a
// subagent's, with the highlight on its first, second and last choice and after Esc declined it,
// and the two prompts of two calls made in one message.
pub(in crate::view::chat) const PERM_AGENT_BASH: &str = include_str!("../fixtures/perm_agent_bash.ansi");
pub(in crate::view::chat) const PERM_AGENT_BASH_DOWN: &str = include_str!("../fixtures/perm_agent_bash_down.ansi");
pub(in crate::view::chat) const PERM_AGENT_BASH_NO: &str = include_str!("../fixtures/perm_agent_bash_no.ansi");
pub(in crate::view::chat) const PERM_AGENT_GONE: &str = include_str!("../fixtures/perm_agent_gone.ansi");
pub(in crate::view::chat) const PERM_AGENT_WRITE: &str = include_str!("../fixtures/perm_agent_write.ansi");
pub(in crate::view::chat) const PERM_TWO_FIRST: &str = include_str!("../fixtures/perm_two_first.ansi");
pub(in crate::view::chat) const PERM_TWO_SECOND: &str = include_str!("../fixtures/perm_two_second.ansi");
// The second of two prompts again (100 x 40), with the hidden cursor where the CLI left it: on the
// first call's output row, fifteen rows above the prompt's footer.
pub(in crate::view::chat) const PERM_TWO_AFTER: &str = include_str!("../fixtures/perm_two_after.ansi");
// A call auto mode's classifier blocked (110 x 46, auto mode), and /permissions after it: the idle
// input with the CLI's notice under it, the command typed, the "Recently blocked" tab with one call
// (not marked, marked to approve, marked to retry), after Esc applied an approval and a retry, the
// tab with two calls (the highlight on each, the second approved, the first to retry and then to
// approve), and the dialog's next tab.
pub(in crate::view::chat) const BLOCKED_IDLE: &str = include_str!("../fixtures/blocked_idle.ansi");
pub(in crate::view::chat) const BLOCKED_TYPED: &str = include_str!("../fixtures/blocked_typed.ansi");
pub(in crate::view::chat) const BLOCKED_ONE: &str = include_str!("../fixtures/blocked_one.ansi");
pub(in crate::view::chat) const BLOCKED_ONE_APPROVED: &str = include_str!("../fixtures/blocked_one_approved.ansi");
pub(in crate::view::chat) const BLOCKED_ONE_CLOSED: &str = include_str!("../fixtures/blocked_one_closed.ansi");
pub(in crate::view::chat) const BLOCKED_ONE_RETRY: &str = include_str!("../fixtures/blocked_one_retry.ansi");
pub(in crate::view::chat) const BLOCKED_RETRY_STARTED: &str = include_str!("../fixtures/blocked_retry_started.ansi");
pub(in crate::view::chat) const BLOCKED_TWO: &str = include_str!("../fixtures/blocked_two.ansi");
pub(in crate::view::chat) const BLOCKED_TWO_DOWN: &str = include_str!("../fixtures/blocked_two_down.ansi");
pub(in crate::view::chat) const BLOCKED_TWO_SECOND: &str = include_str!("../fixtures/blocked_two_second.ansi");
pub(in crate::view::chat) const BLOCKED_TWO_RETRY: &str = include_str!("../fixtures/blocked_two_retry.ansi");
pub(in crate::view::chat) const BLOCKED_TWO_APPROVE: &str = include_str!("../fixtures/blocked_two_approve.ansi");
pub(in crate::view::chat) const PERM_TAB_ALLOW: &str = include_str!("../fixtures/perm_tab_allow.ansi");

fn sends_blocked(screen: &Screen) -> bool {
    !crate::view::chat::submission::screen::clean_prompt(screen)
}

#[test]
fn an_idle_input_drawn_with_a_no_break_space_is_empty() {
    let screen = recorded(IDLE_NBSP);
    assert!(crate::view::chat::submission::screen::clean_prompt(&screen));
    assert!(!permission_like(&screen) && panel(&screen).is_none());
}

#[test]
fn a_single_choice_question_reads_its_options_and_highlight() {
    let screen = recorded(ASK_SINGLE);
    let menu = menu(&screen).expect("the question's menu");
    assert_eq!(menu.texts(), ["Red (warm)", "Green (calm)", "Blue (cool)", "Type something.", "Chat about this"]);
    assert_eq!(menu.highlight(), Some(0));
    assert!(menu.above(15).contains(&"Which colour do you prefer?"));
    assert!(!permission_like(&screen) && permission(&screen).is_none() && sends_blocked(&screen));
    assert_eq!(menu_with_ctrl_g(ASK_SINGLE_OTHER), Some(4), "the footer gains a Notepad hint on Chat about this");
}

fn menu_with_ctrl_g(fixture: &str) -> Option<usize> {
    menu(&recorded(fixture)).and_then(|m| m.highlight())
}

#[test]
fn multi_select_and_multi_question_forms_read_as_menus() {
    let multi = menu(&recorded(ASK_MULTI)).expect("multi-select menu");
    assert_eq!(multi.texts(), ["[✔] Apple", "[ ] Pear", "[ ] Plum", "[ ] Type something", "Chat about this"]);
    let two = menu(&recorded(ASK_TWO)).expect("first of two questions");
    assert_eq!(two.texts(), ["Small", "Large", "Type something.", "Chat about this"]);
    assert!(two.footer_text().contains("tab/arrow keys to navigate"));
    // The review step has no footer: it is no menu, and `review` reads it.
    assert!(menu(&recorded(ASK_REVIEW)).is_none());
}

#[test]
fn the_review_step_reads_its_questions_answers_and_highlight() {
    let pairs = |r: &Review| r.answers.iter().map(|(q, a)| format!("{q} = {a}")).collect::<Vec<_>>();
    let two = review(&recorded(ASK_REVIEW)).expect("the review of two questions");
    assert_eq!(pairs(&two), ["Which size? = Small", "Which speed? = Slow"]);
    assert_eq!(two.highlight, Some(0));
    let form = review(&recorded(FORM_REVIEW)).expect("the review with a multi-select");
    assert_eq!(pairs(&form), ["Which fruits do you like? = Apple, Plum, Kiwi", "Which size? = Large"]);
    let multi = review(&recorded(MULTI_REVIEW)).expect("one multi-select question is reviewed too");
    assert_eq!(pairs(&multi), ["Which fruits do you like? = Pear"]);
    for fixture in [ASK_REVIEW, FORM_REVIEW, MULTI_REVIEW] {
        let screen = recorded(fixture);
        assert!(!permission_like(&screen));
        // The review has no footer: it is a menu of its two entries all the same, for the card
        // that shows a question the transcript does not hold yet.
        let review = panel(&screen).expect("the review as a menu");
        assert_eq!(review.menu.expect("its entries").texts(), ["Submit answers", "Cancel"]);
        assert!(review.question && review.title.starts_with("Review your answers: "));
        assert!(sends_blocked(&screen));
    }
    for fixture in [ASK_SINGLE, ASK_TWO, ASK_MULTI, FORM_MULTI, FORM_SECOND, PERM_BASH, PLAN, IDLE_NBSP] {
        assert!(review(&recorded(fixture)).is_none());
    }
}

#[test]
fn the_review_step_is_read_over_blank_rows_and_the_inputs_rule_under_it() {
    // Under "2. Cancel" Claude Code may leave blank rows and the input's rule, plain or named:
    // taken for the review's last row, the review was not read (owner, 2026-11-28).
    let review_over = |under: &str| {
        let rows = [
            "─".repeat(20).as_str(),
            "←  ☒ Size  ✔ Submit  →",
            "",
            "Review your answers",
            "",
            " │ ● Which size should the test use? A long question",
            " │   that wraps onto a second row.",
            "   → Small",
            "",
            "❯ 1. Submit answers",
            "  2. Cancel",
        ]
        .join("\n");
        recorded(&format!("cursor 9 2\n{rows}\n{under}"))
    };
    let named = "──────────────────── Test session ─";
    for under in [format!("\n\n\n\n\n{named}"), format!("\n\n{}", "─".repeat(20)), String::new()] {
        let screen = review_over(&under);
        let read = review(&screen).expect("the review");
        assert_eq!(read.highlight, Some(0));
        let question = "Which size should the test use? A long question that wraps onto a second row.";
        assert_eq!(read.answers, [(question.to_owned(), "Small".to_owned())]);
        let panel = panel(&screen).expect("the review as a menu");
        assert_eq!(panel.menu.expect("its entries").texts(), ["Submit answers", "Cancel"]);
    }
    // Text under "Cancel" is no review.
    assert!(review(&review_over("\nsomething else")).is_none());
}

#[test]
fn a_multi_select_shows_its_submit_or_next_row_between_type_something_and_chat_about_this() {
    let row = |fixture: &str| {
        let screen = recorded(fixture);
        let menu = menu(&screen).expect("a question's menu");
        submit_row(&screen, &menu, 3).map(|e| (e.text, e.highlighted))
    };
    assert_eq!(row(ASK_MULTI), Some(("Submit".to_owned(), false)), "the only question");
    assert_eq!(row(MULTI_SUBMIT), Some(("Submit".to_owned(), true)));
    assert_eq!(row(FORM_MULTI), Some(("Next".to_owned(), false)), "another question follows");
    assert_eq!(row(FORM_MULTI_TYPED), Some(("Next".to_owned(), false)));
    assert_eq!(row(FORM_MULTI_NEXT), Some(("Next".to_owned(), true)));
    let next = menu(&recorded(FORM_MULTI_NEXT)).unwrap();
    assert_eq!(next.highlight(), None, "no numbered entry is highlighted then");
    assert_eq!(next.texts()[3], "[✔] Kiwi", "typed text ticks its box");
    let single = recorded(ASK_TWO);
    assert_eq!(submit_row(&single, &menu(&single).unwrap(), 2), None, "a single choice has none");
}

#[test]
fn a_bash_permission_prompt_keeps_to_its_box_and_joins_wrapped_choices() {
    let screen = recorded(PERM_BASH);
    let p = permission(&screen).expect("a permission prompt");
    assert_eq!(p.question, "Do you want to proceed?");
    assert_eq!(
        p.body,
        [
            "Bash command",
            "Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below",
            "Create directory probe_dir_one",
            "mkdir probe_dir_one",
        ]
    );
    let texts = p.menu.texts();
    assert_eq!(texts.len(), 4);
    assert!(texts[1].starts_with("Yes, and always allow access to ") && texts[1].ends_with(" from this project"));
    assert_eq!(texts[3], "No");
    assert_eq!((p.tell, p.menu.highlight()), (None, Some(0)));
    assert!(p.menu.footer_text().contains(AMEND));
    assert!(permission_like(&screen) && panel(&screen).is_none() && sends_blocked(&screen));
}

#[test]
fn a_write_permission_prompt_shows_the_file_and_its_content() {
    let p = permission(&recorded(PERM_WRITE)).expect("a permission prompt");
    assert_eq!(p.question, "Do you want to create probe_note.txt?");
    assert_eq!(p.body, ["Create file", "probe_note.txt", "1 hello"]);
    assert_eq!(
        p.menu.texts()[1],
        "Yes, and switch to accept edits (auto-approve file edits and common file commands) for this session (shift+tab)"
    );
}

#[test]
fn a_prompt_taller_than_the_terminal_loses_its_footer_and_is_not_answered() {
    let screen = recorded(PERM_FETCH_CLIPPED);
    assert!(permission(&screen).is_none());
    assert!(sends_blocked(&screen));
}

#[test]
fn the_plan_approval_reads_with_its_wrapped_footer_and_new_wording() {
    let screen = recorded(PLAN);
    let menu = plan(&screen).expect("the plan approval");
    assert_eq!(menu.texts(), ["Yes, and use auto mode", "Yes, manually approve edits", "Tell Claude what to change"]);
    assert_eq!(plan_keep(&menu), Some(2));
    assert!(menu.footer_text().contains("ctrl+g to edit"));
    assert!(sends_blocked(&screen));
}

#[test]
fn menus_and_panels_with_the_cursor_parked_elsewhere_are_cards_and_block_sends() {
    for (name, fixture, choices) in
        [("model", MENU_MODEL, 5), ("effort", MENU_EFFORT, 0), ("usage", PANEL_USAGE, 0), ("config", MENU_CONFIG, 0)]
    {
        let screen = recorded(fixture);
        let panel = panel(&screen).unwrap_or_else(|| panic!("{name} shows as a card"));
        assert_eq!(panel.menu.map_or(0, |m| m.entries.len()), choices, "{name}");
        assert!(panel.esc, "{name}");
        assert!(crate::view::chat::submission::screen::menu(&screen), "{name} blocks sends");
        assert!(sends_blocked(&screen), "{name}");
    }
}

#[test]
fn usage_bars_read_their_percentage_and_other_rows_are_left_alone() {
    let screen = recorded(PANEL_USAGE);
    let row = |i: usize| clean(&screen.row_text(i)).1;
    assert_eq!(usage_bar(&row(1)), Some((99, "99% used")));
    assert_eq!(usage_bar(&row(5)), Some((64, "64% used")), "the empty part is coloured cells, not characters");
    // A 0% bar is all coloured cells: only its words are text.
    assert_eq!(usage_bar("  0% used"), Some((0, "0% used")));
    for other in [row(0), row(2), row(14), "Plugins 2%".to_owned(), "120% used".to_owned()] {
        assert_eq!(usage_bar(&other), None, "{other}");
    }
}

#[test]
fn a_panel_is_read_from_its_own_top() {
    // Taller than the terminal, no rule on screen and no conversation: from the screen's top.
    let usage = panel(&recorded(PANEL_USAGE)).expect("the /usage panel");
    assert_eq!(usage.rows[..3], ["Current week (all models)", usage.rows[1].as_str(), "Resets Dec 2, 3am (UTC)"]);
    assert_eq!(usage_bar(&usage.rows[1]), Some((99, "99% used")));
    assert_eq!(usage.rows.last().map(String::as_str), Some("You're out of usage credits · /usage-credits to buy more"));

    // Under its rule: the settings tab row is dropped, the conversation above is not copied.
    let config = panel(&recorded(MENU_CONFIG)).expect("the /config panel");
    assert_eq!(config.rows.first().map(String::as_str), Some("⌕ Search settings…"));
    assert!(config.rows.iter().all(|row| !row.contains("/effort") && !row.starts_with("Settings")));
    assert!(config.rows.len() > 14, "the whole panel, not its last rows");
    let effort = panel(&recorded(MENU_EFFORT)).expect("the /effort panel");
    assert_eq!(effort.rows.first().map(String::as_str), Some("Effort"));

    // The selected tab names the panel.
    let mut screen = Screen::new(100, 12, 0);
    let rule = "─".repeat(100);
    let tabs = "  Settings  Status   Config   \x1b[7mUsage\x1b[27m   Stats";
    screen.feed(
        format!("❯ /usage\r\n{rule}\r\n{tabs}\r\n\r\n  Current week\r\n  0% used\r\n\r\n  Esc to close").as_bytes(),
    );
    let tabbed = panel(&screen).expect("a tabbed panel");
    assert_eq!(tabbed.title, "Usage");
    assert_eq!(tabbed.rows, ["Current week", "0% used"]);

    // No rule, and conversation above the footer: only the last rows.
    let mut screen = Screen::new(100, 30, 0);
    let mut text = String::from("❯ /usage\r\n  ⎿  Settings dialog dismissed\r\n");
    for i in 0..20 {
        text.push_str(&format!("  row {i}\r\n"));
    }
    text.push_str("  Esc to close");
    screen.feed(text.as_bytes());
    let rows = panel(&screen).expect("a panel").rows;
    assert_eq!((rows.len(), rows[0].as_str()), (14, "row 6"));
}

#[test]
fn a_status_line_that_reads_like_a_menu_hint_under_the_idle_input_is_not_a_menu() {
    let mut screen = recorded(IDLE_NBSP);
    // The idle screen's last row replaced by a custom status line with a footer-like hint.
    screen.feed("\x1b[39;1H\x1b[2K  ⏸ manual mode on · ctrl+g to edit in Notepad\x1b[36;3H".as_bytes());
    assert!(!crate::view::chat::submission::screen::menu(&screen));
    assert!(crate::view::chat::submission::screen::clean_prompt(&screen));
    assert!(panel(&screen).is_none() && !permission_like(&screen));
    // A real menu with the same hint still counts.
    assert!(crate::view::chat::submission::screen::menu(&recorded(PLAN)));
}

// Recorded on 2026-11-27 from Claude Code 2.1.332 with the recorder's `.ansi` snapshots (styles,
// size, wrap marks). `PLAIN_*`: started without `TERM`, as a tab of a program opened from the
// Start menu, where Claude Code draws `>`, `[ ]` and `√`; the rest with `TERM=xterm-256color`.

fn idle(screen: &Screen) -> bool {
    crate::view::chat::submission::screen::idle_slash_prompt(screen, true)
}

#[test]
fn the_idle_input_is_read_in_both_drawings_and_under_any_hint_row() {
    use crate::view::chat::submission::screen::{clean_prompt, menu, prompt, working};
    // After a turn, at the start (a faint placeholder), with the faint next-prompt suggestion,
    // with a background shell, after an interruption, over a status line that reads like a menu's
    // footer, and on a row the terminal still marks as wrapping after a declined prompt.
    for (name, fixture) in [
        ("plain idle", PLAIN_IDLE),
        ("plain start", PLAIN_START),
        ("plain suggestion", PLAIN_SUGGEST),
        ("plain background shell", PLAIN_BG_SHELL),
        ("plain interrupted", PLAIN_INTERRUPTED),
        ("status line", STATUS_FOOTER),
        ("declined, wrap mark left", DECLINED_WRAPPED),
        ("idle", PROMPT_IDLE),
        ("suggestion", PROMPT_SUGGEST),
        ("cleared", PROMPT_CLEARED),
    ] {
        let screen = recorded(fixture);
        assert!(clean_prompt(&screen), "{name}: {:?}", prompt(&screen));
        assert!(idle(&screen) && !working(&screen) && !menu(&screen), "{name}");
        assert!(!permission_like(&screen) && panel(&screen).is_none(), "{name}");
    }
    assert!(recorded(DECLINED_WRAPPED).line(recorded(DECLINED_WRAPPED).cursor().row).unwrap().wrapped());
    // Typed text is never an empty input.
    assert_eq!(prompt(&recorded(PLAIN_TYPED)).as_deref(), Some("Reply with just the word: pong"));
    assert_eq!(prompt(&recorded(PROMPT_PASTED)).as_deref(), Some("fix the typo"));
    assert_eq!(prompt(&recorded(PLAIN_PASTED_MARKER)).as_deref(), Some("[Pasted text #1 +13 lines]"));
    // A draft of several rows has the cursor under its first row: not a one-row prompt.
    assert_eq!(prompt(&recorded(PLAIN_PASTED_LINES)), None);
    assert_eq!(prompt(&recorded(PLAIN_LONG)), None);
}

#[test]
fn a_running_turn_is_known_by_its_hint_or_its_working_line_and_is_never_idle() {
    use crate::view::chat::submission::screen::{prompt, working};
    for (name, fixture, input) in [
        ("a tool runs", WORKING_TOOL, ""),
        ("60 columns", NARROW_WORKING, ""),
        // The hint row drops "esc to interrupt" while the input holds text.
        ("typed while busy", PLAIN_WORKING_TYPED, "hello while busy"),
        ("a message queued in the CLI", PLAIN_QUEUED, ""),
        ("compacting", PLAIN_COMPACTING, ""),
    ] {
        let screen = recorded(fixture);
        assert!(working(&screen) && !idle(&screen), "{name}");
        assert_eq!(prompt(&screen).as_deref(), Some(input), "{name}");
    }
}

#[test]
fn several_rows_of_input_read_back_whatever_the_mark() {
    use crate::view::chat::submission::screen::{shows_lines, shows_wrapped};
    let lines = recorded(PLAIN_PASTED_LINES);
    assert!(shows_lines(&lines, "line one\nline two\nline three"));
    assert!(!shows_lines(&lines, "line one\nline two"));
    assert!(shows_lines(&recorded(PLAIN_PASTED_MARKER), "any\ntext\nof several lines"));
    let long = "The quick brown fox jumps over the lazy dog while the river keeps moving past the old stone \
        bridge and the children watch the little boats drift slowly toward the sea where gulls circle above \
        the harbour walls and fishermen mend their nets in the warm afternoon light.";
    assert!(shows_wrapped(&recorded(PLAIN_LONG), long));
    assert!(!shows_wrapped(&recorded(PLAIN_LONG), &long[..40]));
    assert!(!shows_wrapped(&recorded(PLAIN_IDLE), "x"));
}

#[test]
fn a_question_reads_from_the_screen_alone_with_its_text_choices_and_descriptions() {
    // Claude Code 2.1.332 writes an `AskUserQuestion` call to the transcript only once it is
    // answered, so the screen is all there is while it waits. 170 x 22, drawn without Unicode.
    let screen = recorded(PLAIN_QUESTION);
    assert!(!permission_like(&screen) && sends_blocked(&screen));
    let asked = panel(&screen).expect("the question as a menu");
    assert!(asked.question && asked.esc);
    assert_eq!(
        asked.title,
        "Which agent should do the translation for episode 1035634? The last two episodes used astra at high effort."
    );
    let menu = asked.menu.expect("its choices");
    assert_eq!(
        menu.texts(),
        ["Astra, high (Recommended)", "Astra, medium", "Sol", "Claude", "Type something.", "Chat about this"]
    );
    assert_eq!(menu.highlight(), Some(0));
    assert_eq!(
        asked.notes[0],
        "astra at high effort, as for 1035632. Parallel jobs, then one fix job for the boxes that fail."
    );
    assert_eq!(asked.notes[3], "I translate it myself in this conversation.");
    assert!(asked.notes[4].is_empty() && asked.notes[5].is_empty());
    // The same question drawn with Unicode.
    let unicode = panel(&recorded(ASK_SINGLE)).expect("a question");
    assert!(unicode.question);
    assert_eq!(unicode.title, "Which colour do you prefer?");
    assert_eq!(unicode.notes[..3], ["A warm, energetic red.", "A calm, natural green.", "A cool, steady blue."]);
    // Any other menu keeps its own title and has no descriptions.
    let model = panel(&recorded(MENU_MODEL)).expect("the model picker");
    assert!(!model.question && model.notes.is_empty());
}

#[test]
fn a_form_page_drawn_without_unicode_reads_its_boxes_and_its_next_row() {
    let screen = recorded(PLAIN_FORM);
    let page = panel(&screen).expect("a form page as a menu");
    assert!(page.question);
    assert_eq!(page.title, "Which fruits do you like?");
    let menu = page.menu.expect("its rows");
    // As the arrow keys walk them: the unnumbered Next row comes after "Type something".
    assert_eq!(menu.texts(), ["[√] Apple", "[ ] Pear", "[ ] Plum", "[ ] Type something", "Next", "Chat about this"]);
    assert_eq!(checkbox(&menu.entries[0].text), Some((true, "Apple")));
    assert_eq!(checkbox(&menu.entries[1].text), Some((false, "Pear")));
    assert_eq!(menu.highlight(), Some(0));
    let unicode = panel(&recorded(FORM_MULTI)).expect("the same page with Unicode").menu.unwrap();
    assert_eq!(unicode.texts()[3..], ["[ ] Type something", "Next", "Chat about this"]);
}

#[test]
fn prompts_drawn_without_unicode_are_still_permission_prompts_and_plan_menus() {
    let bash = recorded(PLAIN_PERM_BASH);
    assert!(crate::attention::screen_permission_prompt(&bash) && permission_like(&bash) && panel(&bash).is_none());
    let prompt = permission(&bash).expect("the prompt");
    assert_eq!(prompt.question, "Do you want to proceed?");
    assert_eq!(prompt.menu.texts().len(), 4);
    assert_eq!(prompt.menu.highlight(), Some(0));
    let plan_screen = recorded(PLAIN_PLAN);
    let menu = plan(&plan_screen).expect("the plan approval");
    assert_eq!(menu.texts(), ["Yes, and use auto mode", "Yes, manually approve edits", "Tell Claude what to change"]);
    assert!(permission_like(&plan_screen) && panel(&plan_screen).is_none());
}

#[test]
fn a_footer_with_rows_under_it_still_marks_a_menu_unless_an_input_row_is_under_it() {
    use crate::view::chat::submission::screen::clean_prompt;
    // A rule and a notice under the question's footer (a redraw can leave rows there).
    let rule = "─".repeat(100);
    let mut screen = recorded(ASK_SINGLE);
    screen.feed(format!("\x1b[25;1H{rule}\x1b[26;1H  1 MCP server failed · /mcp").as_bytes());
    assert!(panel(&screen).is_some_and(|p| p.question) && !clean_prompt(&screen));
    // The same words above the input box are the conversation.
    let mut old = Screen::new(100, 40, 0);
    old.feed(
        format!("  1. Yes\r\n  2. No\r\n\r\nEnter to select · Esc to cancel\r\n\r\n● Done.\r\n{rule}\r\n> \r\n{rule}\x1b[8;3H")
            .as_bytes(),
    );
    assert!(panel(&old).is_none() && clean_prompt(&old));
}

#[test]
fn a_subagents_prompt_reads_whole_from_its_top_rule_and_says_whose_call_it_is() {
    let screen = recorded(PERM_AGENT_BASH);
    assert!(crate::attention::screen_permission_prompt(&screen) && panel(&screen).is_none());
    let prompt = permission(&screen).expect("a permission prompt");
    let rows = permission_rows(&screen, &prompt).expect("its top rule is on the screen");
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0], "Bash command · from the general-purpose agent");
    assert_eq!(rows[1], "Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below");
    assert_eq!((rows[2].as_str(), rows[4].as_str()), ("Create directory sub_probe_five", "mkdir sub_probe_five"));
    assert!(is_rule(&rows[3]) && is_rule(&rows[5]) && !is_rule(&rows[4]));
    assert_eq!(prompt.question, "Do you want to proceed?");
    let texts = prompt.menu.texts();
    assert_eq!(texts.len(), 4);
    assert_eq!(texts[1], "Yes, and always allow access to C:\\Users\\user\\work\\scratch from this project");
    assert_eq!((texts[3].as_str(), prompt.menu.highlight()), ("No", Some(0)));
    // Its footer names the background agents; on a choice that takes no comment it drops "Tab to
    // amend". The rows are the same on every highlight.
    assert!(prompt.menu.footer_text().contains("twice to stop background agents"));
    for (fixture, highlight) in [(PERM_AGENT_BASH_DOWN, 1), (PERM_AGENT_BASH_NO, 3)] {
        let screen = recorded(fixture);
        let moved = permission(&screen).expect("the same prompt");
        assert_eq!(moved.menu.highlight(), Some(highlight));
        assert_eq!(permission_rows(&screen, &moved).as_ref(), Some(&rows));
    }
    assert!(!permission(&recorded(PERM_AGENT_BASH_DOWN)).unwrap().menu.footer_text().contains(AMEND));
    assert!(permission(&recorded(PERM_AGENT_GONE)).is_none());

    // A file: the path, then its content between the dashed rules, line numbers and all.
    let screen = recorded(PERM_AGENT_WRITE);
    let prompt = permission(&screen).expect("a permission prompt");
    let rows = permission_rows(&screen, &prompt).expect("whole");
    assert_eq!(rows[..2], ["Create file · from the general-purpose agent", "sub_note.txt"]);
    assert_eq!(rows[3], " 1 hello", "the content keeps its own indent");
    assert_eq!(prompt.question, "Do you want to create sub_note.txt?");
}

#[test]
fn two_calls_of_one_message_ask_one_after_the_other_and_each_prompt_names_its_own() {
    let rows = |fixture: &str| {
        let screen = recorded(fixture);
        let prompt = permission(&screen).expect("a permission prompt");
        permission_rows(&screen, &prompt).expect("whole")
    };
    let (first, second) = (rows(PERM_TWO_FIRST), rows(PERM_TWO_SECOND));
    assert_eq!((first[0].as_str(), first[4].as_str()), ("Bash command", "mkdir par_a"));
    assert_eq!((second[0].as_str(), second[4].as_str()), ("Bash command", "mkdir par_b"));
    assert_ne!(first, second);
}

#[test]
fn a_prompt_is_known_by_its_place_at_the_bottom_not_by_the_cursor() {
    // After the first of two prompts was approved, the cursor stays on the first call's output.
    let screen = recorded(PERM_TWO_AFTER);
    let prompt = permission(&screen).expect("the second call's prompt");
    assert!(prompt.menu.footer > screen.cursor().row + 8, "the cursor is far above the footer");
    assert!(crate::attention::screen_permission_prompt(&screen) && panel(&screen).is_none());
    let rows = permission_rows(&screen, &prompt).expect("whole");
    assert_eq!((rows[0].as_str(), rows[4].as_str()), ("Bash command", "mkdir par_b"));
    assert_eq!(
        prompt.menu.texts()[1],
        "Yes, and always allow access to C:\\Users\\user\\work\\scratch from this project"
    );
    assert_eq!(prompt.menu.highlight(), Some(0));
    // Wherever the cursor is parked, the same prompt reads the same.
    for (row, col) in [(1, 1), (40, 1), (25, 30)] {
        let mut moved = recorded(PERM_TWO_AFTER);
        moved.feed(format!("\x1b[{row};{col}H").as_bytes());
        assert!(crate::attention::screen_permission_prompt(&moved), "cursor at {row}");
    }
    // The same rows left in the conversation, with the live input box under them, are no prompt,
    // with the cursor on the input or still up on the old footer.
    let rule = "─".repeat(100);
    for cursor in ["\x1b[30;3H", "\x1b[25;29H"] {
        let mut old = recorded(PERM_TWO_AFTER);
        old.feed(format!("\x1b[28;1H{rule}\x1b[29;1H❯ \x1b[30;1H{rule}\x1b[31;1H  ? for shortcuts{cursor}").as_bytes());
        assert!(!crate::attention::screen_permission_prompt(&old) && permission(&old).is_none());
        assert!(!permission_like(&old));
    }
}

#[test]
fn a_prompt_without_its_top_rule_on_the_screen_is_not_read_as_whole() {
    // Scrolled so that the rule and the prompt's first rows are above the screen.
    let mut screen = Screen::new(110, 8, 0);
    let rows = [
        "╌".repeat(110),
        " mkdir sub_probe_five".to_owned(),
        "╌".repeat(110),
        " Do you want to proceed?".to_owned(),
        " ❯ 1. Yes".to_owned(),
        "   2. No".to_owned(),
        String::new(),
        " Esc to cancel · Tab to amend".to_owned(),
    ];
    screen.feed(rows.join("\r\n").as_bytes());
    let prompt = permission(&screen).expect("still a permission prompt");
    assert_eq!(permission_rows(&screen, &prompt), None);
    // A rule row inside the prompt starts after the margin, so it is never taken for the top.
    let mut screen = Screen::new(110, 12, 0);
    let mut rows = rows.to_vec();
    rows.splice(0..0, ["─".repeat(110), " Bash command".to_owned(), format!(" {}", "─".repeat(109))]);
    screen.feed(rows.join("\r\n").as_bytes());
    let prompt = permission(&screen).expect("a permission prompt");
    let whole = permission_rows(&screen, &prompt).expect("its top rule is on the screen");
    assert_eq!(whole[0], "Bash command");
    assert_eq!(whole.len(), 5, "the rule-like row is one of the prompt's rows");
    // The main session's own prompt reads the same way: no source after the tool.
    let main = recorded(PERM_BASH);
    let rows = permission_rows(&main, &permission(&main).unwrap()).expect("whole");
    assert_eq!((rows[0].as_str(), rows[4].as_str()), ("Bash command", "mkdir probe_dir_one"));
}

#[test]
fn the_list_of_blocked_calls_reads_its_calls_marks_command_and_what_esc_will_do() {
    let list = |fixture: &str| blocked(&recorded(fixture)).expect("the list of blocked calls");
    let one = list(BLOCKED_ONE);
    assert_eq!(one.note, "Commands auto mode recently blocked. Approving adds no rule.");
    assert_eq!(one.menu.texts(), ["✘ Create directory probe_blocked_two Auto mode: No reason provided"]);
    assert_eq!((one.marks.as_slice(), one.menu.highlight()), (&[Mark::None][..], Some(0)));
    assert_eq!((one.pending.as_deref(), one.esc.as_str()), (None, "close"));
    assert_eq!(one.detail, ["mkdir probe_blocked_two"]);

    // Enter marked it: the row says so, a row under the list and the footer say what Esc will do.
    let approved = list(BLOCKED_ONE_APPROVED);
    assert_eq!(approved.menu.texts(), ["✔ Create directory probe_blocked_two Auto mode: No reason provided"]);
    assert_eq!(approved.marks, [Mark::Approve]);
    assert_eq!(approved.pending.as_deref(), Some("Esc will approve 1 blocked call"));
    assert_eq!(approved.esc, "approve 1 blocked");
    let retry = list(BLOCKED_ONE_RETRY);
    assert_eq!(retry.marks, [Mark::Retry]);
    assert_eq!(retry.menu.texts(), ["✔ Create directory probe_blocked_four (retry) Auto mode: No reason provided"]);
    assert_eq!(
        (retry.pending.as_deref(), retry.esc.as_str()),
        (Some("Esc will retry 1 blocked call"), "retry 1 blocked")
    );

    // Two calls, newest first; the command under the list is the highlighted call's.
    let two = list(BLOCKED_TWO);
    assert_eq!(two.menu.texts().len(), 2);
    assert_eq!((two.menu.highlight(), two.detail.as_slice()), (Some(0), &["mkdir probe_blocked_b".to_owned()][..]));
    let down = list(BLOCKED_TWO_DOWN);
    assert_eq!((down.menu.highlight(), down.detail.as_slice()), (Some(1), &["mkdir probe_blocked_a".to_owned()][..]));
    assert_eq!(down.menu.texts(), two.menu.texts(), "moving the highlight changes no row");
    let second = list(BLOCKED_TWO_SECOND);
    assert_eq!((second.marks.as_slice(), second.menu.highlight()), (&[Mark::None, Mark::Approve][..], Some(1)));
    assert_eq!(list(BLOCKED_TWO_RETRY).marks, [Mark::Retry, Mark::None]);
    // `r` on a call marked to retry makes it an approval.
    assert_eq!(list(BLOCKED_TWO_APPROVE).marks, [Mark::Approve, Mark::None]);
}

#[test]
fn the_list_of_blocked_calls_is_never_a_menu_of_plain_choices_and_always_blocks_sends() {
    for (name, fixture, closes) in [
        ("one", BLOCKED_ONE, true),
        ("approved", BLOCKED_ONE_APPROVED, false),
        ("retry", BLOCKED_ONE_RETRY, false),
        ("two", BLOCKED_TWO, true),
        ("second approved", BLOCKED_TWO_SECOND, false),
    ] {
        let screen = recorded(fixture);
        // Without its own card it would show as rows only: Enter on a row approves that call.
        let panel = panel(&screen).unwrap_or_else(|| panic!("{name}: a dialog"));
        assert!(panel.menu.is_none() && !panel.rows.is_empty(), "{name}");
        assert_eq!(panel.esc, closes, "{name}: Esc only while it just closes");
        assert!(crate::view::chat::submission::screen::menu(&screen) && sends_blocked(&screen), "{name}");
        assert!(!permission_like(&screen), "{name}");
    }
    // Another tab of the dialog, the screens before and after it, and other menus are not the list.
    for fixture in [PERM_TAB_ALLOW, BLOCKED_IDLE, BLOCKED_TYPED, BLOCKED_ONE_CLOSED, BLOCKED_RETRY_STARTED, MENU_MODEL]
    {
        assert!(blocked(&recorded(fixture)).is_none());
    }
    let allow = panel(&recorded(PERM_TAB_ALLOW)).expect("the Allow tab as a panel");
    assert!(allow.menu.is_none() && allow.esc);
    // After a block the input is idle under the CLI's notice; an approval leaves it idle, a retry
    // starts the turn at once.
    use crate::view::chat::submission::screen::{clean_prompt, working};
    assert!(clean_prompt(&recorded(BLOCKED_IDLE)) && idle(&recorded(BLOCKED_IDLE)));
    assert!(clean_prompt(&recorded(BLOCKED_ONE_CLOSED)) && !working(&recorded(BLOCKED_ONE_CLOSED)));
    assert!(working(&recorded(BLOCKED_RETRY_STARTED)));
    // A list with a row it does not know is not read: a mark it has not seen, a tab row that
    // selects another tab.
    let odd = BLOCKED_TWO.replace("\u{2718}", "?");
    assert!(blocked(&recorded(&odd)).is_none(), "an unknown mark");
    let moved = BLOCKED_TWO.replace("\x1b[0;1;7m Recently blocked \x1b[0m", " Recently blocked ");
    assert_ne!(moved, BLOCKED_TWO);
    assert!(blocked(&recorded(&moved)).is_none(), "the tab is not the selected one");
}

/// Prints what the gates make of every `.ansi` recording in the folder `CS_PROBE_DIR` names (the
/// recorder's output): `cargo test probe_recordings -- --ignored --nocapture`.
#[test]
#[ignore = "a tool: reads recordings from CS_PROBE_DIR"]
fn probe_recordings() {
    use crate::view::chat::submission::screen;
    let dir = std::env::var("CS_PROBE_DIR").expect("CS_PROBE_DIR");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ansi"))
        .collect();
    names.sort();
    for path in names {
        let s = recorded(&std::fs::read_to_string(&path).unwrap());
        println!(
            "{:24} prompt={:?} menu={} working={} permission={} like={} panel={} mode={:?}",
            path.file_stem().unwrap().to_string_lossy(),
            screen::prompt(&s),
            screen::menu(&s),
            screen::working(&s),
            crate::attention::screen_permission_prompt(&s),
            permission_like(&s),
            panel(&s).is_some(),
            mode(&s),
        );
    }
}

#[test]
fn a_typed_answer_reads_exactly_on_its_row_with_or_without_unicode() {
    let shows = crate::view::chat::question::entry_shows;
    for (fixture, text) in [
        (ASK_OTHER_TYPED, "my own answer"),
        (PLAIN_ASK_OTHER_TYPED, "my own answer"),
        (FORM_OTHER_TYPED, "kiwi"),
        (PLAIN_FORM_OTHER_TYPED, "kiwi"),
    ] {
        let menu = walk_menu(&recorded(fixture)).expect("the question");
        assert_eq!(menu.highlight(), Some(3));
        assert!(shows(&menu, 3, text), "{text}");
        assert!(!shows(&menu, 3, &text[..text.len() - 1]) && !shows(&menu, 3, &format!("{text} more")));
        // A row that was not typed on is never the answer.
        assert!(!shows(&menu, 0, text));
    }
    // The untouched rows: a pristine "Type something" row is not text, and an empty box is not ticked.
    let pristine = walk_menu(&recorded(ASK_OTHER)).unwrap();
    assert!(!shows(&pristine, 3, "my own answer"));
    let form = walk_menu(&recorded(FORM_OTHER)).unwrap();
    assert!(!shows(&form, 3, "Type something") && !shows(&form, 3, "kiwi"));
}

/// What any reader of menus, questions and dialogs makes of `screen`: the names of those that
/// found one.
pub(super) fn dialogs_read(screen: &Screen) -> Vec<&'static str> {
    let found = [
        ("menu", menu(screen).is_some()),
        ("walk_menu", walk_menu(screen).is_some()),
        ("panel", panel(screen).is_some()),
        ("permission", permission(screen).is_some()),
        ("permission_like", permission_like(screen)),
        ("review", review(screen).is_some()),
        ("plan", plan(screen).is_some()),
        ("blocked", blocked(screen).is_some()),
        ("rewind_list", rewind_list(screen).is_some()),
        ("rewind_actions", rewind_actions(screen).is_some()),
        ("live_footer", live_footer(screen).is_some()),
        ("attention", crate::attention::screen_permission_prompt(screen)),
        ("menu_open", crate::view::chat::submission::screen::menu(screen)),
    ];
    found.into_iter().filter(|(_, read)| *read).map(|(name, _)| name).collect()
}

/// A screen of `rows` with the CLI's input box (full-width rules) holding `input`, `under` it.
fn with_input(above: &str, input: &[&str], under: &str) -> Screen {
    let rule = "─".repeat(100);
    let cursor = above.lines().count() + input.len();
    let mut rows: Vec<String> = above.lines().map(str::to_owned).collect();
    rows.push(rule.clone());
    for (i, line) in input.iter().enumerate() {
        rows.push(format!("{}{line}", if i == 0 { "❯ " } else { "  " }));
    }
    rows.push(rule);
    rows.extend(under.lines().map(str::to_owned));
    recorded(&format!("cursor {cursor} 10\n{}", rows.join("\n")))
}

/// Status lines Claude Code may draw under its input, some of which read like a menu's footer.
const UNDER_INPUT: [&str; 4] = [
    "  ⏵⏵ auto mode on (shift+tab to cycle)",
    "  ⏵⏵ auto mode on · esc to interrupt",
    "  Enter to select · ↑/↓ to navigate · Esc to cancel",
    "  ↑/↓ to select · enter to view · x to stop\n  ● main\n  ◯ a\n  ◯ b\n  ◯ c\n  ◯ d\n  ◯ e\n  ◯ f\n  ◯ g\n  ◯ h",
];

#[test]
fn a_numbered_draft_in_the_input_is_no_menu() {
    // The owner sent a numbered list of several lines: Chat showed a card made of its lines.
    let drafts: [&[&str]; 3] = [
        &["1. say pong", "2. nothing else", "3. done"],
        &["Do you want to proceed?", "1. Yes", "2. No"],
        &["1. Submit answers", "2. Cancel"],
    ];
    for draft in drafts {
        for under in UNDER_INPUT {
            let screen = with_input("❯ earlier\n  ⎿  done\n", draft, under);
            assert_eq!(dialogs_read(&screen), Vec::<&str>::new(), "{draft:?} over {under:?}");
            assert!(crate::view::chat::submission::screen::shows_lines(&screen, &draft.join("\n")), "{draft:?}");
        }
    }
    // With "Esc to cancel" as the last row under it.
    let screen = with_input("", &["Do you want to proceed?", "1. Yes", "2. No"], "\n Esc to cancel");
    assert_eq!(dialogs_read(&screen), Vec::<&str>::new());
}

#[test]
fn the_users_numbered_message_as_sent_is_no_menu() {
    // After Enter: the message in the conversation, the turn starting, the empty input under it.
    let sent = "❯ 1. say pong\n  2. nothing else\n  3. done\n\n✻ Working… (1s)\n";
    for under in UNDER_INPUT {
        let screen = with_input(sent, &[""], under);
        assert_eq!(dialogs_read(&screen), Vec::<&str>::new(), "under {under:?}");
    }
    // A numbered list in Claude's answer, over the idle input.
    let answer = "❯ list the steps\n\n● Here they are:\n  1. Build\n  2. Test\n  3. Ship\n";
    for under in UNDER_INPUT {
        let screen = with_input(answer, &[""], under);
        assert_eq!(dialogs_read(&screen), Vec::<&str>::new(), "under {under:?}");
    }
}

#[test]
fn a_dialog_under_the_users_numbered_message_reads_only_its_own_rows() {
    let rule = "─".repeat(100);
    let sent = "❯ 1. say pong\n  2. nothing else\n\n● Bash(mkdir x)\n  ⎿  Waiting…\n";
    // A dialog with no numbered rows of its own: the message's rows are not its choices.
    let panel_screen = recorded(&format!(
        "cursor 0 0\n{sent}\n{rule}\n Some dialog\n\n Its text, with no choices\n\n Enter to confirm · Esc to cancel"
    ));
    assert!(menu(&panel_screen).is_none());
    assert!(panel(&panel_screen).is_some_and(|p| p.menu.is_none()));
    // A permission prompt: its own choices.
    let prompt = recorded(&format!(
        "cursor 0 0\n{sent}\n{rule}\n Bash command\n mkdir x\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel · Tab to amend"
    ));
    assert_eq!(permission(&prompt).expect("the prompt").menu.texts(), ["Yes", "No"]);
}

#[test]
fn the_real_menus_and_dialogs_are_still_read() {
    let read = |fixture: &str| dialogs_read(&recorded(fixture));
    for (name, fixture, reader) in [
        ("perm_bash", PERM_BASH, "permission"),
        ("plain_perm_bash", PLAIN_PERM_BASH, "attention"),
        ("perm_two_after", PERM_TWO_AFTER, "permission"),
        ("perm_agent_bash", PERM_AGENT_BASH, "permission"),
        ("menu_model", MENU_MODEL, "menu"),
        ("menu_effort", MENU_EFFORT, "panel"),
        ("menu_config", MENU_CONFIG, "panel"),
        ("ask_single", ASK_SINGLE, "panel"),
        ("ask_single_other", ASK_SINGLE_OTHER, "panel"),
        ("plain_question", PLAIN_QUESTION, "panel"),
        ("form_multi", FORM_MULTI, "panel"),
        ("ask_review", ASK_REVIEW, "review"),
        ("form_review", FORM_REVIEW, "review"),
        ("plain_trust", PLAIN_TRUST, "menu"),
        ("plan", PLAN, "plan"),
        ("blocked_one", BLOCKED_ONE, "blocked"),
        ("rewind_open", REWIND_OPEN, "rewind_list"),
        ("rewind_actions", REWIND_ACTIONS, "rewind_actions"),
    ] {
        assert!(read(fixture).contains(&reader), "{name}: {:?}", read(fixture));
    }
    assert_eq!(menu(&recorded(MENU_MODEL)).unwrap().entries.len(), 5);
    assert_eq!(menu(&recorded(PLAIN_TRUST)).unwrap().texts(), ["No, exit", "Yes, I trust this folder"]);
    // The idle input and the agent list under it are no dialog.
    for fixture in [PLAIN_IDLE, PROMPT_IDLE, AGENTS_OPEN, AGENTS_IDLE, PLAIN_TYPED, PLAIN_PASTED_LINES] {
        assert_eq!(read(fixture), Vec::<&str>::new());
    }
    assert!(agent_list(&recorded(AGENTS_OPEN)).is_some());
}

#[test]
fn a_question_with_chat_about_this_highlighted_over_a_leftover_rule_is_no_input_box() {
    // `❯ 5. Chat about this` sits right under the question's inner rule, and Claude Code may leave
    // a plain rule under the dialog (seen at the review step): taken for the input box, no card.
    let rule = "─".repeat(100);
    for under in [format!("\n{rule}"), format!("\n\n\n{rule}"), rule.clone()] {
        let screen = recorded(&format!("{ASK_SINGLE_OTHER}{under}"));
        assert_eq!(input_box(&screen), None, "{under:?}");
        let panel = panel(&screen).expect("the question");
        assert!(panel.question, "{under:?}");
        assert_eq!(panel.menu.expect("its choices").highlight(), Some(4));
    }
}

#[test]
fn a_quote_under_a_rule_in_the_conversation_is_no_input_box() {
    // A `>` row of the conversation right under a rendered rule, the permission prompt's own top
    // rule further down: taken for the input box and its closing rule, no prompt was read and no
    // notification came.
    let rule = "─".repeat(100);
    let prompt = format!(
        "{rule}\n Bash command\n mkdir x\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel · Tab to amend"
    );
    for quote in ["  > a quoted line", "> a quoted line"] {
        let screen = recorded(&format!(
            "cursor 0 0\n● Here is the note:\n{rule}\n{quote}\n  and its second line\n\n● Bash(mkdir x)\n  ⎿  Waiting…\n\n{prompt}"
        ));
        assert_eq!(input_box(&screen), None, "{quote:?}");
        assert!(crate::attention::screen_permission_prompt(&screen), "{quote:?}");
        assert_eq!(permission(&screen).expect("the prompt").menu.texts(), ["Yes", "No"]);
    }
}

#[test]
fn choices_start_under_the_rule_over_the_menus_first_choice() {
    // A full-width rule in the conversation above the user's numbered message: the message is
    // still above the dialog's own rule, and a dialog with no numbered rows has no choices.
    let rule = "─".repeat(100);
    let screen = recorded(&format!(
        "cursor 0 0\n● Done.\n{rule}\n❯ 1. say pong\n  2. nothing else\n\n● On it.\n\n{rule}\n Some dialog\n\n Its text\n\n Enter to confirm · Esc to cancel"
    ));
    assert_eq!(input_box(&screen), None);
    assert!(menu(&screen).is_none());
    assert!(panel(&screen).is_some_and(|p| p.menu.is_none()));
    // A question's choices on both sides of its inner rule are one menu.
    assert_eq!(menu(&recorded(ASK_SINGLE)).expect("the question").entries.len(), 5);
    assert_eq!(menu(&recorded(ASK_SINGLE_OTHER)).expect("the question").entries.len(), 5);
}
