//! The composer's shell mode: a message that is a command for Claude Code's own shell mode (`!`).
//!
//! As in Claude Code, `!` typed into an empty box turns it on and Backspace in an empty box (or a
//! click on its `!` mark) turns it off. Claude Code 2.1.332 switches its input to shell mode when
//! the text that reaches an empty input starts with `!` (its source: the mode follows a first
//! character `!` whenever the input was empty before, so a paste of `!command` does it too). That
//! is not recorded yet: `recorder compat` has the script `shell-paste` for it. Until a recording
//! confirms how the screen shows it, Chat presses Enter only once the input row under the cursor
//! reads `! command` exactly; otherwise nothing more goes and the text is left on the input with
//! the usual notice. A shell command is one line, has no attachments, and goes only into an idle
//! input (never into a running turn), where the transcript records it as `<bash-input>` with its
//! output, which Chat shows as a Shell row.
use mdshared_term::Screen;

use super::Chat;

pub(super) const ONE_LINE: &str =
    "A shell command is one line. Remove the line breaks, or leave shell mode (Backspace).";
pub(super) const HINT: &str = "Shell command, run by Claude Code's shell mode…";

/// The text on Claude Code's input in shell mode, as the row under the cursor shows it: `!`
/// where the prompt's `❯` stands, then the command. `None` when that row is no such input.
pub(in crate::view::chat) fn shell_input(screen: &Screen) -> Option<String> {
    let row = screen.row_text(screen.cursor().row);
    let row = row.trim_start_matches([' ', '│']).trim_end_matches([' ', '│']);
    let rest = row.strip_prefix('!')?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    Some(rest.trim().to_owned())
}

/// The input shows the shell command `expected` (`!command`) Chat pasted, before its Enter.
pub(super) fn shows(screen: &Screen, expected: &str) -> bool {
    let command = expected.strip_prefix('!').unwrap_or(expected).trim();
    !command.is_empty() && shell_input(screen).as_deref() == Some(command)
}

impl Chat {
    /// `!` typed into the empty box: shell mode, as Claude Code's own `!`. True when it took it.
    pub(super) fn shell_key(&mut self, text: &str) -> bool {
        let empty = self.editor.text.is_empty() && self.chips.is_empty();
        if text != "!" || self.shell_mode || !empty || self.locked() {
            return false;
        }
        self.shell_mode = true;
        true
    }

    /// Backspace in the empty box, or a click on the `!` mark: back to a message.
    pub(super) fn leave_shell(&mut self) -> bool {
        if !self.shell_mode || self.locked() {
            return false;
        }
        self.shell_mode = false;
        if self.error == Some(ONE_LINE) {
            self.error = None;
        }
        true
    }
}

#[cfg(test)]
mod tests;
