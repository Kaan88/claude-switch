//! The only terminal input for Shells is one Down key at a verified management footer.
use claude_switch_core::transcript::{ChatPhase, ItemId, ShellStatus};
use mdshared_term::{Screen, input};

use super::chat::submission::screen::clean_prompt;
use crate::dialogs::{Dialog, Pending};
use crate::state::AppState;

pub(super) fn running_label(count: usize) -> String {
    format!("{count} {} running", if count == 1 { "shell" } else { "shells" })
}

fn footer(line: &str) -> bool {
    let Some((count, action)) = line.trim().split_once('·') else { return false };
    let Some((n, words)) = count.trim().split_once(' ') else { return false };
    n.parse::<usize>().is_ok_and(|n| n > 0)
        && matches!(words.trim(), "shell still running" | "shells still running" | "shell running" | "shells running")
        && action.trim() == "↓ to manage"
}

fn manager_key(screen: &Screen) -> Option<Vec<u8>> {
    let modes = screen.modes();
    if !modes.bracketed_paste || !clean_prompt(screen) {
        return None;
    }
    let cursor = screen.cursor();
    ((cursor.row + 1)..(cursor.row + 6).min(screen.rows()))
        .any(|i| footer(&screen.row_text(i)))
        .then(|| input::encode_key(input::Key::Down, input::Mods::default(), &modes))
        .filter(|key| !key.is_empty())
}

impl AppState {
    pub(super) fn ask_shell_manager(&mut self, tool: ItemId) {
        let Some(tab) = self.tabs.shown() else { return };
        let Some(pane) = &tab.pane else { return };
        let Some(shell) = pane.model.shells.iter().find(|s| s.tool == tool && s.status == ShellStatus::Running) else {
            return;
        };
        self.dialog = Some(Dialog::Confirm {
            title: format!("Stop shell {}?", shell.id),
            text:
                "Open Claude Code’s shell manager. Choose the shell and stop it there; Switch will not stop it for you."
                    .into(),
            ok: "Open shell manager".into(),
            then: Pending::ManageShell { tab: tab.id, generation: pane.model.generation, tool },
        });
    }

    pub(crate) fn confirm_shell_manager(&mut self, tab_id: u64, generation: u64, tool: ItemId) {
        // Drain any outstanding screen changes before matching the visible footer.
        self.pump_terms();
        let fallback =
            "Focus the terminal at an empty Claude Code prompt, press ↓ on “↓ to manage”, then choose the shell there.";
        let Some(tab) = self.tabs.shown_mut().filter(|t| t.id == tab_id) else {
            self.warn("Open the shell manager manually", fallback.into());
            return;
        };
        let Some(pane) = &mut tab.pane else { return };
        pane.model.collect();
        if pane.model.generation != generation
            || pane.model.error.is_some()
            || !pane.model.shells.iter().any(|s| s.tool == tool && s.status == ShellStatus::Running)
        {
            self.warn("Shell state changed", "No terminal input was sent. Check the current Shells list.".into());
            return;
        }
        let ready = pane.chat.shell_manager_focus();
        let blocked = pane.model.chat.pending.is_some()
            || pane.model.chat.latest.is_some_and(|(_, phase)| phase == ChatPhase::Active)
            || matches!(tab.program_title.as_str(), "Claude Code — Working" | "Claude Code — Running");
        let sent = if ready
            && !blocked
            && pane.verified_transcript().is_some()
            && tab.term.running()
            && tab.term.scroll == 0
            && let Some(key) = manager_key(&tab.term.screen)
            && let Some(pty) = &tab.term.pty
        {
            pane.note_terminal_input();
            pty.write(&key).is_ok()
        } else {
            false
        };
        self.session_view.text_focus = false;
        if !sent {
            self.warn("Open the shell manager manually", fallback.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(prompt: &str, hint: &str) -> Screen {
        let mut screen = Screen::new(100, 12, 20);
        screen.feed(format!("\x1b[?2004h❯ {prompt}\r\n{hint}\x1b[1;{}H", 3 + prompt.len()).as_bytes());
        screen
    }

    #[test]
    fn manager_only_sends_one_down_from_an_empty_prompt_above_the_exact_footer() {
        let hint = "3 shells still running · ↓ to manage";
        assert_eq!(manager_key(&screen("", hint)), Some(b"\x1b[B".to_vec()));
        assert_eq!(manager_key(&screen("draft", hint)), None);
        assert_eq!(manager_key(&screen("", "see ↓ to manage")), None);
        assert_eq!(manager_key(&screen("", "0 shells still running · ↓ to manage")), None);
        assert_eq!(manager_key(&screen("", &format!("{hint}\r\nEnter to select"))), None);
        let mut alt = screen("", hint);
        alt.feed(b"\x1b[?1049h");
        assert_eq!(manager_key(&alt), None);
    }

    #[test]
    fn manager_accepts_boxed_and_faint_placeholder_prompts_but_not_real_text() {
        let hint = "3 shells still running · ↓ to manage";
        let mut boxed = Screen::new(100, 12, 20);
        boxed.feed(format!("\x1b[?2004h╭────────╮\r\n│ ❯      │\r\n╰────────╯\r\n{hint}\x1b[2;5H").as_bytes());
        assert_eq!(manager_key(&boxed), Some(b"\x1b[B".to_vec()));
        boxed.feed("draft".as_bytes());
        assert_eq!(manager_key(&boxed), None);

        let mut faint = Screen::new(100, 12, 20);
        faint.feed(format!("\x1b[?2004h❯ \x1b[2mTry “fix the tests”\x1b[0m\r\n{hint}\x1b[1;3H").as_bytes());
        assert_eq!(manager_key(&faint), Some(b"\x1b[B".to_vec()));
        faint.feed(b"\x1b[7;2mReal text\x1b[0m");
        assert_eq!(manager_key(&faint), None);
    }
}
