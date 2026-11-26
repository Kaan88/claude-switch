//! Background Bash and MCP task records. Never infer a process, path, or completion from disk.
//!
//! An MCP tool call that runs long is moved to the background by Claude Code. The transcript
//! then holds, and only holds: the tool's result naming the task ("MCP tool "server/tool"
//! started server-side task JOB. It was moved to the background as task ID ..."), a
//! `<task-notification>` user record when it ends (status, a summary "MCP task JOB (server/tool)
//! completed." and the result), and a TaskStop result with `task_type: "mcp_task"` when it was
//! stopped. Progress the server reports while it runs is kept by the CLI in memory only.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

/// A background MCP task as the transcript records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTask {
    /// The MCP tool call that started it.
    pub tool: ItemId,
    /// Claude Code's task id (what TaskStop takes).
    pub id: String,
    /// The server's own task id.
    pub job: String,
    /// `server/tool`.
    pub name: String,
    pub status: McpStatus,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    /// The notification's summary and result, when it ended with one (at most 64 KiB).
    pub summary: Option<String>,
    pub result: Option<String>,
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
    /// MCP tool calls by tool use id: their row and start time.
    mcp_calls: BTreeMap<String, (ItemId, Option<i64>)>,
    /// MCP tasks started, by task id: the call's row, start time, server task id and name.
    mcp_starts: BTreeMap<String, (ItemId, Option<i64>, String, String)>,
    /// How each MCP task ended, by task id.
    mcp_ends: BTreeMap<String, McpEnd>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct McpEnd {
    at: ItemId,
    status: McpStatus,
    time: Option<i64>,
    summary: Option<String>,
    result: Option<String>,
}

