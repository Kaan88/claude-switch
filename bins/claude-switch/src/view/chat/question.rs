//! Claude's questions and permission prompts, answered from Chat. What is asked comes from the
//! transcript (the `AskUserQuestion` input, or the pending tool call); a permission prompt's choices
//! come from the screen's own menu. A key is written only while the terminal shows that same prompt
//! for that same call. The highlight is moved with the arrow keys and Enter is written only after
//! the screen shows it on the chosen row; a typed answer or comment must be visible before Enter.
//! Nothing is pre-selected and nothing is answered without a click. Anything unexpected fails
//! closed, says why and opens the drawer. Screen patterns live in `screens`.
pub(super) mod screens;

use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ItemId, ItemKind, Question, ToolCard, ToolState};
use mdshared_term::Screen;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::textfield::{FieldColors, TextField};
use mdshared_ui::theme::Palette;

use super::{Chat, Control, Drawer};
use crate::state::Target;
use crate::tabs::Tab;
use crate::view::model::Model;
use screens::collapse;

const STEP: Duration = Duration::from_secs(2);
const RESULT: Duration = Duration::from_secs(8);
const MAX_TEXT: usize = 2000;

const NOT_SHOWN: &str = "The terminal is not showing this question, so nothing was sent. Answer it in the terminal.";
const NOT_SHOWN_TOOL: &str =
    "The terminal is not showing this request for this tool call, so nothing was sent. Answer it in the terminal.";
const NOT_MOVED: &str = "The terminal did not move to that choice, so nothing was chosen. Check the terminal.";
const CHANGED: &str = "The prompt changed in the terminal. Check the terminal.";
const NOT_TYPED: &str = "The text did not appear in the terminal's prompt; Enter was not pressed. Check the terminal.";
const NO_COMMENT: &str = "The terminal did not open its comment field; nothing was typed. Check the terminal.";
const NOT_CLOSED: &str = "The answer was sent, but the prompt is still open. Check the terminal.";
const TERMINAL_ONLY: &str = "Answer this in the terminal.";
const NO_FIELD: &str = "The terminal's prompt has no such choice. Answer it in the terminal.";
const BAD_TEXT: &str = "Type a short answer without control characters.";
const BLOCKED: &str = "Chat cannot answer now. Check the terminal.";
pub(super) const WAITING: &str = "Sending your answer to the terminal…";
const ASKING: &str = "Waiting for your answer";
const APPROVING: &str = "Waiting for your approval";
const ESC: &[u8] = b"\x1b";
const NO_ESC: &str = "This menu does not offer Esc. Use the terminal.";
const CTRL_B: &[u8] = b"\x02";
const NOT_BACKGROUND: &str = "The command did not move to the background. Check the terminal.";
const NO_HINT: &str = "The terminal no longer offers to run this command in the background.";

/// What the pending tool call asks for.
#[derive(Clone, Copy)]
pub(super) enum Kind<'a> {
    /// `AskUserQuestion`; `None` when its input could not be read.
    Question(Option<&'a [Question]>),
    /// Any other tool, which may wait for a permission prompt.
    Tool(&'a ItemKind),
}

/// The pending tool call of the main conversation.
pub(super) struct Pending<'a> {
    pub item: ItemId,
    pub kind: Kind<'a>,
    /// No other tool of the turn is pending: a prompt on screen can only be for this one.
    pub alone: bool,
}

pub(super) fn pending(model: &Model) -> Option<Pending<'_>> {
    let at = model.chat.pending.filter(|_| model.agent.is_none())?;
    let running = |item: &claude_switch_core::transcript::Item| {
        matches!(item.kind, ItemKind::Tool { state: ToolState::Running, .. })
    };
    let turn = model.chat.prompt.map_or(0, |p| p.saturating_add(1));
    let alone = model.items.range(ItemId { offset: turn, block: 0 }..).filter(|(_, item)| running(item)).count() == 1;
    let range = ItemId { offset: at, block: 0 }..ItemId { offset: at.saturating_add(1), block: 0 };
    model.items.range(range).rev().find_map(|(id, item)| match &item.kind {
        ItemKind::Tool { name, state: ToolState::Running, card, .. } => {
            let kind = if name == "AskUserQuestion" {
                Kind::Question(match card {
                    Some(ToolCard::Questions(questions)) => Some(questions.as_slice()),
                    _ => None,
                })
            } else {
                Kind::Tool(&item.kind)
            };
            Some(Pending { item: *id, kind, alone })
        }
        _ => None,
    })
}

/// What the working line says while Claude waits for the user instead of working.
pub(in crate::view) fn waiting_label(model: &Model, screen: &Screen) -> Option<&'static str> {
    match pending(model)?.kind {
        Kind::Question(_) => Some(ASKING),
        Kind::Tool(kind) if is_plan(kind) => screens::plan(screen).map(|_| APPROVING),
        Kind::Tool(_) => crate::attention::screen_permission_prompt(screen).then_some(APPROVING),
    }
}

/// One single-choice question is what Chat can answer; the rest stays in the terminal.
pub(super) fn answerable(questions: &[Question]) -> bool {
    matches!(questions, [q] if !q.multi_select)
}

/// `ExitPlanMode`: the plan approval menu instead of a permission prompt.
fn is_plan(kind: &ItemKind) -> bool {
    matches!(kind, ItemKind::Tool { name, .. } if name == "ExitPlanMode")
}

