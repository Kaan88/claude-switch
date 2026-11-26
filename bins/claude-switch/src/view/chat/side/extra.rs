use std::time::{SystemTime, UNIX_EPOCH};

use claude_switch_core::transcript::{BackgroundShell, ItemId, ShellStatus};

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
        self.reveal = Some(key);
    }

    pub(super) fn extra_action(&self, model: &Model, side: Side, index: usize, part: Part) -> Option<Action> {
        if side == Side::Outputs {
            return (part == Part::Open)
                .then_some(())
                .and_then(|_| Some(Action::Output(model.output_list.get(index)?.to.clone())));
        }
        let shell = model.shells.get(index)?;
        let key = Key::Shell(shell.tool);
        if part == Part::Expand {
            return Some(Action::Expand(key));
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

    pub(super) fn extra_lines(&self, out: &mut Lines, f: &Frame<'_>) {
        if f.side == Side::Outputs {
            for (i, output) in f.model.output_list.iter().enumerate() {
                let meta = if output.time.is_empty() {
                    output.kind().into()
                } else {
                    format!("{} · {}", output.kind(), output.time)
                };
                out.push(37.0, Line::Output(i, output.title.clone(), meta));
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
            out.push(ROW, Line::Header { label: command, index, expanded, active: running, file: false, opens: false });
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
