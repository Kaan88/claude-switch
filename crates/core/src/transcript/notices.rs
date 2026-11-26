//! Small notices from records that are not prompts, answers or tool calls.
//!
//! Claude Code's docs call the transcript's format internal. The shapes matched here were
//! confirmed against real transcripts (read-only; the tests use synthetic records):
//!
//! - an assistant `thinking` block `{thinking, signature}`; about half carry no text (omitted
//!   thinking), and those are not shown;
//! - `system` records with a `subtype`: `stop_hook_summary` (`hookErrors` is a list of strings,
//!   `preventedContinuation`, `stopReason`, `hookInfos`), `away_summary` (the session recap,
//!   `content`), `informational` (`content`, `level`: info, notice, suggestion or warning);
//! - Claude Code's own (`isMeta`) user records starting "Stop hook feedback:";
//! - `attachment` records whose `attachment.type` is `hook_non_blocking_error` (`hookName`,
//!   `hookEvent`, `stderr`, `stdout`, `exitCode`), `hook_system_message` (`hookName`, `content`)
//!   or `nested_memory` (`displayPath`, a CLAUDE.md loaded on demand).
//!
//! A prompt blocked by a hook was not seen; it is matched by the wording the docs quote. A record
//! that matches none of these is not shown: a wrong notice is worse than a missing one.
use super::*;

/// The most of a notice's text kept.
const MAX_TEXT: usize = 8 * 1024;

/// A notice for a `system` record, if it is one Switch shows.
pub(super) fn system(value: &Value) -> Option<(NoticeKind, String)> {
    let subtype = value["subtype"].as_str().unwrap_or("");
    let content = text(&value["content"]);
    match subtype {
        "away_summary" => Some((NoticeKind::Recap, content?)),
        "stop_hook_summary" => {
            let errors: Vec<String> = value["hookErrors"].as_array().into_iter().flatten().filter_map(text).collect();
            if !errors.is_empty() {
                return Some((NoticeKind::HookError, bounded(&errors.join("\n"))));
            }
            (value["preventedContinuation"] == true).then(|| {
                let reason = text(&value["stopReason"]).unwrap_or_else(|| "A Stop hook kept Claude working.".into());
                (NoticeKind::StopHook, reason)
            })
        }
        // Other banners only when Claude Code itself calls them a warning.
        "informational" => {
            let content = content?;
            classify(&content).or_else(|| (value["level"] == "warning").then_some((NoticeKind::Info, content)))
        }
        _ => None,
    }
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

/// The attachment kinds that are shown, found without parsing the (often large) record.
pub(super) fn shown_attachment(line: &[u8]) -> bool {
    [&b"\"hook_non_blocking_error\""[..], b"\"hook_system_message\"", b"\"nested_memory\""]
        .iter()
        .any(|kind| line.windows(kind.len()).any(|w| w == *kind))
}

/// A notice for an `attachment` record's `attachment`, if it is one Switch shows.
pub(super) fn attachment(attachment: &Value) -> Option<(NoticeKind, String)> {
    let hook = || attachment["hookName"].as_str().or(attachment["hookEvent"].as_str()).unwrap_or("Hook");
    match attachment["type"].as_str()? {
        // A Stop hook's error is also in the stop_hook_summary written just before it.
        "hook_non_blocking_error" if !matches!(attachment["hookEvent"].as_str(), Some("Stop" | "SubagentStop")) => {
            let detail = text(&attachment["stderr"]).or_else(|| text(&attachment["stdout"])).unwrap_or_default();
            Some((NoticeKind::HookError, bounded(format!("{} hook error\n{detail}", hook()).trim_end())))
        }
        "hook_system_message" => {
            Some((NoticeKind::HookMessage, bounded(&format!("{}: {}", hook(), text(&attachment["content"])?))))
        }
        "nested_memory" => {
            let path = text(&attachment["displayPath"]).or_else(|| text(&attachment["path"]))?;
            Some((NoticeKind::Memory, bounded(&format!("Loaded {path}"))))
        }
        _ => None,
    }
}

/// Matches the notice texts that are written as plain text.
fn classify(text: &str) -> Option<(NoticeKind, String)> {
    let first = text.lines().next().unwrap_or("");
    let kind = if first.starts_with("Stop hook feedback") || first.starts_with("SubagentStop hook feedback") {
        NoticeKind::StopHook
    } else if text.contains("Original prompt:") && first.contains("hook") {
        NoticeKind::PromptBlocked
    } else {
        return None;
    };
    Some((kind, bounded(text)))
}

/// Readable thinking text from an assistant block; omitted or redacted thinking has none.
pub(super) fn thought(block: &Value) -> Option<String> {
    (block["type"] == "thinking").then_some(())?;
    text(&block["thinking"])
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty()).map(bounded)
}

