//! Claude's questions and permission prompts, answered from Chat. What is asked comes from the
//! transcript (the `AskUserQuestion` input, or the pending tool call); a permission prompt's choices
//! come from the screen's own menu. A key is written only while the terminal shows that same prompt
//! for that same call. The highlight is moved with the arrow keys and Enter is written only after
//! the screen shows it on the chosen row; a typed answer or comment must be visible before Enter.
//! Nothing is pre-selected and nothing is answered without a click. Anything unexpected fails
//! closed, says why and opens the drawer. Screen patterns live in `screens`.
//!
//! A form (several questions, or a multi-select one) is picked in the card first; Submit then
//! drives the CLI's form one key at a time, each written only once the screen shows what the key
//! before should have led to (the highlight moved, the box ticked, the text typed, the next
//! question or the review opened), and "Submit answers" only once the review reads every answer
//! as picked.
mod background;
pub(in crate::view) mod rewind;
pub(super) mod screens;

use std::path::Path;
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ItemId, ItemKind, Question, ToolCard, ToolState};
use mdshared_term::Screen;
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb};
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
const SENDING: &str = "A message is on its way to the terminal. Answer once it has gone, or Cancel it.";
const QUEUED_TEXT: &str = "A message is queued, and this text would go to the composer. Cancel the queued message first, or answer without text.";
const HELD: &str = "Your text for Claude goes to the message box once it is empty.";
const DIGIT_FIRST: &str =
    "Start the answer with a word, not a digit (digits pick a choice), or answer in the terminal.";
const BLOCKED: &str = "Chat cannot answer now. Check the terminal.";
/// Card keys in flight stop when the terminal gets other input, Cancel is pressed or Chat is left.
pub(super) const INTERRUPTED: &str =
    "Chat stopped sending keys: the terminal got other input, or Chat was left or cancelled. Check the terminal.";
pub(super) const WAITING: &str = "Sending your answer to the terminal…";
const ASKING: &str = "Waiting for your answer";
const APPROVING: &str = "Waiting for your approval";
const ESC: &[u8] = b"\x1b";
const NO_ESC: &str = "This menu does not offer Esc. Use the terminal.";
const NOT_TICKED: &str = "The terminal did not tick or clear that box, so nothing more was sent. Check the terminal.";
const NO_NEXT: &str = "The terminal did not show the next question, so nothing more was sent. Check the terminal.";
const NO_REVIEW: &str =
    "The terminal did not show the review of your answers, so they were not submitted. Check the terminal.";
const REVIEW_DIFFERS: &str =
    "The terminal's review does not match your answers, so they were not submitted. Check the terminal.";
const OTHER_TAKEN: &str =
    "The terminal's \"Type something\" row already holds other text, so nothing more was sent. Check the terminal.";
const UNANSWERED: &str = "Answer this question first: pick a choice or type an answer.";
const BOTH: &str = "Pick one choice or type an answer for this question, not both.";
/// How many rows above its entries a form question's tab row and text may use.
const FORM_ROWS: usize = 20;

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
    /// No other tool of the turn is pending and no subagent may be asking: a prompt on screen can
    /// only be for this one.
    pub alone: bool,
}

pub(super) fn pending(model: &Model) -> Option<Pending<'_>> {
    let at = model.chat.pending.filter(|_| model.agent.is_none())?;
    let running = |item: &claude_switch_core::transcript::Item| {
        matches!(item.kind, ItemKind::Tool { state: ToolState::Running, .. })
    };
    let turn = model.chat.prompt.map_or(0, |p| p.saturating_add(1));
    let alone = model.items.range(ItemId { offset: turn, block: 0 }..).filter(|(_, item)| running(item)).count() == 1
        && !subagent_may_prompt(model);
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

/// One single-choice question: a click on a choice answers it.
fn simple(questions: &[Question]) -> bool {
    matches!(questions, [q] if !q.multi_select)
}

/// Several questions, or a multi-select one: the card collects every answer and one Submit
/// drives the CLI's form. Two questions worded alike could not be told apart on the screen.
fn form(questions: &[Question]) -> bool {
    !simple(questions)
        && !questions.is_empty()
        && questions
            .iter()
            .enumerate()
            .all(|(i, q)| questions[..i].iter().all(|other| squash(&other.question) != squash(&q.question)))
}

/// Chat can answer the call's questions; anything else stays in the terminal.
pub(super) fn answerable(questions: &[Question]) -> bool {
    simple(questions) || form(questions)
}

/// The text with all whitespace removed, for comparing text the screen may have wrapped.
fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `ExitPlanMode`: the plan approval menu instead of a permission prompt.
fn is_plan(kind: &ItemKind) -> bool {
    matches!(kind, ItemKind::Tool { name, .. } if name == "ExitPlanMode")
}

/// What the screen's prompt must show for a tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Needle {
    /// The whole text, as one or more whole rows of the prompt (whitespace and wrapping aside).
    Rows(String),
    /// A word of the prompt, in one of these forms: never a part of a longer word or path.
    Word(Vec<String>),
}

/// What the screen's prompt must show for this tool call: the whole command, the file's full path
/// (or its path from the session folder), the MCP server and tool, or the address. Empty when the
/// call cannot be told apart.
fn needles(kind: &ItemKind, cwd: &Path) -> Vec<Needle> {
    let ItemKind::Tool { name, summary, file_path, card, .. } = kind else { return Vec::new() };
    let needles = match (name.as_str(), card) {
        // Subagents' own prompts belong to their tools, not to the pending Agent call.
        ("Agent" | "Task", _) => Vec::new(),
        (_, Some(ToolCard::Shell { command, .. })) => vec![Needle::Rows(command.trim().to_owned())],
        _ if file_path.is_some() => {
            let path = file_path.as_deref().unwrap_or_default().trim();
            let mut forms = vec![path.to_owned()];
            if let Ok(rest) = Path::new(path).strip_prefix(cwd) {
                let rest = rest.to_string_lossy();
                forms.extend([rest.replace('\\', "/"), rest.replace('/', "\\")]);
            }
            forms.retain(|f| !f.is_empty());
            forms.dedup();
            vec![Needle::Word(forms)]
        }
        _ if name.starts_with("mcp__") => match name.trim_start_matches("mcp__").rsplit_once("__") {
            Some((server, tool)) => vec![Needle::Word(vec![server.to_owned()]), Needle::Word(vec![tool.to_owned()])],
            None => Vec::new(),
        },
        ("WebFetch", _) => {
            let url = summary.split_once("://").map_or(summary.as_str(), |(_, rest)| rest);
            let host = url.split(['/', '?', '#']).next().unwrap_or_default();
            vec![Needle::Word(vec![summary.trim().to_owned(), host.to_owned()])]
        }
        _ => vec![Needle::Rows(summary.trim().to_owned())],
    };
    let empty = |n: &Needle| match n {
        Needle::Rows(text) => text.trim().is_empty(),
        Needle::Word(forms) => forms.iter().all(|f| f.trim().is_empty()),
    };
    if needles.iter().any(empty) { Vec::new() } else { needles }
}