/// Texts that the screen's prompt must show for this tool call: the command, the file, the
/// MCP tool or the address. Empty when the call cannot be told apart.
fn needles(kind: &ItemKind) -> Vec<String> {
    let ItemKind::Tool { name, summary, file_path, card, .. } = kind else { return Vec::new() };
    let first = |text: &str, n: usize| {
        collapse(text.lines().find(|l| !l.trim().is_empty()).unwrap_or("")).chars().take(n).collect::<String>()
    };
    let needle = match (name.as_str(), card) {
        // Subagents' own prompts belong to their tools, not to the pending Agent call.
        ("Agent" | "Task", _) => String::new(),
        (_, Some(ToolCard::Shell { command, .. })) => first(command, 40),
        _ if file_path.is_some() => {
            let path = file_path.as_deref().unwrap_or_default();
            path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
        }
        _ if name.starts_with("mcp__") => name.rsplit("__").next().unwrap_or_default().to_owned(),
        ("WebFetch", _) => {
            let url = summary.split_once("://").map_or(summary.as_str(), |(_, rest)| rest);
            url.split(['/', '?', '#']).next().unwrap_or_default().to_owned()
        }
        _ => first(summary, 30),
    };
    if needle.trim().is_empty() { Vec::new() } else { vec![needle] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Option(usize),
    Field,
    /// Answer a question with the field's text ("Type something").
    Answer,
    ChatAbout,
    /// Decline a permission prompt, with the field's text as what to do instead.
    Deny,
    Open,
    /// Ctrl+B for the running shell command.
    Background,
    /// Esc: leave the menu, or decline the question or prompt.
    Escape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Then {
    Choose,
    /// Type on the target row, then Enter.
    Type(String),
    /// Choose the row, then put the text in the composer for the next message.
    Chat(String),
    /// Tab to open the prompt's comment field on the row, type, then Enter.
    Amend(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Moving,
    Opening,
    Typing,
    Sent,
}

/// What a step must find on the screen to go on.
#[derive(Clone, Debug)]
enum Subject {
    Question(Question),
    Tool {
        needles: Vec<String>,
        choices: Vec<String>,
    },
    Plan {
        choices: Vec<String>,
    },
    /// Any other CLI menu: its title row and its choices.
    Menu {
        title: String,
        choices: Vec<String>,
    },
    /// A menu Chat cannot read: its rows as shown.
    Panel {
        rows: Vec<String>,
    },
}

#[derive(Clone, Debug)]
struct Step {
    /// The pending call the step answers; `None` for a CLI menu.
    item: Option<ItemId>,
    subject: Subject,
    /// The menu entry, counted from 0.
    target: usize,
    then: Then,
    stage: Stage,
    due: Instant,
    epoch: u64,
}

/// A CLI menu, picker or dialog as last read from the screen, for the card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MenuShown {
    title: String,
    /// Its numbered choices; empty when Chat cannot read them.
    choices: Vec<String>,
    /// Its rows, shown read-only when there are no choices.
    rows: Vec<String>,
    /// Its footer offers Esc.
    pub(super) esc: bool,
}

fn menu_shown(screen: &Screen) -> Option<MenuShown> {
    let panel = screens::panel(screen)?;
    let choices = panel.menu.as_ref().map(screens::Menu::texts).unwrap_or_default();
    Some(MenuShown { title: panel.title, choices, rows: panel.rows, esc: panel.esc })
}

/// A permission prompt as last read from the screen, for the card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Shown {
    question: String,
    body: Vec<String>,
    choices: Vec<String>,
    /// The decline choice for a comment, and whether the comment is the CLI's Tab field.
    deny: Option<usize>,
    amend: bool,
    /// The prompt names this tool call.
    matched: bool,
}

#[derive(Default)]
pub(super) struct Card {
    pub(super) field: TextField,
    pub(super) focus: bool,
    step: Option<Step>,
    error: Option<&'static str>,
    item: Option<ItemId>,
    failed: bool,
    rect: Rect,
    shown: Option<Shown>,
    /// A CLI menu with no question or permission card for it.
    pub(super) menu: Option<MenuShown>,
    /// The running shell call the CLI offers to move to the background, with its command.
    background: Option<(ItemId, String)>,
    /// A Ctrl+B in flight: for which call, until when, and the output it was written after.
    background_step: Option<(ItemId, Instant, u64)>,
    /// The pending call's offset while Chat may answer it: its drawer stays closed.
    pub(super) answerable: Option<u64>,
    /// The card answers a permission prompt, not a question.
    pub(super) approval: bool,
}

impl Card {
    /// The card has room in the conversation.
    pub(super) fn visible(&self) -> bool {
        self.rect.h > 0.0
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.step.as_ref().map(|step| step.due).or(self.background_step.map(|(_, due, _)| due))
    }
}

/// The screen's menu for `subject`, with the question's "Type something" and "Chat about this"
/// rows. `typing` is the entry whose text may differ because an answer is being typed on it.
fn read(
    subject: &Subject,
    screen: &Screen,
    typing: Option<usize>,
) -> Option<(screens::Menu, Option<usize>, Option<usize>)> {
    match subject {
        Subject::Question(question) => question_menu(screen, question, typing.is_some()),
        Subject::Tool { needles, choices } => {
            let prompt = screens::permission(screen)?;
            let texts = prompt.menu.texts();
            let same = texts.len() == choices.len()
                && texts.iter().zip(choices).enumerate().all(|(i, (a, b))| a == b || typing == Some(i));
            (same && names(&prompt, needles)).then_some((prompt.menu, None, None))
        }
        Subject::Plan { choices } => {
            let menu = screens::plan(screen)?;
            let texts = menu.texts();
            let same = texts.len() == choices.len()
                && texts.iter().zip(choices).enumerate().all(|(i, (a, b))| a == b || typing == Some(i));
            same.then_some((menu, None, None))
        }
        Subject::Menu { title, choices } => {
            let panel = screens::panel(screen)?;
            let menu = panel.menu?;
            (panel.title == *title && menu.texts() == *choices).then_some((menu, None, None))
        }
        Subject::Panel { rows } => {
            let panel = screens::panel(screen)?;
            let empty = screens::Menu { entries: Vec::new(), lines: Vec::new(), footer: 0 };
            (panel.menu.is_none() && panel.rows == *rows).then_some((empty, None, None))
        }
    }
}

fn names(prompt: &screens::Permission, needles: &[String]) -> bool {
    let text = collapse(&format!("{} {}", prompt.body.join(" "), prompt.question));
    !needles.is_empty() && needles.iter().all(|n| text.contains(n.as_str()))
}

fn label_matches(text: &str, label: &str) -> bool {
    let after = |long: &str, short: &str| long.strip_prefix(short).is_some_and(|rest| rest.starts_with(' '));
    // The label alone, the label with its description on the same row, or a long label wrapped
    // at a space (its first row only).
    text == label || after(text, label) || text.chars().count() >= 12 && after(label, text)
}

fn question_menu(
    screen: &Screen,
    question: &Question,
    typed: bool,
) -> Option<(screens::Menu, Option<usize>, Option<usize>)> {
    if crate::attention::screen_permission_prompt(screen) {
        return None;
    }
    let menu = screens::menu(screen)?;
    let options = question.options.len();
    if menu.entries.len() < options
        || !menu.entries.iter().zip(&question.options).all(|(e, o)| label_matches(&e.text, &collapse(&o.label)))
    {
        return None;
    }
    let (mut other, mut chat) = (None, None);
    for (i, entry) in menu.entries.iter().enumerate().skip(options) {
        if entry.text.starts_with(screens::CHAT_ABOUT) && chat.is_none() {
            chat = Some(i);
        } else if other.is_none() && chat.is_none() && (typed || entry.text.starts_with(screens::OTHER)) {
            other = Some(i);
        } else {
            return None;
        }
    }
    collapse(&menu.above(15).join(" ")).contains(&collapse(&question.question)).then_some((menu, other, chat))
}

#[cfg(test)]
fn shown(screen: &Screen, question: &Question) -> Option<screens::Menu> {
    question_menu(screen, question, false).map(|(menu, ..)| menu)
}

/// The arrow keys that move the menu's highlight from `from` to `to`.
fn moves(from: usize, to: usize, screen: &Screen) -> Vec<u8> {
    let key = if to > from { mdshared_term::Key::Down } else { mdshared_term::Key::Up };
    let one = mdshared_term::input::encode_key(key, mdshared_term::Mods::NONE, &screen.modes());
    one.repeat(from.abs_diff(to))
}

fn answer_text(field: &TextField) -> Result<String, &'static str> {
    let text = collapse(field.text());
    if text.is_empty() || text.chars().count() > MAX_TEXT || text.chars().any(char::is_control) {
        return Err(BAD_TEXT);
    }
    Ok(text)
}

