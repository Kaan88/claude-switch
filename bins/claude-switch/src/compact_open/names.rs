//! A handover title is distinct from the Markdown and from the OpenCode run title.
use claude_switch_core::{Session, marks::Marks};
use std::collections::HashSet;

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

fn key(name: &str) -> String {
    // The transcript index squeezes whitespace in native titles.
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn suggest(marks: &Marks, sessions: &[Session], source: &str) -> String {
    suggest_at(marks, sessions, source, claude_switch_core::usage::now_ms())
}

fn suggest_at(marks: &Marks, sessions: &[Session], source: &str, now: u64) -> String {
    let indexed: HashSet<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    let used: HashSet<_> = sessions
        .iter()
        .map(|s| displayed(marks, s))
        .chain(marks.handover_launch_names(now).filter(|(id, _)| !indexed.contains(id)).map(|(_, name)| name))
        .map(key)
        .collect();
    next(source, &used)
}

fn next(source: &str, used: &HashSet<String>) -> String {
    let source: String = source.chars().map(|c| if forbidden(c) { ' ' } else { c }).collect();
    let source = source.trim();
    let source = if source.is_empty() { "Session" } else { source };
    for n in 1.. {
        let suffix = if n == 1 { SUFFIX.into() } else { format!("{SUFFIX} {n}") };
        let mut limit = MAX - suffix.encode_utf16().count();
        let end = source
            .char_indices()
            .find_map(|(i, c)| {
                if c.len_utf16() > limit {
                    Some(i)
                } else {
                    limit -= c.len_utf16();
                    None
                }
            })
            .unwrap_or(source.len());
        let candidate = format!("{}{suffix}", source[..end].trim_end());
        if !used.contains(&key(&candidate)) {
            return candidate;
        }
    }
    unreachable!("a finite set cannot reserve every suffix")
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
    fn name_uses_lowest_free_number_case_insensitively_and_keeps_the_suffix_when_truncated() {
        let mut used = HashSet::from(["review · continued".into(), "review · continued 3".into()]);
        assert_eq!(next("Review", &used), "Review · continued 2");
        used.insert("review · continued 2".into());
        assert_eq!(next("Review", &used), "Review · continued 4");
        assert_eq!(next("\t", &used), "Session · continued");
        let long = next(&"😊".repeat(200), &used);
        assert!(validate(&long).is_ok() && long.ends_with(SUFFIX));
        let other = next(&"😊".repeat(200), &HashSet::from([long]));
        assert!(validate(&other).is_ok() && other.ends_with(" · continued 2"));
    }

    #[test]
    fn continuation_uses_switch_names_and_numbers_against_native_titles_and_pending_launches() {
        let scratch = crate::testing::Scratch::new("cs-handover-index");
        let root = scratch.to_path_buf();
        std::fs::create_dir(&root).unwrap();
        let source = "5eed3dde-1c2d-4e5f-8a9b-000000000001";
        let first = "5eed3dde-1c2d-4e5f-8a9b-000000000002";
        let pending = "5eed3dde-1c2d-4e5f-8a9b-000000000003";
        let fixture = |id, title| {
            let file = root.join(format!("{id}.jsonl"));
            let user = serde_json::json!({
                "type": "user", "cwd": root, "message": {"role": "user", "content": "## Goal\n\nSynthetic prompt"}
            });
            let custom = serde_json::json!({"type":"custom-title","customTitle":title,"sessionId":id});
            let ai = serde_json::json!({"type":"ai-title","aiTitle":"Claude Switch handover","sessionId":id});
            std::fs::write(&file, format!("{user}\n{custom}\n{ai}\n")).unwrap();
            claude_switch_core::sessions::read_transcript(&file).unwrap()
        };
        let mut marks = Marks::default();
        let sessions = [fixture(source, "Native source"), fixture(first, "Review · continued")];
        assert_eq!(displayed(&marks, &sessions[0]), "Native source");
        assert!(marks.rename(source, &sessions[0].file, "Review"));
        let from = displayed(&marks, &sessions[0]);
        assert_eq!(from, "Review");
        assert_eq!(displayed(&marks, &sessions[1]), "Review · continued");
        assert_eq!(suggest_at(&marks, &sessions, from, 10), "Review · continued 2");
        assert!(marks.set_handover_launch_name(pending, "Review · continued 2", 10));
        assert_eq!(suggest_at(&marks, &sessions, "Review", 11), "Review · continued 3");
        assert_eq!(suggest_at(&marks, &sessions, "Review", 120_010), "Review · continued 2");
        let [a, b] = sessions;
        let sessions = [a, b, fixture(pending, "Renamed later in Claude")];
        assert_eq!(suggest_at(&marks, &sessions, "Review", 11), "Review · continued 2");
        std::fs::remove_dir_all(root).unwrap();
    }
}