/// `text` holds `word` with nothing of a longer word or path right before or after it (closing
/// punctuation at the end of a sentence aside).
fn has_word(text: &str, word: &str) -> bool {
    let quote = |c: char| matches!(c, '"' | '\'' | '`' | '(' | ')');
    !word.is_empty()
        && text.match_indices(word).any(|(at, _)| {
            let before = text[..at].chars().next_back().is_none_or(|c| c.is_whitespace() || quote(c));
            let mut after = text[at + word.len()..].chars();
            let ends = match after.next() {
                None => true,
                Some(c) if c.is_whitespace() || quote(c) => true,
                Some('.' | '?' | ':' | ',' | ';') => after.next().is_none_or(char::is_whitespace),
                Some(_) => false,
            };
            before && ends
        })
}

/// Some run of whole rows reads `text`, all whitespace aside.
fn has_rows(rows: &[String], text: &str) -> bool {
    let want = squash(text);
    !want.is_empty()
        && (0..rows.len()).any(|i| {
            let mut run = String::new();
            rows[i..].iter().any(|row| {
                run.push_str(&squash(row));
                run == want
            })
        })
}

/// A subagent may be the one asking: one is running, or the turn started one whose end is unknown.
fn subagent_may_prompt(model: &Model) -> bool {
    let finished = |id: &str| model.agents.iter().any(|a| a.id == id && a.finished == Some(true));
    let turn = model.chat.prompt.map_or(0, |p| p.saturating_add(1));
    model.agents.iter().any(|a| a.finished == Some(false))
        || model.items.range(ItemId { offset: turn, block: 0 }..).any(|(_, item)| {
            matches!(&item.kind, ItemKind::Tool { name, subagent, .. }
                if (name == "Agent" || name == "Task") && !subagent.as_deref().is_some_and(finished))
        })
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
    /// Stop the background agent the card offers at this place.
    StopAgent(usize),
    /// Put away a card that stopped (a rewind's); nothing is sent.
    Close,
    /// Esc: leave the menu, or decline the question or prompt.
    Escape,
    /// A form's previous and next question in the card (nothing is sent).
    Back,
    Next,
    /// Send the form's answers.
    Submit,
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
    /// Question `at` of a form, as `form_question` reads it ("Chat about this" and Esc).
    FormQuestion {
        questions: Vec<Question>,
        at: usize,
    },
    /// A form's answers on their way, one checked key at a time.
    Form(FormRun),
    Tool {
        needles: Vec<Needle>,
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
    /// A question of Claude's: the description under each choice (empty for other menus).
    notes: Vec<String>,
    /// It is a question Claude asks, read from the screen.
    question: bool,
}

impl MenuShown {
    /// How many choices have a description to show under them.
    fn noted(&self) -> usize {
        self.notes.iter().filter(|note| !note.is_empty()).count()
    }

    /// It only shows information: no choices to pick.
    pub(super) fn panel(&self) -> bool {
        self.choices.is_empty()
    }
}

fn menu_shown(screen: &Screen) -> Option<MenuShown> {
    let panel = screens::panel(screen)?;
    let choices = panel.menu.as_ref().map(screens::Menu::texts).unwrap_or_default();
    let (notes, question) = (panel.notes, panel.question);
    Some(MenuShown { title: panel.title, choices, rows: panel.rows, esc: panel.esc, notes, question })
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
    /// The running shell call the CLI offers to move to the background.
    background: Option<background::Shell>,
    /// A Ctrl+B in flight: for which call, until when, and the output it was written after.
    background_step: Option<(background::Shell, Instant, u64)>,
    /// The background agents the CLI's list shows running, which Chat can stop.
    agents: Vec<background::AgentOffer>,
    /// A Stop agent in flight. It outlives the pending call: the main turn may move on meanwhile.
    agent_step: Option<background::AgentStep>,
    /// A rewind from a prompt bubble, from its first Esc until it is done or its card is closed.
    rewind: Option<rewind::Rewind>,
    /// The pending call's offset while Chat may answer it: its drawer stays closed.
    pub(super) answerable: Option<u64>,
    /// The card answers a permission prompt, not a question.
    pub(super) approval: bool,
    /// Text a "Chat about this" or "No, and tell Claude" answer left for the composer, until it
    /// lands there (once the composer is empty).
    pub(super) held: Option<String>,
    /// The notice a click got because other keys were on their way: it goes when they have.
    busy: Option<&'static str>,
    /// What the user picked in a form's card.
    form: Picks,
}

/// A form's answers as picked in the card, per question. Nothing reaches the terminal before
/// Submit. The field holds the text of the question on `page`; `texts` keeps the others'.
#[derive(Clone, Debug, Default)]
pub(super) struct Picks {
    page: usize,
    chosen: Vec<Vec<bool>>,
    texts: Vec<String>,
}

impl Picks {
    /// Starts over unless the picks are for these questions.
    fn fit(&mut self, questions: &[Question], field: &TextField) {
        let same = self.chosen.len() == questions.len()
            && self.chosen.iter().zip(questions).all(|(chosen, q)| chosen.len() == q.options.len());
        if !same {
            let mut texts = vec![String::new(); questions.len()];
            if let Some(first) = texts.first_mut() {
                *first = field.text().to_owned();
            }
            *self = Picks { page: 0, chosen: questions.iter().map(|q| vec![false; q.options.len()]).collect(), texts };
        }
    }
}

/// One question's answer: the chosen options, in order, and the typed text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Answer {
    picks: Vec<usize>,
    text: Option<String>,
}

/// A form being answered: every key waits for the screen to show what the last one should have
/// led to.
#[derive(Clone, Debug)]
struct FormRun {
    questions: Vec<Question>,
    answers: Vec<Answer>,
    expect: Expect,
    /// What to say when the screen does not show `expect` in time.
    why: &'static str,
}

/// What the screen must show before the next key of a form.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Expect {
    /// Question `at`, as it opens.
    Opened(usize),
    /// Question `at` with the highlight on `cursor`, these boxes and this "Type something" row.
    Shows { at: usize, cursor: usize, marks: Vec<bool>, other: String },
    /// Question `at` with `text` typed on its "Type something" row, highlighted.
    Typed { at: usize, cursor: usize, marks: Vec<bool>, text: String },
    /// The review, with the highlight on this entry (any while it opens).
    Review(Option<usize>),
    /// Enter went on "Submit answers".
    Submitted,
}