fn write(tab: &mut Tab, bytes: &[u8]) -> Result<(), &'static str> {
    let chat = &mut tab.pane.as_mut().expect("Chat pane").chat;
    #[cfg(test)]
    chat.writes.push(bytes.to_vec());
    if !chat.demo {
        let pty =
            tab.term.pty.as_ref().filter(|p| tab.term.ended.is_none() && p.running()).ok_or("The session ended.")?;
        pty.write(bytes).map_err(|_| "The terminal write failed. Check the terminal.")?;
    }
    tab.term.scroll = 0;
    tab.term.selection = None;
    Ok(())
}

fn tool_prompt(kind: &ItemKind, alone: bool, screen: &Screen) -> Option<Shown> {
    if is_plan(kind) {
        // Only the plan's own menu: the plan itself is in the conversation, from the transcript.
        let menu = screens::plan(screen)?;
        let amend = menu.footer_text().contains(screens::AMEND);
        return Some(Shown {
            matched: alone,
            question: screens::PLAN_ASKS.to_owned(),
            body: Vec::new(),
            choices: menu.texts(),
            deny: menu.entries.iter().position(|e| e.text.contains(screens::PLAN_KEEP)),
            amend,
        });
    }
    let prompt = screens::permission(screen)?;
    let needles = needles(kind);
    let deny = prompt.menu.entries.iter().rposition(|e| e.text.starts_with("No"));
    let amend = prompt.menu.footer_text().contains(screens::AMEND);
    Some(Shown {
        matched: alone && names(&prompt, &needles),
        question: prompt.question,
        body: prompt.body,
        choices: prompt.menu.texts(),
        deny: if amend { deny } else { prompt.tell },
        amend,
    })
}

impl Chat {
    fn question_failed(&mut self, error: &'static str) {
        self.question.step = None;
        self.question.error = Some(error);
        self.question.failed = true;
        self.question.answerable = None;
        if self.drawer == Drawer::Closed {
            self.drawer = Drawer::Manual;
        }
    }

