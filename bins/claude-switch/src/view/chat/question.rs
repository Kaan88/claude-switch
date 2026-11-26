//! Claude's `AskUserQuestion`, answered from Chat. The card is built from the transcript's tool
//! call, never from the screen. A key is written only while the terminal shows that same
//! question: every option row and the question text must match. The highlight is moved with
//! the arrow keys and Enter is written only after the screen shows it on the chosen row. Anything
//! else fails closed, says why and opens the drawer. Permission questions are never answered
//! here: they are not `AskUserQuestion` calls, and a screen that looks like one refuses.
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

const STEP: Duration = Duration::from_secs(2);
const RESULT: Duration = Duration::from_secs(8);
const OTHER: &str = "Type something";
const CHAT_ABOUT: &str = "Chat about this";
const MAX_TEXT: usize = 2000;
/// How far above the menu's footer its rows may start.
const MENU_ROWS: usize = 60;

const NOT_SHOWN: &str = "The terminal is not showing this question, so nothing was sent. Answer it in the terminal.";
const NOT_MOVED: &str = "The terminal did not move to that choice, so nothing was chosen. Check the terminal.";
const CHANGED: &str = "The question changed in the terminal. Check the terminal.";
const NOT_TYPED: &str =
    "The answer did not appear in the terminal's question; Enter was not pressed. Check the terminal.";
const NOT_CLOSED: &str = "The answer was sent, but the question is still open. Check the terminal.";
const TERMINAL_ONLY: &str = "Answer this question in the terminal.";
const NO_FIELD: &str = "The terminal's question has no such choice. Answer it in the terminal.";
const BAD_TEXT: &str = "Type a short answer without control characters.";
const BLOCKED: &str = "Chat cannot answer now. Check the terminal.";
pub(super) const WAITING: &str = "Sending your answer to the terminal…";

/// The pending `AskUserQuestion` of the main conversation; `questions` is `None` when the card
/// could not be built from its input.
pub(super) struct Pending<'a> {
    pub item: ItemId,
    pub questions: Option<&'a [Question]>,
}

pub(super) fn pending(model: &Model) -> Option<Pending<'_>> {
    let at = model.chat.pending.filter(|_| model.agent.is_none())?;
    let range = ItemId { offset: at, block: 0 }..ItemId { offset: at.saturating_add(1), block: 0 };
    model.items.range(range).rev().find_map(|(id, item)| match &item.kind {
        ItemKind::Tool { name, state: ToolState::Running, card, .. } if name == "AskUserQuestion" => {
            let questions = match card {
                Some(ToolCard::Questions(questions)) => Some(questions.as_slice()),
                _ => None,
            };
            Some(Pending { item: *id, questions })
        }
        _ => None,
    })
}

/// Claude is waiting for the answer to a question: the working line says so.
pub(in crate::view) fn asking(model: &Model) -> bool {
    pending(model).is_some()
}

/// One single-choice question is what Chat can answer; the rest stays in the terminal.
pub(super) fn answerable(questions: &[Question]) -> bool {
    matches!(questions, [q] if !q.multi_select)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Option(usize),
    Field,
    Answer,
    ChatAbout,
    Open,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Then {
    Choose,
    Type(String),
    Chat(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Moving,
    Typing,
    Sent,
}

#[derive(Clone, Debug)]
struct Step {
    item: ItemId,
    /// The menu entry, counted from 0: an option, then "Type something", then "Chat about this".
    target: usize,
    then: Then,
    stage: Stage,
    due: Instant,
    epoch: u64,
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
    /// The pending question's offset while Chat may answer it: its drawer stays closed.
    pub(super) answerable: Option<u64>,
}

impl Card {
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.step.as_ref().map(|step| step.due)
    }
}

/// A menu entry as the screen shows it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    row: usize,
    highlighted: bool,
    text: String,
}

/// The question as the terminal shows it, matched to its transcript call.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Menu {
    entries: Vec<Entry>,
    other: Option<usize>,
    chat: Option<usize>,
}

