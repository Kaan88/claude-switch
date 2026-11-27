//! Settings › Chat: the quick-command chips under the message box and whether Chat answers the
//! CLI's menus. Stored under `chat` in the settings file:
//!
//! ```json
//! "chat": {"chips": ["model", "effort", "context", "mode", "compact", "clear", "commands"],
//!          "commands": ["/review --strict", "", ""],
//!          "answerMenus": true}
//! ```
//!
//! Every key is optional; a missing or wrong one gives its default. A chip key this version does
//! not know (from a newer version) is kept in the list and not shown.

use serde_json::{Value, json};

/// The built-in chips, in their default order: key and the label Settings shows.
pub const QUICK_CHIPS: [(&str, &str); 7] = [
    ("model", "Model"),
    ("effort", "Effort"),
    ("context", "Context"),
    ("mode", "Permission mode"),
    ("compact", "Compact"),
    ("clear", "Clear"),
    ("commands", "Command list (/ …)"),
];

/// How many own commands Settings offers.
pub const OWN_COMMANDS: usize = 3;
const MAX_COMMAND: usize = 200;
const MAX_CHIPS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatPrefs {
    /// Chip keys, shown in this order; known ones from [`QUICK_CHIPS`], unknown ones kept.
    pub chips: Vec<String>,
    /// [`OWN_COMMANDS`] slots, each empty or a valid command line ("/name args").
    pub commands: Vec<String>,
    /// Show the CLI's menus, pickers and permission prompts as cards Chat can answer.
    pub answer_menus: bool,
}

impl Default for ChatPrefs {
    fn default() -> Self {
        Self {
            chips: QUICK_CHIPS.iter().map(|(key, _)| (*key).to_owned()).collect(),
            commands: vec![String::new(); OWN_COMMANDS],
            answer_menus: true,
        }
    }
}

impl ChatPrefs {
    pub(super) fn read(v: &Value) -> Self {
        let d = Self::default();
        let chips = v.get("chips").and_then(Value::as_array).map_or(d.chips, |items| {
            let mut chips: Vec<String> = Vec::new();
            for key in items.iter().filter_map(Value::as_str) {
                if valid_key(key) && !chips.iter().any(|c| c == key) && chips.len() < MAX_CHIPS {
                    chips.push(key.to_owned());
                }
            }
            chips
        });
        let mut commands: Vec<String> = v
            .get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(OWN_COMMANDS)
            .map(|c| c.as_str().and_then(command_line).unwrap_or_default())
            .collect();
        commands.resize(OWN_COMMANDS, String::new());
        Self { chips, commands, answer_menus: v.get("answerMenus").and_then(Value::as_bool).unwrap_or(d.answer_menus) }
    }

    pub(super) fn json(&self) -> Value {
        json!({"chips": self.chips, "commands": self.commands, "answerMenus": self.answer_menus})
    }

    /// The known chips that are shown, in order.
    pub fn shown(&self) -> impl Iterator<Item = &str> {
        self.chips.iter().map(String::as_str).filter(|key| QUICK_CHIPS.iter().any(|(k, _)| k == key))
    }

    /// The own commands that are set, with their slot.
    pub fn own(&self) -> impl Iterator<Item = (usize, &str)> {
        self.commands.iter().enumerate().filter(|(_, c)| !c.is_empty()).map(|(i, c)| (i, c.as_str()))
    }

    /// Puts `key` at `slot` of the known chips (swapping with the chip there, or appending), or
    /// hides it with `None` at that slot. Unknown keys keep their place after the known ones.
    pub fn set_slot(&mut self, slot: usize, key: Option<&str>) {
        let mut known: Vec<String> = self.shown().map(str::to_owned).collect();
        let unknown: Vec<String> =
            self.chips.iter().filter(|c| !QUICK_CHIPS.iter().any(|(k, _)| k == c)).cloned().collect();
        match key.filter(|key| QUICK_CHIPS.iter().any(|(k, _)| k == key)) {
            None => {
                if slot < known.len() {
                    known.remove(slot);
                }
            }
            Some(key) => {
                let from = known.iter().position(|c| c == key);
                match (from, slot < known.len()) {
                    (Some(from), true) => known.swap(from, slot),
                    (Some(_), false) => {}
                    (None, true) => known[slot] = key.to_owned(),
                    (None, false) => known.push(key.to_owned()),
                }
            }
        }
        known.extend(unknown);
        self.chips = known;
    }