    /// Follows the transcript's pending call and the screen's prompt; true when the card changed.
    fn sync_question(&mut self, model: &Model, screen: &Screen) -> bool {
        let pending = pending(model);
        let item = pending.as_ref().map(|p| p.item);
        let mut changed = false;
        if self.question.item != item {
            if let Some(Step { then: Then::Chat(text), stage: Stage::Sent, .. }) = &self.question.step
                && !text.is_empty()
                && self.editor.text.trim().is_empty()
            {
                // "Chat about this" and "No, and tell Claude" return to the prompt; the text
                // goes as the next message.
                self.editor.set(text.clone());
            }
            let field = std::mem::take(&mut self.question.field);
            self.question = Card { item, field, ..Card::default() };
            if item.is_none() {
                self.question.field.clear();
            }
            changed = true;
        }
        let shown = match pending.as_ref().map(|p| p.kind) {
            Some(Kind::Tool(kind)) => tool_prompt(kind, pending.as_ref().is_some_and(|p| p.alone), screen),
            _ => None,
        };
        changed |= self.question.shown != shown;
        self.question.approval = shown.is_some();
        let answerable = pending.as_ref().filter(|_| !self.question.failed).filter(|p| match p.kind {
            Kind::Question(questions) => questions.is_some_and(answerable),
            Kind::Tool(_) => shown.as_ref().is_some_and(|s| s.matched),
        });
        let answerable = answerable.map(|p| p.item.offset);
        // Any other menu the screen shows, unless a question or prompt card already covers it.
        let menu = (shown.is_none() && !matches!(pending.as_ref().map(|p| p.kind), Some(Kind::Question(_))))
            .then(|| menu_shown(screen))
            .flatten();
        changed |= self.question.menu != menu;
        let background = pending.as_ref().filter(|p| p.alone && shown.is_none() && menu.is_none()).and_then(|p| {
            let Kind::Tool(ItemKind::Tool { card: Some(ToolCard::Shell { command, .. }), .. }) = p.kind else {
                return None;
            };
            screens::background_hint(screen).then(|| (p.item, collapse(command.lines().next().unwrap_or_default())))
        });
        changed |= self.question.background != background;
        self.question.background = background;
        self.question.menu = menu;
        self.question.shown = shown;
        changed |= std::mem::replace(&mut self.question.answerable, answerable) != answerable;
        changed
    }

    pub(super) fn question_notice(&self) -> Option<&'static str> {
        self.question.step.is_some().then_some(WAITING)
    }
}

fn ready(tab: &Tab) -> bool {
    let pane = tab.pane.as_ref().expect("Chat pane");
    !pane.chat.identity_unverified
        && pane.model.error.is_none()
        && tab.term.ended.is_none()
        && (pane.chat.demo || tab.term.running())
}

/// A click in the card. Writes nothing unless the screen shows the same prompt.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit) -> bool {
    let pane = tab.pane.as_mut().expect("Chat pane");
    pane.chat.sync_question(&pane.model, &tab.term.screen);
    let chat = &mut pane.chat;
    match hit {
        Hit::Field => {
            chat.question.focus = true;
            return true;
        }
        Hit::Open => {
            chat.open_drawer();
            return true;
        }
        _ if chat.question.step.is_some() || chat.locked() => return false,
        _ => {}
    }
    if hit == Hit::Background {
        return press_background(tab, now);
    }
    if let Some(menu) = chat.question.menu.clone() {
        return press_menu(tab, now, hit, menu);
    }
    let Some(Pending { item, kind, alone }) = pending(&pane.model) else { return false };
    let subject = match kind {
        Kind::Question(questions) => match questions.filter(|q| answerable(q)).and_then(<[Question]>::first) {
            Some(q) => Subject::Question(q.clone()),
            None => {
                pane.chat.question_failed(TERMINAL_ONLY);
                return true;
            }
        },
        Kind::Tool(kind) => {
            let needles = needles(kind);
            match tool_prompt(kind, alone, &tab.term.screen).filter(|s| s.matched) {
                Some(shown) if is_plan(kind) => Subject::Plan { choices: shown.choices },
                Some(shown) => Subject::Tool { needles, choices: shown.choices },
                None => {
                    pane.chat.question_failed(NOT_SHOWN_TOOL);
                    return true;
                }
            }
        }
    };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let Some((menu, other, chat_row)) = read(&subject, &tab.term.screen, None) else {
        let error = if matches!(subject, Subject::Question(_)) { NOT_SHOWN } else { NOT_SHOWN_TOOL };
        tab.pane.as_mut().unwrap().chat.question_failed(error);
        return true;
    };
    if hit == Hit::Escape {
        return escape(tab, now, Some(item), subject);
    }
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let shown = chat.question.shown.clone().unwrap_or_default();
    let text = collapse(chat.question.field.text());
    let (target, then) = match (&subject, hit) {
        (Subject::Question(q), Hit::Option(i)) if i < q.options.len() => (Some(i), Then::Choose),
        (Subject::Tool { choices, .. } | Subject::Plan { choices }, Hit::Option(i)) if i < choices.len() => {
            (Some(i), Then::Choose)
        }
        (Subject::Question(_), Hit::Answer) => match answer_text(&chat.question.field) {
            Ok(text) => (other, Then::Type(text)),
            Err(error) => {
                chat.question.error = Some(error);
                return true;
            }
        },
        (Subject::Question(_), Hit::ChatAbout) => (chat_row, Then::Chat(text)),
        (Subject::Tool { .. } | Subject::Plan { .. }, Hit::Deny) if text.is_empty() => (shown.deny, Then::Choose),
        (Subject::Tool { .. } | Subject::Plan { .. }, Hit::Deny) => match answer_text(&chat.question.field) {
            Ok(text) if shown.amend => (shown.deny, Then::Amend(text)),
            Ok(text) => (shown.deny, Then::Chat(text)),
            Err(error) => {
                chat.question.error = Some(error);
                return true;
            }
        },
        _ => return false,
    };
    let Some(target) = target else {
        chat.question_failed(NO_FIELD);
        return true;
    };
    let Some(from) = menu.highlight() else {
        chat.question_failed(CHANGED);
        return true;
    };
    chat.question.error = None;
    chat.question.focus = false;
    let epoch = chat.output_epoch;
    let step = Step { item: Some(item), subject, target, then, stage: Stage::Moving, due: now + STEP, epoch };
    start(tab, now, step, from)
}

/// Writes the first keys of a step whose menu shows its highlight on `from`.
fn start(tab: &mut Tab, now: Instant, mut step: Step, from: usize) -> bool {
    let target = step.target;
    let result =
        if from == target { arrived(tab, &mut step, now) } else { write(tab, &moves(from, target, &tab.term.screen)) };
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match result {
        Ok(()) => chat.question.step = Some(step),
        Err(error) => chat.question_failed(error),
    }
    true
}

