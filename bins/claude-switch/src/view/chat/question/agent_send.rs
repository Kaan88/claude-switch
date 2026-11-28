//! The message box over a subagent's conversation sends to that agent, the way Claude Code does
//! when an agent is picked in the list under its input (Claude Code 2.1.332; the list's screens
//! are the recorded `agents_*` fixtures, the rest is from its bundle and the `agent-message`
//! compat script records it):
//!
//! 1. At a clean, empty input whose hint row offers "↓ to manage", ↓ gives the list the keys with
//!    the highlight on the main session ("↑/↓ to select").
//! 2. ↓ (or ↑), one row at a time, each once the highlight moved, to the agent's row, which must
//!    read running ("enter to view · x to stop").
//! 3. Enter shows the agent: the list gives the keys back and the empty input's placeholder reads
//!    "Message @<name>…".
//! 4. The message as a paste, and Enter once the input shows it; done when the input is empty
//!    again. The text then goes to the agent, not to the main session.
//!
//! Back in Chat returns Claude Code to the main session through the same list: ↓, the highlight
//! to `main` (row 0), Enter; done once no placeholder names an agent. Esc is never sent while
//! Claude Code shows an agent: there it may stop the agent.
//!
//! Anything else stops with a word and keeps the message in the box. An agent that ended, or that
//! the list does not show running, gets no message.
use claude_switch_core::transcript::Subagent;

use super::screens::{self, AgentList, AgentState, ListKeys};
use super::*;
use crate::view::agent_status::Listed;
use crate::view::chat::submission::screen::{clean_prompt, prompt, shows_lines, shows_wrapped};

pub(in crate::view::chat) const ENDED: &str =
    "This agent has ended, so it cannot take a message. ‹ Back returns the box to the main session.";
pub(in crate::view::chat) const UNLISTED: &str = "The terminal's agent list does not show this agent running, so \
     nothing was sent. ‹ Back returns the box to the main session.";
pub(in crate::view::chat) const NOT_READY: &str = "Claude Code's input is not at rest with its agent list under it, \
     so the message is held here. Send it again once the turn has ended.";
pub(in crate::view::chat) const NOT_SELECTED: &str =
    "The terminal did not show this agent selected, so the message was held here. Check the terminal.";
pub(in crate::view::chat) const NOT_TAKEN: &str =
    "The terminal did not take the message for the agent; it is still here. Check the terminal.";
pub(in crate::view::chat) const SHELL: &str =
    "A shell command goes to the main session only: ‹ Back first, then send it.";
pub(in crate::view::chat) const ATTACHED: &str =
    "Attachments go to the main session only: remove them to message the agent, or ‹ Back first.";
/// Keys typed in the terminal (or a click elsewhere) stopped a message on its way to an agent.
pub(in crate::view::chat) const INTERRUPTED_SEND: &str =
    "The message to the agent stopped before it went, so it is still here. Check the terminal and send again.";
/// Back could not bring Claude Code back to the main session.
pub(in crate::view::chat) const STILL_AGENT: &str = "Claude Code still shows the agent: in the terminal, ↓ \
     and then Enter on main returns it to the main session. Messages wait until then.";
/// A message for the main session while Claude Code shows an agent would go to the agent.
pub(in crate::view::chat) const SHOWS_AGENT: &str =
    "Claude Code shows an agent, so this message waits until it is back on the main session.";

/// A pick in flight: to an agent with a message, or back to the main session.
#[derive(Clone, Debug)]
pub(super) struct Step {
    /// The agent's id; `None` on the way back to the main session.
    agent: Option<String>,
    /// The agent's type, which the input's placeholder names.
    name: String,
    /// The target row in the list (0: the main session).
    row: usize,
    /// The list's rows when the pick began: another list is not this one.
    names: Vec<(String, String)>,
    stage: Stage,
    /// The message's bytes and the text the input must show before Enter.
    message: Option<(Vec<u8>, String)>,
    due: Instant,
    epoch: u64,
}

