//! Background controls: Ctrl+B for the running shell command, and Stop agent through the CLI's
//! agent list. Each key is written only while the screen shows what the key before should have
//! led to (Claude Code 2.1.332, the `bg_*` and `agents_*` fixtures); anything else stops with a
//! notice and leaves the rest to the terminal.
//!
//! Stop agent: at a clean, empty input whose hint row offers "↓ to manage", ↓ gives the list the
//! keys with the highlight on the main session; ↓ moves it one row at a time to the agent; `x`
//! stops it ("x to stop" under that row); Esc gives the keys back once the row reads "stopped".
//! Esc there leaves a running turn alone (recorded with one running).
use super::screens::{AgentList, AgentState, ListKeys};
use super::*;

pub(super) const NOT_BACKGROUND: &str = "The command did not move to the background. Check the terminal.";
pub(super) const NO_HINT: &str = "The terminal no longer offers to run this command in the background.";
pub(super) const NO_AGENT: &str = "The terminal no longer lists this agent as running, so nothing was sent.";
pub(super) const NO_LIST: &str =
    "The terminal's agent list did not show what Chat expected, so nothing more was sent. Check the terminal.";
pub(super) const NOT_STOPPED: &str =
    "The terminal did not stop the agent, so nothing more was sent. Check the terminal.";
pub(super) const LIST_OPEN: &str = "The agent was stopped, but its list still has the keys. Press Esc in the terminal.";
const CTRL_B: &[u8] = b"\x02";
const STOP: &[u8] = b"x";

/// The running shell call the CLI offers to move to the background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shell {
    pub item: ItemId,
    /// The tool's name as its row shows it (`Bash`, `PowerShell`).
    pub tool: String,
    pub command: String,
}

/// A running background agent Chat can stop: the transcript knows it and the CLI's list shows it
/// once, as running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentOffer {
    pub name: String,
    pub description: String,
    /// Its row in the CLI's list (the main session is row 0).
    pub row: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// The list has the keys with the highlight on this row.
    Highlight(usize),
    /// The agent's row reads "stopped", still highlighted.
    Stopped,
    /// The input has the keys again.
    Closed,
}

/// A Stop agent in flight.
#[derive(Clone, Debug)]
pub(super) struct AgentStep {
    offer: AgentOffer,
    /// The list's rows as they were at the click: any other list is not the one being driven.
    names: Vec<(String, String)>,
    expect: Expect,
    pub(super) due: Instant,
    epoch: u64,
}

/// The shell call the CLI offers Ctrl+B for: the only pending call, a shell command, with the
/// hint under that call's own row.
pub(super) fn shell_offer(pending: Option<&Pending>, screen: &Screen) -> Option<Shell> {
    let p = pending.filter(|p| p.alone)?;
    let Kind::Tool(ItemKind::Tool { name, card: Some(ToolCard::Shell { command, .. }), .. }) = p.kind else {
        return None;
    };
    screens::background_hint(screen, name, command).then(|| Shell {
        item: p.item,
        tool: name.clone(),
        command: collapse(command.lines().next().unwrap_or_default()),
    })
}

/// The agent list while the input has the keys and is empty.
fn idle_list(screen: &Screen) -> Option<AgentList> {
    let list = screens::agent_list(screen).filter(|l| l.keys == ListKeys::Input)?;
    crate::view::chat::submission::screen::clean_prompt(screen).then_some(list)
}

/// The running background agents Chat can stop: each is in the transcript (its type and
/// description, not known to have finished) and once in the CLI's list, as running.
pub(super) fn agent_offers(model: &Model, screen: &Screen) -> Vec<AgentOffer> {
    let Some(list) = idle_list(screen) else { return Vec::new() };
    let names = list.names();
    list.rows
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, row)| row.state == AgentState::Running)
        .filter(|(_, row)| names.iter().filter(|n| **n == (row.name.as_str(), row.description.as_str())).count() == 1)
        .filter(|(_, row)| {
            let known = |a: &&claude_switch_core::transcript::Subagent| {
                a.agent_type.as_deref() == Some(row.name.as_str())
                    && a.description.as_deref() == Some(row.description.as_str())
            };
            let mut known = model.agents.iter().filter(known);
            known.next().is_some_and(|a| a.finished != Some(true)) && known.next().is_none()
        })
        .map(|(i, row)| AgentOffer { name: row.name.clone(), description: row.description.clone(), row: i })
        .collect()
}

