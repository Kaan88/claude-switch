//! Settings › Chat: the quick-command chips under the message box (which, in what order, the
//! user's own command lines, what the Compact chip runs) and the switch for answering the CLI's
//! menus from the chat.
//!
//! The order is a row of slots, each a dropdown of the chips: picking a chip that sits in another
//! slot swaps the two, "None" hides the slot's chip. Own commands are text rows saved as typed
//! whenever the text is a command line; the page is not rebuilt for them, so the caret stays.
use super::{AppState, Dialog, Row};
use claude_switch_core::config::{ChatPrefs, OWN_COMMANDS, QUICK_CHIPS};
use mdshared_ui::settings::Value;

pub(crate) const CHAT: &str = "Chat";
/// Slot `n` of the chip order is `CHIP_SLOT + n`.
pub(super) const CHIP_SLOT: u32 = 200;
/// Own command `n` is `OWN_COMMAND + n`.
pub(super) const OWN_COMMAND: u32 = 220;
pub(super) const RESET_CHIPS: u32 = 230;
pub(super) const SET_ANSWER_MENUS: u32 = 231;
pub(super) const SET_COMPACT_CHIP: u32 = 232;
const NONE: &str = "None";
/// What the Compact chip runs, in the order of the choice: Switch's handover first (the default).
const COMPACT_CHOICES: [&str; 2] = ["Switch handover", "Claude Code /compact"];

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let prefs = &st.config.chat;
    let shown: Vec<&str> = prefs.shown().collect();
    let mut options: Vec<&str> = QUICK_CHIPS.iter().map(|(_, label)| *label).collect();
    options.push(NONE);
    let mut rows = vec![Row::header("Quick commands")];
    for slot in 0..QUICK_CHIPS.len() {
        let value =
            shown.get(slot).and_then(|key| QUICK_CHIPS.iter().position(|(k, _)| k == key)).unwrap_or(QUICK_CHIPS.len());
        let row = Row::choice(CHIP_SLOT + slot as u32, format!("Chip {}", slot + 1), &options, value);
        rows.push(match slot {
            0 => row.help("Left to right. Taking a chip from another slot swaps."),
            _ => row,
        });
    }
    for (slot, command) in prefs.commands.iter().enumerate().take(OWN_COMMANDS) {
        rows.push(
            Row::text(OWN_COMMAND + slot as u32, format!("Own command {}", slot + 1), command.as_str())
                .placeholder("/review --strict")
                .help(if slot == 0 {
                    "A chip after the others sends this line."
                } else {
                    "Shown when there is room on the chip row."
                }),
        );
    }
    rows.push(Row::action(RESET_CHIPS, "Chips and own commands", "Restore defaults"));
    rows.push(
        Row::choice(SET_COMPACT_CHIP, "Compact chip runs", &COMPACT_CHOICES, usize::from(!prefs.compact_handover))
            .help("Handover: summary, then fresh context."),
    );
    rows.push(Row::header("Claude Code's menus"));
    rows.push(
        Row::toggle(SET_ANSWER_MENUS, "Answer menus from the chat", prefs.answer_menus)
            .help("Menus and permission prompts as cards. Off: terminal."),
    );
    rows
}

pub(super) fn owns(id: u32) -> bool {
    (CHIP_SLOT..CHIP_SLOT + QUICK_CHIPS.len() as u32).contains(&id)
        || (OWN_COMMAND..OWN_COMMAND + OWN_COMMANDS as u32).contains(&id)
        || matches!(id, RESET_CHIPS | SET_ANSWER_MENUS | SET_COMPACT_CHIP)
}

/// Applies a change or action of this page and saves it; false when it was not this page's.
pub(super) fn apply(st: &mut AppState, id: u32, value: Option<Value>) -> bool {
    if !owns(id) {
        return false;
    }
    if st.config_damaged {
        return true;
    }
    let prefs = &mut st.config.chat;
    let rebuild = match (id, value) {
        (RESET_CHIPS, None) => {
            let (answer_menus, compact_handover) = (prefs.answer_menus, prefs.compact_handover);
            *prefs = ChatPrefs { answer_menus, compact_handover, ..ChatPrefs::default() };
            true
        }
        (SET_COMPACT_CHIP, Some(Value::Choice(i))) if i < COMPACT_CHOICES.len() => {
            prefs.compact_handover = i == 0;
            false
        }
        (SET_ANSWER_MENUS, Some(Value::Bool(b))) => {
            prefs.answer_menus = b;
            false
        }
        (id, Some(Value::Choice(i))) if id >= CHIP_SLOT && id < CHIP_SLOT + QUICK_CHIPS.len() as u32 => {
            prefs.set_slot((id - CHIP_SLOT) as usize, QUICK_CHIPS.get(i).map(|(key, _)| *key));
            true
        }
        // Not a command line (yet): nothing is saved.
        (id, Some(Value::Text(text))) if id >= OWN_COMMAND && prefs.set_command((id - OWN_COMMAND) as usize, &text) => {
            false
        }
        _ => return true,
    };
    st.save_config();
    if rebuild {
        refresh(st);
    }
    true
}

