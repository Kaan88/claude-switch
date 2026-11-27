use std::time::{SystemTime, UNIX_EPOCH};

use claude_switch_core::transcript::{BackgroundShell, ItemId, McpStatus, McpTask, ShellStatus};

use mdshared_ui::gfx::Painter;

use super::paint::{Frame, Line, Lines, ROW};
use super::*;
use crate::view::outputs::Destination;

impl Details {
    pub(in crate::view) fn select_shell(&mut self, id: ItemId, generation: u64) {
        if self.generation != Some(generation) {
            *self = Self { generation: Some(generation), ..Self::default() };
        }
        let key = Key::Shell(id);
        self.expanded.insert(key.clone());
        // Its section of Chat's panel shows it.
        self.folded.remove(&Side::Shells);
        self.reveal = Some(key);
    }

    pub(super) fn extra_action(&self, model: &Model, side: Side, index: usize, part: Part) -> Option<Action> {
        if side == Side::Outputs {
            return (part == Part::Open)
                .then_some(())
                .and_then(|_| Some(Action::Output(model.output_list.get(index)?.to.clone())));
        }
        if let Some(i) = index.checked_sub(model.shells.len()) {
            return self.mcp_action(model.mcp_tasks.get(i)?, part);
        }
        let shell = model.shells.get(index)?;
        let key = Key::Shell(shell.tool);
        match part {
            Part::Expand => return Some(Action::Expand(key)),
            Part::Open => return model.shell_log(shell).is_some().then_some(Action::ShellFull(shell.tool)),
            _ => {}
        }
        if !self.expanded.contains(&key) {
            return None;
        }
        match part {
            Part::Full if model.shell_log(shell).is_some() => Some(Action::ShellFull(shell.tool)),
            Part::Follow if shell.status == ShellStatus::Running => Some(Action::ShellFollow(shell.tool)),
            Part::Stop if shell.status == ShellStatus::Running => Some(Action::ShellStop(shell.tool)),
            _ => None,
        }
    }

    fn mcp_action(&self, task: &McpTask, part: Part) -> Option<Action> {
        let key = Key::Mcp(task.id.clone());
        let result = task.result.is_some();
        match part {
            Part::Expand => Some(Action::Expand(key)),
            // The row opens the result when there is one, else the call that started the task.
            Part::Open if result => Some(Action::McpResult(task.id.clone())),
            Part::Open => Some(Action::Jump(task.tool)),
            _ if !self.expanded.contains(&key) => None,
            Part::Full if result => Some(Action::McpResult(task.id.clone())),
            Part::Call => Some(Action::Jump(task.tool)),
            _ => None,
        }
    }

    pub(super) fn extra_lines(&self, out: &mut Lines, p: &mut Painter, f: &Frame<'_>, width: f32, side: Side) {
        if side == Side::Outputs {
            for (i, output) in f.model.output_list.iter().enumerate() {
                out.push(37.0, Line::Output(i, output.title.clone(), output.meta()));
            }
            return;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|n| i64::try_from(n.as_millis()).ok());
        let mut finished = false;
        for (index, shell) in f.model.shells.iter().enumerate() {
            let running = shell.status == ShellStatus::Running;
            if !running && !finished {
                out.push(22.0, Line::Heading("Finished"));
                finished = true;
            }
            let key = Key::Shell(shell.tool);
            let expanded = self.expanded.contains(&key);
            let command = shell.command.split_whitespace().collect::<Vec<_>>().join(" ");
            let log = f.model.shell_log(shell).is_some();
            out.push(ROW, Line::Header { label: command, index, expanded, active: running, split: log, opens: log });
            out.push(17.0, Line::ShellMeta(format!("{} · {}", shell.id, status(shell, now))));
            if !expanded {
                continue;
            }
            let top = out.height - 17.0;
            let mut actions = Vec::new();
            if f.model.shell_log(shell).is_some() {
                actions.push(("Full output", Part::Full));
            }
            if running {
                let (follow, snapshot) = self
                    .frozen
                    .get(&shell.tool)
                    .map_or_else(|| (true, f.model.shell_tails.get(&shell.tool)), |frozen| (false, Some(frozen)));
                let lines = match snapshot {
                    None => vec!["Reading named output…".into()],
                    Some(None) => vec!["Output file unavailable.".into()],
                    Some(Some(lines)) if lines.is_empty() => vec!["No output yet.".into()],
                    Some(Some(lines)) => lines.clone(),
                };
                // Fixed height keeps the buttons from moving as new lines arrive.
                out.push(8.0 * 15.0 + 8.0, Line::Tail(shell.tool, lines));
                actions.push((if follow { "Following" } else { "Follow" }, Part::Follow));
                actions.push(("Stop…", Part::Stop));
            }
            if !actions.is_empty() {
                out.push(ROW, Line::Actions(index, actions));
            }
            self.reveal(out, &key, top);
        }
        self.mcp_lines(out, p, f, width, now);
    }