/// Esc while the screen shows the same question, prompt or menu; then the step waits for it to close.
fn escape(tab: &mut Tab, now: Instant, item: Option<ItemId>, subject: Subject) -> bool {
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    if read(&subject, &tab.term.screen, None).is_none() {
        tab.pane.as_mut().unwrap().chat.question_failed(CHANGED);
        return true;
    }
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    let step = Step { item, subject, target: 0, then: Then::Choose, stage: Stage::Sent, due: now + RESULT, epoch };
    match write(tab, ESC) {
        Ok(()) => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.error = None;
            chat.question.focus = false;
            chat.question.step = Some(step);
        }
        Err(error) => tab.pane.as_mut().unwrap().chat.question_failed(error),
    }
    true
}

/// Ctrl+B, only while the CLI still offers it for the same call, which alone is pending.
fn press_background(tab: &mut Tab, now: Instant) -> bool {
    let chat = &tab.pane.as_ref().unwrap().chat;
    if chat.question.background_step.is_some() {
        return false;
    }
    let Some((item, _)) = chat.question.background.clone() else {
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
            chat.question.background_step = Some((item, now + STEP, epoch));
        }
        Err(error) => tab.pane.as_mut().unwrap().chat.question_failed(error),
    }
    true
}

/// Ends a Ctrl+B in flight: done once newer output no longer shows the hint or the call ended.
fn advance_background(tab: &mut Tab, now: Instant) -> bool {
    let pane = tab.pane.as_mut().unwrap();
    let Some((item, due, epoch)) = pane.chat.question.background_step else { return false };
    let ended = pending(&pane.model).is_none_or(|p| p.item != item);
    let moved = pane.chat.output_epoch != epoch && !screens::background_hint(&tab.term.screen);
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

/// A click in a CLI menu's card: the same checks, against the menu's title and choices.
fn press_menu(tab: &mut Tab, now: Instant, hit: Hit, shown: MenuShown) -> bool {
    if hit == Hit::Escape {
        if !shown.esc {
            tab.pane.as_mut().unwrap().chat.question_failed(NO_ESC);
            return true;
        }
        let subject = if shown.choices.is_empty() {
            Subject::Panel { rows: shown.rows }
        } else {
            Subject::Menu { title: shown.title, choices: shown.choices }
        };
        return escape(tab, now, None, subject);
    }
    let Hit::Option(target) = hit else { return false };
    if target >= shown.choices.len() {
        return false;
    }
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let subject = Subject::Menu { title: shown.title, choices: shown.choices };
    let from = read(&subject, &tab.term.screen, None).and_then(|(menu, ..)| menu.highlight());
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let Some(from) = from else {
        chat.question_failed(CHANGED);
        return true;
    };
    chat.question.error = None;
    let epoch = chat.output_epoch;
    let step = Step { item: None, subject, target, then: Then::Choose, stage: Stage::Moving, due: now + STEP, epoch };
    start(tab, now, step, from)
}

/// The highlight is on the target: choose it, open its comment field, or type first.
fn arrived(tab: &mut Tab, step: &mut Step, now: Instant) -> Result<(), &'static str> {
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    match &step.then {
        Then::Type(text) => {
            write(tab, text.as_bytes())?;
            step.stage = Stage::Typing;
            step.due = now + STEP;
        }
        Then::Amend(_) => {
            write(tab, b"\t")?;
            step.stage = Stage::Opening;
            step.due = now + STEP;
        }
        Then::Choose | Then::Chat(_) => {
            write(tab, b"\r")?;
            step.stage = Stage::Sent;
            step.due = now + RESULT;
        }
    }
    step.epoch = epoch;
    Ok(())
}

/// Moves an answer in flight on; true when the card changed.
pub(super) fn advance(tab: &mut Tab, now: Instant, may_write: bool) -> bool {
    let pane = tab.pane.as_mut().expect("Chat pane");
    let changed = pane.chat.sync_question(&pane.model, &tab.term.screen) | advance_background(tab, now);
    let pane = tab.pane.as_mut().expect("Chat pane");
    let Some(mut step) = pane.chat.question.step.clone() else { return changed };
    if step.item.is_some() && pending(&pane.model).map(|p| p.item) != step.item {
        // The transcript resolved or replaced the call; `sync_question` cleared the step.
        return changed;
    }
    let fresh = pane.chat.output_epoch != step.epoch;
    let action =
        if !ready(tab) { Action::Fail(BLOCKED) } else { look(&tab.term.screen, &step, now, may_write && fresh) };
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    let result = match action {
        Action::Wait => return changed,
        Action::Fail(error) => Err(error),
        Action::Closed => {
            tab.pane.as_mut().unwrap().chat.question.step = None;
            return true;
        }
        Action::Arrive => arrived(tab, &mut step, now),
        Action::Type(text) => write(tab, text.as_bytes()).map(|()| {
            step.stage = Stage::Typing;
            step.due = now + STEP;
            step.epoch = epoch;
        }),
        Action::Enter => write(tab, b"\r").map(|()| {
            step.stage = Stage::Sent;
            step.due = now + RESULT;
        }),
    };
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match result {
        Ok(()) => chat.question.step = Some(step),
        Err(error) => chat.question_failed(error),
    }
    true
}

