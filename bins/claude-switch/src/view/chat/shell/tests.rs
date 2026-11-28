//! The composer's shell mode.
use std::time::Duration;

use super::*;
use crate::state::AppState;
use crate::view::chat::question::screens::recorded::{PLAIN_IDLE, WORKING_TOOL, recorded};
use crate::view::chat::submission::ready_tests::Live;

const SECOND: Duration = Duration::from_secs(1);

/// Claude Code's input in shell mode holding `command`, as Chat expects to read it: `!` where
/// the prompt's `❯` stands. Not recorded yet (`recorder compat` script `shell-paste`).
fn shell_screen(command: &str) -> String {
    let rule = "─".repeat(100);
    format!(
        "cursor 2 {}\n@size 100 6\n\n{rule}\n! {command}\n{rule}\n  ! for shell mode\n",
        2 + command.chars().count()
    )
}

#[test]
fn the_input_in_shell_mode_reads_as_its_command() {
    assert_eq!(shell_input(&recorded(&shell_screen("git status --short"))).as_deref(), Some("git status --short"));
    assert!(shows(&recorded(&shell_screen("ls")), "!ls"));
    assert!(!shows(&recorded(&shell_screen("ls -a")), "!ls"));
    assert_eq!(shell_input(&recorded(PLAIN_IDLE)), None, "the prompt's input is not shell mode");
}

#[test]
fn bang_in_the_empty_box_turns_shell_mode_on_and_backspace_leaves_it() {
    let mut chat = Chat::default();
    assert!(chat.shell_key("!") && chat.shell_mode && chat.editor.text.is_empty(), "the `!` is the mode, not text");
    assert!(!chat.shell_key("!"), "a second `!` is text");
    let mut chat = Chat::default();
    chat.editor.set("say".into());
    assert!(!chat.shell_key("!") && !chat.shell_mode, "only in an empty box");
    let mut chat = Chat { shell_mode: true, ..Chat::default() };
    assert!(chat.leave_shell() && !chat.shell_mode);
}

#[test]
fn a_shell_command_goes_as_bang_and_enter_waits_for_the_input_to_show_it() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.chat().shell_mode = true;
    f.send("git status --short");
    assert_eq!(f.writes(), [b"\x1b[200~!git status --short\x1b[201~".to_vec()], "the paste, no Enter yet");
    // The input does not show it in shell mode: no Enter.
    f.pass(SECOND / 10);
    assert!(f.writes().is_empty());
    f.show(&shell_screen("git status --short"));
    f.pass(SECOND / 10);
    assert_eq!(f.writes(), [b"\r".to_vec()]);
    let chat = f.chat();
    assert!(!chat.shell_mode && chat.editor.text.is_empty(), "back to a message once it went");
}

#[test]
fn a_shell_command_that_does_not_show_is_never_entered() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.chat().shell_mode = true;
    f.send("ls");
    assert_eq!(f.writes().len(), 1);
    for _ in 0..7 {
        f.pass(SECOND);
    }
    assert!(f.writes().is_empty(), "no Enter without the screen showing the command");
    assert!(f.chat().error.is_some(), "it says the text may be on the input");
}

#[test]
fn a_shell_command_is_one_line_and_never_goes_into_a_running_turn() {
    let mut f = Live::new();
    f.show(PLAIN_IDLE);
    f.pass(SECOND);
    f.chat().shell_mode = true;
    f.send("ls\npwd");
    assert!(f.writes().is_empty());
    assert_eq!(f.chat().error, Some(ONE_LINE));
    let mut f = Live::new();
    f.turn();
    f.show(WORKING_TOOL);
    f.pass(SECOND);
    f.chat().shell_mode = true;
    f.send("ls");
    assert!(f.writes().is_empty(), "held, not typed into the turn");
    assert!(f.chat().queue.is_some());
    let now = f.now;
    AppState::chat_send(f.tab(), now, true);
    assert!(f.writes().is_empty(), "Send now does not push it into the turn either");
}