impl Step {
    pub(super) fn due(&self) -> Instant {
        self.due
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// ↓ went from the input: the list must take the keys.
    Opening,
    /// An arrow went: the highlight must be on this row.
    Moving(usize),
    /// Enter went on the target row: the list must give the keys back, the agent shown (or no
    /// agent, on the way back).
    Picking,
    /// The message went as a paste: the input must show it.
    Pasted,
    /// Enter went: the input must be empty again.
    Entered,
}

/// The agent the message box sends to: the subagent whose conversation Chat shows.
pub(in crate::view::chat) fn target(model: &Model) -> Option<&Subagent> {
    let id = model.agent.as_deref()?;
    model.agents.iter().find(|a| a.id == id)
}

/// The agent's row in the list under the input, when the list shows it running.
fn agent_row(model: &Model, list: &AgentList, id: &str) -> Option<usize> {
    (1..list.rows.len()).find(|&i| {
        list.rows[i].state == AgentState::Running && background::row_agent(model, list, i).is_some_and(|a| a.id == id)
    })
}

fn owned(list: &AgentList) -> Vec<(String, String)> {
    list.names().into_iter().map(|(n, d)| (n.to_owned(), d.to_owned())).collect()
}

/// The input is empty: nothing typed, or only the placeholder naming the agent Claude Code shows
/// (drawn dim, it reads as empty; read whole, as that placeholder).
fn empty_input(screen: &Screen) -> bool {
    clean_prompt(screen) || screens::viewing_agent(screen).is_some()
}

/// The list under an empty input that has the keys.
fn idle_list(screen: &Screen) -> Option<AgentList> {
    let list = screens::agent_list(screen).filter(|l| l.keys == ListKeys::Input)?;
    empty_input(screen).then_some(list)
}

/// Send pressed with a subagent's conversation shown: true when the box sends to the agent (sent,
/// held or refused, the box says which); false for the main session.
pub(in crate::view::chat) fn send(tab: &mut Tab, now: Instant) -> bool {
    let pane = tab.pane.as_ref().expect("Chat pane");
    if target(&pane.model).is_none() {
        return false;
    }
    if pane.chat.editor.text.trim().is_empty() || pane.chat.question.agent_send.is_some() {
        return true;
    }
    match plan(tab, now) {
        Ok(step) if step.stage == Stage::Pasted => {
            let bytes = step.message.as_ref().map(|(bytes, _)| bytes.clone()).unwrap_or_default();
            match write(tab, &bytes) {
                Ok(()) => {
                    let chat = &mut tab.pane.as_mut().unwrap().chat;
                    chat.error = None;
                    chat.question.agent_send = Some(step);
                }
                Err(error) => tab.pane.as_mut().unwrap().chat.error = Some(error),
            }
            true
        }
        Ok(step) => open(tab, step),
        Err(why) => {
            tab.pane.as_mut().unwrap().chat.error = Some(why);
            true
        }
    }
}

/// What a send to the shown agent does first, or why it cannot go.
fn plan(tab: &Tab, now: Instant) -> Result<Step, &'static str> {
    let pane = tab.pane.as_ref().expect("Chat pane");
    let chat = &pane.chat;
    let agent = target(&pane.model).ok_or(UNLISTED)?;
    let text = chat.editor.text.trim();
    if chat.shell_mode {
        return Err(SHELL);
    }
    if !chat.chips.is_empty() {
        return Err(ATTACHED);
    }
    if pane.model.agent_status(agent).ended() {
        return Err(ENDED);
    }
    if pane.model.listed.get(&agent.id) != Some(&Listed::Running) {
        return Err(UNLISTED);
    }
    let bytes = crate::view::chat::submission::send_bytes(&[], text).ok_or(crate::view::chat::submission::BAD_TEXT)?;
    if !ready(tab) || screen_busy(tab) {
        return Err(NOT_READY);
    }
    let name = agent.agent_type.clone().unwrap_or_default();
    let screen = &tab.term.screen;
    let mut step = Step {
        agent: Some(agent.id.clone()),
        name,
        row: 0,
        names: Vec::new(),
        stage: Stage::Opening,
        message: Some((bytes, text.replace('\r', "\n"))),
        due: now + STEP,
        epoch: chat.output_epoch,
    };
    // Claude Code shows this agent already (Chat picked it, and its input says so): the message
    // goes straight in.
    let shown = chat.question.cli_agent.as_deref() == Some(agent.id.as_str())
        && screens::viewing_agent(screen).is_some_and(|shown| screens::names_agent(&shown, &step.name));
    if shown {
        step.stage = Stage::Pasted;
        return Ok(step);
    }
    let list = idle_list(screen).ok_or(NOT_READY)?;
    step.row = agent_row(&pane.model, &list, &agent.id).ok_or(UNLISTED)?;
    step.names = owned(&list);
    Ok(step)
}

