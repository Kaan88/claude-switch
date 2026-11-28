//! Rewind from a prompt bubble, through the CLI's own rewind list (Claude Code 2.1.332, the
//! `rewind_*` fixtures). Every key is written only once the screen shows what the key before
//! should have led to:
//!
//! 1. Esc at a verified idle, quiet, empty prompt (no turn, no pending call, no menu). It shows
//!    nothing, so the second Esc follows after a short gap only while the screen still shows that
//!    same idle prompt.
//! 2. The list opens with "(current)" highlighted. Up, one at a time, each only once the highlight
//!    moved one entry up (or the list scrolled by one), until the highlighted row reads the chosen
//!    prompt. A later prompt the same row could also read stops it: the row would be ambiguous.
//! 3. Enter, then the prompt's choices, which the card shows as the screen words them, quote
//!    included; nothing is chosen without a click. A click moves the highlight one row at a time
//!    and Enter goes only while it is on that choice with the same choices around it.
//! 4. Done once neither rewind screen shows and the input is back.
//!
//! Cancel writes Esc (back to the list), then Esc (back to the prompt). Anything unexpected stops,
//! says why and leaves the rest to the terminal.
use super::screens::{RewindActions, RewindList};
use super::*;

/// How long after the first Esc the second one goes; the CLI takes two within about a second, so
/// a second Esc that cannot go within `ESC_LATEST` of the first stops the rewind instead.
const ESC_GAP: Duration = Duration::from_millis(150);
/// What the card says while a rewind's keys are on their way.
const REWINDING: &str = "Rewinding in Claude Code…";
const ESC_LATEST: Duration = Duration::from_millis(600);

/// The menu was opened over a subagent's transcript: its prompts are not the session's.
pub(super) const IN_AGENT: &str =
    "Rewind works in the session's own conversation, not in a subagent's: go back to it first.";

/// Rewind asked for from a subagent's transcript: nothing is written, and the box says why.
pub(in crate::view) fn refuse_in_agent(tab: &mut Tab) {
    tab.pane.as_mut().expect("Chat pane").chat.error = Some(IN_AGENT);
}
pub(super) const NOT_IDLE: &str =
    "Claude Code is not at an idle, empty prompt, so nothing was sent. Rewind once the turn has ended.";
pub(super) const NOT_PROMPT: &str =
    "Only a prompt you typed can be rewound to: Claude Code's rewind list does not show slash commands.";
/// After a stop with the rewind list on screen, one Esc (screen-checked) closes it again.
pub(super) const CLOSED: &str = "Chat closed the terminal's rewind list again.";
pub(super) const LEFT_OPEN: &str = "The terminal's rewind screen is still open: press Esc there to leave it.";
pub(super) const NOT_OPENED: &str =
    "The terminal did not open its rewind list, so nothing more was sent. Check the terminal.";
pub(super) const NOT_LISTED: &str =
    "The terminal's rewind list does not show this prompt, so nothing was chosen. Rewind in the terminal.";
pub(super) const AMBIGUOUS: &str =
    "The terminal's rewind list shows a later prompt the same way, so nothing was chosen. Rewind in the terminal.";
pub(super) const NO_ACTIONS: &str =
    "The terminal did not show the rewind's choices for this prompt, so nothing more was sent. Check the terminal.";
pub(super) const ACTIONS_CHANGED: &str =
    "The rewind's choices or the prompt they are for changed in the terminal. Check the terminal.";
pub(super) const MISPLACED: &str = "The terminal's rewind list does not show the prompts after this one as the \
    transcript has them, so nothing was chosen. Rewind in the terminal.";
pub(super) const HELD_BACK: &str = "Chat could not send the rewind's next key in time (something was open over the \
    window), so it stopped. Check the terminal.";