impl Menu {
    fn highlight(&self) -> Option<usize> {
        let mut marked = self.entries.iter().enumerate().filter(|(_, e)| e.highlighted);
        let (first, _) = marked.next()?;
        marked.next().is_none().then_some(first)
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean(row: &str) -> (bool, String) {
    let row = row.trim().trim_matches(['│', '┃', '║', '┆', '┊']).trim();
    let highlighted = row.starts_with(['❯', '>']);
    (highlighted, collapse(row.trim_start_matches(['❯', '>'])))
}

fn numbered(text: &str) -> Option<(usize, &str)> {
    let (n, rest) = text.split_once(". ")?;
    (!n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit())).then_some(())?;
    Some((n.parse().ok()?, rest))
}

fn label_matches(text: &str, label: &str) -> bool {
    let after = |long: &str, short: &str| long.strip_prefix(short).is_some_and(|rest| rest.starts_with(' '));
    // The label alone, the label with its description on the same row, or a long label wrapped
    // at a space (its first row only).
    text == label || after(text, label) || text.chars().count() >= 12 && after(label, text)
}

/// The question on the live screen, or `None`. `typed` allows the "Type something" row to
/// show the answer being typed instead of its own label.
fn shown_as(screen: &Screen, question: &Question, typed: bool) -> Option<Menu> {
    if crate::attention::screen_permission_prompt(screen) {
        return None;
    }
    let lines: Vec<_> = (0..screen.rows()).map(|row| clean(&screen.row_text(row))).collect();
    let footer = lines.iter().rposition(|(_, text)| !text.is_empty())?;
    let hint = lines[footer].1.to_lowercase();
    if !hint.contains("esc to cancel") && !hint.contains("enter to select") {
        return None;
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (row, (highlighted, text)) in lines.iter().enumerate().take(footer).skip(footer.saturating_sub(MENU_ROWS)) {
        let Some((n, rest)) = numbered(text) else { continue };
        if n == 1 {
            entries.clear();
        }
        if n == entries.len() + 1 {
            entries.push(Entry { row, highlighted: *highlighted, text: rest.to_owned() });
        } else {
            entries.clear();
        }
    }
    let options = question.options.len();
    if entries.len() < options
        || !entries
            .iter()
            .zip(&question.options)
            .all(|(entry, option)| label_matches(&entry.text, &collapse(&option.label)))
    {
        return None;
    }
    let (mut other, mut chat) = (None, None);
    for (i, entry) in entries.iter().enumerate().skip(options) {
        if entry.text.starts_with(CHAT_ABOUT) && chat.is_none() {
            chat = Some(i);
        } else if other.is_none() && chat.is_none() && (typed || entry.text.starts_with(OTHER)) {
            other = Some(i);
        } else {
            return None;
        }
    }
    let first = entries[0].row;
    let above = lines[first.saturating_sub(15)..first].iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>();
    collapse(&above.join(" ")).contains(&collapse(&question.question)).then_some(Menu { entries, other, chat })
}

pub(super) fn shown(screen: &Screen, question: &Question) -> Option<Menu> {
    shown_as(screen, question, false)
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

    /// Follows the transcript's pending question; true when the card changed.
    fn sync_question(&mut self, model: &Model) -> bool {
        let pending = pending(model);
        let item = pending.as_ref().map(|p| p.item);
        let mut changed = false;
        if self.question.item != item {
            if let Some(Step { then: Then::Chat(text), stage: Stage::Sent, .. }) = &self.question.step
                && self.editor.text.trim().is_empty()
            {
                // "Chat about this" returns to the prompt; the text goes as the next message.
                self.editor.set(text.clone());
            }
            let field = std::mem::take(&mut self.question.field);
            self.question = Card { item, field, ..Card::default() };
            if item.is_none() {
                self.question.field.clear();
            }
            changed = true;
        }
        let answerable =
            pending.filter(|p| !self.question.failed && p.questions.is_some_and(answerable)).map(|p| p.item.offset);
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

/// A click in the card. Writes nothing unless the screen shows the same question.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit) -> bool {
    let pane = tab.pane.as_mut().expect("Chat pane");
    pane.chat.sync_question(&pane.model);
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
    let Some(Pending { item, questions }) = pending(&pane.model) else { return false };
    let Some(question) = questions.filter(|q| answerable(q)).map(|q| q[0].clone()) else {
        pane.chat.question_failed(TERMINAL_ONLY);
        return true;
    };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let Some(menu) = shown(&tab.term.screen, &question) else {
        tab.pane.as_mut().unwrap().chat.question_failed(NOT_SHOWN);
        return true;
    };
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let (target, then) = match hit {
        Hit::Option(i) if i < question.options.len() => (Some(i), Then::Choose),
        Hit::Answer => match answer_text(&chat.question.field) {
            Ok(text) => (menu.other, Then::Type(text)),
            Err(error) => {
                chat.question.error = Some(error);
                return true;
            }
        },
        Hit::ChatAbout => (menu.chat, Then::Chat(collapse(chat.question.field.text()))),
        _ => return false,
    };
    let Some(target) = target else {
        chat.question_failed(NO_FIELD);
        return true;
    };
    let Some(from) = menu.highlight() else {
        chat.question_failed(NOT_SHOWN);
        return true;
    };
    chat.question.error = None;
    chat.question.focus = false;
    let epoch = chat.output_epoch;
    let mut step = Step { item, target, then, stage: Stage::Moving, due: now + STEP, epoch };
    let result =
        if from == target { arrived(tab, &mut step, now) } else { write(tab, &moves(from, target, &tab.term.screen)) };
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    match result {
        Ok(()) => chat.question.step = Some(step),
        Err(error) => chat.question_failed(error),
    }
    true
}

/// The highlight is on the target: choose it, or type the answer first.
fn arrived(tab: &mut Tab, step: &mut Step, now: Instant) -> Result<(), &'static str> {
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    match &step.then {
        Then::Type(text) => {
            write(tab, text.as_bytes())?;
            step.stage = Stage::Typing;
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
    let changed = pane.chat.sync_question(&pane.model);
    let Some(mut step) = pane.chat.question.step.clone() else { return changed };
    let question = pending(&pane.model)
        .filter(|p| p.item == step.item)
        .and_then(|p| p.questions)
        .filter(|q| answerable(q))
        .map(|q| q[0].clone());
    let Some(question) = question else {
        // The transcript resolved or replaced the question; `sync_question` cleared the step.
        return changed;
    };
    let fresh = pane.chat.output_epoch != step.epoch;
    let action = if !ready(tab) {
        Action::Fail(BLOCKED)
    } else {
        look(&tab.term.screen, &question, &step, now, may_write && fresh)
    };
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let result = match action {
        Action::Wait => return changed,
        Action::Fail(error) => Err(error),
        Action::Closed => {
            chat.question.step = None;
            return true;
        }
        Action::Arrive => arrived(tab, &mut step, now),
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
    /// The answer went and the deadline passed with the question gone from the screen.
    Closed,
    Arrive,
    Enter,
}

/// What the screen says about an answer in flight. `may_act`: the window allows writes and the
/// screen changed since the last key.
fn look(screen: &Screen, question: &Question, step: &Step, now: Instant, may_act: bool) -> Action {
    let late = now >= step.due;
    match step.stage {
        Stage::Sent if late && shown(screen, question).is_some() => Action::Fail(NOT_CLOSED),
        Stage::Sent if late => Action::Closed,
        Stage::Sent => Action::Wait,
        Stage::Moving => match shown(screen, question) {
            Some(menu) if may_act && menu.highlight() == Some(step.target) => Action::Arrive,
            Some(_) if late => Action::Fail(NOT_MOVED),
            None if late => Action::Fail(CHANGED),
            _ => Action::Wait,
        },
        Stage::Typing => {
            let Then::Type(text) = &step.then else { return Action::Fail(CHANGED) };
            let typed = shown_as(screen, question, true).is_some_and(|menu| {
                let row = menu.entries[step.target].row;
                let rows = (row..screen.rows()).map(|r| clean(&screen.row_text(r)).1).collect::<Vec<_>>();
                menu.highlight() == Some(step.target) && collapse(&rows.join(" ")).contains(text.as_str())
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
            press(tab, now, Hit::Answer);
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
const FIELD: f32 = 30.0;
const STATUS: f32 = 22.0;

/// Takes the card's room from the bottom of the conversation while a question is pending.
pub(super) fn place(chat: &mut Chat, model: &Model) {
    let Some(p) = pending(model) else {
        chat.question.rect = Rect::default();
        return;
    };
    let conversation = &mut chat.layout.conversation;
    let options = p.questions.map_or(0, |q| q[0].options.len());
    let wanted = 2.0 * PAD + HEADER + QUESTION + options as f32 * (OPTION + 4.0) + FIELD + STATUS;
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
        let Some(pending) = pending(model).filter(|_| r.h > 0.0) else { return };
        let pal = self.pal;
        let clip = p.clip(r);
        p.frame(r, 8.0, pal.popup, pal.popup_line);
        let inner = r.inset(PAD);
        let mut y = inner.y;
        let Some(questions) = pending.questions else {
            p.text(Face::Semibold, 12.5, inner.x, y + HEADER / 2.0, "Claude is asking a question", pal.text, inner.w);
            self.footer(p, chat, inner, false, hits);
            p.restore(clip);
            return;
        };
        let question = &questions[0];
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
                p.text(
                    Face::Ui,
                    11.0,
                    row.x + 10.0,
                    row.y + 27.0,
                    &collapse(&option.description),
                    pal.muted,
                    row.w - 20.0,
                );
            }
            if can_answer {
                hits.push((row, target));
            }
            y += OPTION + 4.0;
        }
        self.footer(p, chat, Rect::new(inner.x, y, inner.w, inner.bottom() - y), can_answer, hits);
        p.restore(clip);
    }

    fn footer(&self, p: &mut Painter, chat: &mut Chat, area: Rect, can_answer: bool, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let button = |p: &mut Painter, hits: &mut Vec<(Rect, Target)>, r: Rect, label: &str, hit: Hit| {
            let target = Target::Chat(Control::Question(hit));
            super::paint::button(p, pal, r, label, self.hover == Some(target), false);
            hits.push((r, target));
        };
        let row = Rect::new(area.x, area.y, area.w, FIELD - 4.0);
        if can_answer {
            let buttons = 104.0 + 4.0 + 120.0;
            let field = Rect::new(row.x, row.y, (row.w - buttons - 8.0).max(0.0), row.h);
            p.frame(field, 5.0, pal.field, if chat.question.focus && self.focused { pal.accent } else { pal.line });
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            let focus = chat.question.focus && self.focused;
            chat.question.field.draw(p, field.inset(6.0), Face::Ui, 12.0, &colors, "Type something…", focus, focus);
            hits.push((field, Target::Chat(Control::Question(Hit::Field))));
            let x = field.right() + 8.0;
            button(p, hits, Rect::new(x, row.y, 104.0, row.h), "Answer", Hit::Answer);
            button(p, hits, Rect::new(x + 108.0, row.y, 120.0, row.h), "Chat about this", Hit::ChatAbout);
        } else {
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
