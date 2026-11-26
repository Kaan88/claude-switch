//! Small notices from records that are not prompts, answers or tool calls.
//!
//! Claude Code's docs call the transcript's format internal and do not describe these records.
//! What is matched comes from the documented Agent SDK system messages (`informational` with
//! `content` and `level`, `hook_response` with `outcome`, `stderr` and `hook_name`) and the notice
//! texts the docs quote ("<hook name> hook error", "Stop hook feedback", a prompt block ending in
//! "Original prompt:", "Saved 2 memories", the session recap). A record that matches none of them
//! is not shown: a wrong notice is worse than a missing one.
use super::*;

/// The most of a notice's text kept.
const MAX_TEXT: usize = 8 * 1024;

/// A notice for a `system` record, if it is one Switch shows.
pub(super) fn system(value: &Value) -> Option<(NoticeKind, String)> {
    let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(bounded);
    let subtype = value["subtype"].as_str().unwrap_or("");
    let content = text(&value["content"]).or_else(|| text(&value["message"]["content"]));
    match subtype {
        "away_summary" | "recap" | "session_recap" => return Some((NoticeKind::Recap, content?)),
        "hook_response" if value["outcome"] == "error" => {
            let name = value["hook_name"].as_str().or(value["hook_event"].as_str()).unwrap_or("Hook");
            let detail = text(&value["stderr"]).or_else(|| text(&value["output"])).unwrap_or_default();
            return Some((NoticeKind::HookError, bounded(&format!("{name} hook error\n{detail}"))));
        }
        "stop_hook_summary" => {
            let errors: Vec<String> = value["hookErrors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| text(e).or_else(|| text(&e["error"])).or_else(|| text(&e["message"])))
                .collect();
            if !errors.is_empty() {
                return Some((NoticeKind::HookError, bounded(&errors.join("\n"))));
            }
            if value["preventedContinuation"] == true {
                let reason = text(&value["stopReason"]).unwrap_or_else(|| "A Stop hook kept Claude working.".into());
                return Some((NoticeKind::StopHook, reason));
            }
            return None;
        }
        _ => {}
    }
    let content = content?;
    if subtype.starts_with("memory") {
        return Some((NoticeKind::Memory, content));
    }
    classify(&content).or_else(|| {
        // Other banners only when Claude Code itself calls them a warning.
        (subtype == "informational" && value["level"] == "warning").then_some((NoticeKind::Info, content))
    })
}

/// A notice for a `user` record Claude Code wrote itself (`isMeta`), if it is one Switch shows.
pub(super) fn meta(value: &Value) -> Option<(NoticeKind, String)> {
    let content = &value["message"]["content"];
    let text = content.as_str().map(str::to_owned).unwrap_or_else(|| {
        blocks(content)
            .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten())
            .collect::<Vec<_>>()
            .join("\n")
    });
    classify(text.trim())
}

/// Matches the notice texts the docs quote.
fn classify(text: &str) -> Option<(NoticeKind, String)> {
    let first = text.lines().next().unwrap_or("");
    let kind = if first.starts_with("Stop hook feedback") || first.starts_with("SubagentStop hook feedback") {
        NoticeKind::StopHook
    } else if text.contains("Original prompt:") || (first.contains("blocked by") && first.contains("hook")) {
        NoticeKind::PromptBlocked
    } else if first.contains(" hook error") {
        NoticeKind::HookError
    } else if memory_line(first) {
        NoticeKind::Memory
    } else {
        return None;
    };
    Some((kind, bounded(text)))
}

/// "Saved 2 memories", "Recalled 1 memory".
fn memory_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("Saved ").or_else(|| line.strip_prefix("Recalled ")) else { return false };
    let mut words = rest.split_whitespace();
    words.next().is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        && words.next().is_some_and(|w| matches!(w.trim_end_matches('.'), "memories" | "memory"))
}

/// Readable thinking text from an assistant block; redacted or omitted thinking has none.
pub(super) fn thought(block: &Value) -> Option<String> {
    (block["type"] == "thinking").then_some(())?;
    block["thinking"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(bounded)
}

fn bounded(text: &str) -> String {
    text[..text.floor_char_boundary(MAX_TEXT)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_notices_are_recognised_and_other_records_are_not() {
        let sys = |v: Value| system(&v).map(|(k, t)| (k, t.lines().next().unwrap_or("").to_owned()));
        assert_eq!(
            sys(json!({"type":"system","subtype":"informational","level":"warning",
                "content":"UserPromptSubmit operation blocked by hook:\nno secrets\n\nOriginal prompt: hi"})),
            Some((NoticeKind::PromptBlocked, "UserPromptSubmit operation blocked by hook:".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"hook_response","hook_name":"PostToolUse:Bash",
                "outcome":"error","stderr":"boom"})),
            Some((NoticeKind::HookError, "PostToolUse:Bash hook error".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[],
                "preventedContinuation":true,"stopReason":"Tests still fail"})),
            Some((NoticeKind::StopHook, "Tests still fail".into()))
        );
        assert_eq!(sys(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[]})), None);
        assert_eq!(
            sys(json!({"type":"system","subtype":"away_summary","content":"Fixed the parser; tests pass."})),
            Some((NoticeKind::Recap, "Fixed the parser; tests pass.".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"informational","level":"notice","content":"Saved 2 memories"})),
            Some((NoticeKind::Memory, "Saved 2 memories".into()))
        );
        for quiet in [
            json!({"type":"system","subtype":"informational","level":"info","content":"Tip: press ? for shortcuts"}),
            json!({"type":"system","subtype":"turn_duration","durationMs":1200}),
            json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}),
            json!({"type":"system","subtype":"local_command","content":"<command-name>/model</command-name>"}),
            json!({"type":"system","subtype":"away_summary"}),
        ] {
            assert_eq!(system(&quiet), None, "{quiet}");
        }
        let user = |text: &str| meta(&json!({"type":"user","isMeta":true,"message":{"content":text}})).map(|(k, _)| k);
        assert_eq!(user("Stop hook feedback:\n[hook]: run the tests"), Some(NoticeKind::StopHook));
        assert_eq!(
            user("PreToolUse:Bash hook error\nFailed with non-blocking status code: broke"),
            Some(NoticeKind::HookError)
        );
        assert_eq!(user("Recalled 1 memory"), Some(NoticeKind::Memory));
        assert_eq!(user("Saved the file"), None);
        assert_eq!(user("<system-reminder>context</system-reminder>"), None);
    }

    #[test]
    fn only_readable_thinking_becomes_a_thought() {
        let block = json!({"type":"thinking","thinking":" Check the gate. ","signature":"s"});
        assert_eq!(thought(&block), Some("Check the gate.".into()));
        assert_eq!(thought(&json!({"type":"thinking","thinking":"","signature":"s"})), None);
        assert_eq!(thought(&json!({"type":"redacted_thinking","data":"x"})), None);
    }
}
