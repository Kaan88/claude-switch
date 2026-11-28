//! Denied permissions, said out loud: the tool calls of the current turn that Claude Code refused
//! to run, with who refused and why, and the least Chat can do about each kind.
//!
//! "Current" is the turn of the newest prompt the user typed, in the main conversation: every
//! denied call after that prompt which no later call of the same tool with the same target has
//! run successfully since. The next prompt starts over. A subagent's denials are in its own
//! transcript and are not counted here.
//!
//! The card never approves anything by itself. For a call auto mode blocked it opens Claude
//! Code's own list (`/permissions`, through the slash-command path and its checks), which then
//! shows as the list's card (`blocked`). "Ask Claude to retry" only puts a sentence in the
//! message box. A rule's denial names where the rule comes from; the user's own No needs nothing.
use claude_switch_core::transcript::{Denial, DenialKind};

use super::*;

const REVIEW: &str = "/permissions";
/// The most denied calls the open card lists; the count names them all.
const LISTED: usize = 4;
const MAX_TARGET: usize = 120;
/// The most calls the prepared sentence names.
const ASKED: usize = 5;

/// One denied call of the current turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Denied {
    pub item: ItemId,
    pub tool: String,
    /// The command, file or other target, on one line.
    pub target: String,
    pub denial: Denial,
}

/// What the tool row says instead of "failed".
pub(in crate::view) fn label(kind: DenialKind) -> &'static str {
    match kind {
        DenialKind::User => "✗ you said no",
        DenialKind::Rule => "✗ denied by a rule",
        DenialKind::AutoMode => "✗ blocked by auto mode",
        DenialKind::Other => "✗ denied",
    }
}

/// Where a deciding rule comes from, in plain words. Only `flagSettings` was recorded (Claude Code
/// 2.1.332); the others are Claude Code's documented names for its settings files, and a name not
/// listed here is shown as it is.
fn rule_source(source: &str) -> String {
    match source {
        "flagSettings" => "set with --settings when Claude Code was started".to_owned(),
        "userSettings" => "in your user settings".to_owned(),
        "projectSettings" => "in the project's .claude/settings.json".to_owned(),
        "localSettings" => "in the project's .claude/settings.local.json".to_owned(),
        "policySettings" => "set by managed policy".to_owned(),
        other => format!("from {other}"),
    }
}

/// Who refused and why, and what that means for approving it.
pub(in crate::view) fn why(denial: &Denial) -> String {
    match denial.kind {
        DenialKind::User => "You said no. Nothing needs approving.".to_owned(),
        DenialKind::Rule => match denial.rule_source.as_deref() {
            Some(source) => format!("A permission rule denies it ({}). It stays denied while that rule stands.", {
                rule_source(source)
            }),
            None => "A permission rule denies it. It stays denied while that rule stands.".to_owned(),
        },
        DenialKind::AutoMode => format!("Auto mode's classifier blocked it: {}", denial.reason),
        DenialKind::Other => format!("Claude Code denied it: {}", denial.reason),
    }
}

fn one_line(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first: String = lines.next().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let cut = first.chars().count() > MAX_TARGET || lines.next().is_some();
    let mut line: String = first.chars().take(MAX_TARGET).collect();
    if cut {
        line.push('…');
    }
    line
}

/// The denied calls of the current turn, oldest first.
pub(super) fn current(model: &Model) -> Vec<Denied> {
    if model.agent.is_some() {
        return Vec::new();
    }
    let turn = model.chat.prompt.map_or(0, |prompt| prompt.saturating_add(1));
    let mut denied: Vec<Denied> = Vec::new();
    for (id, item) in model.items.range(ItemId { offset: turn, block: 0 }..) {
        let ItemKind::Tool { name, summary, file_path, state, denial, card, .. } = &item.kind else { continue };
        let target = || match card {
            Some(ToolCard::Shell { command, .. }) => one_line(command),
            _ => one_line(file_path.as_deref().unwrap_or(summary)),
        };
        match denial {
            Some(denial) => {
                denied.push(Denied {
                    item: *id,
                    tool: name.clone(),
                    target: target(),
                    denial: denial.as_ref().clone(),
                });
            }
            // The same call ran after all (approved, retried, or allowed the second time).
            None if *state == ToolState::Ok && !denied.is_empty() => {
                let target = target();
                denied.retain(|d| d.tool != *name || d.target != target);
            }
            None => {}
        }
    }
    denied
}

/// "2 permissions denied since your last message".
fn count_line(count: usize) -> String {
    let noun = if count == 1 { "permission" } else { "permissions" };
    format!("✗ {count} {noun} denied since your last message")
}

/// The sentence "Ask Claude to retry" puts in the message box: the calls auto mode did not
/// block (its own list tells Claude better), named as the transcript has them.
fn retry_sentence(denied: &[Denied]) -> Option<String> {
    let mut calls: Vec<String> = denied
        .iter()
        .filter(|d| d.denial.kind != DenialKind::AutoMode)
        .map(|d| if d.target.is_empty() { d.tool.clone() } else { format!("{}: {}", d.tool, d.target) })
        .collect();
    calls.dedup();
    if calls.is_empty() {
        return None;
    }
    let more = calls.len().saturating_sub(ASKED);
    calls.truncate(ASKED);
    let tail = if more > 0 { format!(" (and {more} more)") } else { String::new() };
    Some(format!("Please try again what was denied before, I allow it now: {}{tail}", calls.join("; ")))
}