pub(super) const NOT_DONE: &str = "The terminal did not finish the rewind. Check the terminal.";
pub(super) const NOT_CANCELLED: &str = "The terminal did not close its rewind list. Check the terminal.";
pub(super) const REWOUND: &str = "Rewound in Claude Code. The conversation here still shows the later turns: Switch reads the transcript as written.";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    /// The first Esc went; the second goes at `due` if nothing changed.
    FirstEsc,
    /// The second Esc went: the list must open on "(current)".
    Opening,
    /// An Up went from this list.
    Up(RewindList),
    /// Enter went on the prompt's row: its choices must show.
    Entering,
    /// The choices show; the user picks one in the card.
    Choosing,
    /// An arrow went towards choice `target`, from `from`.
    Moving { target: usize, from: usize },
    /// Enter went on a choice.
    Confirming,
    /// Esc went from the choices (`true`: the list must show) or from the list (the prompt).
    Cancelling(bool),
    /// It stopped with the list on screen, and Esc went to close it: the input must be back.
    Leaving,
    /// Stopped; the card says why until it is closed.
    Failed,
}

/// A rewind in progress, for one prompt of the transcript.
#[derive(Clone, Debug)]
pub(super) struct Rewind {
    /// The prompt as typed.
    prompt: String,
    /// The prompts typed after it, newest last: a list row that could read one of them is not
    /// taken for this one.
    later: Vec<String>,
    stage: Stage,
    /// The choices as last read, for the card.
    pub(super) actions: Option<RewindActions>,
    pub(super) error: Option<&'static str>,
    /// Once stopped: whether the rewind screen was closed again or is still open.
    pub(super) note: Option<&'static str>,
    pub(super) due: Option<Instant>,
    epoch: u64,
}

impl Rewind {
    /// Keys are on their way: the composer waits, and a hidden tab goes on with them.
    pub(super) fn in_flight(&self) -> bool {
        !matches!(self.stage, Stage::Choosing | Stage::Failed)
    }

    pub(super) fn failed(&self) -> bool {
        self.stage == Stage::Failed
    }

    pub(super) fn choosing(&self) -> bool {
        self.stage == Stage::Choosing
    }

    /// Stops it before its next key, saying why.
    pub(super) fn interrupt(&mut self, why: &'static str) {
        fail(self, why);
    }
}

/// The CLI's input is idle and empty, nothing is pending and no menu shows: as for a slash command.
fn idle(tab: &Tab, now: Instant) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    let screen = &tab.term.screen;
    crate::view::chat::submission::blocked(tab, now, crate::view::chat::submission::Intent::Slash).is_none()
        && crate::view::chat::submission::screen::idle_slash_prompt(screen, chat.quiet(now))
        && screens::rewind_list(screen).is_none()
        && screens::rewind_actions(screen).is_none()
        && ready(tab)
}

/// The rewind list shows this prompt: it lists what the user typed, never a slash command
/// (`/context` recorded from Claude Code 2.1.332, which the reader shows as its `/` name).
pub(in crate::view) fn listed(prompt: &str) -> bool {
    let prompt = prompt.trim_start();
    !prompt.is_empty() && !prompt.starts_with(['/', '<'])
}

