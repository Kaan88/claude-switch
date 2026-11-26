use super::{Lines, contains, flag, parse, prompt_of, squeeze, str_field, texts};
use serde_json::Value;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

pub fn row_activity(file: &Path) -> Option<(String, bool)> {
    Some(activity(&Lines::read(file, 0)?))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnActivity {
    pub prompt: Option<u64>,
    pub waiting: bool,
    pub waiting_since: Option<i64>,
}

/// Uses the list's meaningful-record classification; separate records with identical text are
/// separate turns. A missing prompt in a bounded tail cannot establish a new turn.
pub fn turn_activity(file: &Path) -> Option<TurnActivity> {
    let lines = Lines::read(file, 0)?;
    let mut turn = TurnActivity::default();
    let mut got_state = false;
    for line in lines.recent() {
        if !contains(line, b"\"user\"") && !contains(line, b"\"assistant\"") {
            continue;
        }
        let Some(v) = parse(line) else { continue };
        if flag(&v, "isSidechain") || flag(&v, "isMeta") || flag(&v, "isCompactSummary") {
            continue;
        }
        let kind = str_field(&v, "type");
        let prompt = kind == Some("user") && prompt_of(&v, 500).is_some();
        if !got_state && let Some(waiting) = waiting_of(&v, kind, prompt) {
            got_state = true;
            turn.waiting = waiting;
            if waiting {
                turn.waiting_since = str_field(&v, "timestamp").and_then(crate::usage::timestamp);
            }
        }
        if prompt {
            let mut hash = DefaultHasher::new();
            line.hash(&mut hash);
            turn.prompt = Some(hash.finish());
            break;
        }
    }
    Some(turn)
}

pub(super) fn activity(lines: &Lines) -> (String, bool) {
    let (mut latest, mut ended) = (None, None);
    for line in lines.recent() {
        if ![b"\"user\"".as_slice(), b"\"assistant\"", b"\"last-prompt\""].iter().any(|needle| contains(line, needle)) {
            continue;
        }
        let Some(v) = parse(line) else { continue };
        if flag(&v, "isSidechain") || flag(&v, "isMeta") || flag(&v, "isCompactSummary") {
            continue;
        }
        let kind = str_field(&v, "type");
        let prompt = prompt_of(&v, 500);
        if latest.is_none() {
            latest = if kind == Some("last-prompt") {
                str_field(&v, "lastPrompt").map(|s| squeeze(s, 500)).filter(|s| !s.is_empty() && !s.starts_with('<'))
            } else {
                prompt.clone()
            };
        }
        if ended.is_none() {
            ended = waiting_of(&v, kind, prompt.is_some());
        }
        if latest.is_some() && ended.is_some() {
            break;
        }
    }
    (latest.unwrap_or_default(), ended.unwrap_or_default())
}

fn waiting_of(v: &Value, kind: Option<&str>, prompt: bool) -> Option<bool> {
    match kind {
        Some("assistant") => Some(assistant_ended(v)),
        Some("user") if prompt || has_block(v, "tool_result") => Some(false),
        _ => None,
    }
}

fn has_block(v: &Value, kind: &str) -> bool {
    v["message"]["content"].as_array().is_some_and(|bs| bs.iter().any(|b| str_field(b, "type") == Some(kind)))
}

fn assistant_ended(v: &Value) -> bool {
    !has_block(v, "tool_use")
        && match str_field(&v["message"], "stop_reason") {
            Some("end_turn" | "stop_sequence") => true,
            Some(_) => false,
            None => texts(v).iter().any(|t| !t.trim().is_empty()),
        }
}

/// First real prompt from the head and up to three recent real prompts, oldest first.
pub fn naming_prompts(file: &Path) -> (Option<String>, Vec<String>) {
    let Some(lines) = Lines::read(file, super::CHUNK) else { return (None, Vec::new()) };
    let prompt = |line: &[u8]| contains(line, b"\"user\"").then(|| prompt_of(&parse(line)?, 500)).flatten();
    let first = super::split_lines(&lines.head).into_iter().find_map(prompt);
    let mut last: Vec<_> = lines.recent().filter_map(prompt).take(3).collect();
    last.reverse();
    (first, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, put};

    #[test]
    fn uses_last_prompt_and_skips_internal_user_records() {
        let dir = TempDir::new("prompt");
        let file = dir.0.join("test.jsonl");
        let content = r#"{"type":"user","message":{"content":"first"}}
{"type":"user","message":{"content":[{"type":"text","text":"second"},{"type":"text","text":"part"}]}}
{"type":"user","isMeta":true,"message":{"content":"not a prompt"}}
{"type":"user","message":{"content":" <command/>"}}
{"type":"user","message":{"content":[{"type":"tool_result"}]}}
"#;
        put(&file, content);
        assert_eq!(row_activity(&file), Some(("second part".into(), false)));
        put(&file, &format!("{content}{{\"type\":\"last-prompt\",\"lastPrompt\":\"latest  recorded\"}}\n"));
        assert_eq!(row_activity(&file).unwrap().0, "latest recorded");
        assert_eq!(naming_prompts(&file), (Some("first".into()), vec!["first".into(), "second part".into()]));
    }

    #[test]
    fn waiting_ignores_bookkeeping_but_not_tool_calls_or_results() {
        let check = |s: &str| activity(&Lines { head: s.as_bytes().to_vec(), tail: vec![], whole: true }).1;
        let ended = r#"{"type":"assistant","message":{"content":"Done","stop_reason":"end_turn"}}"#;
        assert!(check(&format!("{ended}\n{{\"type\":\"last-prompt\",\"lastPrompt\":\"go\"}}")));
        for next in [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Checking"},{"type":"tool_use"}]}}"#,
            r#"{"type":"assistant","message":{"content":"Checking","stop_reason":"tool_use"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
            r#"{"type":"user","message":{"content":"Try again"}}"#,
        ] {
            assert!(!check(&format!("{ended}\n{next}")), "{next}");
        }
    }

    #[test]
    fn turn_activity_agrees_with_waiting_and_identifies_repeated_prompt_text_by_record() {
        let dir = TempDir::new("turn-notices");
        let file = dir.0.join("test.jsonl");
        let one = r#"{"type":"user","uuid":"first","message":{"content":"go"}}"#;
        let two = r#"{"type":"user","uuid":"second","message":{"content":"go"}}"#;
        let ended = r#"{"type":"assistant","timestamp":"2026-11-26T16:00:00Z","message":{"content":"Done"}}"#;
        put(&file, &format!("{one}\n{ended}\n"));
        let first = turn_activity(&file).unwrap();
        assert!(first.waiting && row_activity(&file).unwrap().1);
        assert!(first.waiting_since.is_some());
        put(&file, &format!("{one}\n{ended}\n{two}\n"));
        let second = turn_activity(&file).unwrap();
        assert!(!second.waiting && !row_activity(&file).unwrap().1);
        assert_ne!(first.prompt, second.prompt);
    }
}
