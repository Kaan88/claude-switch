//! Sequential snapshot for handovers. Shares the view's record, tool and task parsers,
//! without opening agent sidecars or the tool-result/picture pipeline.
use super::*;
use std::io::BufReader;

mod native_tasks;

pub(crate) enum Entry {
    Prompt(Option<String>, String),
    Assistant(String),
    Tool { id: String, kind: &'static str, path: Option<String>, command: Option<String> },
}

pub(crate) fn read(path: &Path, cancelled: impl Fn() -> bool, mut visit: impl FnMut(Entry)) -> io::Result<Vec<Task>> {
    if path.parent().and_then(Path::file_name).is_some_and(|n| n == "subagents") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Expected a main transcript"));
    }
    // Bound the snapshot to the initial file length; a simultaneous append is for the next handover.
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > 64 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Handover transcript exceeds 64 MiB"));
    }
    let mut reader = BufReader::new(file.take(size));
    let mut line = Vec::new();
    let mut tasks = Vec::new();
    let mut native = native_tasks::Tasks::default();
    let mut native_is_latest = false;
    loop {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Handover cancelled"));
        }
        let mut oversized = false;
        let (count, _) = bounded_line(&mut reader, &mut line, &mut oversized, MAX_LINE)?;
        if count == 0 {
            break;
        }
        if oversized {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "A transcript record exceeds 48 MiB"));
        }
        if let Some((kind, value)) = message_line(&line)
            && !flag(&value, "isSidechain")
            && !flag(&value, "isCompactSummary")
            && !flag(&value, "isApiErrorMessage")
            && value["parent_tool_use_id"].is_null()
        {
            let content = &value["message"]["content"];
            match kind {
                LineKind::User if value["message"]["role"].as_str().is_none_or(|r| r == "user") => {
                    if native.results(content, &value["toolUseResult"]) {
                        native_is_latest = true;
                    }
                    let parts = content.as_str().into_iter().chain(blocks(content).filter_map(|block| {
                        match block["type"].as_str()? {
                            "text" => block["text"].as_str(),
                            "image" => Some("[picture]"),
                            _ => None,
                        }
                    }));
                    let text = parts.filter_map(real_text).collect::<Vec<_>>().join("\n");
                    if !text.is_empty() {
                        visit(Entry::Prompt(string(&value["timestamp"]), text));
                    }
                }
                LineKind::Assistant if value["message"]["role"].as_str().is_none_or(|r| r == "assistant") => {
                    if let Some(text) = content.as_str() {
                        visit(Entry::Assistant(text.into()));
                    }
                    for block in blocks(content) {
                        if block["type"] == "text" {
                            if let Some(text) = block["text"].as_str() {
                                visit(Entry::Assistant(text.into()));
                            }
                        } else if let Some((id, name, input)) = tool_use(block) {
                            native.call(id, name, input);
                            if let Some(plan) = Plan::parse(name, input) {
                                native_is_latest = false;
                                match plan {
                                    Plan::Snapshot(snapshot) => tasks = snapshot,
                                    other => other.apply(&mut tasks),
                                }
                            }
                            let path = matches!(name, "Write" | "Edit" | "MultiEdit")
                                .then(|| string(&input["file_path"]).or_else(|| string(&input["path"])))
                                .flatten();
                            let command =
                                matches!(name, "Bash" | "PowerShell").then(|| string(&input["command"])).flatten();
                            let kind = match name {
                                "Write" => "Write",
                                "Edit" | "MultiEdit" => "Edit",
                                "Read" => "Read",
                                "Bash" | "PowerShell" => "Shell",
                                "Grep" | "Glob" => "Search",
                                "WebFetch" | "WebSearch" => "Web",
                                _ => "Other",
                            };
                            visit(Entry::Tool { id: id.into(), kind, path, command });
                        }
                    }
                }
                _ => {}
            }
        }
        line.clear();
    }
    Ok(if native_is_latest { native.finish() } else { tasks })
}

fn real_text(mut text: &str) -> Option<&str> {
    while text.trim_start().starts_with("<system-reminder") {
        let (_, next) = text.split_once("</system-reminder>")?;
        text = next.trim_start_matches(['\n', '\r']);
    }
    let trimmed = text.trim_start();
    // Do not drop ordinary prompts starting with "<" (e.g. code/XML).
    (!trimmed.is_empty()
        && ![
            "<local-command-stdout>",
            "<local-command-stderr>",
            "<local-command-caveat>",
            "<task-notification>",
            "<teammate-message",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix)))
    .then_some(text)
}