/// "Rewind to here…" on the prompt at `item`: the first Esc, at a verified idle prompt.
pub(in crate::view) fn start(tab: &mut Tab, now: Instant, item: ItemId) {
    let pane = tab.pane.as_ref().expect("Chat pane");
    let card = &pane.chat.question;
    if card.rewind.as_ref().is_some_and(|r| !r.failed()) || card.step.is_some() || card.agent_step.is_some() {
        return;
    }
    let items = &pane.model.items;
    // A message sent while a turn ran is listed too (recorded from 2.1.332, `recorder compat`'s
    // `queued` script), where the transcript has it.
    let prompt =
        items.get(&item).and_then(crate::view::chat::recall::prompt_text).filter(|p| listed(p)).map(str::to_owned);
    let later: Vec<String> = items
        .range(ItemId { offset: item.offset.saturating_add(1), block: 0 }..)
        .filter_map(|(_, item)| crate::view::chat::recall::prompt_text(item))
        .filter(|text| listed(text))
        .map(str::to_owned)
        .collect();
    let epoch = pane.chat.output_epoch;
    let mut rewind = Rewind {
        prompt: prompt.clone().unwrap_or_default(),
        later,
        stage: Stage::Failed,
        actions: None,
        error: None,
        note: None,
        due: None,
        epoch,
    };
    let busy = pane.chat.waiting().is_some() || pane.chat.locked();
    rewind.error = match () {
        _ if prompt.is_none() => Some(NOT_PROMPT),
        _ if busy || !idle(tab, now) => Some(NOT_IDLE),
        _ => None,
    };
    if rewind.error.is_none() {
        match write(tab, ESC) {
            Ok(()) => {
                rewind.stage = Stage::FirstEsc;
                rewind.due = Some(now + ESC_GAP);
            }
            Err(error) => rewind.error = Some(error),
        }
    }
    tab.pane.as_mut().unwrap().chat.question.rewind = Some(rewind);
}

