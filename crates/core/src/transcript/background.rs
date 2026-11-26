//! Background Bash records. Never infer a process, path, or completion from disk.
use std::collections::BTreeMap;

use serde_json::Value;

use super::{ItemId, chat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellStatus {
    Running,
    Finished(Option<i32>),
    Killed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundShell {
    pub tool: ItemId,
    pub id: String,
    pub command: String,
    pub output: String,
    pub status: ShellStatus,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultDetails {
    pub text: String,
    pub timestamp: Option<String>,
}

impl ResultDetails {
    /// Explicit saved-output labels, not paths scraped from surrounding prose.
    pub fn saved_paths(&self) -> impl Iterator<Item = &str> {
        self.text.lines().filter_map(saved_path).take(16)
    }
}

fn saved_path(line: &str) -> Option<&str> {
    let line = line.trim();
    let path = ["Saved to:", "Saved file:", "Image saved to:", "Wrote file:"]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))?
        .trim();
    let path = ['`', '"'].into_iter().find_map(|quote| path.strip_prefix(quote)?.strip_suffix(quote)).unwrap_or(path);
    (!path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control)).then_some(path)
}

#[derive(Default)]
pub(super) struct Background {
    launches: BTreeMap<String, (ItemId, String, Option<i64>)>,
    starts: BTreeMap<String, Start>,
    status: BTreeMap<String, (ItemId, ShellStatus, Option<i64>)>,
    observers: BTreeMap<String, (String, bool)>,
    observations: BTreeMap<String, (ItemId, ResultDetails, bool)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Start {
    id: String,
    output: String,
}

pub(super) fn bounded_text(content: &Value) -> String {
    const MAX: usize = 32 * 1024;
    let mut text = String::new();
    for raw in chat::result_text(content).take(32) {
        let cap = MAX.saturating_sub(text.len());
        if cap <= 1 {
            break;
        }
        let raw = &raw[..raw.floor_char_boundary(raw.len().min(cap - 1))];
        let start = raw.trim_start();
        if !["Command running in background", "Stopped task ", "<retrieval_status>", "<task_id>"]
            .iter()
            .any(|prefix| start.starts_with(prefix))
            && ![b"https://".as_slice(), b"http://"]
                .into_iter()
                .any(|scheme| raw.as_bytes().windows(scheme.len()).any(|window| window.eq_ignore_ascii_case(scheme)))
            && !raw.lines().any(|line| saved_path(line).is_some())
        {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(raw);
    }
    text
}

impl Background {
    pub(super) fn tool(&mut self, tool: &str, item: ItemId, name: &str, input: &Value, timestamp: Option<&str>) {
        if name == "Bash"
            && input["run_in_background"].as_bool() == Some(true)
            && let Some(command) = input["command"].as_str()
        {
            self.launches
                .insert(tool.into(), (item, command.chars().take(240).collect(), timestamp.and_then(chat::clock_ms)));
        }
        if matches!(name, "TaskOutput" | "TaskStop" | "KillShell" | "BashOutput")
            && let Some(id) = input["task_id"].as_str().or_else(|| input["shell_id"].as_str()).filter(|id| valid_id(id))
        {
            self.observers.insert(tool.into(), (id.into(), matches!(name, "TaskStop" | "KillShell")));
            self.observe_result(tool);
        }
    }

    pub(super) fn result(&mut self, tool: &str, at: ItemId, details: &ResultDetails, structured: &Value, ok: bool) {
        // A text match is anchored to the Bash tool's own startup result, never assistant prose.
        if ok && let Some(start) = start(&details.text, structured) {
            self.starts.insert(tool.into(), start);
        }
        if self.observers.contains_key(tool)
            || details.text.starts_with("Stopped task ")
            || ["<retrieval_status>", "<task_id>"].iter().any(|p| details.text.trim_start().starts_with(p))
        {
            self.observations.insert(tool.into(), (at, details.clone(), ok));
            self.observe_result(tool);
        }
    }

    fn observe_result(&mut self, tool: &str) {
        let Some((id, stop)) = self.observers.get(tool) else { return };
        let Some((item, details, true)) = self.observations.get(tool) else { return };
        let next = if *stop && stopped_task(&details.text, id) {
            Some(ShellStatus::Killed)
        } else {
            status_xml(&details.text).filter(|(named, _)| named == id).map(|(_, status)| status)
        };
        if let Some(next) = next {
            let (item, id, time) = (*item, id.clone(), details.timestamp.clone());
            self.status(item, &id, next, time.as_deref());
        }
    }

    pub(super) fn status(&mut self, at: ItemId, id: &str, status: ShellStatus, time: Option<&str>) {
        if !valid_id(id) || status == ShellStatus::Running {
            return;
        }
        let time = time.and_then(chat::clock_ms);
        if let Some((old, previous, previous_time)) = self.status.get_mut(id) {
            if let (ShellStatus::Finished(code), ShellStatus::Finished(next)) = (&mut *previous, status) {
                if code.is_none() || at >= *old && next.is_some() {
                    *code = next;
                }
                if at < *old || previous_time.is_none() {
                    *previous_time = time.or(*previous_time);
                }
                *old = (*old).max(at);
                return;
            }
            if *old > at {
                return;
            }
        }
        self.status.insert(id.into(), (at, status, time));
    }

    pub(super) fn notification(&mut self, at: ItemId, value: &Value) -> bool {
        let time = value["timestamp"].as_str();
        let content = &value["message"]["content"];
        let raw = content.as_str().or_else(|| {
            let [block] = content.as_array()?.as_slice() else { return None };
            (block["type"] == "text").then(|| block["text"].as_str()).flatten()
        });
        if let Some(raw) = raw.map(str::trim).filter(|s| s.starts_with("<task-notification>")) {
            if let Some((id, status)) = status_xml(raw) {
                self.status(at, id, status, time);
            }
            return true;
        }
        if value["type"] == "system" && value["subtype"] == "task_notification" {
            let task = value["task_id"].as_str().or_else(|| value["taskId"].as_str());
            if let Some((id, status)) = task.zip(
                value["status"]
                    .as_str()
                    .and_then(|s| status(s, value["exit_code"].as_i64().and_then(|c| i32::try_from(c).ok()))),
            ) {
                self.status(at, id, status, time);
            }
            return true;
        }
        false
    }

    pub(super) fn shells(&self) -> Vec<BackgroundShell> {
        let mut unique = BTreeMap::<&str, BackgroundShell>::new();
        for (tool, (item, command, start_ms)) in &self.launches {
            let Some(start) = self.starts.get(tool) else { continue };
            let (status, end_ms) = self
                .status
                .get(&start.id)
                .filter(|(at, _, _)| at > item)
                .map_or((ShellStatus::Running, None), |(_, status, time)| (*status, *time));
            if unique.get(start.id.as_str()).is_none_or(|old| old.tool < *item) {
                unique.insert(
                    &start.id,
                    BackgroundShell {
                        tool: *item,
                        id: start.id.clone(),
                        command: command.clone(),
                        output: start.output.clone(),
                        status,
                        start_ms: *start_ms,
                        end_ms,
                    },
                );
            }
        }
        let mut shells = unique.into_values().collect::<Vec<_>>();
        shells.sort_unstable_by_key(|s| (s.status != ShellStatus::Running, std::cmp::Reverse(s.tool)));
        shells
    }
}

fn valid_id(id: &str) -> bool {
    (1..=96).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn start(text: &str, structured: &Value) -> Option<Start> {
    let pair = structured["backgroundTaskId"]
        .as_str()
        .zip(structured["outputFile"].as_str().or_else(|| structured["outputPath"].as_str()));
    let (id, output) = pair.or_else(|| {
        let rest = text.trim_start().strip_prefix("Command running in background with ID: ")?;
        let (id, output) = rest.split_once(". Output is being written to: ")?;
        let output = output.lines().next()?;
        let output = output.split_once(". You will be notified when it completes").map_or(output, |(p, _)| p);
        Some((id, output))
    })?;
    let output = output.trim().strip_prefix('`').and_then(|p| p.strip_suffix('`')).unwrap_or(output.trim());
    (valid_id(id) && (1..=4096).contains(&output.len()) && !output.chars().any(char::is_control))
        .then(|| Start { id: id.into(), output: output.into() })
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(&format!("<{key}>"))?;
    Some(rest.split_once(&format!("</{key}>"))?.0.trim())
}

fn status_xml(text: &str) -> Option<(&str, ShellStatus)> {
    let text = text.trim();
    if !["<task-notification>", "<retrieval_status>", "<task_id>"].iter().any(|p| text.starts_with(p)) {
        return None;
    }
    let id = field(text, "task-id").or_else(|| field(text, "task_id"))?;
    let code =
        field(text, "exit-code").or_else(|| field(text, "exit_code")).and_then(|s| s.parse().ok()).or_else(|| {
            let summary = field(text, "summary")?;
            let (_, end) = summary.rsplit_once("(exit code ")?;
            end.strip_suffix(')')?.parse().ok()
        });
    Some((id, status(field(text, "status")?, code)?))
}

fn status(raw: &str, code: Option<i32>) -> Option<ShellStatus> {
    match raw {
        "completed" | "failed" | "finished" => Some(ShellStatus::Finished(code)),
        "stopped" | "killed" | "cancelled" | "canceled" => Some(ShellStatus::Killed),
        _ => None,
    }
}

fn stopped_task(text: &str, id: &str) -> bool {
    text.strip_prefix("Stopped task ")
        .is_some_and(|s| s == id || s.strip_prefix(id).is_some_and(|suffix| suffix.starts_with([' ', '.', ':'])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{ItemKind, Reader};
    use crate::util::testing::{TempDir, put};
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn retains_plain_http_tool_results_for_the_outputs_projection() {
        let text = "Report at HTTP://example.test/report";
        assert_eq!(bounded_text(&json!([{"type":"text","text":text}])), text);
    }

    #[test]
    fn pairs_exact_background_bash_results_and_tracks_later_lifecycle_records() {
        let dir = TempDir::new("background-shells");
        let path = dir.0.join("session.jsonl");
        let tool = |id: &str, name, run| {
            json!({
                "type":"assistant","timestamp":"2026-11-26T03:45:01Z",
                "message":{"content":[{"id":id,"type":"tool_use","name":name,
                    "input":{"command":"test fixture --watch","run_in_background":run}
                }]}
            })
        };
        let result = |id: &str, task| {
            json!({
                "type":"user","timestamp":"2026-11-26T03:45:02Z",
                "message":{"content":[{"type":"tool_result","tool_use_id":id,"content":
                    format!("Command running in background with ID: {task}. Output is being written to: C:\\tmp\\{task}.output")
                }]}
            })
        };
        let entries = [
            tool("good", "Bash", true),
            result("good", "b1"),
            tool("unflagged", "Bash", false),
            result("unflagged", "b2"),
            tool("other", "PowerShell", true),
            result("other", "b3"),
            tool("started", "Bash", true),
        ];
        put(&path, &format!("{}\n", entries.map(|v| v.to_string()).join("\n")));
        let mut reader = Reader::open(&path, 1024 * 1024).unwrap();
        let [shell] = reader.background_shells().try_into().unwrap();
        assert_eq!(shell.status, ShellStatus::Running);
        assert_eq!(shell.id, "b1");
        assert_eq!(shell.output, r"C:\tmp\b1.output");
        assert!(matches!(reader.item(shell.tool).unwrap().kind, ItemKind::Tool { .. }));
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        let append = |file: &mut std::fs::File, id, status, code| {
            writeln!(file, "{}", json!({"type":"user","isMeta":true,"timestamp":"2026-11-26T03:45:46Z",
                "message":{"content":format!("<task-notification><task-id>{id}</task-id><status>{status}</status><summary>Command ended (exit code {code})</summary></task-notification>")}
            })).unwrap();
        };
        append(&mut file, "b2", "killed", 130);
        append(&mut file, "b1", "failed", 7);
        assert!(reader.poll().unwrap().background_changed);
        let [done] = reader.background_shells().try_into().unwrap();
        assert_eq!(done.status, ShellStatus::Finished(Some(7)));
        assert_eq!(done.end_ms.unwrap() - done.start_ms.unwrap(), 45_000);
        assert!(reader.items().all(|(_, item)| !matches!(item.kind, ItemKind::Prompt { .. })));
        append(&mut file, "b1", "killed", 130);
        reader.poll().unwrap();
        assert_eq!(reader.background_shells()[0].status, ShellStatus::Killed);
    }
}
