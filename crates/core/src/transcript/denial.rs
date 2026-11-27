//! A tool call Claude Code refused to run: the denial as its transcript records it.
//!
//! Recorded from Claude Code 2.1.332 (2026-11-27; the tests hold the records with ids and paths
//! replaced). The `user` record that carries the call's `tool_result` has, at its top level:
//!
//! - `toolDenialKind`: `"user-rejected"` (the user's own No, or Esc, at the permission prompt;
//!   with `toolDenialBeforeRun: true`), `"permission-rule"` (a deny rule) or `"automode-blocked"`
//!   (the auto mode classifier);
//! - for a rule and for auto mode, `permissionDecision` with `decision: "reject"`,
//!   `reasonType` (`"subcommandResults"` for the rule seen, `"classifier"` for auto mode) and, for
//!   a rule, `ruleSource` (`"flagSettings"`: a rule from `--settings`);
//! - `toolUseResult`, the text Claude was given, as is the result block's content.
//!
//! An allowed call in auto mode has `permissionDecision` with `decision: "accept"` and no
//! `toolDenialKind`. Only a record with `toolDenialKind` and a failed result is a denial: a kind
//! this file does not know is kept as `Other`, never guessed from the result's words.
use serde_json::Value;

/// The most of a denial's reason kept.
const MAX_REASON: usize = 400;
const MAX_SOURCE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenialKind {
    /// The user said No (or pressed Esc) at the permission prompt.
    User,
    /// A permission rule denies the call.
    Rule,
    /// Auto mode's classifier blocked it.
    AutoMode,
    /// A kind Claude Code names that Switch does not know.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    pub kind: DenialKind,
    /// `toolDenialBeforeRun`: the call never started. `None` when the record does not say.
    pub before_run: Option<bool>,
    /// What Claude was told: the classifier's reason for auto mode, else the result's first line.
    pub reason: String,
    /// Where the deciding rule comes from, as Claude Code names it (`flagSettings`, …).
    pub rule_source: Option<String>,
}

/// The denial a `user` record carries for its one failed tool result (`block`).
pub(super) fn from_record(record: &Value, block: &Value) -> Option<Denial> {
    let kind = match record["toolDenialKind"].as_str()? {
        "user-rejected" => DenialKind::User,
        "permission-rule" => DenialKind::Rule,
        "automode-blocked" => DenialKind::AutoMode,
        _ => DenialKind::Other,
    };
    if block["is_error"] != true {
        return None;
    }
    let text = result_text(&block["content"]);
    let reason = match kind {
        DenialKind::AutoMode => classifier_reason(&text).unwrap_or_else(|| first_line(&text)),
        _ => first_line(&text),
    };
    let rule_source = record["permissionDecision"]["ruleSource"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= MAX_SOURCE && !s.chars().any(char::is_control))
        .map(str::to_owned);
    Some(Denial { kind, before_run: record["toolDenialBeforeRun"].as_bool(), reason, rule_source })
}

/// A tool result's text: a string, or the text blocks of a list.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The classifier's own words: what follows "Reason: " up to Claude Code's advice to Claude.
fn classifier_reason(text: &str) -> Option<String> {
    let (_, rest) = text.split_once("Reason: ")?;
    let reason = rest.split_once(" If you have other tasks").map_or(rest, |(reason, _)| reason);
    let reason = bounded(reason.trim());
    (!reason.is_empty()).then_some(reason)
}

fn first_line(text: &str) -> String {
    bounded(text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(""))
}