/// What comes after the last key, from the screen.
enum Next {
    Wait,
    Key(Vec<u8>, Stage),
    /// The choices show; the card offers them.
    Choose(RewindActions),
    Done,
    /// The list it stopped on is closed again.
    Closed,
    Fail(&'static str),
}

fn look(tab: &Tab, now: Instant, rewind: &Rewind) -> Next {
    let screen = &tab.term.screen;
    let fresh = tab.pane.as_ref().unwrap().chat.output_epoch != rewind.epoch;
    let back = || {
        screens::input_row(screen).is_some()
            && screens::rewind_list(screen).is_none()
            && screens::rewind_actions(screen).is_none()
    };
    match &rewind.stage {
        Stage::FirstEsc if rewind.due.is_some_and(|due| now < due) => Next::Wait,
        // Nothing may have changed: the same idle prompt, no list yet.
        Stage::FirstEsc if idle(tab, now) => Next::Key(ESC.to_vec(), Stage::Opening),
        Stage::FirstEsc => Next::Fail(NOT_IDLE),
        _ if !fresh => Next::Wait,
        Stage::Opening => match screens::rewind_list(screen) {
            Some(list) if list.highlight + 1 == list.entries.len() => pick_or_up(screen, rewind, list),
            _ => Next::Wait,
        },
        Stage::Up(before) => match screens::rewind_list(screen) {
            Some(list) if moved_up(before, &list) => pick_or_up(screen, rewind, list),
            _ => Next::Wait,
        },
        Stage::Entering => match screens::rewind_actions(screen) {
            Some(actions) if for_prompt(&actions, rewind) => Next::Choose(actions),
            Some(_) => Next::Fail(NO_ACTIONS),
            None => Next::Wait,
        },
        Stage::Moving { target, from } => {
            let Some(actions) = screens::rewind_actions(screen) else { return Next::Wait };
            let same = same_actions(&actions, rewind);
            let step = if target > from { from + 1 } else { from.saturating_sub(1) };
            match actions.highlight {
                _ if !same => Next::Fail(ACTIONS_CHANGED),
                Some(h) if h == *target && h == step => Next::Key(b"\r".to_vec(), Stage::Confirming),
                Some(h) if h == step => {
                    Next::Key(toward(h, *target, screen), Stage::Moving { target: *target, from: h })
                }
                _ => Next::Wait,
            }
        }
        Stage::Confirming if back() => Next::Done,
        Stage::Cancelling(true) => match screens::rewind_list(screen) {
            Some(_) => Next::Key(ESC.to_vec(), Stage::Cancelling(false)),
            None => Next::Wait,
        },
        Stage::Cancelling(false) if back() => Next::Done,
        Stage::Leaving if back() => Next::Closed,
        _ => Next::Wait,
    }
}

/// One arrow key from `from` towards `target`.
fn toward(from: usize, target: usize, screen: &Screen) -> Vec<u8> {
    moves(from, if target > from { from + 1 } else { from.saturating_sub(1) }, screen)
}

/// The choices quote the chosen prompt: as the list cuts it, or whole, wrapped onto several rows.
fn for_prompt(actions: &RewindActions, rewind: &Rewind) -> bool {
    screens::rewind_row_is(&actions.quoted, &rewind.prompt) || squash(&actions.quoted) == squash(&rewind.prompt)
}

/// The screen still shows the choices the card offers, for the chosen prompt: the CLI may have
/// been moved to another prompt whose choices read the same.
fn same_actions(shown: &RewindActions, rewind: &Rewind) -> bool {
    rewind.actions.as_ref().is_some_and(|a| a.choices == shown.choices) && for_prompt(shown, rewind)
}

/// One Up from `before` led to `after`: the highlight one entry higher in the same list, or the
/// list scrolled by one with the highlight on its top entry.
fn moved_up(before: &RewindList, after: &RewindList) -> bool {
    let up = before.highlight.checked_sub(1);
    if after.entries == before.entries {
        return up == Some(after.highlight);
    }
    let n = before.entries.len();
    before.highlight == 0
        && after.highlight == 0
        && after.entries.len() == n
        && after.entries[1..] == before.entries[..n - 1]
}

/// On the chosen prompt's row: Enter, unless a later prompt could read the same or the rows under
/// it are not the later prompts, one for one (an earlier prompt worded alike would be taken for it
/// when the list leaves the chosen one out). Else Up.
fn pick_or_up(screen: &Screen, rewind: &Rewind, list: RewindList) -> Next {
    let row = &list.entries[list.highlight];
    let current = list.highlight + 1 == list.entries.len();
    if !current && screens::rewind_row_is(row, &rewind.prompt) {
        if rewind.later.iter().any(|later| screens::rewind_row_is(row, later)) {
            return Next::Fail(AMBIGUOUS);
        }
        let below = &list.entries[list.highlight + 1..list.entries.len() - 1];
        let fits = below.len() == rewind.later.len()
            && below.iter().zip(&rewind.later).all(|(row, later)| screens::rewind_row_is(row, later));
        if !fits {
            return Next::Fail(MISPLACED);
        }
        return Next::Key(b"\r".to_vec(), Stage::Entering);
    }
    Next::Key(moves(1, 0, screen), Stage::Up(list))
}

/// Moves a rewind in flight on; true when the card changed.
pub(super) fn advance(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    let Some(mut rewind) = chat.question.rewind.clone().filter(Rewind::in_flight) else { return false };
    let epoch = chat.output_epoch;
    let next = if ready(tab) { look(tab, now, &rewind) } else { Next::Fail(BLOCKED) };
    // A key that may not be written now waits like any other, and its deadline still holds: a
    // dialog over the window must not keep the composer locked and resume the keys much later.
    let held = matches!(next, Next::Key(..)) && !may_write;
    // A cancel's own Esc may still be on its way (the CLI did not repaint): another would land on
    // whatever it led to, so a cancel that stops never closes the list itself.
    let cancelling = matches!(rewind.stage, Stage::Cancelling(_));
    let next = if held { Next::Wait } else { next };
    let deadline = match rewind.stage {
        Stage::FirstEsc => rewind.due.is_some_and(|due| now >= due + ESC_LATEST),
        _ => rewind.due.is_some_and(|due| now >= due),
    };
    match next {
        Next::Key(key, stage) => match write(tab, &key) {
            Ok(()) => {
                let wait = if stage == Stage::Confirming { RESULT } else { STEP };
                rewind.stage = stage;
                rewind.due = Some(now + wait);
                rewind.epoch = epoch;
            }
            Err(error) => fail(&mut rewind, error),
        },
        Next::Choose(actions) => {
            rewind.actions = Some(actions);
            rewind.stage = Stage::Choosing;
            rewind.due = None;
        }
        Next::Done => {
            let cancelled = matches!(rewind.stage, Stage::Cancelling(_));
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.rewind = None;
            if !cancelled {
                chat.error = Some(REWOUND);
            }
            return true;
        }
        Next::Closed => {
            rewind.stage = Stage::Failed;
            rewind.note = Some(CLOSED);
            rewind.due = None;
        }
        Next::Fail(why) => fail(&mut rewind, why),
        Next::Wait if deadline && rewind.stage == Stage::Leaving => {
            rewind.stage = Stage::Failed;
            rewind.note = Some(LEFT_OPEN);
            rewind.due = None;
        }
        Next::Wait if deadline => {
            let why = match rewind.stage {
                _ if held => HELD_BACK,
                Stage::FirstEsc | Stage::Opening => NOT_OPENED,
                Stage::Up(_) => NOT_LISTED,
                Stage::Entering => NO_ACTIONS,
                Stage::Moving { .. } => NOT_MOVED,
                Stage::Cancelling(_) => NOT_CANCELLED,
                _ => NOT_DONE,
            };
            fail(&mut rewind, why);
        }
        Next::Wait => return false,
    }
    if rewind.failed() && rewind.note.is_none() {
        leave(tab, &mut rewind, may_write && !cancelling, now);
    }
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    if (rewind.failed() || rewind.stage == Stage::Leaving) && chat.drawer == Drawer::Closed {
        chat.drawer = Drawer::Manual;
    }
    chat.question.rewind = Some(rewind);
    true
}

/// A rewind that just stopped: with the list on screen, one Esc to close it (checked on the next
/// screen, `Stage::Leaving`); with the choices on screen, or no key allowed now, the card says
/// that the rewind screen is still open.
fn leave(tab: &mut Tab, rewind: &mut Rewind, may_write: bool, now: Instant) {
    let screen = &tab.term.screen;
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    if screens::rewind_list(screen).is_some() {
        if may_write && ready(tab) && write(tab, ESC).is_ok() {
            rewind.stage = Stage::Leaving;
            rewind.due = Some(now + STEP);
            rewind.epoch = epoch;
            return;
        }
        rewind.note = Some(LEFT_OPEN);
    } else if screens::rewind_actions(screen).is_some() {
        rewind.note = Some(LEFT_OPEN);
    }
}

fn fail(rewind: &mut Rewind, why: &'static str) {
    rewind.stage = Stage::Failed;
    rewind.error = Some(why);
    rewind.due = None;
}

/// A click in the rewind card: a choice, Cancel (Esc) or Close.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit) -> bool {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let Some(mut rewind) = chat.question.rewind.clone() else { return false };
    match hit {
        Hit::Open => {
            chat.open_drawer();
            return true;
        }
        Hit::Close => {
            if !rewind.in_flight() {
                chat.question.rewind = None;
            }
            return true;
        }
        Hit::Option(_) | Hit::Escape if rewind.choosing() => {}
        _ => return false,
    }
    let screen = &tab.term.screen;
    let shown = screens::rewind_actions(screen);
    let same = shown.as_ref().is_some_and(|shown| same_actions(shown, &rewind));
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    let key = match (hit, shown) {
        _ if !ready(tab) => Err(BLOCKED),
        (_, Some(_)) if !same => Err(ACTIONS_CHANGED),
        (Hit::Escape, Some(_)) => Ok((ESC.to_vec(), Stage::Cancelling(true))),
        (Hit::Option(target), Some(shown)) => match shown.highlight {
            _ if target >= shown.choices.len() => Err(NO_FIELD),
            Some(h) if h == target => Ok((b"\r".to_vec(), Stage::Confirming)),
            Some(h) => Ok((toward(h, target, screen), Stage::Moving { target, from: h })),
            None => Err(ACTIONS_CHANGED),
        },
        _ => Err(ACTIONS_CHANGED),
    };
    let result = key.and_then(|(key, stage)| write(tab, &key).map(|()| stage));
    match result {
        Ok(stage) => {
            let wait = if stage == Stage::Confirming { RESULT } else { STEP };
            rewind.stage = stage;
            rewind.due = Some(now + wait);
            rewind.epoch = epoch;
            rewind.error = None;
        }
        Err(why) => {
            fail(&mut rewind, why);
            leave(tab, &mut rewind, true, now);
        }
    }
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    if (rewind.failed() || rewind.stage == Stage::Leaving) && chat.drawer == Drawer::Closed {
        chat.drawer = Drawer::Manual;
    }
    chat.question.rewind = Some(rewind);
    true
}