fn bounded(text: &str) -> String {
    text[..text.floor_char_boundary(MAX_TEXT)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_recorded_notices_are_recognised_and_other_records_are_not() {
        let sys = |v: Value| system(&v).map(|(k, t)| (k, t.lines().next().unwrap_or("").to_owned()));
        assert_eq!(
            sys(json!({"type":"system","subtype":"stop_hook_summary","hookCount":1,"hookInfos":[{"command":"x"}],
                "hookErrors":["Failed with non-blocking status code: broke"],"preventedContinuation":false,
                "stopReason":"","hasOutput":true,"level":"suggestion"})),
            Some((NoticeKind::HookError, "Failed with non-blocking status code: broke".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[],
                "preventedContinuation":true,"stopReason":"Tests still fail"})),
            Some((NoticeKind::StopHook, "Tests still fail".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"away_summary","isMeta":false,"content":"Fixed the parser."})),
            Some((NoticeKind::Recap, "Fixed the parser.".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"informational","level":"warning",
                "content":"A hook blocked the turn from ending 8 times."})),
            Some((NoticeKind::Info, "A hook blocked the turn from ending 8 times.".into()))
        );
        assert_eq!(
            sys(json!({"type":"system","subtype":"informational","level":"warning",
                "content":"UserPromptSubmit operation blocked by hook:\nno secrets\n\nOriginal prompt: hi"})),
            Some((NoticeKind::PromptBlocked, "UserPromptSubmit operation blocked by hook:".into()))
        );
        for quiet in [
            json!({"type":"system","subtype":"stop_hook_summary","hookErrors":[],"preventedContinuation":false}),
            json!({"type":"system","subtype":"informational","level":"notice","content":"Usage limit reset"}),
            json!({"type":"system","subtype":"turn_duration","durationMs":1200,"isMeta":false}),
            json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted","level":"info"}),
            json!({"type":"system","subtype":"local_command","content":"<command-name>/model</command-name>"}),
            json!({"type":"system","subtype":"api_error","level":"error","retryAttempt":1}),
            json!({"type":"system","subtype":"away_summary","content":""}),
        ] {
            assert_eq!(system(&quiet), None, "{quiet}");
        }
        let user = |text: &str| meta(&json!({"type":"user","isMeta":true,"message":{"content":text}})).map(|(k, _)| k);
        assert_eq!(user("Stop hook feedback:\n[hook]: run the tests"), Some(NoticeKind::StopHook));
        assert_eq!(user("A session-scoped Stop hook is now active"), None);
        assert_eq!(user("<system-reminder>context</system-reminder>"), None);
    }

    #[test]
    fn hook_messages_and_loaded_memory_show_and_other_attachments_do_not() {
        let att = |v: Value| attachment(&v);
        let error = json!({"type":"hook_non_blocking_error","hookName":"PostToolUse:Bash","hookEvent":"PostToolUse",
            "command":"x","exitCode":1,"stderr":"broke","stdout":"","durationMs":5,"toolUseID":"t"});
        assert_eq!(att(error), Some((NoticeKind::HookError, "PostToolUse:Bash hook error\nbroke".into())));
        let stop = json!({"type":"hook_non_blocking_error","hookName":"Stop","hookEvent":"Stop","stderr":"broke"});
        assert_eq!(att(stop), None, "the stop_hook_summary before it already says so");
        let message = json!({"type":"hook_system_message","hookName":"PostToolUse:Edit","content":"Formatted."});
        assert_eq!(att(message), Some((NoticeKind::HookMessage, "PostToolUse:Edit: Formatted.".into())));
        let memory = json!({"type":"nested_memory","displayPath":"src/CLAUDE.md","path":"/r/src/CLAUDE.md",
            "content":{"content":"rules","path":"/r/src/CLAUDE.md","type":"Project","contentDiffersFromDisk":false}});
        assert_eq!(att(memory), Some((NoticeKind::Memory, "Loaded src/CLAUDE.md".into())));
        for quiet in [
            json!({"type":"hook_success","hookName":"PostToolUse:Bash","stdout":"ok"}),
            json!({"type":"hook_additional_context","hookName":"SessionStart","content":["for Claude"]}),
            json!({"type":"file","filename":"a.md","displayPath":"a.md","content":{"type":"text"}}),
            json!({"type":"edited_text_file","filename":"a.md","snippet":"x"}),
            json!({"type":"instructions","files":[]}),
        ] {
            assert_eq!(att(quiet.clone()), None, "{quiet}");
        }
        let line = br#"{"type":"attachment","attachment":{"type":"nested_memory","displayPath":"x"}}"#;
        assert!(shown_attachment(line));
        assert!(!shown_attachment(br#"{"type":"attachment","attachment":{"type":"prompt_snapshot"}}"#));
    }

    #[test]
    fn only_readable_thinking_becomes_a_thought() {
        let block = json!({"type":"thinking","thinking":" Check the gate. ","signature":"s"});
        assert_eq!(thought(&block), Some("Check the gate.".into()));
        assert_eq!(thought(&json!({"type":"thinking","thinking":"","signature":"s"})), None);
        assert_eq!(thought(&json!({"type":"redacted_thinking","data":"x"})), None);
    }
}