/// The most of an MCP task's result kept in memory.
const MCP_RESULT_MAX: usize = 64 * 1024;

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
        if !["Command running in background", "Stopped task ", "<retrieval_status>", "<task_id>", "MCP tool \""]
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
        if name.starts_with("mcp__") {
            self.mcp_calls.insert(tool.into(), (item, timestamp.and_then(chat::clock_ms)));
        }
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
        if ok && let Some((name, job, id)) = mcp_start(&details.text) {
            let (item, start) = self
                .mcp_calls
                .get(tool)
                .copied()
                .unwrap_or((at, details.timestamp.as_deref().and_then(chat::clock_ms)));
            self.mcp_starts.insert(id.into(), (item, start, job.into(), name.into()));
        }
        // A stop of an MCP task: the structured result says so, or a stop named a known task.
        let stopped = if structured["task_type"] == "mcp_task" {
            structured["task_id"].as_str()
        } else {
            self.observers
                .get(tool)
                .filter(|(id, stop)| *stop && self.mcp_starts.contains_key(id))
                .map(|(id, _)| id.as_str())
        };
        if ok && let Some(id) = stopped.filter(|id| valid_id(id)).map(str::to_owned) {
            let time = details.timestamp.as_deref().and_then(chat::clock_ms);
            self.mcp_end(id, McpEnd { at, status: McpStatus::Stopped, time, summary: None, result: None });
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
            if let Some((id, end)) = mcp_notification(at, raw, &value["origin"], time) {
                self.mcp_end(id, end);
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

    fn mcp_end(&mut self, id: String, end: McpEnd) {
        // The latest record wins, but a stop does not hide the result an end brought.
        match self.mcp_ends.get(&id) {
            Some(old) if old.at > end.at => {}
            Some(old) if end.status == McpStatus::Stopped && old.result.is_some() => {}
            _ => {
                self.mcp_ends.insert(id, end);
            }
        }
    }

    /// The MCP tasks, running ones first, then newest first.
    pub(super) fn mcp_tasks(&self) -> Vec<McpTask> {
        let mut tasks: Vec<McpTask> = self
            .mcp_starts
            .iter()
            .map(|(id, (tool, start_ms, job, name))| {
                let end = self.mcp_ends.get(id).filter(|end| end.at > *tool);
                McpTask {
                    tool: *tool,
                    id: id.clone(),
                    job: job.clone(),
                    name: name.clone(),
                    status: end.map_or(McpStatus::Running, |e| e.status),
                    start_ms: *start_ms,
                    end_ms: end.and_then(|e| e.time),
                    summary: end.and_then(|e| e.summary.clone()),
                    result: end.and_then(|e| e.result.clone()),
                }
            })
            .collect();
        tasks.sort_unstable_by_key(|t| (t.status != McpStatus::Running, std::cmp::Reverse(t.tool)));
        tasks
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

/// `MCP tool "server/tool" started server-side task JOB. It was moved to the background as task
/// ID ...`: the name, the server's job and Claude Code's task id.
fn mcp_start(text: &str) -> Option<(&str, &str, &str)> {
    let rest = text.trim_start().strip_prefix("MCP tool \"")?;
    let (name, rest) = rest.split_once("\" started server-side task ")?;
    let (job, rest) = rest.split_once(". It was moved to the background as task ")?;
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(rest.len());
    let id = &rest[..end];
    let plain = |s: &str| (1..=200).contains(&s.len()) && !s.chars().any(|c| c.is_control() || c == '"');
    (plain(name) && plain(job) && !job.contains(' ') && valid_id(id)).then_some((name, job, id))
}

/// A `<task-notification>` of an MCP task: by its origin (`taskKind: mcp-task`) or its summary
/// ("MCP task JOB (server/tool) completed.").
fn mcp_notification(at: ItemId, raw: &str, origin: &Value, time: Option<&str>) -> Option<(String, McpEnd)> {
    let id = field(raw, "task-id").filter(|id| valid_id(id))?;
    let summary = field(raw, "summary");
    if origin["taskKind"] != "mcp-task" && !summary.is_some_and(|s| s.starts_with("MCP task ")) {
        return None;
    }
    let status = match field(raw, "status")? {
        "completed" | "finished" => McpStatus::Completed,
        "failed" | "error" => McpStatus::Failed,
        "stopped" | "killed" | "cancelled" | "canceled" => McpStatus::Stopped,
        _ => return None,
    };
    // The result is the notification's last part and may itself hold any text.
    let result = raw.split_once("<result>").map(|(_, rest)| {
        let rest = rest.rsplit_once("</result>").map_or(rest, |(inner, _)| inner).trim();
        rest[..rest.floor_char_boundary(MCP_RESULT_MAX)].to_owned()
    });
    let summary = summary.map(|s| s.chars().take(400).collect());
    Some((id.into(), McpEnd { at, status, time: time.and_then(chat::clock_ms), summary, result }))
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

    #[test]
    fn mcp_tasks_start_from_the_calls_result_and_end_by_notification_or_stop() {
        let dir = TempDir::new("background-mcp");
        let path = dir.0.join("session.jsonl");
        let call = |id: &str, at: &str| {
            json!({
                "type":"assistant","timestamp":at,
                "message":{"content":[{"id":id,"type":"tool_use","name":"mcp__fixture__long_job","input":{"prompt":"x"}}]}
            })
        };
        let started = |id: &str, job: &str, task: &str, at: &str| {
            json!({
                "type":"user","timestamp":at,
                "message":{"content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":
                    format!("MCP tool \"fixture/long_job\" started server-side task {job}. It was moved to the background as task {task} and keeps running on the server; you'll receive a notification with the result when it completes, and you can keep working in the meantime. To stop it, use TaskStop with task_id \"{task}\".")
                }]}]}
            })
        };
        let entries = [
            call("one", "2026-11-26T10:00:00Z"),
            started("one", "job_1", "kfixture1", "2026-11-26T10:02:00Z"),
            call("two", "2026-11-26T10:05:00Z"),
            started("two", "job_2", "kfixture2", "2026-11-26T10:07:00Z"),
            call("three", "2026-11-26T10:08:00Z"),
            started("three", "job 3", "kfixture3", "2026-11-26T10:09:00Z"),
        ];
        put(&path, &format!("{}\n", entries.map(|v| v.to_string()).join("\n")));
        let mut reader = Reader::open(&path, 1024 * 1024).unwrap();
        let tasks = reader.mcp_tasks();
        assert_eq!(tasks.len(), 2, "a job id with a space is not the CLI's line");
        assert!(tasks.iter().all(|t| t.status == McpStatus::Running && t.name == "fixture/long_job"));
        assert_eq!(tasks[0].id, "kfixture2", "newest first");
        assert_eq!((tasks[1].job.as_str(), tasks[1].start_ms.is_some()), ("job_1", true));
        assert!(matches!(reader.item(tasks[1].tool).unwrap().kind, ItemKind::Tool { .. }));

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let notice = json!({
            "type":"user","timestamp":"2026-11-26T10:12:00Z",
            "origin":{"kind":"task-notification","producer":"session-task","taskKind":"mcp-task"},
            "message":{"role":"user","content":"<task-notification>\n<task-id>kfixture1</task-id>\n<status>completed</status>\n<summary>MCP task job_1 (fixture/long_job) completed.</summary>\n<result>\nAll **done**; see </result> above.\n</result>\n</task-notification>"}
        });
        let stop = json!({
            "type":"assistant","timestamp":"2026-11-26T10:13:00Z",
            "message":{"content":[{"id":"stop","type":"tool_use","name":"TaskStop","input":{"task_id":"kfixture2"}}]}
        });
        let stopped = json!({
            "type":"user","timestamp":"2026-11-26T10:13:01Z",
            "message":{"content":[{"type":"tool_result","tool_use_id":"stop","content":"{\"message\":\"Task kfixture2 is stopped.\"}"}]},
            "toolUseResult":{"message":"Task kfixture2 is stopped.","task_id":"kfixture2","task_type":"mcp_task","command":"fixture/long_job"}
        });
        for record in [notice, stop, stopped] {
            writeln!(file, "{record}").unwrap();
        }
        let changes = reader.poll().unwrap();
        assert!(changes.background_changed);
        assert!(reader.items().all(|(_, item)| !matches!(item.kind, ItemKind::Prompt { .. })), "a notice is no prompt");
        let tasks = reader.mcp_tasks();
        let done = tasks.iter().find(|t| t.id == "kfixture1").unwrap();
        assert_eq!(done.status, McpStatus::Completed);
        assert_eq!(done.end_ms.unwrap() - done.start_ms.unwrap(), 12 * 60_000, "from the call to the notice");
        assert_eq!(done.result.as_deref(), Some("All **done**; see </result> above."));
        assert_eq!(done.summary.as_deref(), Some("MCP task job_1 (fixture/long_job) completed."));
        let halted = tasks.iter().find(|t| t.id == "kfixture2").unwrap();
        assert_eq!((halted.status, halted.result.as_deref()), (McpStatus::Stopped, None));
        assert!(reader.background_shells().is_empty(), "MCP tasks are not shells");

        writeln!(file, "{}", json!({
            "type":"user","timestamp":"2026-11-26T10:20:00Z",
            "message":{"content":"<task-notification><task-id>kfixture2</task-id><status>failed</status><summary>MCP task job_2 (fixture/long_job) failed.</summary><result>boom</result></task-notification>"}
        })).unwrap();
        reader.poll().unwrap();
        let failed = reader.mcp_tasks().into_iter().find(|t| t.id == "kfixture2").unwrap();
        assert_eq!((failed.status, failed.result.as_deref()), (McpStatus::Failed, Some("boom")));
    }
}
