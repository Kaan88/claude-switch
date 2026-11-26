use super::{Lines, contains, flag, parse, prompt_of, squeeze, str_field, texts};
use serde_json::Value;
use std::path::Path;

pub fn row_activity(file: &Path) -> Option<(String, bool)> {
    Some(activity(&Lines::read(file, 0)?))
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
            ended = match kind {
                Some("assistant") => Some(assistant_ended(&v)),
                Some("user") if prompt.is_some() || has_block(&v, "tool_result") => Some(false),
                _ => None,
            };
        }
        if latest.is_some() && ended.is_some() {
            break;
        }
    }
    (latest.unwrap_or_default(), ended.unwrap_or_default())
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
}