fn owned(list: &AgentList) -> Vec<(String, String)> {
    list.names().into_iter().map(|(n, d)| (n.to_owned(), d.to_owned())).collect()
}

/// Ctrl+B, only while the CLI still offers it for the same call, which alone is pending.
pub(super) fn press_background(tab: &mut Tab, now: Instant) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    if chat.question.background_step.is_some() || chat.question.agent_step.is_some() {
        return false;
    }
    let Some(shell) = chat.question.background.clone() else {
        tab.pane.as_mut().unwrap().chat.question_failed(NO_HINT);
        return true;
    };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    match write(tab, CTRL_B) {
        Ok(()) => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.error = None;
            chat.question.background_step = Some((shell, now + STEP, epoch));
        }
        Err(error) => tab.pane.as_mut().unwrap().chat.question_failed(error),
    }
    true
}

/// Ends a Ctrl+B in flight: done once the call ended or newer output shows it running in the
/// background.
pub(super) fn advance_background(tab: &mut Tab, now: Instant) -> bool {
    let pane = tab.pane.as_mut().unwrap();
    let Some((shell, due, epoch)) = pane.chat.question.background_step.clone() else { return false };
    let ended = pending(&pane.model).is_none_or(|p| p.item != shell.item);
    let moved = pane.chat.output_epoch != epoch && screens::backgrounded(&tab.term.screen, &shell.tool, &shell.command);
    if ended || moved {
        pane.chat.question.background_step = None;
    } else if now >= due {
        pane.chat.question.background_step = None;
        pane.chat.question_failed(NOT_BACKGROUND);
    } else {
        return false;
    }
    true
}

/// Stop agent: the first ↓, only while the list shows that agent running and the input is empty.
pub(super) fn press_stop(tab: &mut Tab, now: Instant, which: usize) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    if chat.question.background_step.is_some() || chat.question.agent_step.is_some() {
        return false;
    }
    let Some(offer) = chat.question.agents.get(which).cloned() else {
        tab.pane.as_mut().unwrap().chat.question_failed(NO_AGENT);
        return true;
    };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let list = idle_list(&tab.term.screen).filter(|list| {
        list.rows.get(offer.row).is_some_and(|row| {
            row.name == offer.name && row.description == offer.description && row.state == AgentState::Running
        })
    });
    let Some(list) = list else {
        tab.pane.as_mut().unwrap().chat.question_failed(NO_AGENT);
        return true;
    };
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    let down = moves(0, 1, &tab.term.screen);
    match write(tab, &down) {
        Ok(()) => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.error = None;
            let step = AgentStep { offer, names: owned(&list), expect: Expect::Highlight(0), due: now + STEP, epoch };
            chat.question.agent_step = Some(step);
        }
        Err(error) => tab.pane.as_mut().unwrap().chat.question_failed(error),
    }
    true
}

/// What a Stop agent in flight does next.
enum Next {
    Wait,
    Done,
    Key(Vec<u8>, Expect),
}

/// What the screen shows after the last key of a Stop agent, and so what comes next.
fn next(screen: &Screen, step: &AgentStep) -> Next {
    let list = screens::agent_list(screen).filter(|l| owned(l) == step.names);
    let row = step.offer.row;
    match (step.expect, list.as_ref()) {
        (Expect::Highlight(h), Some(list)) if list.highlight == Some(h) => {
            let keys_fit = match h {
                0 => list.keys == ListKeys::Select,
                _ if h < row => matches!(list.keys, ListKeys::Stop | ListKeys::Clear),
                _ => list.keys == ListKeys::Stop,
            };
            match () {
                _ if !keys_fit => Next::Wait,
                _ if h < row => Next::Key(moves(0, 1, screen), Expect::Highlight(h + 1)),
                _ if list.rows[h].state == AgentState::Running => Next::Key(STOP.to_vec(), Expect::Stopped),
                _ => Next::Wait,
            }
        }
        (Expect::Stopped, Some(list))
            if list.keys == ListKeys::Clear
                && list.highlight == Some(row)
                && list.rows[row].state == AgentState::Stopped =>
        {
            Next::Key(ESC.to_vec(), Expect::Closed)
        }
        // The list stays drawn under the input, which has the keys again: no highlight.
        (Expect::Closed, _)
            if crate::view::chat::submission::screen::clean_prompt(screen)
                && screens::agent_list(screen).is_none_or(|l| l.keys == ListKeys::Input) =>
        {
            Next::Done
        }
        _ => Next::Wait,
    }
}

