//! A handover title is distinct from the Markdown and from the OpenCode run title.
use claude_switch_core::{Session, marks::Marks};

const MAX: usize = 200;
const SUFFIX: &str = " · continued";

fn forbidden(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')
}

pub fn validate(name: &str) -> Result<&str, String> {
    let error = "Enter a session name on one line (1–200 UTF-16 code units, no control characters).";
    if name.chars().any(forbidden) {
        return Err(error.into());
    }
    let name = name.trim();
    if name.is_empty() || name.encode_utf16().count() > MAX {
        return Err(error.into());
    }
    Ok(name)
}

pub fn displayed<'a>(marks: &'a Marks, session: &'a Session) -> &'a str {
    marks.name(&session.id).unwrap_or(&session.title)
}

/// The name a handover suggests for the new session: the session's own name, kept as it is
/// (owner, 2026-11-28). The old session is marked as handed over, so the list tells the two apart.
/// Endings " · continued" and " · continued N" that earlier versions added are removed.
pub fn suggest(source: &str) -> String {
    let source: String = source.chars().map(|c| if forbidden(c) { ' ' } else { c }).collect();
    let mut name = source.trim();
    let numbered = format!("{SUFFIX} ");
    loop {
        // "… · continued", or "… · continued N".
        let digits = name.trim_end_matches(|c: char| c.is_ascii_digit());
        let rest = name
            .strip_suffix(SUFFIX)
            .or_else(|| (digits.len() < name.len()).then(|| digits.strip_suffix(&numbered)).flatten());
        match rest {
            Some(rest) => name = rest.trim_end(),
            None => break,
        }
    }
    let name = if name.is_empty() { "Session" } else { name };
    let mut limit = MAX;
    let end = name
        .char_indices()
        .find_map(|(i, c)| {
            if c.len_utf16() > limit {
                Some(i)
            } else {
                limit -= c.len_utf16();
                None
            }
        })
        .unwrap_or(name.len());
    name[..end].trim_end().to_owned()
}

/// Only used at the fixed shell bootstrap boundary, which takes validated whitespace-free words.
/// The child decodes and validates again; the native CLI gets the original Unicode name.
pub(crate) fn encode(name: &str) -> Result<String, String> {
    use std::fmt::Write as _;
    let name = validate(name)?;
    let mut word = String::with_capacity(name.len() * 2);
    for byte in name.bytes() {
        let _ = write!(word, "{byte:02x}");
    }
    Ok(word)
}

pub(crate) fn decode(word: &str) -> Result<String, String> {
    let bad = "Invalid handover window name.";
    // BMP characters need at most three bytes per UTF-16 unit.
    if word.len() > MAX * 6 || !word.len().is_multiple_of(2) || !word.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad.into());
    }
    let bytes = (0..word.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&word[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| bad)?;
    let text = String::from_utf8(bytes).map_err(|_| bad)?;
    validate(&text).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handover_keeps_the_name_and_drops_old_continued_endings() {
        assert_eq!(suggest("mdapps"), "mdapps");
        assert_eq!(suggest("mdapps · continued · continued · continued"), "mdapps");
        assert_eq!(suggest("Review · continued 2"), "Review");
        assert_eq!(suggest("Review · continued 3 · continued"), "Review");
        assert_eq!(suggest("Plan 2"), "Plan 2", "a number that is part of the name stays");
        assert_eq!(suggest("Plan · continued 12"), "Plan");
        assert_eq!(suggest("	"), "Session");
        assert_eq!(suggest("Name · continued"), "Name");
        let long = suggest(&"😊".repeat(200));
        assert!(validate(&long).is_ok());
    }
}