    fn mcp_lines(&self, out: &mut Lines, p: &mut Painter, f: &Frame<'_>, width: f32, now: Option<i64>) {
        if f.model.mcp_tasks.is_empty() {
            return;
        }
        out.push(22.0, Line::Heading("MCP tasks"));
        for (i, task) in f.model.mcp_tasks.iter().enumerate() {
            let index = f.model.shells.len() + i;
            let key = Key::Mcp(task.id.clone());
            let expanded = self.expanded.contains(&key);
            let running = task.status == McpStatus::Running;
            out.push(
                ROW,
                Line::Header { label: task.name.clone(), index, expanded, active: running, split: true, opens: true },
            );
            out.push(17.0, Line::ShellMeta(format!("{} · {}", task.id, mcp_status(task, now))));
            if !expanded {
                continue;
            }
            let top = out.height - 17.0;
            let mut text = format!("Server task {}", task.job);
            if running {
                text.push_str("\nThe last step is not on disk: Claude Code keeps an MCP task's progress in memory.");
            } else if let Some(summary) = &task.summary {
                text.push('\n');
                text.push_str(summary);
            } else if task.status == McpStatus::Stopped {
                text.push_str("\nStopped before it ended; there is no result.");
            }
            if let Some(result) = &task.result {
                let head: Vec<&str> = result.lines().filter(|l| !l.trim().is_empty()).take(4).collect();
                if !head.is_empty() {
                    text.push_str("\n\n");
                    text.push_str(&head.join("\n"));
                }
            }
            out.detail(p, width, text);
            let mut actions = Vec::new();
            if task.result.is_some() {
                actions.push(("Full result", Part::Full));
            }
            actions.push(("Show call", Part::Call));
            out.push(ROW, Line::Actions(index, actions));
            self.reveal(out, &key, top);
        }
    }
}

/// Shows an MCP task's result, as the transcript holds it, in the viewer.
pub(super) fn open_result(pane: &mut Pane, id: &str) -> bool {
    let Some(task) = pane.model.mcp_tasks.iter().find(|t| t.id == id) else { return false };
    let Some(result) = task.result.clone() else { return false };
    let status = match task.status {
        McpStatus::Failed => "failed",
        McpStatus::Stopped => "stopped",
        _ => "completed",
    };
    let title = format!("{} · result", task.name);
    let subtitle = format!("MCP task {} · {status} · from the transcript", task.id);
    pane.ui.open_text(title, subtitle, result);
    true
}

pub(super) fn mcp_status(task: &McpTask, now: Option<i64>) -> String {
    let status = match task.status {
        McpStatus::Running => "Running",
        McpStatus::Completed => "Completed",
        McpStatus::Failed => "Failed",
        McpStatus::Stopped => "Stopped",
    };
    let end = if task.status == McpStatus::Running { now } else { task.end_ms };
    match task.start_ms.zip(end).and_then(|(a, b)| b.checked_sub(a)).filter(|&v| v >= 0) {
        Some(ms) => format!("{status} · {}", mdshared_app::fmt::duration((ms / 1000) as u64)),
        None => status.into(),
    }
}

pub(super) fn open_full(pane: &mut Pane, id: ItemId) -> bool {
    let Some(file) = pane.model.shells.iter().find(|s| s.tool == id).and_then(|s| pane.model.shell_log(s)) else {
        return false;
    };
    pane.ui.open_output(Destination::File(file), &mut pane.model);
    true
}

fn status(shell: &BackgroundShell, now: Option<i64>) -> String {
    let status = match shell.status {
        ShellStatus::Running => "Running".into(),
        ShellStatus::Finished(Some(code)) => format!("Finished · exit {code}"),
        ShellStatus::Finished(None) => "Finished".into(),
        ShellStatus::Killed => "Killed".into(),
    };
    let end = if shell.status == ShellStatus::Running { now } else { shell.end_ms };
    match shell.start_ms.zip(end).and_then(|(a, b)| b.checked_sub(a)).filter(|&v| v >= 0) {
        Some(ms) => format!("{status} · {}", mdshared_app::fmt::duration((ms / 1000) as u64)),
        None => status,
    }
}
