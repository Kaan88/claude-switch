use std::time::Instant;

use claude_switch_core::transcript::ChatInfo;
use mdshared_term::Screen;

use super::{Chat, Drawer, clean_prompt};
use crate::view::chat::QUIET;

#[derive(Default)]
pub(in crate::view::chat) struct CliReport {
    pub model: Option<String>,
    pub effort: Option<&'static str>,
    pub(super) baseline: Vec<(u64, String)>,
    pub(super) epoch: u64,
    pub(super) tracking: bool,
    transcript: Option<(u64, u64)>,
    floor: Option<u64>,
    pub(super) expects: Option<Choice>,
    confirmed: bool,
    pub(super) touched_terminal: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Model,
    Effort,
}

impl CliReport {
    pub(super) fn begin(
        &mut self,
        baseline: Vec<(u64, String)>,
        epoch: u64,
        demo: bool,
        floor: Option<u64>,
        line: &str,
    ) {
        self.baseline = baseline;
        self.epoch = epoch;
        self.tracking = !demo;
        self.floor = floor;
        self.expects = match line.split_once(' ') {
            Some(("/model", arg)) if !arg.trim().is_empty() => Some(Choice::Model),
            Some(("/effort", arg)) if !arg.trim().is_empty() => Some(Choice::Effort),
            _ => None,
        };
        self.confirmed = false;
        self.touched_terminal = false;
    }
}

// Read only local-command output rows, not arbitrary Markdown, the prompt itself or scrollback.
pub(super) fn local_reports(screen: &Screen) -> Vec<(u64, String)> {
    let row = screen.cursor().row;
    (row.saturating_sub(12)..(row + 3).min(screen.rows()))
        .filter_map(|i| {
            let line = screen.row_text(i);
            let text = line.trim_start().strip_prefix('⎿')?.trim();
            Some((screen.screen_start() + i as u64, text.to_owned()))
        })
        .collect()
}

pub(super) fn reported_effort(line: &str) -> Option<&'static str> {
    let value = line.strip_prefix("Set effort level to ").or_else(|| line.strip_prefix("Effort level: "))?;
    let word = value.split(|c: char| !c.is_ascii_alphabetic()).next()?;
    crate::view::chat::commands::EFFORT.into_iter().chain(["auto"]).find(|s| word.eq_ignore_ascii_case(s))
}

pub(super) fn reported_model(line: &str) -> Option<String> {
    let value = line.strip_prefix("Set model to ")?;
    let value = value.split_once(" (").map_or(value, |(name, _)| name).trim();
    (!value.is_empty()
        && value.len() <= 70
        && value.chars().all(|c| c.is_ascii_alphanumeric() || " .-_:[]".contains(c)))
    .then(|| value.to_owned())
}

impl Chat {
    pub(in crate::view::chat) fn observe_cli(&mut self, info: &ChatInfo, screen: &Screen, now: Instant) -> bool {
        let report = &mut self.cli_report;
        let mut changed = false;
        let usage = info.usage_offset().map(|at| (info.generation, at));
        if let Some((_, offset)) = usage.filter(|u| Some(*u) != report.transcript) {
            report.transcript = usage;
            if report.floor.is_none_or(|floor| offset >= floor) {
                changed |= report.model.take().is_some();
                report.floor = None;
            }
        }
        if report.tracking && self.output_epoch != report.epoch {
            let mut newest_model = None;
            let mut newest_effort = None;
            for (_, line) in local_reports(screen).into_iter().filter(|s| !report.baseline.contains(s)) {
                if let Some(model) = reported_model(&line) {
                    newest_model = Some(model);
                    newest_effort = None;
                }
                newest_effort = reported_effort(&line).or(newest_effort);
            }
            if let Some(model) = newest_model {
                if report.model.as_ref() != Some(&model) {
                    changed = true;
                    report.model = Some(model);
                    changed |= report.effort.take().is_some();
                }
                report.confirmed |= report.expects == Some(Choice::Model);
            }
            if let Some(effort) = newest_effort {
                changed |= report.effort != Some(effort);
                report.effort = Some(effort);
                report.confirmed |= report.expects == Some(Choice::Effort);
            }
            if !self.identity_unverified
                && clean_prompt(screen)
                && self.last_output.is_some_and(|at| now.saturating_duration_since(at) >= QUIET)
            {
                report.tracking = false;
                self.slash = false;
                self.suppressed = None;
                self.drawer_focus &= report.touched_terminal;
                if let Some(choice) = report.expects.take().filter(|_| !report.confirmed) {
                    self.drawer = Drawer::Manual;
                    self.error = Some(match choice {
                        Choice::Model => "Claude Code did not report a model change. Check the terminal.",
                        Choice::Effort => "Claude Code did not report an effort change. Check the terminal.",
                    });
                    self.retry = None;
                }
                changed = true;
            }
        }
        changed
    }
}