/// A menu or a prompt has the keys: nothing is picked while one shows. (A running turn does not:
/// the list is driven under it, as Stop agent does, and the input reads empty only without a
/// prompt over it.)
fn screen_busy(tab: &Tab) -> bool {
    let screen = &tab.term.screen;
    crate::view::chat::submission::screen::menu(screen) || crate::attention::screen_permission_prompt(screen)
}

/// ↓ from the input: the list takes the keys.
fn open(tab: &mut Tab, step: Step) -> bool {
    let down = moves(0, 1, &tab.term.screen);
    match write(tab, &down) {
        Ok(()) => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.error = None;
            chat.question.agent_send = Some(step);
        }
        Err(error) => tab.pane.as_mut().unwrap().chat.error = Some(error),
    }
    true
}

/// Back in Chat while Claude Code shows the agent Chat picked: its list brings it back to the main
/// session, once its input is at rest. True when keys went.
fn start_back(tab: &mut Tab, now: Instant) -> bool {
    let pane = tab.pane.as_ref().expect("Chat pane");
    let chat = &pane.chat;
    if chat.question.cli_agent.is_none()
        || chat.question.agent_send.is_some()
        || pane.model.agent.is_some()
        || !ready(tab)
        || screen_busy(tab)
    {
        return false;
    }
    let screen = &tab.term.screen;
    if screens::viewing_agent(screen).is_none() && clean_prompt(screen) {
        // Back on the main session already (the terminal did it): nothing to do.
        tab.pane.as_mut().unwrap().chat.question.cli_agent = None;
        return true;
    }
    let Some(list) = idle_list(screen) else { return false };
    let step = Step {
        agent: None,
        name: String::new(),
        row: 0,
        names: owned(&list),
        stage: Stage::Opening,
        message: None,
        due: now + STEP,
        epoch: pane.chat.output_epoch,
    };
    open(tab, step)
}

enum Next {
    Wait,
    Key(Vec<u8>, Stage),
    /// The agent shows (or the main session again): the message goes next, if any.
    Picked,
    Done,
}

