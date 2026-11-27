//! Screens recorded from Claude Code 2.1.332 (Haiku, 100 x 40 cells, classic renderer) on
//! 2026-11-27, with personal paths and names replaced. Each fixture is the screen's rows and where
//! the CLI left the cursor. When a Claude Code update changes one of these screens, record it again
//! the same way and keep the old one if both versions are in use.
use super::*;

/// The fixture's rows on a 100 x 40 screen, with the cursor where the CLI left it.
pub(in crate::view::chat) fn recorded(fixture: &str) -> Screen {
    let mut lines = fixture.lines();
    let cursor = lines.next().and_then(|l| l.strip_prefix("cursor ")).expect("cursor line");
    let (row, col) = cursor.split_once(' ').expect("row and column");
    let (row, col): (usize, usize) = (row.parse().unwrap(), col.parse().unwrap());
    let mut screen = Screen::new(100, 40, 0);
    for (i, line) in lines.enumerate() {
        screen.feed(format!("\x1b[{};1H{line}", i + 1).as_bytes());
    }
    screen.feed(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    screen
}

pub(in crate::view::chat) const ASK_SINGLE: &str = include_str!("../fixtures/ask_single.txt");
pub(in crate::view::chat) const ASK_SINGLE_OTHER: &str = include_str!("../fixtures/ask_single_other.txt");
pub(in crate::view::chat) const ASK_MULTI: &str = include_str!("../fixtures/ask_multi.txt");
pub(in crate::view::chat) const ASK_TWO: &str = include_str!("../fixtures/ask_two.txt");
pub(in crate::view::chat) const ASK_REVIEW: &str = include_str!("../fixtures/ask_review.txt");
pub(in crate::view::chat) const PERM_BASH: &str = include_str!("../fixtures/perm_bash.txt");
pub(in crate::view::chat) const PERM_WRITE: &str = include_str!("../fixtures/perm_write.txt");
pub(in crate::view::chat) const PERM_FETCH_CLIPPED: &str = include_str!("../fixtures/perm_fetch_clipped.txt");
pub(in crate::view::chat) const IDLE_NBSP: &str = include_str!("../fixtures/idle_nbsp.txt");
pub(in crate::view::chat) const PLAN: &str = include_str!("../fixtures/plan.txt");
pub(in crate::view::chat) const MENU_MODEL: &str = include_str!("../fixtures/menu_model.txt");
pub(in crate::view::chat) const MENU_EFFORT: &str = include_str!("../fixtures/menu_effort.txt");
pub(in crate::view::chat) const PANEL_USAGE: &str = include_str!("../fixtures/panel_usage.txt");
pub(in crate::view::chat) const MENU_CONFIG: &str = include_str!("../fixtures/menu_config.txt");

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
    // The review step has no footer; the cards do not read it yet.
    assert!(menu(&recorded(ASK_REVIEW)).is_none());
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