impl Card {
    /// The card has room in the conversation.
    pub(super) fn visible(&self) -> bool {
        self.rect.h > 0.0
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        let background = self.background_step.as_ref().map(|(_, due, _)| *due);
        let rewind = self.rewind.as_ref().and_then(|r| r.due);
        self.step
            .as_ref()
            .map(|step| step.due)
            .or(background)
            .or(self.agent_step.as_ref().map(|step| step.due))
            .or(rewind)
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
        Subject::FormQuestion { questions, at } => {
            form_question(screen, questions, *at).map(|shown| (shown.menu, Some(shown.other), Some(shown.chat)))
        }
        Subject::Form(_) => None,
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

fn names(prompt: &screens::Permission, needles: &[Needle]) -> bool {
    let text = collapse(&format!("{} {}", prompt.body.join(" "), prompt.question));
    !needles.is_empty()
        && needles.iter().all(|n| match n {
            Needle::Rows(want) => has_rows(&prompt.body, want),
            Needle::Word(forms) => forms.iter().any(|f| has_word(&text, &collapse(f))),
        })
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

/// One question of a form as the screen shows it. The menu's entries are the rows in the order the
/// arrow keys walk them: a multi-select's unnumbered Submit or Next row is an entry of its own,
/// after "Type something".
#[derive(Clone, Debug)]
struct FormShown {
    menu: screens::Menu,
    other: usize,
    submit: Option<usize>,
    chat: usize,
    /// The options' boxes (multi-select only), in order.
    marks: Vec<bool>,
    /// The "Type something" row with its box, the rows it wraps onto joined.
    typed: String,
}

/// Question `at` of the form, only as its own screen: a menu footer; right above the entries,
/// the tab row naming the form's questions in order, then this question's text and nothing
/// else; every option in order (with its box for a multi-select), then "Type something", the
/// multi-select's Submit or Next row, and "Chat about this".
fn form_question(screen: &Screen, questions: &[Question], at: usize) -> Option<FormShown> {
    if crate::attention::screen_permission_prompt(screen) {
        return None;
    }
    let question = questions.get(at)?;
    let mut menu = screens::menu(screen)?;
    let n = question.options.len();
    (menu.entries.len() == n + 2).then_some(())?;
    let above = menu.above_box(FORM_ROWS);
    let (tabs, asks) = above.split_first()?;
    (screens::tab_bar(tabs) && squash(&asks.join(" ")) == squash(&question.question)).then_some(())?;
    let tabs = squash(tabs);
    let mut from = 0;
    for header in questions.iter().map(|q| squash(&q.header)).filter(|h| !h.is_empty()) {
        from += tabs.get(from..)?.find(&header)? + header.len();
    }
    let multi = question.multi_select;
    let mut marks = Vec::new();
    for (entry, option) in menu.entries.iter().zip(&question.options) {
        let label = if multi {
            let (on, label) = screens::checkbox(&entry.text)?;
            marks.push(on);
            label
        } else {
            entry.text.as_str()
        };
        label_matches(label, &collapse(&option.label)).then_some(())?;
    }
    menu.entries[n + 1].text.starts_with(screens::CHAT_ABOUT).then_some(())?;
    let submit = if multi { Some(screens::submit_row(screen, &menu, n)?) } else { None };
    let other = &menu.entries[n];
    let end = submit.as_ref().map_or(menu.entries[n + 1].row, |s| s.row);
    let typed = collapse(&format!("{} {}", other.text, screens::rows_between(&menu, other.row + 1, end)));
    let fits = if multi { screens::checkbox(&typed).is_some() } else { !typed.starts_with('[') };
    fits.then_some(())?;
    if let Some(submit) = submit {
        menu.entries.insert(n + 1, submit);
    }
    let chat = menu.entries.len() - 1;
    Some(FormShown { menu, other: n, submit: multi.then_some(n + 1), chat, marks, typed })
}

/// The "Type something" row as it opens, holding no text.
fn pristine(multi: bool, row: &str) -> bool {
    if multi { row == screens::OTHER_BOX } else { row == format!("{}.", screens::OTHER) }
}

/// The "Type something" row shows `text`: in its ticked box (multi-select), or in place of the
/// row's own words (the single-choice rendering is not recorded yet, so only that it is there).
fn typed_shows(multi: bool, row: &str, text: &str) -> bool {
    if multi {
        [squash(&format!("[✔] {text}")), squash(&format!("[√] {text}"))].contains(&squash(row))
    } else {
        !pristine(false, row) && squash(row).contains(&squash(text))
    }
}

/// A question's answer as the review writes it: the chosen labels in order, then the text.
fn answer_line(question: &Question, answer: &Answer) -> String {
    let mut parts: Vec<String> = answer.picks.iter().map(|&i| collapse(&question.options[i].label)).collect();
    parts.extend(answer.text.clone());
    parts.join(", ")
}

/// The answers the card holds, checked: every question answered, a single choice by one choice
/// or a text, texts short, without control characters and not starting with a digit. `Err`
/// names the question to show and why.
fn form_answers(
    questions: &[Question],
    picks: &Picks,
    field: &TextField,
) -> Result<Vec<Answer>, (usize, &'static str)> {
    questions
        .iter()
        .enumerate()
        .map(|(k, question)| {
            let raw = if k == picks.page { field.text() } else { picks.texts.get(k).map_or("", String::as_str) };
            let text = collapse(raw);
            let text = match text {
                _ if text.is_empty() => None,
                _ if text.chars().count() > MAX_TEXT || text.chars().any(char::is_control) => {
                    return Err((k, BAD_TEXT));
                }
                // Digits pick choices in the CLI's menus.
                _ if text.starts_with(|c: char| c.is_ascii_digit()) => return Err((k, DIGIT_FIRST)),
                text => Some(text),
            };
            let chosen = picks.chosen.get(k).map_or(&[][..], Vec::as_slice);
            let picks: Vec<usize> = chosen.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect();
            match (question.multi_select, picks.len(), &text) {
                (_, 0, None) => Err((k, UNANSWERED)),
                (false, n, text) if n > 1 || n == 1 && text.is_some() => Err((k, BOTH)),
                _ => Ok(Answer { picks, text }),
            }
        })
        .collect()
}

/// The next key of a form, from a screen that must show what the last key should have led to:
/// the key, what it should lead to, and what to say if it does not. `Err` (with why) while the
/// screen does not show the expected state, or shows something the run must not go past.
fn form_next(run: &FormRun, screen: &Screen) -> Result<(Vec<u8>, Expect, &'static str), &'static str> {
    let at = match &run.expect {
        Expect::Review(want) => {
            let review = screens::review(screen).filter(|r| want.is_none() || r.highlight == *want).ok_or(run.why)?;
            return review_next(run, &review, screen);
        }
        Expect::Submitted => return Err(NOT_CLOSED),
        Expect::Opened(at) | Expect::Shows { at, .. } | Expect::Typed { at, .. } => *at,
    };
    let shown = form_question(screen, &run.questions, at).ok_or(run.why)?;
    let cursor = shown.menu.highlight().ok_or(run.why)?;
    let multi = run.questions[at].multi_select;
    let met = match &run.expect {
        Expect::Shows { cursor: c, marks, other, .. } => cursor == *c && shown.marks == *marks && shown.typed == *other,
        Expect::Typed { cursor: c, marks, text, .. } => {
            cursor == *c && shown.marks == *marks && typed_shows(multi, &shown.typed, text)
        }
        _ => true,
    };
    if !met {
        return Err(run.why);
    }
    question_next(run, at, &shown, cursor, screen)
}

/// The next key on question `at`: tick or clear each box that differs from the answer, type the
/// text, then choose the answer (or the Submit or Next row), which leads to the next question or
/// the review.
fn question_next(
    run: &FormRun,
    at: usize,
    shown: &FormShown,
    cursor: usize,
    screen: &Screen,
) -> Result<(Vec<u8>, Expect, &'static str), &'static str> {
    let answer = &run.answers[at];
    let multi = run.questions[at].multi_select;
    let to = |target: usize| {
        let expect = Expect::Shows { at, cursor: target, marks: shown.marks.clone(), other: shown.typed.clone() };
        (moves(cursor, target, screen), expect, NOT_MOVED)
    };
    if multi {
        for (j, &on) in shown.marks.iter().enumerate() {
            if on != answer.picks.contains(&j) {
                if cursor != j {
                    return Ok(to(j));
                }
                let mut marks = shown.marks.clone();
                marks[j] = !on;
                return Ok((
                    b"\r".to_vec(),
                    Expect::Shows { at, cursor, marks, other: shown.typed.clone() },
                    NOT_TICKED,
                ));
            }
        }
    }
    match &answer.text {
        Some(text) if typed_shows(multi, &shown.typed, text) => {}
        Some(text) if pristine(multi, &shown.typed) => {
            if cursor != shown.other {
                return Ok(to(shown.other));
            }
            let expect = Expect::Typed { at, cursor, marks: shown.marks.clone(), text: text.clone() };
            return Ok((text.as_bytes().to_vec(), expect, NOT_TYPED));
        }
        None if pristine(multi, &shown.typed) => {}
        _ => return Err(OTHER_TAKEN),
    }
    let target = match (multi, answer.picks.first()) {
        (true, _) => shown.submit.ok_or(CHANGED)?,
        (false, Some(&i)) => i,
        (false, None) => shown.other,
    };
    if cursor != target {
        return Ok(to(target));
    }
    let (expect, why) = if at + 1 < run.questions.len() {
        (Expect::Opened(at + 1), NO_NEXT)
    } else {
        (Expect::Review(None), NO_REVIEW)
    };
    Ok((b"\r".to_vec(), expect, why))
}

/// On the review: Enter on "Submit answers" once every row reads the question and the answer
/// the card holds.
fn review_next(
    run: &FormRun,
    review: &screens::Review,
    screen: &Screen,
) -> Result<(Vec<u8>, Expect, &'static str), &'static str> {
    let same = review.answers.len() == run.questions.len()
        && review.answers.iter().zip(run.questions.iter().zip(&run.answers)).all(|((q, a), (question, answer))| {
            squash(q) == squash(&question.question) && squash(a) == squash(&answer_line(question, answer))
        });
    if !same {
        return Err(REVIEW_DIFFERS);
    }
    match review.highlight {
        Some(0) => Ok((b"\r".to_vec(), Expect::Submitted, NOT_CLOSED)),
        Some(h) => Ok((moves(h, 0, screen), Expect::Review(Some(0)), NOT_MOVED)),
        None => Err(CHANGED),
    }
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

fn tool_prompt(kind: &ItemKind, alone: bool, cwd: &Path, screen: &Screen) -> Option<Shown> {
    if is_plan(kind) {
        // Only the plan's own menu: the plan itself is in the conversation, from the transcript.
        let menu = screens::plan(screen)?;
        let amend = menu.footer_text().contains(screens::AMEND);
        return Some(Shown {
            matched: alone,
            question: screens::PLAN_ASKS.to_owned(),
            body: Vec::new(),
            choices: menu.texts(),
            deny: screens::plan_keep(&menu),
            amend,
        });
    }
    let prompt = screens::permission(screen)?;
    let needles = needles(kind, cwd);
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
    /// Terminal input, Cancel, or a switch away from Chat: every card key sequence in flight
    /// stops before its next key and says so, instead of going on later from a screen it no
    /// longer knows. True when one did.
    ///
    /// Only an answer stopped before its Enter marks its prompt as one to answer in the terminal:
    /// the user is answering it there. An answer whose Enter went was sent, so it ends quietly (its
    /// text still goes to the composer). A rewind keeps its own notice on its card; a Ctrl+B or a
    /// Stop agent leaves a notice only, which goes when the card shows something else, so a later
    /// prompt for the pending call is still answered from the card.
    pub(in crate::view::chat) fn interrupt_card(&mut self) -> bool {
        let card = &mut self.question;
        let rewind = card.rewind.as_mut().filter(|r| r.in_flight()).map(|r| r.interrupt(INTERRUPTED)).is_some();
        let side = card.background_step.take().is_some() | card.agent_step.take().is_some();
        if side {
            card.error = Some(INTERRUPTED);
        }
        let answer = match self.question.step.take() {
            // Enter already went: the step stays and keeps the card from taking a second click
            // until the prompt closes or its wait runs out, as it does without an interruption.
            Some(step) if step.stage == Stage::Sent => {
                self.question.step = Some(step);
                false
            }
            Some(step) => {
                self.question.step = Some(step);
                self.question_failed(INTERRUPTED);
                true
            }
            None => false,
        };
        rewind || side || answer
    }

    fn question_failed(&mut self, error: &'static str) {
        if let Some(step) = self.question.step.take()
            && step.stage == Stage::Sent
        {
            // Enter went: the text is kept for the composer all the same.
            self.hold(&step);
        }
        self.question.error = Some(error);
        self.question.failed = true;
        self.question.answerable = None;
        if self.drawer == Drawer::Closed {
            self.drawer = Drawer::Manual;
        }
    }

    /// Keeps the text of an answer meant for the composer until it lands there.
    fn hold(&mut self, step: &Step) {
        let Then::Chat(text) = &step.then else { return };
        if text.is_empty() {
            return;
        }
        if collapse(self.question.field.text()) == *text {
            self.question.field.clear();
        }
        self.question.held = Some(text.clone());
        self.land_held();
    }

    /// Drops a click's "other keys are on their way" notice once they have gone.
    fn clear_busy(&mut self) -> bool {
        // A queued draft is no key on its way: the card answers ahead of it.
        let Some(busy) = self.question.busy.filter(|_| self.waiting().is_none() && !self.enter_only()) else {
            return false;
        };
        self.question.busy = None;
        self.question.error.take_if(|error| *error == busy).is_some()
    }

    /// Puts the held text in the composer once it is empty and nothing is being sent.
    fn land_held(&mut self) -> bool {
        if self.question.held.is_none() || !self.editor.text.trim().is_empty() || self.locked() {
            return false;
        }
        self.editor.set(self.question.held.take().unwrap_or_default());
        true
    }

    /// Follows the transcript's pending call and the screen's prompt; true when the card changed.
    pub(super) fn sync_question(&mut self, model: &Model, cwd: &Path, screen: &Screen) -> bool {
        let pending = pending(model);
        let item = pending.as_ref().map(|p| p.item);
        let mut changed = false;
        if self.question.item != item {
            if let Some(step) = self.question.step.take().filter(|step| step.stage == Stage::Sent) {
                // "Chat about this" and "No, and tell Claude" return to the prompt; the text
                // goes as the next message.
                self.hold(&step);
            }
            let field = std::mem::take(&mut self.question.field);
            let held = self.question.held.take();
            let agent_step = self.question.agent_step.take();
            let rewind = self.question.rewind.take();
            let error = self.question.error.filter(|_| agent_step.is_some());
            self.question = Card { item, field, held, agent_step, rewind, error, ..Card::default() };
            if item.is_none() {
                self.question.field.clear();
            }
            changed = true;
        }
        let shown = match pending.as_ref().map(|p| p.kind) {
            Some(Kind::Tool(kind)) if !self.menus_off => {
                tool_prompt(kind, pending.as_ref().is_some_and(|p| p.alone), cwd, screen)
            }
            _ => None,
        };
        changed |= self.question.shown != shown;
        self.question.approval = shown.is_some();
        let answerable = pending.as_ref().filter(|_| !self.question.failed).filter(|p| match p.kind {
            Kind::Question(questions) => questions.is_some_and(answerable),
            Kind::Tool(_) => shown.as_ref().is_some_and(|s| s.matched),
        });
        let answerable = answerable.map(|p| p.item.offset);
        // A rewind that stopped and left its screen open keeps its card (its reason and that the
        // screen is still open) while that screen shows, and goes once it is gone.
        let rewind_screen = screens::rewind_list(screen).is_some() || screens::rewind_actions(screen).is_some();
        let left_open = self.question.rewind.as_ref().is_some_and(|r| r.failed() && r.note == Some(rewind::LEFT_OPEN));
        if left_open && !rewind_screen {
            self.question.rewind = None;
            changed = true;
        }
        let rewind_holds = self.question.rewind.as_ref().is_some_and(|r| !r.failed() || (left_open && rewind_screen));
        // Any other menu the screen shows, unless a question or prompt card already covers it.
        // A Stop agent or a rewind drives the CLI's list itself; so does a stopped one left open.
        let menu = (!self.menus_off
            && shown.is_none()
            && self.question.agent_step.is_none()
            && !rewind_holds
            && !matches!(pending.as_ref().map(|p| p.kind), Some(Kind::Question(_))))
        .then(|| menu_shown(screen))
        .flatten();
        changed |= self.question.menu != menu;
        let covered = shown.is_some() || menu.is_some();
        let background = pending.as_ref().filter(|_| !covered).and_then(|p| background::shell_offer(Some(p), screen));
        let background_changed = self.question.background != background;
        changed |= background_changed;
        self.question.background = background;
        let question = matches!(pending.as_ref().map(|p| p.kind), Some(Kind::Question(_)));
        let agents = if covered || question { Vec::new() } else { background::agent_offers(model, screen) };
        let other = self.question.menu != menu || self.question.agents != agents;
        changed |= self.question.agents != agents;
        self.question.agents = agents;
        self.question.menu = menu;
        // An interruption's notice is about the card that was shown: not the next one's status.
        if (other || background_changed) && !self.question.failed && self.question.error == Some(INTERRUPTED) {
            self.question.error = None;
            changed = true;
        }
        // Only a question or permission card has a field: no hidden card keeps the keyboard.
        let field = matches!(pending.as_ref().map(|p| p.kind), Some(Kind::Question(_))) || shown.is_some();
        if !field && std::mem::take(&mut self.question.focus) {
            changed = true;
        }
        self.question.shown = shown;
        changed |= std::mem::replace(&mut self.question.answerable, answerable) != answerable;
        changed | self.clear_busy() | self.land_held()
    }

    /// The working line's words while the card shows a CLI menu with choices: the CLI waits for
    /// the user, whatever the transcript says.
    pub(in crate::view) fn menu_waiting(&self) -> Option<&'static str> {
        self.question.menu.as_ref().filter(|menu| !menu.panel()).map(|_| ASKING)
    }

    pub(super) fn question_notice(&self) -> Option<&'static str> {
        let card = &self.question;
        let rewind = card.rewind.as_ref().is_some_and(rewind::Rewind::in_flight);
        (card.step.is_some() || card.background_step.is_some() || card.agent_step.is_some() || rewind)
            .then_some(WAITING)
    }

    /// The composer's notice while a card's text waits for it.
    pub(super) fn held_notice(&self) -> Option<&'static str> {
        self.question.held.as_ref().map(|_| HELD)
    }
}

fn ready(tab: &Tab) -> bool {
    let pane = tab.pane.as_ref().expect("Chat pane");
    !pane.chat.identity_unverified
        && pane.model.error.is_none()
        && tab.term.ended.is_none()
        && (pane.chat.demo || pane.chat.scripted() || tab.term.running())
}

/// The rewind's card is the one shown: while it runs or offers its choices, and once it stopped
/// only while no question, permission prompt or CLI menu needs the card (they come first).
fn rewind_shown(card: &Card, model: &Model) -> bool {
    let other = card.shown.is_some()
        || card.menu.is_some()
        || matches!(pending(model).map(|p| p.kind), Some(Kind::Question(_)));
    card.rewind.as_ref().is_some_and(|r| !r.failed() || !other)
}

/// A click in the card. Writes nothing unless the screen shows the same prompt.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit) -> bool {
    let pane = tab.pane.as_mut().expect("Chat pane");
    pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen);
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
        Hit::Option(_) | Hit::Escape | Hit::Close if rewind_shown(&chat.question, &pane.model) => {
            return rewind::press(tab, now, hit);
        }
        _ if chat.question.step.is_some() => return false,
        _ => {}
    }
    // A form's picks stay in the card until Submit: nothing is written, so nothing waits for them.
    let form = pending(&pane.model).and_then(|p| match p.kind {
        Kind::Question(Some(questions)) if form(questions) => Some((p.item, questions.to_vec())),
        _ => None,
    });
    if let Some((item, questions)) = form.clone()
        && matches!(hit, Hit::Option(_) | Hit::Back | Hit::Next | Hit::Answer | Hit::Submit)
    {
        // Enter in the field (`Answer`) turns the page, and on the last question submits.
        let card = &mut pane.chat.question;
        card.form.fit(&questions, &card.field);
        let last = card.form.page + 1 >= questions.len();
        if hit != Hit::Submit && !(hit == Hit::Answer && last) {
            return press_form(card, hit, &questions);
        }
        if busy(&mut pane.chat) {
            return true;
        }
        return submit_form(tab, now, item, questions);
    }
    let chat = &mut pane.chat;
    if busy(chat) {
        return true;
    }
    if hit == Hit::Background {
        return background::press_background(tab, now);
    }
    if let Hit::StopAgent(which) = hit {
        return background::press_stop(tab, now, which);
    }
    if let Some(menu) = chat.question.menu.clone() {
        return press_menu(tab, now, hit, menu);
    }
    let Some(Pending { item, kind, alone }) = pending(&pane.model) else { return false };
    let subject = match kind {
        Kind::Question(Some(questions)) if simple(questions) => Subject::Question(questions[0].clone()),
        // Esc and "Chat about this" act on the form's question the screen shows.
        Kind::Question(Some(questions)) if form.is_some() => {
            match (0..questions.len()).find(|&at| form_question(&tab.term.screen, questions, at).is_some()) {
                Some(at) => Subject::FormQuestion { questions: questions.to_vec(), at },
                None => {
                    pane.chat.question_failed(NOT_SHOWN);
                    return true;
                }
            }
        }
        Kind::Question(_) => {
            pane.chat.question_failed(TERMINAL_ONLY);
            return true;
        }
        Kind::Tool(kind) => {
            let needles = needles(kind, &pane.cwd);
            match tool_prompt(kind, alone, &pane.cwd, &tab.term.screen).filter(|s| s.matched) {
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
        let question = matches!(subject, Subject::Question(_) | Subject::FormQuestion { .. });
        let error = if question { NOT_SHOWN } else { NOT_SHOWN_TOOL };
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
            // The row shows no field of its own: a leading digit could pick a choice instead.
            Ok(text) if text.starts_with(|c: char| c.is_ascii_digit()) => {
                chat.question.error = Some(DIGIT_FIRST);
                return true;
            }
            Ok(text) => (other, Then::Type(text)),
            Err(error) => {
                chat.question.error = Some(error);
                return true;
            }
        },
        (Subject::Question(_) | Subject::FormQuestion { .. }, Hit::ChatAbout)
        | (Subject::Tool { .. } | Subject::Plan { .. }, Hit::Deny)
            if !text.is_empty() && chat.queue.is_some() && !(shown.amend && hit == Hit::Deny) =>
        {
            chat.question.error = Some(QUEUED_TEXT);
            return true;
        }
        (Subject::Question(_) | Subject::FormQuestion { .. }, Hit::ChatAbout) => (chat_row, Then::Chat(text)),
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

/// Keys on their way to the terminal (a message, a mode change, Ctrl+B): the card's must not mix
/// with them, and the click says so. A queued draft waits for the turn to end, which waits for
/// this answer: the answer goes first, under the same checks.
fn busy(chat: &mut Chat) -> bool {
    if chat.waiting().is_none() && !chat.enter_only() {
        return false;
    }
    let busy = chat.waiting().filter(|_| !chat.enter_only()).unwrap_or(SENDING);
    chat.question.error = Some(busy);
    chat.question.busy = Some(busy);
    true
}

impl Card {
    /// Shows question `to` of the form, keeping the field's text for the one it leaves.
    fn turn(&mut self, to: usize) {
        if let Some(text) = self.form.texts.get_mut(self.form.page) {
            *text = self.field.text().to_owned();
        }
        self.form.page = to;
        let text = self.form.texts.get(to).cloned().unwrap_or_default();
        self.field.set_text(&text);
        self.error = None;
    }
}

/// A pick or a page turn in a form's card: nothing goes to the terminal.
fn press_form(card: &mut Card, hit: Hit, questions: &[Question]) -> bool {
    card.form.fit(questions, &card.field);
    let page = card.form.page.min(questions.len().saturating_sub(1));
    match hit {
        Hit::Option(i) => {
            let Some(row) = card.form.chosen.get_mut(page).filter(|row| i < row.len()) else { return false };
            if questions[page].multi_select {
                row[i] = !row[i];
            } else {
                let on = !row[i];
                row.fill(false);
                row[i] = on;
            }
            card.error = None;
        }
        Hit::Back => match page.checked_sub(1) {
            Some(to) => card.turn(to),
            None => return false,
        },
        // Enter in the field turns the page too (on the last one it submits, see `press`).
        Hit::Next | Hit::Answer if page + 1 < questions.len() => card.turn(page + 1),
        _ => return false,
    }
    true
}

/// Submit: the card's answers, checked, then the first key, written only while the screen shows
/// the form's first question; `advance` sends the rest one checked key at a time.
fn submit_form(tab: &mut Tab, now: Instant, item: ItemId, questions: Vec<Question>) -> bool {
    let card = &mut tab.pane.as_mut().unwrap().chat.question;
    card.form.fit(&questions, &card.field);
    let answers = match form_answers(&questions, &card.form, &card.field) {
        Ok(answers) => answers,
        Err((page, error)) => {
            if page != card.form.page {
                card.turn(page);
            }
            card.error = Some(error);
            return true;
        }
    };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let mut run = FormRun { questions, answers, expect: Expect::Opened(0), why: NOT_SHOWN };
    let next = form_next(&run, &tab.term.screen);
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let (key, expect, why) = match next {
        Ok(next) => next,
        Err(error) => {
            chat.question_failed(error);
            return true;
        }
    };
    chat.question.error = None;
    chat.question.focus = false;
    let epoch = chat.output_epoch;
    run.expect = expect;
    run.why = why;
    let subject = Subject::Form(run);
    let step =
        Step { item: Some(item), subject, target: 0, then: Then::Choose, stage: Stage::Moving, due: now + STEP, epoch };
    match write(tab, &key) {
        Ok(()) => tab.pane.as_mut().unwrap().chat.question.step = Some(step),
        Err(error) => tab.pane.as_mut().unwrap().chat.question_failed(error),
    }
    true
}

/// What the screen says about a form in flight. `may_act`: the window allows writes and the
/// screen changed since the last key.
fn look_form(screen: &Screen, step: &Step, run: &FormRun, now: Instant, may_act: bool) -> Action {
    let late = now >= step.due;
    if step.stage == Stage::Sent {
        return if late && screens::review(screen).is_some() {
            Action::Fail(NOT_CLOSED)
        } else if late {
            Action::Closed
        } else {
            Action::Wait
        };
    }
    match form_next(run, screen) {
        Ok((key, expect, why)) if may_act => Action::Key(key, expect, why),
        Ok(_) if late => Action::Fail(run.why),
        Err(error) if late => Action::Fail(error),
        _ => Action::Wait,
    }
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
    let changed = pane.chat.sync_question(&pane.model, &pane.cwd, &tab.term.screen)
        | background::advance_background(tab, now)
        | background::advance_agent(tab, now, may_write)
        | rewind::advance(tab, now, may_write);
    let pane = tab.pane.as_mut().expect("Chat pane");
    let changed = changed | pane.chat.clear_busy();
    let Some(mut step) = pane.chat.question.step.clone() else { return changed };
    if step.item.is_some() && pending(&pane.model).map(|p| p.item) != step.item {
        // The transcript resolved or replaced the call; `sync_question` cleared the step.
        return changed;
    }
    let fresh = pane.chat.output_epoch != step.epoch;
    let action = match &step.subject {
        _ if !ready(tab) => Action::Fail(BLOCKED),
        Subject::Form(run) => look_form(&tab.term.screen, &step, run, now, may_write && fresh),
        _ => look(&tab.term.screen, &step, now, may_write && fresh),
    };
    let epoch = tab.pane.as_ref().unwrap().chat.output_epoch;
    let result = match action {
        Action::Wait => return changed,
        Action::Fail(error) => Err(error),
        Action::Closed => {
            let chat = &mut tab.pane.as_mut().unwrap().chat;
            chat.question.step = None;
            chat.hold(&step);
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
        Action::Key(key, expect, why) => write(tab, &key).map(|()| {
            let sent = expect == Expect::Submitted;
            if let Subject::Form(run) = &mut step.subject {
                run.expect = expect;
                run.why = why;
            }
            step.stage = if sent { Stage::Sent } else { Stage::Moving };
            step.due = now + if sent { RESULT } else { STEP };
            step.epoch = epoch;
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
    /// A form's next key, what it should lead to, and what to say if it does not.
    Key(Vec<u8>, Expect, &'static str),
}

/// The text in the comment field open on the subject's highlighted `target` choice; `None` when
/// the screen shows no such field.
fn comment(subject: &Subject, screen: &Screen, target: usize) -> Option<String> {
    let (Subject::Tool { choices, .. } | Subject::Plan { choices }) = subject else { return None };
    let (menu, ..) = read(subject, screen, Some(target))?;
    (menu.highlight() == Some(target)).then_some(())?;
    menu.comment(target, choices.get(target)?)
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
        // Only a field the screen positively shows takes the text: on a menu row, digits pick choices.
        Stage::Opening => match comment(&step.subject, screen, step.target) {
            Some(typed) if may_act && typed.is_empty() => Action::Type(typed_text.unwrap_or_default().to_owned()),
            _ if late => Action::Fail(NO_COMMENT),
            _ => Action::Wait,
        },
        Stage::Typing => {
            let text = typed_text.unwrap_or_default();
            let typed = if matches!(step.then, Then::Amend(_)) {
                comment(&step.subject, screen, step.target).is_some_and(|typed| typed.contains(text))
            } else {
                read(&step.subject, screen, Some(step.target)).is_some_and(|(menu, ..)| {
                    menu.highlight() == Some(step.target)
                        && menu.rows_from(step.target).is_some_and(|rows| rows.contains(text))
                })
            };
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
    if !chat.question.visible() {
        chat.question.focus = false;
    }
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
    if !chat.question.visible() {
        chat.question.focus = false;
    }
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
/// The widest a usage bar is drawn.
const BAR_W: f32 = 320.0;

/// A row copied from the CLI: a spinner row keeps its glyph in a fixed-width slot (as the working
/// line does), so the animation does not move the text after it.
#[allow(clippy::too_many_arguments)]
fn copied_row(p: &mut Painter, face: Face, size: f32, x: f32, cy: f32, text: &str, color: Rgb, max: f32) {
    if crate::view::working::spinner_row(text) {
        crate::view::working::draw_line(p, size, x, cy, text, color, max);
    } else {
        p.text(face, size, x, cy, text, color, max);
    }
}

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
        _ if rewind_shown(&chat.question, model) => chat.question.rewind.as_ref().map(rewind::height),
        Some(Kind::Question(questions)) => {
            // A form keeps one height for all its questions.
            let options = questions.into_iter().flatten().map(|q| q.options.len()).max().unwrap_or(0);
            Some(HEADER + QUESTION + options as f32 * (OPTION + 4.0))
        }
        Some(Kind::Tool(kind)) => chat.question.shown.as_ref().map(|s| {
            let lines = detail(kind).len() + s.body.len().min(BODY_LINES);
            HEADER + lines as f32 * LINE + 8.0 + LINE + 6.0 + s.choices.len() as f32 * (CHOICE + 4.0)
        }),
        None => None,
    };
    let wanted = wanted.or_else(|| background::height(&chat.question)).or_else(|| {
        chat.question.menu.as_ref().map(|m| {
            // A panel has no title row: its rows say what it is.
            if m.choices.is_empty() {
                let title = if m.title.is_empty() { 0.0 } else { LINE };
                HEADER + 6.0 + title + m.rows.len() as f32 * LINE
            } else {
                HEADER + LINE + 6.0 + m.choices.len() as f32 * (CHOICE + 4.0) + m.noted() as f32 * LINE
            }
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
            _ if rewind_shown(&chat.question, model) => self.rewind(p, chat, inner, hits),
            Some(Kind::Question(questions)) => self.question(p, chat, inner, questions, hits),
            Some(Kind::Tool(kind)) if chat.question.shown.is_some() => self.permission(p, chat, inner, kind, hits),
            _ if chat.question.menu.is_some() => self.menu(p, chat, inner, hits),
            _ => self.background(p, chat, inner, hits),
        }
        // Esc leaves a menu or declines what the card shows, under the same checks as a choice.
        let escape = match pending(model).map(|p| p.kind) {
            _ if chat.question.step.is_some() || rewind_shown(&chat.question, model) => false,
            Some(Kind::Question(_)) => chat.question.answerable.is_some(),
            Some(Kind::Tool(_)) if chat.question.shown.is_some() => chat.question.answerable.is_some(),
            // A panel offers Close in its own footer instead.
            _ => chat.question.menu.as_ref().is_some_and(|m| m.esc && !m.choices.is_empty()),
        };
        if escape {
            let r = Rect::new(inner.right() - 64.0, inner.y - 2.0, 64.0, HEADER + 2.0);
            let target = Target::Chat(Control::Question(Hit::Escape));
            super::paint::button(p, pal, r, "Esc", self.hover == Some(target), false);
            hits.push((r, target));
        }
        p.restore(clip);
    }

    /// A CLI menu, picker or dialog: its choices as buttons, or its rows read-only.
    fn menu(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some(menu) = chat.question.menu.clone() else { return };
        if menu.choices.is_empty() {
            return self.panel(p, chat, inner, &menu, hits);
        }
        let mut y = inner.y;
        let kind = if menu.question { "Claude asks" } else { "Claude Code menu" };
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, kind, pal.muted, inner.w);
        y += HEADER;
        let title = if menu.title.is_empty() { "Choose in the terminal's menu" } else { menu.title.as_str() };
        copied_row(p, Face::Semibold, 12.5, inner.x, y + LINE / 2.0, title, pal.text, inner.w);
        y += LINE + 6.0;
        for (i, choice) in menu.choices.iter().enumerate() {
            let note = menu.notes.get(i).filter(|note| !note.is_empty());
            let row = Rect::new(inner.x, y, inner.w, CHOICE + if note.is_some() { LINE } else { 0.0 });
            let target = Target::Chat(Control::Question(Hit::Option(i)));
            p.rbox(row, 6.0, if self.hover == Some(target) { pal.hover } else { pal.bar });
            p.text(
                Face::Semibold,
                12.0,
                row.x + 10.0,
                row.y + CHOICE / 2.0,
                // A question's rows are not all numbered on the screen (Next, Submit).
                &if menu.question { choice.clone() } else { format!("{}. {choice}", i + 1) },
                pal.text,
                row.w - 20.0,
            );
            if let Some(note) = note {
                p.text(Face::Ui, 11.0, row.x + 10.0, row.y + CHOICE + LINE / 2.0 - 4.0, note, pal.muted, row.w - 20.0);
            }
            hits.push((row, target));
            y += row.h + 4.0;
        }
        let row = Rect::new(inner.x, y, inner.w, FIELD - 4.0);
        let status = chat.question.error.or_else(|| chat.question_notice()).unwrap_or("");
        p.text(Face::Ui, 11.5, row.x, row.cy(), status, pal.muted, (row.w - 130.0).max(0.0));
        let open = Rect::new(row.right() - 120.0, row.y, 120.0, row.h);
        let target = Target::Chat(Control::Question(Hit::Open));
        super::paint::button(p, pal, open, "Open terminal", self.hover == Some(target), false);
        hits.push((open, target));
    }

    /// A CLI panel that only shows information (/usage, /status): its rows, usage bars drawn as
    /// bars, and Close (Esc) when its footer offers Esc, else Open terminal.
    fn panel(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, menu: &MenuShown, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let mut y = inner.y;
        p.text(Face::Semibold, 11.5, inner.x, y + HEADER / 2.0, "Claude Code panel", pal.muted, inner.w);
        y += HEADER + 6.0;
        if !menu.title.is_empty() {
            p.text(Face::Semibold, 12.5, inner.x, y + LINE / 2.0, &menu.title, pal.text, inner.w);
            y += LINE;
        }
        // From the panel's top, as many rows as the card has room for; the rest is named.
        let room = ((inner.bottom() - y - FIELD - STATUS) / LINE).floor().max(1.0) as usize;
        let (shown, more) = if menu.rows.len() > room { (room - 1, menu.rows.len() - room + 1) } else { (room, 0) };
        for row in menu.rows.iter().take(shown) {
            let cy = y + LINE / 2.0;
            match screens::usage_bar(row) {
                Some((percent, label)) => {
                    let label_w = p.measure(Face::Ui, 11.0, label);
                    let track_w = (inner.w - label_w - 10.0).clamp(0.0, BAR_W);
                    let track = Rect::new(inner.x, cy - 3.0, track_w, 6.0);
                    p.rbox(track, 3.0, pal.bar);
                    let filled = (track_w * f32::from(percent) / 100.0).round();
                    if filled > 0.0 {
                        p.rbox(Rect::new(track.x, track.y, filled.max(6.0), track.h), 3.0, pal.accent);
                    }
                    p.text(Face::Ui, 11.0, track.right() + 10.0, cy, label, pal.text, label_w + 2.0);
                }
                None => copied_row(p, Face::Mono, 11.0, inner.x, cy, row, pal.muted, inner.w),
            }
            y += LINE;
        }
        if more > 0 {
            let note = format!("{more} more rows in the terminal");
            p.text(Face::Ui, 11.0, inner.x, y + LINE / 2.0, &note, pal.muted, inner.w);
            y += LINE;
        }
        let row = Rect::new(inner.x, y, inner.w, FIELD - 4.0);
        let status = chat.question.error.or_else(|| chat.question_notice()).unwrap_or("");
        p.text(Face::Ui, 11.5, row.x, row.cy(), status, pal.muted, (row.w - 130.0).max(0.0));
        let (label, hit) = if menu.esc { ("Close (Esc)", Hit::Escape) } else { ("Open terminal", Hit::Open) };
        let r = Rect::new(row.right() - 120.0, row.y, 120.0, row.h);
        let target = Target::Chat(Control::Question(hit));
        super::paint::button(p, pal, r, label, self.hover == Some(target), false);
        hits.push((r, target));
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
        let Some(questions) = questions.filter(|q| !q.is_empty()) else {
            p.text(Face::Semibold, 12.5, inner.x, y + HEADER / 2.0, "Claude is asking a question", pal.text, inner.w);
            self.footer(p, chat, inner, false, hits);
            return;
        };
        let can_answer = chat.question.answerable.is_some() || chat.question.failed && answerable(questions);
        let is_form = form(questions);
        if is_form {
            chat.question.form.fit(questions, &chat.question.field);
        }
        let page = if is_form { chat.question.form.page.min(questions.len() - 1) } else { 0 };
        let question = &questions[page];
        let chosen =
            if is_form { chat.question.form.chosen.get(page).cloned().unwrap_or_default() } else { Vec::new() };
        let asks = if question.header.is_empty() { "Claude asks" } else { question.header.as_str() };
        let more = questions.len().saturating_sub(1);
        let header = match more {
            _ if is_form && more > 0 => format!("{asks} · {} of {}", page + 1, questions.len()),
            0 => asks.to_owned(),
            n => format!("{asks} · {n} more in the terminal"),
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
            // A form's picks are the card's own, shown as such; the terminal's highlight never is.
            let on = chosen.get(i).copied().unwrap_or(false);
            let fill = if hot {
                pal.hover
            } else if on {
                pal.selection
            } else {
                pal.bar
            };
            if on {
                p.frame(row, 6.0, fill, pal.accent);
                p.text_right(Face::Semibold, 12.0, row.right() - 10.0, row.cy(), "✓", pal.accent);
            } else {
                p.rbox(row, 6.0, fill);
            }
            let label = format!("{}. {}", i + 1, option.label);
            let cy = if option.description.is_empty() { row.cy() } else { row.y + 12.0 };
            p.text(Face::Semibold, 12.0, row.x + 10.0, cy, &label, pal.text, row.w - 36.0);
            if !option.description.is_empty() {
                let description = collapse(&option.description);
                p.text(Face::Ui, 11.0, row.x + 10.0, row.y + 27.0, &description, pal.muted, row.w - 36.0);
            }
            if can_answer {
                hits.push((row, target));
            }
            y += OPTION + 4.0;
        }
        let area = Rect::new(inner.x, y, inner.w, inner.bottom() - y);
        if is_form {
            self.form_footer(p, chat, area, can_answer, (page, questions.len()), hits);
        } else {
            self.footer(p, chat, area, can_answer, hits);
        }
    }

    /// A form's field (the text for the question shown), Back, Next or Submit, and "Chat about
    /// this".
    fn form_footer(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        area: Rect,
        can_answer: bool,
        (page, pages): (usize, usize),
        hits: &mut Vec<(Rect, Target)>,
    ) {
        let pal = self.pal;
        let button = |p: &mut Painter, hits: &mut Vec<(Rect, Target)>, r: Rect, label: &str, hit: Hit| {
            let target = Target::Chat(Control::Question(hit));
            super::paint::button(p, pal, r, label, self.hover == Some(target), false);
            hits.push((r, target));
        };
        let row = Rect::new(area.x, area.y, area.w, FIELD - 4.0);
        if can_answer {
            let buttons = 64.0 + 4.0 + 84.0 + 4.0 + 120.0;
            let field = Rect::new(row.x, row.y, (row.w - buttons - 8.0).max(0.0), row.h);
            let focus = chat.question.focus && self.focused;
            p.frame(field, 5.0, pal.field, if focus { pal.accent } else { pal.line });
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            chat.question.field.draw(p, field.inset(6.0), Face::Ui, 12.0, &colors, "Type something…", focus, focus);
            hits.push((field, Target::Chat(Control::Question(Hit::Field))));
            let mut x = field.right() + 8.0;
            if page > 0 {
                button(p, hits, Rect::new(x, row.y, 64.0, row.h), "Back", Hit::Back);
            }
            x += 68.0;
            if page + 1 < pages {
                button(p, hits, Rect::new(x, row.y, 84.0, row.h), "Next", Hit::Next);
            } else {
                button(p, hits, Rect::new(x, row.y, 84.0, row.h), "Submit", Hit::Submit);
            }
            button(p, hits, Rect::new(x + 88.0, row.y, 120.0, row.h), "Chat about this", Hit::ChatAbout);
        } else {
            p.text(Face::Ui, 11.5, row.x, row.cy(), TERMINAL_ONLY, pal.muted, (row.w - 130.0).max(0.0));
            button(p, hits, Rect::new(row.right() - 120.0, row.y, 120.0, row.h), "Open terminal", Hit::Open);
        }
        let status = chat.question.error.or_else(|| chat.question_notice());
        if let Some(status) = status {
            p.text(Face::Ui, 11.0, area.x, row.bottom() + STATUS / 2.0 + 2.0, status, pal.muted, area.w);
        }
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
            copied_row(p, Face::Ui, 11.0, inner.x, y + LINE / 2.0, line, pal.muted, inner.w);
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