fn next(model: &Model, screen: &Screen, step: &Step) -> Next {
    match step.stage {
        Stage::Opening | Stage::Moving(_) => {
            let Some(list) = screens::agent_list(screen).filter(|l| owned(l) == step.names) else { return Next::Wait };
            let Some(h) = list.highlight.filter(|_| list.keys != ListKeys::Input) else { return Next::Wait };
            if matches!(step.stage, Stage::Moving(want) if want != h) {
                return Next::Wait;
            }
            if h != step.row {
                let to = if h < step.row { h + 1 } else { h - 1 };
                return Next::Key(moves(h, to, screen), Stage::Moving(to));
            }
            // Enter on the main session's row, or on the agent's while it runs and is still the
            // one Chat shows.
            let fits = match &step.agent {
                None => list.keys == ListKeys::Select,
                Some(id) => {
                    list.keys == ListKeys::Stop
                        && list.rows[h].state == AgentState::Running
                        && background::row_agent(model, &list, h).is_some_and(|a| &a.id == id)
                }
            };
            if fits { Next::Key(b"\r".to_vec(), Stage::Picking) } else { Next::Wait }
        }
        Stage::Picking => {
            let keys_back = screens::agent_list(screen).is_none_or(|l| l.keys == ListKeys::Input);
            let shown = screens::viewing_agent(screen);
            let picked = match &step.agent {
                Some(_) => shown.is_some_and(|shown| screens::names_agent(&shown, &step.name)),
                None => shown.is_none() && screens::input_row(screen).is_some(),
            };
            if keys_back && picked && empty_input(screen) { Next::Picked } else { Next::Wait }
        }
        Stage::Pasted => {
            let expected = step.message.as_ref().map_or("", |(_, text)| text.as_str());
            let shows = if expected.contains('\n') {
                shows_lines(screen, expected)
            } else {
                prompt(screen).as_deref() == Some(expected)
                    || shows_wrapped(screen, expected)
                    || shows_lines(screen, expected)
            };
            if shows { Next::Key(b"\r".to_vec(), Stage::Entered) } else { Next::Wait }
        }
        Stage::Entered => {
            if empty_input(screen) {
                Next::Done
            } else {
                Next::Wait
            }
        }
    }
}

/// The next key of a pick in flight, once newer output shows what the last one led to; and Back's
/// way to the main session when Chat left Claude Code on an agent.
pub(super) fn advance(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let pane = tab.pane.as_ref().expect("Chat pane");
    let Some(mut step) = pane.chat.question.agent_send.clone() else {
        return may_write && start_back(tab, now);
    };
    let epoch = pane.chat.output_epoch;
    let fail = |tab: &mut Tab, why: &'static str| {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.question.agent_send = None;
        chat.error = Some(why);
        true
    };
    if !ready(tab) {
        return fail(tab, NOT_SELECTED);
    }
    let next = if epoch == step.epoch { Next::Wait } else { next(&pane.model, &tab.term.screen, &step) };
    let (key, stage) = match next {
        Next::Done => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.agent_send = None;
            chat.error = None;
            chat.editor.set(String::new());
            chat.editor.clear_history();
            return true;
        }
        Next::Picked => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.cli_agent = step.agent.clone();
            match step.message.clone() {
                None => {
                    chat.question.agent_send = None;
                    if chat.error == Some(STILL_AGENT) || chat.error == Some(SHOWS_AGENT) {
                        chat.error = None;
                    }
                    return true;
                }
                Some((bytes, _)) if may_write => (bytes, Stage::Pasted),
                Some(_) => return false,
            }
        }
        Next::Key(key, stage) if may_write => (key, stage),
        _ if now >= step.due => {
            let why = match (&step.agent, step.stage) {
                (None, _) => STILL_AGENT,
                (Some(_), Stage::Pasted | Stage::Entered) => NOT_TAKEN,
                _ => NOT_SELECTED,
            };
            return fail(tab, why);
        }
        _ => return false,
    };
    if let Err(error) = write(tab, &key) {
        return fail(tab, error);
    }
    step.stage = stage;
    step.epoch = epoch;
    step.due = now + STEP;
    tab.pane.as_mut().unwrap().chat.question.agent_send = Some(step);
    true
}

#[cfg(test)]
pub(in crate::view::chat) const VIEWING_SCREEN: &str = "cursor 28 2
@size 100 40



























────────────────────────────────────────────────────────────────────────────────────────────────────
❯ Message @general-purpose…
────────────────────────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts · ↓ to manage

  ● main
  ◯ general-purpose  First pinger                                              9s · ↓ 37.7k tokens
  ◯ general-purpose  Second pinger                                             9s · ↓ 37.5k tokens
";