enum Action {
    Wait,
    Fail(&'static str),
    /// The answer went and the deadline passed with the prompt gone from the screen.
    Closed,
    Arrive,
    Type(String),
    Enter,
}

/// What the screen says about an answer in flight. `may_act`: the window allows writes and the
/// screen changed since the last key.
fn look(screen: &Screen, step: &Step, now: Instant, may_act: bool) -> Action {
    let late = now >= step.due;
    let typed_text = match &step.then {
        Then::Type(text) | Then::Amend(text) => Some(text.as_str()),
        Then::Choose | Then::Chat(_) => None,
    };
    match step.stage {
        // A menu that moved on (closed, or opened the next one) is done at once.
        Stage::Sent
            if may_act
                && matches!(step.subject, Subject::Menu { .. } | Subject::Panel { .. })
                && read(&step.subject, screen, None).is_none() =>
        {
            Action::Closed
        }
        Stage::Sent if late && read(&step.subject, screen, Some(step.target)).is_some() => Action::Fail(NOT_CLOSED),
        Stage::Sent if late => Action::Closed,
        Stage::Sent => Action::Wait,
        Stage::Moving => match read(&step.subject, screen, None) {
            Some((menu, ..)) if may_act && menu.highlight() == Some(step.target) => Action::Arrive,
            Some(_) if late => Action::Fail(NOT_MOVED),
            None if late => Action::Fail(CHANGED),
            _ => Action::Wait,
        },
        Stage::Opening => match read(&step.subject, screen, Some(step.target)) {
            Some((menu, ..)) if may_act && menu.highlight() == Some(step.target) => {
                Action::Type(typed_text.unwrap_or_default().to_owned())
            }
            _ if late => Action::Fail(NO_COMMENT),
            _ => Action::Wait,
        },
        Stage::Typing => {
            let text = typed_text.unwrap_or_default();
            let typed = read(&step.subject, screen, Some(step.target)).is_some_and(|(menu, ..)| {
                menu.highlight() == Some(step.target)
                    && menu.rows_from(step.target).is_some_and(|rows| rows.contains(text))
            });
            if typed && may_act {
                Action::Enter
            } else if late {
                Action::Fail(NOT_TYPED)
            } else {
                Action::Wait
            }
        }
    }
}

/// A key while the card's field has the keyboard. Enter answers, Escape leaves the field.
pub(super) fn key(tab: &mut Tab, now: Instant, name: &KeyName, mods: Mods, text: Option<&str>) -> bool {
    let chat = &mut tab.pane.as_mut().expect("Chat pane").chat;
    if !chat.question.focus {
        return false;
    }
    match name {
        KeyName::Enter if mods.none() => {
            let hit = if chat.question.approval { Hit::Deny } else { Hit::Answer };
            press(tab, now, hit);
        }
        KeyName::Escape => chat.question.focus = false,
        _ => {
            if chat.question.field.handle_key(name, mods, text, true).is_some() {
                chat.question.error = None;
            }
        }
    }
    true
}

pub(super) fn text(chat: &mut Chat, text: &str) -> bool {
    if !chat.question.focus {
        return false;
    }
    chat.question.field.insert(text);
    true
}

const PAD: f32 = 12.0;
const HEADER: f32 = 18.0;
const QUESTION: f32 = 38.0;
const OPTION: f32 = 38.0;
const CHOICE: f32 = 28.0;
const LINE: f32 = 16.0;
const FIELD: f32 = 30.0;
const STATUS: f32 = 22.0;
const DETAIL_LINES: usize = 6;
const BODY_LINES: usize = 4;
const MENU_LINES: usize = 10;

/// The tool call as the transcript has it, a few lines for the card.
fn detail(kind: &ItemKind) -> Vec<String> {
    let ItemKind::Tool { summary, file_path, card, .. } = kind else { return Vec::new() };
    if is_plan(kind) {
        return vec!["The plan is shown above, in the conversation.".to_owned()];
    }
    let mut lines: Vec<String> = match card {
        Some(ToolCard::Shell { command, .. }) => command.lines().map(str::to_owned).collect(),
        Some(ToolCard::Edit { path, added, removed, lines }) => {
            std::iter::once(format!("{path}  +{added} −{removed}")).chain(lines.iter().cloned()).collect()
        }
        _ => file_path.iter().cloned().chain(std::iter::once(summary.clone()).filter(|s| !s.is_empty())).collect(),
    };
    lines.truncate(DETAIL_LINES);
    lines
}

fn tool_title(kind: &ItemKind) -> String {
    let ItemKind::Tool { name, .. } = kind else { return String::new() };
    if is_plan(kind) {
        return "Claude's plan is ready".to_owned();
    }
    match name.strip_prefix("mcp__").and_then(|s| s.split_once("__")) {
        Some((server, tool)) => format!("Claude wants to use {server} / {tool}"),
        None => format!("Claude wants to use {name}"),
    }
}

/// Takes the card's room from the bottom of the conversation while a question or a permission
/// prompt waits.
pub(super) fn place(chat: &mut Chat, model: &Model) {
    let wanted = match pending(model).map(|p| p.kind) {
        Some(Kind::Question(questions)) => {
            let options = questions.and_then(<[Question]>::first).map_or(0, |q| q.options.len());
            Some(HEADER + QUESTION + options as f32 * (OPTION + 4.0))
        }
        Some(Kind::Tool(kind)) => chat.question.shown.as_ref().map(|s| {
            let lines = detail(kind).len() + s.body.len().min(BODY_LINES);
            HEADER + lines as f32 * LINE + 8.0 + LINE + 6.0 + s.choices.len() as f32 * (CHOICE + 4.0)
        }),
        None => None,
    };
    let wanted = wanted.or_else(|| chat.question.background.as_ref().map(|_| HEADER + LINE + 6.0)).or_else(|| {
        chat.question.menu.as_ref().map(|m| {
            let body = if m.choices.is_empty() {
                m.rows.len().min(MENU_LINES) as f32 * LINE
            } else {
                m.choices.len() as f32 * (CHOICE + 4.0)
            };
            HEADER + LINE + 6.0 + body
        })
    });
    let Some(wanted) = wanted else {
        chat.question.rect = Rect::default();
        return;
    };
    let conversation = &mut chat.layout.conversation;
    let wanted = 2.0 * PAD + wanted + FIELD + STATUS;
    let h = wanted.min((conversation.h * 0.6).floor()).max(0.0);
    let w = (conversation.w - 24.0).clamp(0.0, 760.0);
    chat.question.rect =
        Rect::new(conversation.x + ((conversation.w - w) / 2.0).round(), conversation.bottom() - h - 8.0, w, h);
    conversation.h = (conversation.h - h - 16.0).max(0.0);
}

pub(super) struct Draw<'a> {
    pub pal: &'a Palette,
    pub hover: Option<Target>,
    pub focused: bool,
}