fn bounded(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    text[..text.floor_char_boundary(MAX_REASON)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::super::{ItemKind, Reader, ToolState};
    use super::*;
    use serde_json::json;

    /// The three denial records and an allowed one, as Claude Code 2.1.332 wrote them (cut to the
    /// keys that matter, ids and folders replaced).
    fn recorded() -> [(&'static str, Value); 4] {
        let classifier = "Permission for this action was denied by the Claude Code auto mode classifier. Reason: The \
            server-side auto mode classifier judged this action dangerous (it gave no explanation). If you have other \
            tasks that don't depend on this action, continue working on those. IMPORTANT: You *may* attempt to \
            accomplish this action using other tools that might naturally be used to accomplish this goal, e.g. using \
            head instead of cat.";
        [
            (
                "toolu_user",
                json!({"type":"user","toolDenialKind":"user-rejected","toolDenialBeforeRun":true,
                    "toolUseResult":"User rejected tool use",
                    "message":{"role":"user","content":[{"type":"tool_result","is_error":true,"tool_use_id":"toolu_user",
                        "content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed."}]}}),
            ),
            (
                "toolu_rule",
                json!({"type":"user","toolDenialKind":"permission-rule",
                    "permissionDecision":{"decision":"reject","source":"config","reasonType":"subcommandResults",
                        "ruleSource":"flagSettings","reasonCode":"rule:flagSettings"},
                    "toolUseResult":"Error: Permission to use Bash with command mkdir probe_d4 has been denied.",
                    "message":{"role":"user","content":[{"type":"tool_result","is_error":true,"tool_use_id":"toolu_rule",
                        "content":"Permission to use Bash with command mkdir probe_d4 has been denied."}]}}),
            ),
            (
                "toolu_auto",
                json!({"type":"user","toolDenialKind":"automode-blocked",
                    "permissionDecision":{"decision":"reject","source":"config","reasonType":"classifier",
                        "reasonCode":"classifier:auto-mode"},
                    "toolUseResult": format!("Error: {classifier}"),
                    "serverClassifierContext":{"request":"00000000-0000-0000-0000-000000000000"},
                    "message":{"role":"user","content":[{"type":"tool_result","is_error":true,"tool_use_id":"toolu_auto",
                        "content":classifier}]}}),
            ),
            (
                "toolu_ok",
                json!({"type":"user","permissionDecision":{"decision":"accept","source":"config","reasonType":"classifier"},
                    "toolUseResult":{"stdout":"","stderr":"","interrupted":false},
                    "message":{"role":"user","content":[{"type":"tool_result","is_error":false,"tool_use_id":"toolu_ok",
                        "content":"done"}]}}),
            ),
        ]
    }

    #[test]
    fn the_recorded_denials_are_read_with_their_kind_reason_and_source() {
        let denial = |record: &Value| from_record(record, &record["message"]["content"][0]);
        let [user, rule, auto, ok] = recorded().map(|(_, record)| denial(&record));
        let user = user.expect("the user's No");
        assert_eq!((user.kind, user.before_run, user.rule_source), (DenialKind::User, Some(true), None));
        assert_eq!(
            user.reason,
            "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed."
        );
        let rule = rule.expect("a rule");
        assert_eq!((rule.kind, rule.before_run), (DenialKind::Rule, None));
        assert_eq!(rule.rule_source.as_deref(), Some("flagSettings"));
        assert_eq!(rule.reason, "Permission to use Bash with command mkdir probe_d4 has been denied.");
        let auto = auto.expect("auto mode");
        assert_eq!((auto.kind, auto.rule_source), (DenialKind::AutoMode, None));
        assert_eq!(
            auto.reason,
            "The server-side auto mode classifier judged this action dangerous (it gave no explanation)."
        );
        assert_eq!(ok, None, "an allowed call is no denial, whatever its permission decision");
    }

    #[test]
    fn only_a_named_denial_of_a_failed_result_counts() {
        let block = json!({"type":"tool_result","is_error":true,"tool_use_id":"t","content":"Permission denied"});
        // A failure whose words read like a denial is a failure.
        assert_eq!(from_record(&json!({"type":"user"}), &block), None);
        // A kind from a newer Claude Code is kept, not guessed.
        let other = from_record(&json!({"toolDenialKind":"hook-blocked"}), &block).expect("a denial");
        assert_eq!((other.kind, other.reason.as_str()), (DenialKind::Other, "Permission denied"));
        // A result that did not fail is not a denial.
        let fine = json!({"type":"tool_result","is_error":false,"content":"ok"});
        assert_eq!(from_record(&json!({"toolDenialKind":"user-rejected"}), &fine), None);
        // A reason is bounded, and an odd rule source is dropped.
        let long = json!({"type":"tool_result","is_error":true,"content":"x".repeat(5000)});
        let record = json!({"toolDenialKind":"permission-rule","permissionDecision":{"ruleSource":"a\nb"}});
        let denial = from_record(&record, &long).unwrap();
        assert_eq!((denial.reason.len(), denial.rule_source), (MAX_REASON, None));
        // A classifier that gives its reason: the words between "Reason: " and the advice.
        let said = json!({"type":"tool_result","is_error":true,
            "content":"Permission for this action was denied by the Claude Code auto mode classifier. Reason: Force-pushing rewrites shared history. If you have other tasks that don't depend on this action, continue."});
        let denial = from_record(&json!({"toolDenialKind":"automode-blocked"}), &said).unwrap();
        assert_eq!(denial.reason, "Force-pushing rewrites shared history.");
    }

    #[test]
    fn the_reader_puts_a_denial_on_its_tool_item_and_on_no_other() {
        let dir = crate::util::testing::TempDir::new("denial");
        let path = dir.0.join("session.jsonl");
        let mut lines = String::new();
        for (id, _) in recorded() {
            let call = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":"Bash",
                "input":{"command":format!("mkdir {id}")}}]}});
            lines.push_str(&format!("{call}\n"));
        }
        for (_, record) in recorded() {
            lines.push_str(&format!("{record}\n"));
        }
        std::fs::write(&path, lines).unwrap();
        let reader = Reader::open(&path, 4096).unwrap();
        let tools: Vec<_> = reader
            .items()
            .filter_map(|(_, item)| match &item.kind {
                ItemKind::Tool { id, state, denial, .. } => {
                    Some((id.as_str(), *state, denial.as_ref().map(|d| d.kind)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            tools,
            [
                ("toolu_user", ToolState::Failed, Some(DenialKind::User)),
                ("toolu_rule", ToolState::Failed, Some(DenialKind::Rule)),
                ("toolu_auto", ToolState::Failed, Some(DenialKind::AutoMode)),
                ("toolu_ok", ToolState::Ok, None),
            ]
        );
    }
}