fn refresh(st: &mut AppState) {
    let rows = rows(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == CHAT
    {
        settings.form.set_rows(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{page_form, settings, settings_outcome};
    use super::*;
    use crate::source::Source;
    use mdshared_ui::settings::Outcome;

    fn change(st: &mut AppState, id: u32, value: Value) {
        settings_outcome(st, Outcome::Changed { id, value });
    }

    fn slots(st: &AppState) -> Vec<usize> {
        let Some(Dialog::Settings(s)) = &st.dialog else { panic!("Settings must be open") };
        (0..QUICK_CHIPS.len())
            .map(|n| match s.form.value(CHIP_SLOT + n as u32) {
                Some(Value::Choice(i)) => i,
                _ => panic!("slot {n} must be a choice"),
            })
            .collect()
    }

    #[test]
    fn slots_reorder_hide_and_restore_the_chips() {
        let mut st = AppState::new(Source::Demo, None, Default::default());
        st.dialog = Some(settings(&mut st, Some(CHAT)));
        assert_eq!(slots(&st), [0, 1, 2, 3, 4, 5, 6]);
        change(&mut st, CHIP_SLOT, Value::Choice(5));
        assert_eq!(st.config.chat.chips[..2], ["clear", "effort"]);
        assert_eq!(slots(&st), [5, 1, 2, 3, 4, 0, 6], "the page shows the swap");
        change(&mut st, CHIP_SLOT + 1, Value::Choice(QUICK_CHIPS.len()));
        assert!(!st.config.chat.chips.iter().any(|c| c == "effort"));
        assert_eq!(slots(&st)[6], QUICK_CHIPS.len(), "the last slot is empty");
        change(&mut st, CHIP_SLOT + 6, Value::Choice(1));
        assert_eq!(st.config.chat.chips.last().map(String::as_str), Some("effort"));
        change(&mut st, OWN_COMMAND + 2, Value::Text("review --strict".into()));
        assert_eq!(st.config.chat.commands[2], "/review --strict");
        change(&mut st, OWN_COMMAND + 2, Value::Text("/".into()));
        assert_eq!(st.config.chat.commands[2], "/review --strict", "a non-command is not saved");
        change(&mut st, SET_ANSWER_MENUS, Value::Bool(false));
        settings_outcome(&mut st, Outcome::Action(RESET_CHIPS));
        assert_eq!(st.config.chat, ChatPrefs { answer_menus: false, ..ChatPrefs::default() });
        assert_eq!(slots(&st), [0, 1, 2, 3, 4, 5, 6]);
    }

    /// What the Compact chip runs: Switch's handover by default, saved when changed, and kept by
    /// Restore defaults (which is for the chips and own commands).
    #[test]
    fn the_compact_chip_choice_defaults_to_the_handover_and_is_saved() {
        let mut st = AppState::new(Source::Demo, None, Default::default());
        st.dialog = Some(settings(&mut st, Some(CHAT)));
        let choice = |st: &AppState| {
            let Some(Dialog::Settings(s)) = &st.dialog else { panic!("Settings must be open") };
            s.form.value(SET_COMPACT_CHIP)
        };
        assert!(st.config.chat.compact_handover);
        assert_eq!(choice(&st), Some(Value::Choice(0)), "Switch handover is the default");
        change(&mut st, SET_COMPACT_CHIP, Value::Choice(1));
        assert!(!st.config.chat.compact_handover, "Claude Code /compact");
        change(&mut st, SET_COMPACT_CHIP, Value::Choice(7));
        assert!(!st.config.chat.compact_handover, "a choice the page does not have changes nothing");
        settings_outcome(&mut st, Outcome::Action(RESET_CHIPS));
        assert!(!st.config.chat.compact_handover, "Restore defaults keeps it");
        assert_eq!(choice(&st), Some(Value::Choice(1)));
        change(&mut st, SET_COMPACT_CHIP, Value::Choice(0));
        assert!(st.config.chat.compact_handover);
    }

    #[test]
    fn the_page_uses_one_control_per_kind_of_setting() {
        let st = AppState::new(Source::Demo, None, Default::default());
        let index = super::super::PAGES.iter().position(|(name, _)| *name == CHAT).unwrap();
        let form = page_form(&st, index);
        for row in form.rows() {
            match row {
                Row::Choice { id, options, .. } if *id == SET_COMPACT_CHIP => assert_eq!(options.len(), 2),
                Row::Choice { id, options, .. } => {
                    assert!(owns(*id) && *id < OWN_COMMAND && options.len() == QUICK_CHIPS.len() + 1)
                }
                Row::Text { id, .. } => assert!((OWN_COMMAND..RESET_CHIPS).contains(id)),
                Row::Toggle { id, .. } => assert_eq!(*id, SET_ANSWER_MENUS),
                Row::Action { id, .. } => assert_eq!(*id, RESET_CHIPS),
                Row::Header(_) => {}
                Row::Number { .. } => panic!("no number rows"),
            }
        }
    }
}