/// What the card follows about the denials between syncs.
#[derive(Clone, Debug, Default)]
pub(super) struct Watch {
    /// The first sync was made: denials found then are old news and start folded.
    started: bool,
    /// The newest denial seen so far.
    newest: Option<ItemId>,
    /// The card lists them; otherwise it is the one-line strip with the count.
    pub open: bool,
}

impl Card {
    /// Follows the model's denials; true when the card changed.
    pub(super) fn sync_denials(&mut self, denied: Vec<Denied>) -> bool {
        let newest = denied.last().map(|d| d.item);
        let watch = &mut self.denials_watch;
        // A denial that happens while the tab is open opens the card; ones already in the
        // transcript when the tab was opened only show the count.
        let fresh = watch.started && newest.is_some_and(|id| watch.newest.is_none_or(|old| id > old));
        watch.started = true;
        watch.newest = watch.newest.max(newest);
        if fresh {
            watch.open = true;
        }
        let changed = fresh || self.denials != denied;
        self.denials = denied;
        changed
    }
}

impl Card {
    /// The card shows only the denied permissions: nothing that waits for an answer, so the
    /// conversation's own notices keep their place above it.
    pub(in crate::view::chat) fn only_denials(&self) -> bool {
        self.denials_only && self.visible()
    }
}

/// The card's whole height: a one-line strip, or the list.
pub(super) fn height(card: &Card) -> Option<f32> {
    if card.denials.is_empty() {
        return None;
    }
    if !card.denials_watch.open {
        return Some(2.0 * 7.0 + FIELD - 4.0);
    }
    let rows = card.denials.len().min(LISTED) as f32 * 2.0 * LINE;
    let more = if card.denials.len() > LISTED { LINE } else { 0.0 };
    Some(2.0 * cards::INSET - 6.0 + cards::HEAD + 8.0 + rows + more + 6.0 + cards::FOOT)
}

/// A click in the denials card: nothing here writes a key by itself.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit) -> bool {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match hit {
        Hit::Denials(open) => chat.question.denials_watch.open = open,
        // Claude Code's own list of blocked calls, or its rules: the command goes through the
        // checks of any slash command, and its dialog then shows as a card.
        Hit::Review => {
            chat.question.error = None;
            crate::state::AppState::submit_slash(tab, now, REVIEW, false);
        }
        Hit::AskRetry => {
            let Some(text) = retry_sentence(&chat.question.denials) else { return false };
            // Into the message box, once it is empty: never sent from here, and after any other
            // text already waiting for the box.
            chat.hold_text(text);
        }
        _ => return false,
    }
    true
}

impl Draw<'_> {
    /// The current turn's denied permissions: the count, and the list when open.
    pub(super) fn denials(&self, p: &mut Painter, chat: &mut Chat, card: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let denied = chat.question.denials.clone();
        let (x, w) = (card.x + cards::INSET, (card.w - 2.0 * cards::INSET).max(1.0));
        if !chat.question.denials_watch.open {
            let cy = card.cy();
            let show = self.plain(p, hits, card.right() - cards::INSET + 8.0, cy, "Show", Hit::Denials(true), true);
            p.text(Face::Semibold, 12.0, x, cy, &count_line(denied.len()), pal.warn, (w - show - 8.0).max(0.0));
            return;
        }
        let mut y = card.y + cards::INSET;
        let cy = y + cards::HEAD / 2.0;
        let hide = self.plain(p, hits, x + w + 8.0, cy, "Hide", Hit::Denials(false), false);
        p.text(Face::Semibold, 12.0, x, cy, &count_line(denied.len()), pal.warn, (w - hide - 8.0).max(0.0));
        y += cards::HEAD + 8.0;
        // The newest ones: the conversation's rows above have them all.
        let skipped = denied.len().saturating_sub(LISTED);
        for d in denied.iter().skip(skipped) {
            let name_w = p.text(Face::Semibold, 12.5, x, y + LINE / 2.0, &d.tool, pal.text, w * 0.3);
            let tx = x + name_w + 8.0;
            p.text(Face::Mono, 11.5, tx, y + LINE / 2.0, &d.target, pal.text, (x + w - tx).max(0.0));
            y += LINE;
            p.text(Face::Ui, 12.0, x + 12.0, y + LINE / 2.0, &why(&d.denial), pal.muted, w - 12.0);
            y += LINE;
        }
        if skipped > 0 {
            let more = format!("and {skipped} earlier, marked in the conversation above");
            p.text(Face::Ui, 12.0, x, y + LINE / 2.0, &more, pal.muted, w);
            y += LINE;
        }
        y += 6.0;
        let mut left: Vec<(&str, Hit)> = Vec::new();
        let has = |kind: DenialKind| denied.iter().any(|d| d.denial.kind == kind);
        if has(DenialKind::AutoMode) {
            left.push(("Review blocked calls…", Hit::Review));
        } else if has(DenialKind::Rule) {
            left.push(("Open /permissions", Hit::Review));
        }
        if retry_sentence(&denied).is_some() {
            left.push(("Ask Claude to retry", Hit::AskRetry));
        }
        let status = chat.question.error.unwrap_or("");
        self.foot(p, hits, Rect::new(x, y, w, cards::FOOT), &left, status, &[]);
    }
}

#[cfg(test)]
mod tests;