impl Draw<'_> {
    /// The card under the conversation; called by the Chat painter.
    pub(super) fn card(&self, p: &mut Painter, chat: &mut Chat, model: &Model, hits: &mut Vec<(Rect, Target)>) {
        let r = chat.question.rect;
        if r.h <= 0.0 {
            return;
        }
        let pal = self.pal;
        let clip = p.clip(r);
        p.frame(r, 8.0, pal.popup, pal.popup_line);
        let inner = r.inset(PAD);
        match pending(model).map(|p| p.kind) {
            Some(Kind::Question(questions)) => self.question(p, chat, inner, questions, hits),
            Some(Kind::Tool(kind)) if chat.question.shown.is_some() => self.permission(p, chat, inner, kind, hits),
            _ if chat.question.menu.is_some() => self.menu(p, chat, inner, hits),
            _ => self.background(p, chat, inner, hits),
        }
        // Esc leaves a menu or declines what the card shows, under the same checks as a choice.
        let escape = match pending(model).map(|p| p.kind) {
            _ if chat.question.step.is_some() => false,
            Some(Kind::Question(_)) => chat.question.answerable.is_some(),
            Some(Kind::Tool(_)) if chat.question.shown.is_some() => chat.question.answerable.is_some(),
            _ => chat.question.menu.as_ref().is_some_and(|m| m.esc),
        };
        if escape {
            let r = Rect::new(inner.right() - 64.0, inner.y - 2.0, 64.0, HEADER + 2.0);
            let target = Target::Chat(Control::Question(Hit::Escape));
            super::paint::button(p, pal, r, "Esc", self.hover == Some(target), false);
            hits.push((r, target));
        }
        p.restore(clip);
    }

    /// The running shell command the CLI offers to move to the background.
    fn background(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some((_, command)) = chat.question.background.clone() else { return };
        let mut y = inner.y;
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, "Running a shell command", pal.muted, inner.w);
        y += HEADER;
        p.text(Face::Mono, 11.5, inner.x, y + LINE / 2.0, &command, pal.text, inner.w);
        y += LINE + 6.0;
        let row = Rect::new(inner.x, y, inner.w, FIELD - 4.0);
        let status = chat.question.error.or_else(|| chat.question.background_step.map(|_| WAITING)).unwrap_or("");
        p.text(Face::Ui, 11.5, row.x, row.cy(), status, pal.muted, (row.w - 160.0).max(0.0));
        let r = Rect::new(row.right() - 150.0, row.y, 150.0, row.h);
        let target = Target::Chat(Control::Question(Hit::Background));
        super::paint::button(p, pal, r, "Run in background", self.hover == Some(target), false);
        hits.push((r, target));
    }

    /// A CLI menu, picker or dialog: its choices as buttons, or its rows read-only.
    fn menu(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some(menu) = chat.question.menu.clone() else { return };
        let mut y = inner.y;
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, "Claude Code menu", pal.muted, inner.w);
        y += HEADER;
        let title = if menu.title.is_empty() { "Choose in the terminal's menu" } else { menu.title.as_str() };
        p.text(Face::Semibold, 12.5, inner.x, y + LINE / 2.0, title, pal.text, inner.w);
        y += LINE + 6.0;
        if menu.choices.is_empty() {
            for row in menu.rows.iter().rev().take(MENU_LINES).rev() {
                p.text(Face::Mono, 11.0, inner.x, y + LINE / 2.0, row, pal.muted, inner.w);
                y += LINE;
            }
        }
        for (i, choice) in menu.choices.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHOICE);
            let target = Target::Chat(Control::Question(Hit::Option(i)));
            p.rbox(row, 6.0, if self.hover == Some(target) { pal.hover } else { pal.bar });
            p.text(
                Face::Semibold,
                12.0,
                row.x + 10.0,
                row.cy(),
                &format!("{}. {choice}", i + 1),
                pal.text,
                row.w - 20.0,
            );
            hits.push((row, target));
            y += CHOICE + 4.0;
        }
        let row = Rect::new(inner.x, y, inner.w, FIELD - 4.0);
        let note = if menu.choices.is_empty() { TERMINAL_ONLY } else { "" };
        let status = chat.question.error.or_else(|| chat.question_notice()).unwrap_or(note);
        p.text(Face::Ui, 11.5, row.x, row.cy(), status, pal.muted, (row.w - 130.0).max(0.0));
        let open = Rect::new(row.right() - 120.0, row.y, 120.0, row.h);
        let target = Target::Chat(Control::Question(Hit::Open));
        super::paint::button(p, pal, open, "Open terminal", self.hover == Some(target), false);
        hits.push((open, target));
    }

    fn question(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        inner: Rect,
        questions: Option<&[Question]>,
        hits: &mut Vec<(Rect, Target)>,
    ) {
        let pal = self.pal;
        let mut y = inner.y;
        let Some((questions, question)) = questions.and_then(|q| Some((q, q.first()?))) else {
            p.text(Face::Semibold, 12.5, inner.x, y + HEADER / 2.0, "Claude is asking a question", pal.text, inner.w);
            self.footer(p, chat, inner, false, hits);
            return;
        };
        let can_answer = chat.question.answerable.is_some() || chat.question.failed && answerable(questions);
        let more = questions.len().saturating_sub(1);
        let header = match (question.header.is_empty(), more) {
            (true, 0) => "Claude asks".to_owned(),
            (false, 0) => question.header.clone(),
            (true, n) => format!("Claude asks · {n} more in the terminal"),
            (false, n) => format!("{} · {n} more in the terminal", question.header),
        };
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, &header, pal.muted, inner.w);
        if question.multi_select {
            p.text_right(Face::Ui, 11.5, inner.right(), y + HEADER / 2.0, "Several answers", pal.muted);
        }
        y += HEADER;
        let text_clip = p.clip(Rect::new(inner.x, y, inner.w, QUESTION));
        p.paragraph(Face::Semibold, 13.0, inner.x, y, inner.w, &question.question, pal.text);
        p.restore(text_clip);
        y += QUESTION;
        for (i, option) in question.options.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, OPTION);
            let target = Target::Chat(Control::Question(Hit::Option(i)));
            let hot = can_answer && self.hover == Some(target);
            p.rbox(row, 6.0, if hot { pal.hover } else { pal.bar });
            let label = format!("{}. {}", i + 1, option.label);
            let cy = if option.description.is_empty() { row.cy() } else { row.y + 12.0 };
            p.text(Face::Semibold, 12.0, row.x + 10.0, cy, &label, pal.text, row.w - 20.0);
            if !option.description.is_empty() {
                let description = collapse(&option.description);
                p.text(Face::Ui, 11.0, row.x + 10.0, row.y + 27.0, &description, pal.muted, row.w - 20.0);
            }
            if can_answer {
                hits.push((row, target));
            }
            y += OPTION + 4.0;
        }
        self.footer(p, chat, Rect::new(inner.x, y, inner.w, inner.bottom() - y), can_answer, hits);
    }

    fn permission(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        inner: Rect,
        kind: &ItemKind,
        hits: &mut Vec<(Rect, Target)>,
    ) {
        let pal = self.pal;
        let Some(shown) = chat.question.shown.clone() else { return };
        let can_answer = shown.matched && (chat.question.answerable.is_some() || chat.question.failed);
        let mut y = inner.y;
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, &tool_title(kind), pal.muted, inner.w);
        y += HEADER;
        for line in detail(kind) {
            p.text(Face::Mono, 11.5, inner.x, y + LINE / 2.0, &line, pal.text, inner.w);
            y += LINE;
        }
        // The terminal's own words, including warnings such as a countdown.
        for line in shown.body.iter().rev().take(BODY_LINES).rev() {
            p.text(Face::Ui, 11.0, inner.x, y + LINE / 2.0, line, pal.muted, inner.w);
            y += LINE;
        }
        y += 8.0;
        p.text(Face::Semibold, 12.5, inner.x, y + LINE / 2.0, &shown.question, pal.text, inner.w);
        y += LINE + 6.0;
        for (i, choice) in shown.choices.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHOICE);
            let target = Target::Chat(Control::Question(Hit::Option(i)));
            // Nothing is pre-selected: the terminal's highlight is not shown as a choice.
            p.rbox(row, 6.0, if can_answer && self.hover == Some(target) { pal.hover } else { pal.bar });
            p.text(
                Face::Semibold,
                12.0,
                row.x + 10.0,
                row.cy(),
                &format!("{}. {choice}", i + 1),
                pal.text,
                row.w - 20.0,
            );
            if can_answer {
                hits.push((row, target));
            }
            y += CHOICE + 4.0;
        }
        self.footer(p, chat, Rect::new(inner.x, y, inner.w, inner.bottom() - y), can_answer, hits);
    }

    fn footer(&self, p: &mut Painter, chat: &mut Chat, area: Rect, can_answer: bool, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let button = |p: &mut Painter, hits: &mut Vec<(Rect, Target)>, r: Rect, label: &str, hit: Hit| {
            let target = Target::Chat(Control::Question(hit));
            super::paint::button(p, pal, r, label, self.hover == Some(target), false);
            hits.push((r, target));
        };
        let row = Rect::new(area.x, area.y, area.w, FIELD - 4.0);
        let approval = chat.question.approval;
        let comment = !approval || chat.question.shown.as_ref().is_some_and(|s| s.deny.is_some());
        if can_answer && comment {
            let buttons = if approval { 160.0 } else { 104.0 + 4.0 + 120.0 };
            let field = Rect::new(row.x, row.y, (row.w - buttons - 8.0).max(0.0), row.h);
            let focus = chat.question.focus && self.focused;
            p.frame(field, 5.0, pal.field, if focus { pal.accent } else { pal.line });
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            let placeholder = if approval { "Tell Claude what to do instead…" } else { "Type something…" };
            chat.question.field.draw(p, field.inset(6.0), Face::Ui, 12.0, &colors, placeholder, focus, focus);
            hits.push((field, Target::Chat(Control::Question(Hit::Field))));
            let x = field.right() + 8.0;
            if approval {
                button(p, hits, Rect::new(x, row.y, 160.0, row.h), "No, and tell Claude", Hit::Deny);
            } else {
                button(p, hits, Rect::new(x, row.y, 104.0, row.h), "Answer", Hit::Answer);
                button(p, hits, Rect::new(x + 108.0, row.y, 120.0, row.h), "Chat about this", Hit::ChatAbout);
            }
        } else if !can_answer {
            p.text(Face::Ui, 11.5, row.x, row.cy(), TERMINAL_ONLY, pal.muted, (row.w - 130.0).max(0.0));
            button(p, hits, Rect::new(row.right() - 120.0, row.y, 120.0, row.h), "Open terminal", Hit::Open);
        }
        let status = chat.question.error.or_else(|| chat.question_notice());
        if let Some(status) = status {
            p.text(Face::Ui, 11.0, area.x, row.bottom() + STATUS / 2.0 + 2.0, status, pal.muted, area.w);
        }
    }
}

#[cfg(test)]
mod tests;