/// The next key of a Stop agent in flight, once newer output shows what the last one led to.
pub(super) fn advance_agent(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    let Some(mut step) = chat.question.agent_step.clone() else { return false };
    let epoch = chat.output_epoch;
    let fail = |tab: &mut Tab, why| {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.question.agent_step = None;
        chat.question_failed(why);
        true
    };
    if !ready(tab) {
        return fail(tab, BLOCKED);
    }
    let next = if epoch == step.epoch { Next::Wait } else { next(&tab.term.screen, &step) };
    let (key, expect) = match next {
        Next::Done => {
            tab.pane.as_mut().unwrap().chat.question.agent_step = None;
            return true;
        }
        Next::Key(key, expect) if may_write => (key, expect),
        _ if now >= step.due => {
            let why = match step.expect {
                Expect::Highlight(_) => NO_LIST,
                Expect::Stopped => NOT_STOPPED,
                Expect::Closed => LIST_OPEN,
            };
            return fail(tab, why);
        }
        _ => return false,
    };
    if let Err(error) = write(tab, &key) {
        return fail(tab, error);
    }
    step.expect = expect;
    step.epoch = epoch;
    step.due = now + STEP;
    tab.pane.as_mut().unwrap().chat.question.agent_step = Some(step);
    true
}

/// The card's height for the background controls, without its padding, field and status rows.
pub(super) fn height(card: &Card) -> Option<f32> {
    let shell = card.background.as_ref().map_or(0.0, |_| LINE + 6.0);
    let agents = card.agents.len() as f32 * (CHOICE + 4.0);
    (card.background.is_some() || !card.agents.is_empty()).then_some(HEADER + shell + agents)
}

impl Draw<'_> {
    /// The running shell command the CLI offers to move to the background, and the background
    /// agents it can stop.
    pub(super) fn background(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let card = &chat.question;
        let shell = card.background.clone();
        let mut y = inner.y;
        let title = if shell.is_some() { "Running a shell command" } else { "Background agents" };
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, title, pal.muted, inner.w);
        y += HEADER;
        if let Some(shell) = &shell {
            p.text(Face::Mono, 11.5, inner.x, y + LINE / 2.0, &shell.command, pal.text, inner.w);
            y += LINE + 6.0;
        }
        for (i, agent) in card.agents.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHOICE);
            let label = format!("{} · {}", agent.name, agent.description);
            p.text(Face::Ui, 12.0, row.x, row.cy(), &label, pal.text, (row.w - 130.0).max(0.0));
            let r = Rect::new(row.right() - 120.0, row.y, 120.0, row.h);
            let target = Target::Chat(Control::Question(Hit::StopAgent(i)));
            super::super::paint::button(p, pal, r, "Stop agent", self.hover == Some(target), false);
            hits.push((r, target));
            y += CHOICE + 4.0;
        }
        let row = Rect::new(inner.x, y, inner.w, FIELD - 4.0);
        let status = card.error.or_else(|| chat.question_notice()).unwrap_or("");
        let room = if shell.is_some() { 160.0 } else { 0.0 };
        p.text(Face::Ui, 11.5, row.x, row.cy(), status, pal.muted, (row.w - room).max(0.0));
        if shell.is_some() {
            let r = Rect::new(row.right() - 150.0, row.y, 150.0, row.h);
            let target = Target::Chat(Control::Question(Hit::Background));
            super::super::paint::button(p, pal, r, "Run in background", self.hover == Some(target), false);
            hits.push((r, target));
        }
    }
}