/// The height of a rewind's card inside its padding: its head, prompt, the CLI's choices for it
/// while they show, a note and the foot.
pub(super) fn height(rewind: &Rewind) -> f32 {
    let actions = rewind.actions.as_ref().filter(|_| rewind.choosing());
    let rows = actions.map_or(0, |a| a.body.len().min(BODY_LINES + 2));
    let choices = actions.map_or(0, |a| a.choices.len());
    let list = if choices > 0 { choices as f32 * cards::CHOICE_ROW + 10.0 } else { 0.0 };
    let note = if rewind.note.is_some() { LINE } else { 0.0 };
    cards::HEAD + 10.0 + LINE + 8.0 + rows as f32 * LINE + list + note + cards::FOOT
}

impl Draw<'_> {
    /// The rewind: its prompt, then the CLI's choices for it while they show.
    pub(super) fn rewind(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some(rewind) = chat.question.rewind.clone() else { return };
        let mut y = inner.y;
        self.head(p, hits, Rect::new(inner.x, y, inner.w, cards::HEAD), "Rewind to before", ("", ""), "", false);
        y += cards::HEAD + 10.0;
        let prompt = collapse(&rewind.prompt);
        p.text(Face::Semibold, 13.5, inner.x, y + LINE / 2.0, &prompt, pal.text, inner.w);
        y += LINE + 8.0;
        if let Some(actions) = rewind.actions.as_ref().filter(|_| rewind.choosing()) {
            for row in actions.body.iter().take(BODY_LINES + 2) {
                copied_row(p, Face::Ui, 12.0, inner.x, y + LINE / 2.0, row, pal.muted, inner.w);
                y += LINE;
            }
            let rows: Vec<cards::Choice> = actions
                .choices
                .iter()
                .enumerate()
                .map(|(i, choice)| cards::Choice {
                    number: Some(i + 1),
                    label: choice.clone(),
                    note: String::new(),
                    hit: Hit::Option(i),
                    on: false,
                })
                .collect();
            if !rows.is_empty() {
                y += self.choices(p, hits, inner.x, y, inner.w, &rows, true, cards::Density::Plain) + 10.0;
            }
        }
        if let Some(note) = rewind.note {
            p.text(Face::Ui, 12.0, inner.x, y + LINE / 2.0, note, pal.muted, inner.w);
            y += LINE;
        }
        let status =
            rewind.error.or_else(|| rewind.in_flight().then_some(REWINDING)).or(self.typed(chat)).unwrap_or("");
        let mut right = Vec::new();
        if rewind.failed() {
            right.extend([("Open terminal", Hit::Open), ("Close", Hit::Close)]);
        }
        if rewind.choosing() {
            right.push(("Cancel (Esc)", Hit::Escape));
        }
        self.foot(p, hits, Rect::new(inner.x, y, inner.w, cards::FOOT), &[], status, &right);
    }

    /// The hint a card shows once a key went nowhere while it stands in the composer's place.
    fn typed(&self, chat: &Chat) -> Option<&'static str> {
        chat.question.typed_hint()
    }
}

#[cfg(test)]
impl Rewind {
    /// A rewind whose Enter went on `prompt`'s row: the next screen decides.
    pub(super) fn entering(prompt: &str) -> Rewind {
        Rewind {
            prompt: prompt.into(),
            later: Vec::new(),
            stage: Stage::Entering,
            actions: None,
            error: None,
            note: None,
            due: None,
            epoch: u64::MAX,
        }
    }
}
