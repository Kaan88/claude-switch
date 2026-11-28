//! Background controls: Ctrl+B for the running shell command, and Stop agent through the CLI's
//! agent list. Each key is written only while the screen shows what the key before should have
//! led to (Claude Code 2.1.332, the `bg_*` and `agents_*` fixtures); anything else stops with a
//! notice and leaves the rest to the terminal.
//!
//! Stop agent: at a clean, empty input whose hint row offers "↓ to manage", ↓ gives the list the
//! keys with the highlight on the main session; ↓ moves it one row at a time to the agent; `x`
//! stops it ("x to stop" under that row); Esc gives the keys back once the row reads "stopped".
//! Esc there leaves a running turn alone (recorded with one running).
use std::collections::BTreeMap;

use claude_switch_core::transcript::Subagent;

use super::screens::{AgentList, AgentState, ListKeys};
use super::*;
use crate::view::agent_status::Listed;

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
    // Its last key is Esc, which while Claude Code shows an agent may stop that agent instead.
    if screens::agent_shown(screen) {
        return Vec::new();
    }
    let Some(list) = idle_list(screen) else { return Vec::new() };
    list.rows
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, row)| row.state == AgentState::Running)
        .filter(|&(i, _)| row_agent(model, &list, i).is_some_and(|a| a.end.is_none()))
        .map(|(i, row)| AgentOffer { name: row.name.clone(), description: row.description.clone(), row: i })
        .collect()
}

/// The transcript's agent a row of the list is: the row is the list's only one with its type and
/// description, and the transcript knows one agent with both.
pub(super) fn row_agent<'a>(model: &'a Model, list: &AgentList, i: usize) -> Option<&'a Subagent> {
    let row = list.rows.get(i).filter(|_| i > 0)?;
    let names = list.names();
    if names.iter().filter(|n| **n == (row.name.as_str(), row.description.as_str())).count() != 1 {
        return None;
    }
    let mut known = model.agents.iter().filter(|a| {
        a.agent_type.as_deref() == Some(row.name.as_str()) && a.description.as_deref() == Some(row.description.as_str())
    });
    known.next().filter(|_| known.next().is_none())
}

/// The agents the CLI's list shows, by id, running or stopped: `None` when the screen cannot tell
/// (something covers the input, or the list is in a state Chat does not read), empty when the
/// input shows with no list under it.
pub(super) fn listed(model: &Model, screen: &Screen) -> Option<BTreeMap<String, Listed>> {
    let Some(list) = screens::agent_list(screen) else {
        return screens::no_agent_list(screen).then(BTreeMap::new);
    };
    let mut listed = BTreeMap::new();
    for (i, row) in list.rows.iter().enumerate().skip(1) {
        let state = match row.state {
            AgentState::Running => Listed::Running,
            AgentState::Stopped => Listed::Stopped,
            AgentState::Other => continue,
        };
        if let Some(agent) = row_agent(model, &list, i) {
            listed.insert(agent.id.clone(), state);
        }
    }
    Some(listed)
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
                && list.rows[row].state == AgentState::Stopped
                && list.rows[0].shown
                && screens::viewing_agent(screen).is_none() =>
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

impl Card {
    /// The shell call whose "Run in background" stands on its own line in the conversation, and
    /// whether that key is on its way: the offer alone, with no agent to stop and no notice the
    /// card would have to show.
    pub(in crate::view::chat) fn background_inline(&self) -> Option<(ItemId, bool)> {
        let shell = self.background.as_ref().filter(|_| self.agents.is_empty() && self.error.is_none())?;
        Some((shell.item, self.background_step.is_some()))
    }
}

/// The height of the background controls' card inside its padding.
pub(super) fn height(card: &Card) -> Option<f32> {
    // The offer alone is on the command's line in the conversation, not in a card; while that
    // line is out of view (scrolled away, folded into a group) the card offers it.
    let inline = card.background_inline().is_some() && card.offer_inline;
    let offered = card.background.is_some() && !inline;
    let shell = if offered { LINE + 8.0 } else { 0.0 };
    let agents = card.agents.len() as f32 * (CHOICE + 4.0);
    (offered || !card.agents.is_empty()).then_some(cards::HEAD + 10.0 + shell + agents + 4.0 + cards::FOOT)
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
        self.head(p, hits, Rect::new(inner.x, y, inner.w, cards::HEAD), title, ("", ""), "", false);
        y += cards::HEAD + 10.0;
        if let Some(shell) = &shell {
            p.text(Face::Mono, 12.0, inner.x, y + LINE / 2.0, &shell.command, pal.text, inner.w);
            y += LINE + 8.0;
        }
        for (i, agent) in card.agents.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHOICE);
            let label = format!("{} · {}", agent.name, agent.description);
            let w = self.plain(p, hits, row.right() + 8.0, row.cy(), "Stop agent", Hit::StopAgent(i), true);
            p.text(Face::Ui, 12.5, row.x, row.cy(), &label, pal.text, (row.w - w - 8.0).max(0.0));
            y += CHOICE + 4.0;
        }
        y += 4.0;
        let status = card.error.or_else(|| chat.question_notice()).unwrap_or("");
        let right: &[(&str, Hit)] = if shell.is_some() { &[("Run in background", Hit::Background)] } else { &[] };
        self.foot(p, hits, Rect::new(inner.x, y, inner.w, cards::FOOT), &[], status, right);
    }
}