    /// Sets own command `slot` from typed text; false (nothing changed) when it is not a command
    /// line. Empty text clears the slot; the leading "/" may be left out.
    pub fn set_command(&mut self, slot: usize, text: &str) -> bool {
        let Some(line) = command_line(text) else { return false };
        let Some(entry) = self.commands.get_mut(slot) else { return false };
        *entry = line;
        true
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 32 && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

/// A typed command as stored: trimmed, with a leading "/", the name as the CLI's command names
/// (`plugin:name`, letters, digits, `-`, `_`, `.`), the rest one line of printable text. Empty
/// text is the empty command.
pub fn command_line(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return Some(String::new());
    }
    let body = text.strip_prefix('/').unwrap_or(text);
    let (name, args) = body.split_once(' ').map_or((body, ""), |(n, a)| (n, a.trim()));
    let name_ok = !name.is_empty()
        && name.len() <= 96
        && name.split(':').all(|part| {
            part.starts_with(|c: char| c.is_ascii_alphanumeric())
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        });
    let line = if args.is_empty() { format!("/{name}") } else { format!("/{name} {args}") };
    (name_ok && !args.chars().any(char::is_control) && line.len() <= MAX_COMMAND).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_wrong_keys_give_defaults_and_unknown_chips_are_kept() {
        assert_eq!(ChatPrefs::read(&Value::Null), ChatPrefs::default());
        assert_eq!(ChatPrefs::read(&json!({"chips": 3, "commands": "x", "answerMenus": "no"})), ChatPrefs::default());
        let p = ChatPrefs::read(&json!({
            "chips": ["clear", "future-chip", "model", "clear", "Bad Key", 7],
            "commands": ["/review  --strict ", "not a\u{7}command", "skill:go", "/fourth"],
            "answerMenus": false,
        }));
        assert_eq!(p.chips, ["clear", "future-chip", "model"]);
        assert_eq!(p.shown().collect::<Vec<_>>(), ["clear", "model"]);
        assert_eq!(p.commands, ["/review --strict", "", "/skill:go"]);
        assert!(!p.answer_menus);
        assert_eq!(ChatPrefs::read(&p.json()), p);
    }

    #[test]
    fn slots_swap_hide_and_append_without_losing_unknown_chips() {
        let mut p = ChatPrefs { chips: vec!["model".into(), "x-new".into(), "clear".into()], ..Default::default() };
        p.set_slot(0, Some("clear"));
        assert_eq!(p.chips, ["clear", "model", "x-new"]);
        p.set_slot(1, None);
        assert_eq!(p.chips, ["clear", "x-new"]);
        p.set_slot(5, Some("effort"));
        assert_eq!(p.chips, ["clear", "effort", "x-new"]);
        p.set_slot(0, Some("context"));
        assert_eq!(p.chips, ["context", "effort", "x-new"]);
        p.set_slot(0, Some("unknown"));
        assert_eq!(p.chips, ["effort", "x-new"], "an unknown key at a slot hides it");
    }

    #[test]
    fn own_commands_are_validated_and_slots_keep_their_place() {
        let mut p = ChatPrefs::default();
        assert!(p.set_command(1, "review src/main.rs"));
        assert_eq!(p.commands, ["", "/review src/main.rs", ""]);
        assert!(!p.set_command(1, "/"));
        assert!(!p.set_command(1, "/-flag"));
        assert!(!p.set_command(9, "/x"));
        assert_eq!(p.own().collect::<Vec<_>>(), [(1, "/review src/main.rs")]);
        assert!(p.set_command(1, "  "));
        assert_eq!(p.own().count(), 0);
    }
}
