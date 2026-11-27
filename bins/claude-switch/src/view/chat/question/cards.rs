//! How a question, a permission prompt and a CLI menu are laid out in the card: the composer's
//! box in the composer's place, with a head (what asks, and Esc), the question, the choices as
//! one list with their numbers, a field for another answer, and a foot.
//!
//! A card is first described as a list of [`Part`]s, from the same state the old cards read. The
//! list gives the card's height when Chat is laid out and is what the painter draws, so the two
//! cannot disagree. Nothing here decides what a click does: every target is a [`Hit`] that
//! `press` checks against the screen as before.
use mdshared_ui::gfx::mix;
use mdshared_ui::glyph;

use super::*;
use crate::view::look;
use crate::view::text::TextLayout;

/// The card's padding: its text starts where the reading column's does.
pub(super) const INSET: f32 = look::INSET;
/// The head: a chip or a title, and the close button.
pub(super) const HEAD: f32 = 24.0;
const HEAD_GAP: f32 = 10.0;
/// A choice without and with a description under it.
pub(super) const CHOICE_ROW: f32 = 38.0;
const NOTED_ROW: f32 = 54.0;
/// A choice when the card must be short.
const TIGHT_ROW: f32 = 30.0;
const LIST_GAP: f32 = 10.0;
const FIELD_ROW: f32 = 40.0;
const FIELD_GAP: f32 = 12.0;
/// The foot: plain buttons and what Esc does.
pub(super) const FOOT: f32 = 24.0;
const STATUS_ROW: f32 = 22.0;
/// A row copied from the CLI as text.
const TEXT_ROW: f32 = 20.0;
/// A line of the code well.
const WELL_LINE: f32 = 18.0;
const WELL_PAD: f32 = 9.0;
const WELL_GAP: f32 = 12.0;
const ASK_GAP: f32 = 12.0;
/// A panel's row: as the CLI drew it, or a usage bar.
const PANEL_ROW: f32 = 16.0;

/// How much of a card's detail is shown: a short window drops the descriptions under the
/// choices, then makes the choices lower, before anything is cut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Density {
    #[default]
    Roomy,
    /// No descriptions under the choices.
    Plain,
    /// Low choices too.
    Tight,
}

/// A choice of a card's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    /// Its number in the CLI's menu, when the CLI numbers it.
    pub number: Option<usize>,
    pub label: String,
    /// The description under it.
    pub note: String,
    pub hit: Hit,
    /// Picked in a form's card (the card's own pick, never the terminal's highlight).
    pub on: bool,
}

/// What the field's row offers beside the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Send {
    /// A round arrow: answer with the text.
    Arrow(Hit),
    /// Buttons with their words (a form's Back and Next, a multi-select's Add answer).
    Buttons(Vec<(&'static str, Hit)>),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Part {
    /// What asks: a small chip (a question's own header) or an icon and a title (a prompt's own
    /// first row), a note at the right, and the close button, which is Esc.
    Head {
        chip: String,
        title: String,
        icon: &'static str,
        note: String,
        close: bool,
    },
    /// The question, wrapped: its height is measured when the card is laid out.
    Ask {
        text: String,
        size: f32,
        height: f32,
    },
    /// Rows of the CLI as text, one line each.
    Rows(Vec<String>),
    /// Code as the transcript has it (a command, an edit's lines), in a well.
    Well(Vec<String>),
    /// A read-only panel's rows; `more` of them are only in the terminal.
    Panel {
        rows: Vec<String>,
        more: usize,
    },
    Choices {
        rows: Vec<Choice>,
        enabled: bool,
    },
    Field {
        placeholder: &'static str,
        send: Send,
    },
    /// A notice: a refusal in the warning colour, or what the card is waiting for.
    Status {
        text: &'static str,
        warn: bool,
    },
    Foot {
        left: Vec<(&'static str, Hit)>,
        text: &'static str,
        right: Vec<(&'static str, Hit)>,
    },
}

impl Part {
    fn height(&self, density: Density) -> f32 {
        match self {
            Part::Head { .. } => HEAD + HEAD_GAP,
            Part::Ask { height, .. } => height + ASK_GAP,
            Part::Rows(rows) => rows.len() as f32 * TEXT_ROW + 6.0,
            Part::Well(lines) => lines.len() as f32 * WELL_LINE + 2.0 * WELL_PAD + WELL_GAP,
            Part::Panel { rows, more } => (rows.len() + usize::from(*more > 0)) as f32 * PANEL_ROW + 8.0,
            Part::Choices { rows, .. } => rows.iter().map(|row| choice_height(row, density)).sum::<f32>() + LIST_GAP,
            Part::Field { .. } => FIELD_ROW + FIELD_GAP,
            Part::Status { .. } => STATUS_ROW,
            Part::Foot { .. } => FOOT,
        }
    }
}

fn choice_height(choice: &Choice, density: Density) -> f32 {
    match density {
        Density::Roomy if !choice.note.is_empty() => NOTED_ROW,
        Density::Roomy | Density::Plain => CHOICE_ROW,
        Density::Tight => TIGHT_ROW,
    }
}

/// The card's whole height for these parts.
pub(super) fn height(parts: &[Part], density: Density) -> f32 {
    INSET + parts.iter().map(|part| part.height(density)).sum::<f32>() + INSET - 6.0
}

/// The roomiest way these parts fit in `room`.
pub(super) fn density_for(parts: &[Part], room: f32) -> Density {
    [Density::Roomy, Density::Plain]
        .into_iter()
        .find(|density| height(parts, *density) <= room)
        .unwrap_or(Density::Tight)
}

/// The question's text, set as the card sets it: semibold, wrapped at `width`.
fn ask_layout(text: &str, pal: &Palette, scale: f32, width: f32, size: f32) -> TextLayout {
    TextLayout::heading(text, pal, scale, width.max(40.0), size)
}

fn ask(text: &str, width: f32, size: f32) -> Part {
    // Measured as it is drawn, at any scale's logical size; at most four lines are kept.
    let layout = ask_layout(text, &mdshared_ui::theme::DARK, 1.0, width, size);
    let height = (layout.height() + 1.0).ceil().min((size * 1.45 * 4.0).ceil());
    Part::Ask { text: text.to_owned(), size, height }
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A question's "Chat about this" row: a button of the foot, not a choice of the list.
fn chat_about(label: &str) -> bool {
    label.trim().eq_ignore_ascii_case("Chat about this")
}

fn status(chat: &Chat) -> Option<Part> {
    match (chat.question.error, chat.question_notice()) {
        (Some(text), _) => Some(Part::Status { text, warn: true }),
        (None, Some(text)) => Some(Part::Status { text, warn: false }),
        _ => None,
    }
}

/// An icon for the tool a prompt is about.
fn tool_icon(kind: &ItemKind) -> &'static str {
    let ItemKind::Tool { name, .. } = kind else { return glyph::SLIDERS };
    match name.as_str() {
        "Bash" | "PowerShell" => look::TERMINAL,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => glyph::EDIT,
        "Read" | "NotebookRead" => glyph::DOC,
        "Grep" | "Glob" => glyph::SEARCH,
        "WebFetch" | "WebSearch" => glyph::GLOBE,
        "Agent" | "Task" => look::AGENT,
        "ExitPlanMode" => glyph::TASK,
        _ => glyph::SLIDERS,
    }
}

/// The parts of the card for the pending call or the CLI's menu, `width` wide inside its
/// padding; `None` when another card shows (a rewind, a prompt read from the screen alone, the
/// list of blocked calls) or none of these does.
pub(super) fn parts(chat: &Chat, model: &Model, width: f32) -> Option<Vec<Part>> {
    let card = &chat.question;
    if rewind_shown(card, model) || card.prompt.is_some() || card.blocked.is_some() {
        return None;
    }
    match pending(model).map(|p| p.kind) {
        Some(Kind::Question(questions)) => Some(question(chat, questions, width)),
        Some(Kind::Tool(kind)) if card.shown.is_some() => Some(permission(chat, kind, width)),
        _ => card.menu.as_ref().map(|menu| self::menu(chat, menu, width)),
    }
}

/// Claude's question, from the transcript: one question, or a form's question on its page.
fn question(chat: &Chat, questions: Option<&[Question]>, width: f32) -> Vec<Part> {
    let card = &chat.question;
    let open = ("Open terminal", Hit::Open);
    let Some(questions) = questions.filter(|q| !q.is_empty()) else {
        return vec![
            Part::Head {
                chip: "Claude asks".into(),
                title: String::new(),
                icon: "",
                note: String::new(),
                close: false,
            },
            ask("Claude is asking a question", width, 15.5),
            Part::Foot { left: Vec::new(), text: TERMINAL_ONLY, right: vec![open] },
        ];
    };
    let can_answer = card.answerable.is_some() || card.failed && answerable(questions);
    let is_form = form(questions);
    let page = if is_form { card.form.page.min(questions.len() - 1) } else { 0 };
    let question = &questions[page];
    let chosen = if is_form { card.form.chosen.get(page).cloned().unwrap_or_default() } else { Vec::new() };
    let more = questions.len() - 1;
    let mut note = match more {
        _ if is_form && more > 0 => format!("{} of {}", page + 1, questions.len()),
        0 => String::new(),
        n => format!("{n} more in the terminal"),
    };
    if question.multi_select {
        note = if note.is_empty() { "Several answers".into() } else { format!("{note} · Several answers") };
    }
    // Esc declines what the card shows, under the same checks as a choice.
    let close = card.answerable.is_some() && card.step.is_none();
    let chip = if question.header.is_empty() { "Claude asks" } else { question.header.as_str() };
    let mut parts = vec![
        Part::Head { chip: chip.into(), title: String::new(), icon: "", note, close },
        ask(&question.question, width, 15.5),
    ];
    let rows = question
        .options
        .iter()
        .enumerate()
        .map(|(i, option)| Choice {
            number: Some(i + 1),
            label: option.label.clone(),
            note: collapse(&option.description),
            hit: Hit::Option(i),
            on: chosen.get(i).copied().unwrap_or(false),
        })
        .collect();
    parts.push(Part::Choices { rows, enabled: can_answer });
    if can_answer {
        let send = if is_form {
            let mut buttons = Vec::new();
            if page > 0 {
                buttons.push(("Back", Hit::Back));
            }
            buttons.push(if page + 1 < questions.len() { ("Next", Hit::Next) } else { ("Submit", Hit::Submit) });
            Send::Buttons(buttons)
        } else {
            Send::Arrow(Hit::Answer)
        };
        parts.push(Part::Field { placeholder: "Something else…", send });
    }
    parts.extend(status(chat));
    let mut right = vec![open];
    if close {
        right.push(("Esc to skip", Hit::Escape));
    }
    parts.push(if can_answer {
        Part::Foot { left: vec![("Chat about this", Hit::ChatAbout)], text: "", right }
    } else {
        Part::Foot { left: Vec::new(), text: TERMINAL_ONLY, right }
    });
    parts
}

/// A permission prompt for the pending call: what the prompt itself says it is, the command or
/// the edit once (from the transcript), the question in the CLI's words and its choices.
fn permission(chat: &Chat, kind: &ItemKind, width: f32) -> Vec<Part> {
    let card = &chat.question;
    let Some(shown) = card.shown.as_ref() else { return Vec::new() };
    let can_answer = shown.matched && (card.answerable.is_some() || card.failed);
    let close = card.answerable.is_some() && card.step.is_none();
    let detail = detail(kind);
    // The prompt's own first row says what is asked for ("Bash command").
    let title = shown.body.first().cloned().unwrap_or_else(|| tool_title(kind));
    let mut parts = vec![Part::Head { chip: String::new(), title, icon: tool_icon(kind), note: String::new(), close }];
    // Its other rows, in the terminal's own words (warnings such as a countdown among them),
    // without the ones that only repeat the command the well shows.
    let whole = squash(&detail.concat());
    let repeats = |row: &String| {
        let row = squash(row);
        detail.iter().any(|line| squash(line) == row) || row.len() >= 12 && whole.contains(&row)
    };
    let kept: Vec<&String> = shown.body.iter().skip(1).filter(|row| !repeats(row)).collect();
    let rows: Vec<String> = kept.iter().rev().take(BODY_LINES).rev().map(|row| (*row).clone()).collect();
    if !rows.is_empty() {
        parts.push(Part::Rows(rows));
    }
    if is_plan(kind) {
        parts.push(Part::Rows(detail));
    } else if !detail.is_empty() {
        parts.push(Part::Well(detail));
    }
    parts.push(ask(&shown.question, width, 14.0));
    let rows = shown
        .choices
        .iter()
        .enumerate()
        .map(|(i, choice)| Choice {
            number: Some(i + 1),
            label: choice.clone(),
            note: String::new(),
            hit: Hit::Option(i),
            on: false,
        })
        .collect();
    parts.push(Part::Choices { rows, enabled: can_answer });
    if can_answer && shown.deny.is_some() {
        parts
            .push(Part::Field {
                placeholder: "No, and tell Claude what to do instead…", send: Send::Arrow(Hit::Deny)
            });
    }
    parts.extend(status(chat));
    let mut right = vec![("Open terminal", Hit::Open)];
    if close {
        right.push(("Esc to cancel", Hit::Escape));
    }
    parts.push(Part::Foot { left: Vec::new(), text: if can_answer { "" } else { TERMINAL_ONLY }, right });
    parts
}

/// A CLI menu, picker or dialog read from the screen: a question of Claude's, any other menu
/// with choices, or a panel that only shows rows.
fn menu(chat: &Chat, menu: &MenuShown, width: f32) -> Vec<Part> {
    let card = &chat.question;
    let head = |chip: &str, close| Part::Head {
        chip: chip.into(),
        title: String::new(),
        icon: "",
        note: String::new(),
        close,
    };
    if menu.choices.is_empty() {
        // A panel has no title row of its own: its rows say what it is.
        let mut parts = vec![head("Claude Code panel", false)];
        if !menu.title.is_empty() {
            parts.push(ask(&menu.title, width, 14.0));
        }
        parts.push(Part::Panel { rows: menu.rows.clone(), more: 0 });
        parts.extend(status(chat));
        let action = if menu.esc { ("Close (Esc)", Hit::Escape) } else { ("Open terminal", Hit::Open) };
        parts.push(Part::Foot { left: Vec::new(), text: "", right: vec![action] });
        return parts;
    }
    // Esc leaves a menu, under the same checks as a choice.
    let close = menu.esc && card.step.is_none();
    let chip = match () {
        _ if !menu.question => "Claude Code menu",
        _ if menu.header.is_empty() => "Claude asks",
        _ => menu.header.as_str(),
    };
    let title = if menu.title.is_empty() { "Choose in the terminal's menu" } else { menu.title.as_str() };
    let mut parts = vec![head(chip, close), ask(title, width, 15.5)];
    // The row for the user's own answer is the field, and "Chat about this" a button of the
    // foot: neither is listed again as a choice.
    let other = menu.other.map(|(at, _)| at);
    let about = menu.choices.iter().position(|choice| menu.question && chat_about(choice));
    let rows = menu
        .choices
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != other && Some(*i) != about)
        .map(|(i, choice)| Choice {
            // A question's rows are not all numbered on the screen (Next, Submit): only its
            // options, which come before the row for another answer.
            number: (!menu.question || other.is_some_and(|at| i < at)).then_some(i + 1),
            label: choice.clone(),
            note: menu.notes.get(i).cloned().unwrap_or_default(),
            hit: Hit::Option(i),
            on: false,
        })
        .collect();
    parts.push(Part::Choices { rows, enabled: true });
    if let Some((_, boxed)) = menu.other {
        // A multi-select question adds the text to its answers; any other answers with it.
        let send = if boxed { Send::Buttons(vec![("Add answer", Hit::Answer)]) } else { Send::Arrow(Hit::Answer) };
        parts.push(Part::Field { placeholder: "Something else…", send });
    }
    parts.extend(status(chat));
    let mut right = vec![("Open terminal", Hit::Open)];
    if close {
        right.push((if menu.question { "Esc to skip" } else { "Esc to close" }, Hit::Escape));
    }
    let left = about.map(|at| ("Chat about this", Hit::Option(at))).into_iter().collect();
    parts.push(Part::Foot { left, text: "", right });
    parts
}

impl Draw<'_> {
    fn target(hit: Hit) -> Target {
        Target::Chat(Control::Question(hit))
    }

    /// A plain button of a card: words that fill only under the pointer.
    #[allow(clippy::too_many_arguments)] // drawing call: a place plus what to draw there
    pub(super) fn plain(
        &self,
        p: &mut Painter,
        hits: &mut Vec<(Rect, Target)>,
        right: f32,
        cy: f32,
        label: &str,
        hit: Hit,
        strong: bool,
    ) -> f32 {
        let pal = self.pal;
        let (face, ink) = if strong { (Face::Semibold, pal.text) } else { (Face::Ui, pal.muted) };
        let w = p.measure(face, 12.5, label);
        let r = Rect::new(right - w - 16.0, cy - 12.0, w + 16.0, 24.0);
        let target = Self::target(hit);
        if self.hover == Some(target) {
            p.rbox(r, 6.0, pal.hover);
        }
        p.text(face, 12.5, r.x + 8.0, cy, label, if self.hover == Some(target) { pal.text } else { ink }, w + 2.0);
        hits.push((r, target));
        r.w
    }

    /// The head of a card: what asks, a note, and the close button (Esc).
    #[allow(clippy::too_many_arguments)] // drawing call: a place plus what to draw there
    pub(super) fn head(
        &self,
        p: &mut Painter,
        hits: &mut Vec<(Rect, Target)>,
        r: Rect,
        chip: &str,
        (title, icon): (&str, &str),
        note: &str,
        close: bool,
    ) {
        let pal = self.pal;
        let ground = look::raised(pal);
        let tint = mix(ground, pal.text, 0.09);
        let mut right = r.right();
        if close {
            let b = Rect::new(r.right() - HEAD, r.y, HEAD, HEAD);
            let target = Self::target(Hit::Escape);
            if self.hover == Some(target) {
                p.rbox(b, 6.0, pal.hover);
            }
            p.icon(glyph::CLOSE, 13.0, b.cx(), b.cy(), if self.hover == Some(target) { pal.text } else { pal.muted });
            hits.push((b, target));
            right = b.x - 8.0;
        }
        let mut x = r.x;
        if !chip.is_empty() {
            let w = (p.measure(Face::Semibold, 11.5, chip) + 18.0).min((right - x).max(0.0));
            let pill = Rect::new(x, r.y + 1.0, w, 22.0);
            p.rbox(pill, 11.0, tint);
            p.text(Face::Semibold, 11.5, pill.x + 9.0, pill.cy(), chip, pal.muted, (w - 16.0).max(0.0));
            x += w + 10.0;
        }
        if !title.is_empty() {
            if !icon.is_empty() {
                let tile = Rect::new(x, r.y, 26.0, HEAD);
                p.rbox(tile, 7.0, tint);
                p.icon(icon, 14.0, tile.cx(), tile.cy(), pal.text);
                x += 36.0;
            }
            let note_w = if note.is_empty() { 0.0 } else { p.measure(Face::Ui, 11.5, note) + 12.0 };
            x += p.text(Face::Semibold, 14.5, x, r.cy(), title, pal.text, (right - x - note_w).max(0.0)) + 10.0;
        }
        if !note.is_empty() && right - x > 40.0 {
            p.text(Face::Ui, 11.5, x, r.cy() + 0.5, note, pal.muted, right - x);
        }
    }

    /// The choices as one list with hairlines between them: a key cap with the choice's number,
    /// its label, and its description when there is room for one. Returns the list's height.
    #[allow(clippy::too_many_arguments)] // drawing call: a place plus what to draw there
    pub(super) fn choices(
        &self,
        p: &mut Painter,
        hits: &mut Vec<(Rect, Target)>,
        x: f32,
        y: f32,
        w: f32,
        rows: &[Choice],
        enabled: bool,
        density: Density,
    ) -> f32 {
        let pal = self.pal;
        let edge = look::edge(pal);
        let total: f32 = rows.iter().map(|row| choice_height(row, density)).sum();
        if rows.is_empty() {
            return 0.0;
        }
        p.frame(Rect::new(x, y, w, total), 10.0, pal.page, edge);
        let mut top = y;
        for (i, choice) in rows.iter().enumerate() {
            let h = choice_height(choice, density);
            let row = Rect::new(x, top, w, h);
            let target = Self::target(choice.hit);
            // Nothing is pre-selected: the terminal's highlight is never shown as a choice. A
            // form's own picks are.
            let hot = enabled && self.hover == Some(target);
            if hot || choice.on {
                p.rbox(row.inset(2.0), 8.0, if hot { pal.hover } else { pal.selection });
            }
            if i > 0 {
                p.fill(Rect::new(x + 1.0, top, w - 2.0, p.px()), edge);
            }
            let noted = density == Density::Roomy && !choice.note.is_empty();
            let cy = if noted { top + 18.0 } else { row.cy() };
            let mut tx = x + 14.0;
            if let Some(number) = choice.number {
                let cap = Rect::new(x + 12.0, cy - 10.0, 20.0, 20.0);
                p.frame(cap, 6.0, look::raised(pal), edge);
                p.text_center(Face::Ui, 11.5, cap, &number.to_string(), pal.muted);
                tx = x + 44.0;
            }
            let mark = if choice.on { 30.0 } else { 14.0 };
            let ink = if enabled { pal.text } else { pal.muted };
            p.text(Face::Semibold, 13.5, tx, cy, &choice.label, ink, (row.right() - tx - mark).max(0.0));
            if noted {
                p.text(Face::Ui, 12.5, tx, top + 38.0, &choice.note, pal.muted, (row.right() - tx - mark).max(0.0));
            }
            if choice.on {
                p.icon(glyph::CHECK, 13.0, row.right() - 18.0, cy, pal.accent);
            }
            if enabled {
                hits.push((row, target));
            }
            top += h;
        }
        total
    }

    /// The card's own text field, with what sends it.
    fn field(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        hits: &mut Vec<(Rect, Target)>,
        r: Rect,
        placeholder: &str,
        send: &Send,
    ) {
        let pal = self.pal;
        let edge = look::edge(pal);
        let focus = chat.question.focus && self.focused;
        p.frame(r, 10.0, pal.page, if focus { mix(edge, pal.text, 0.45) } else { edge });
        let mut right = r.right() - 6.0;
        match send {
            Send::Arrow(hit) => {
                let b = Rect::new(right - 28.0, r.cy() - 14.0, 28.0, 28.0);
                let target = Self::target(*hit);
                let typed = !chat.question.field.text().trim().is_empty();
                let hot = self.hover == Some(target);
                let (fill, ink) = match (typed, hot) {
                    (true, _) => (pal.accent, pal.on_accent),
                    (false, true) => (pal.hover, pal.text),
                    _ => (look::quiet_fill(pal, pal.page), pal.muted),
                };
                look::round_icon(p, b, fill, look::ARROW_UP, 14.0, ink);
                hits.push((b, target));
                right = b.x - 6.0;
            }
            Send::Buttons(buttons) => {
                for (label, hit) in buttons.iter().rev() {
                    let w = p.measure(Face::Semibold, 12.0, label) + 22.0;
                    let b = Rect::new(right - w, r.cy() - 14.0, w, 28.0);
                    let target = Self::target(*hit);
                    let hot = self.hover == Some(target);
                    p.rbox(b, 14.0, if hot { pal.hover } else { look::quiet_fill(pal, pal.page) });
                    p.text_center(Face::Semibold, 12.0, b, label, pal.text);
                    hits.push((b, target));
                    right = b.x - 6.0;
                }
            }
        }
        let text = Rect::new(r.x + 14.0, r.y + 8.0, (right - r.x - 14.0).max(0.0), r.h - 16.0);
        let colors = FieldColors { text: pal.text, placeholder: pal.faint, selection: pal.selection, caret: pal.text };
        chat.question.field.draw(p, text, Face::Ui, 13.5, &colors, placeholder, focus, focus);
        hits.push((Rect::new(r.x, r.y, (right - r.x).max(0.0), r.h), Self::target(Hit::Field)));
    }

    /// The foot: plain buttons at the left, a word of the card's own, and "Open terminal" and
    /// what Esc does at the right.
    #[allow(clippy::too_many_arguments)] // drawing call: a place plus what to draw there
    pub(super) fn foot(
        &self,
        p: &mut Painter,
        hits: &mut Vec<(Rect, Target)>,
        r: Rect,
        left: &[(&str, Hit)],
        text: &str,
        right: &[(&str, Hit)],
    ) {
        let cy = r.cy();
        let mut end = r.right() + 8.0;
        for (label, hit) in right.iter().rev() {
            end -= self.plain(p, hits, end, cy, label, *hit, false) + 2.0;
        }
        let mut x = r.x - 8.0;
        for (label, hit) in left {
            let w = p.measure(Face::Semibold, 12.5, label) + 16.0;
            if x + w > end {
                break;
            }
            self.plain(p, hits, x + w, cy, label, *hit, true);
            x += w + 2.0;
        }
        if !text.is_empty() {
            let x = x + 8.0;
            p.text(Face::Ui, 12.0, x, cy, text, self.pal.muted, (end - x - 8.0).max(0.0));
        }
    }

    /// Lines of code in a well. An edit's lines keep their signs and get the diff's colours.
    pub(super) fn well(&self, p: &mut Painter, r: Rect, lines: &[String], colours: look::Diff) {
        let pal = self.pal;
        let ground = if pal.dark { pal.field } else { pal.bar };
        p.frame(r, 8.0, ground, look::edge(pal));
        let ink = colours.on(ground);
        let clip = p.clip(r.inset(1.0));
        for (i, line) in lines.iter().enumerate() {
            let cy = r.y + WELL_PAD + WELL_LINE * (i as f32 + 0.5);
            let tone = match line.chars().next() {
                Some('+') if line.starts_with("+ ") => Some(ink.add),
                Some('−') if line.starts_with("− ") => Some(ink.del),
                _ => None,
            };
            if let Some(tone) = tone {
                let row = Rect::new(r.x + 1.0, cy - WELL_LINE / 2.0, r.w - 2.0, WELL_LINE);
                p.fill(row, mix(ground, tone, if pal.dark { 0.17 } else { 0.13 }));
            }
            p.text(Face::Mono, 12.5, r.x + 12.0, cy, line, pal.text, (r.w - 24.0).max(0.0));
        }
        p.restore(clip);
    }

    /// Draws a card from its parts, inside the card's box.
    pub(super) fn parts(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        r: Rect,
        parts: &[Part],
        hits: &mut Vec<(Rect, Target)>,
    ) {
        let pal = self.pal;
        let density = chat.question.density;
        let colours = chat.question.colours.unwrap_or_else(|| look::Diff::fallback(pal));
        let (x, w) = (r.x + INSET, (r.w - 2.0 * INSET).max(1.0));
        let mut y = r.y + INSET;
        for part in parts {
            let h = part.height(density);
            match part {
                Part::Head { chip, title, icon, note, close } => {
                    self.head(p, hits, Rect::new(x, y, w, HEAD), chip, (title, icon), note, *close);
                }
                Part::Ask { text, size, height } => {
                    let clip = p.clip(Rect::new(x, y, w, *height));
                    if crate::view::working::spinner_row(text) {
                        // A row the CLI animates keeps its glyph in a fixed slot.
                        crate::view::working::draw_line(p, *size, x, y + height / 2.0, text, pal.text, w);
                    } else {
                        ask_layout(text, pal, p.scale, w, *size).draw(p, x, y, None, pal);
                    }
                    p.restore(clip);
                }
                Part::Rows(rows) => {
                    for (i, row) in rows.iter().enumerate() {
                        let cy = y + TEXT_ROW * (i as f32 + 0.5);
                        copied_row(p, Face::Ui, 13.5, x, cy, row, look::soft(pal), w);
                    }
                }
                Part::Well(lines) => self.well(p, Rect::new(x, y, w, h - WELL_GAP), lines, colours),
                Part::Panel { rows, more } => {
                    for (i, row) in rows.iter().enumerate() {
                        let cy = y + PANEL_ROW * (i as f32 + 0.5);
                        match screens::usage_bar(row) {
                            Some((percent, label)) => {
                                let label_w = p.measure(Face::Ui, 11.0, label);
                                let track_w = (w - label_w - 10.0).clamp(0.0, BAR_W);
                                let track = Rect::new(x, cy - 3.0, track_w, 6.0);
                                p.rbox(track, 3.0, pal.bar);
                                let filled = (track_w * f32::from(percent) / 100.0).round();
                                if filled > 0.0 {
                                    p.rbox(Rect::new(track.x, track.y, filled.max(6.0), track.h), 3.0, pal.accent);
                                }
                                p.text(Face::Ui, 11.0, track.right() + 10.0, cy, label, pal.text, label_w + 2.0);
                            }
                            None => copied_row(p, Face::Mono, 11.0, x, cy, row, pal.muted, w),
                        }
                    }
                    if *more > 0 {
                        let cy = y + PANEL_ROW * (rows.len() as f32 + 0.5);
                        p.text(Face::Ui, 11.0, x, cy, &format!("{more} more rows in the terminal"), pal.muted, w);
                    }
                }
                Part::Choices { rows, enabled } => {
                    self.choices(p, hits, x, y, w, rows, *enabled, density);
                }
                Part::Field { placeholder, send } => {
                    self.field(p, chat, hits, Rect::new(x, y, w, FIELD_ROW), placeholder, send);
                }
                Part::Status { text, warn } => {
                    let ink = if *warn { pal.warn } else { pal.muted };
                    p.text(Face::Ui, 12.0, x, y + STATUS_ROW / 2.0 - 2.0, text, ink, w);
                }
                Part::Foot { left, text, right } => self.foot(p, hits, Rect::new(x, y, w, FOOT), left, text, right),
            }
            y += h;
        }
    }
}

/// Fits a panel's rows to the room its card got: the rows from its top, and how many are left
/// to the terminal.
pub(super) fn fit_panel(parts: &mut [Part], room: f32, density: Density) {
    let over = height(parts, density) - room;
    if over <= 0.0 {
        return;
    }
    for part in parts {
        if let Part::Panel { rows, more } = part {
            let cut = ((over / PANEL_ROW).ceil() as usize + 1).min(rows.len().saturating_sub(1));
            *more = cut;
            rows.truncate(rows.len() - cut);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::demo::Transcript;
    use crate::view::chat::readme_shots::{self as shots, Size};

    const WINDOW: Size = Size { w: 1100.0, h: 820.0, scale: 1.0 };
    const BENCH: (&str, &str, &str) = (
        "Measure eviction before and after the change.",
        "I’ll run the eviction benchmark on this version, then on the one before it.",
        "cargo bench -p tiles --bench eviction",
    );

    fn window(then: (&str, &str, &str), screen: &str) -> (Transcript, crate::state::AppState, u64, Instant) {
        let demo = Transcript::new_showcase(Some(then)).unwrap();
        let (mut st, id) = shots::chat_window(true, &demo);
        shots::running(&mut st, id, "Waiting");
        let later = shots::show(&mut st, id, screen, Instant::now() + Duration::from_secs(1));
        (demo, st, id, later)
    }

    fn chat(st: &mut crate::state::AppState, id: u64) -> &mut Chat {
        &mut shots::tab(st, id).pane.as_mut().unwrap().chat
    }

    fn labels(parts: &[Part]) -> Vec<(Option<usize>, String, String)> {
        parts
            .iter()
            .find_map(|part| match part {
                Part::Choices { rows, .. } => {
                    Some(rows.iter().map(|c| (c.number, c.label.clone(), c.note.clone())).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The CLI's menu for a question has rows for "Type something." and "Chat about this"; the
    /// card showed them as choices and again as a field and a button. Each is there once.
    #[test]
    fn a_question_lists_its_options_once_with_the_field_and_chat_about_this_apart() {
        let then = ("Now add elevation tiles to the same cache.", "One choice first.", "");
        let (_demo, mut st, id, _) = window(then, shots::QUESTION);
        let pane = shots::tab(&mut st, id).pane.as_ref().unwrap();
        let parts = parts(&pane.chat, &pane.model, 700.0).expect("the question's card");
        // The CLI's own header, as the card's chip; Esc is the close button.
        assert!(matches!(&parts[0], Part::Head { chip, close: true, .. } if chip == "Elevation"), "{:?}", parts[0]);
        assert!(matches!(&parts[1], Part::Ask { text, .. } if text == "Where should the elevation tiles be cached?"));
        let choices = labels(&parts);
        assert_eq!(
            choices.iter().map(|(n, label, _)| (*n, label.as_str())).collect::<Vec<_>>(),
            [
                (Some(1), "Beside the map tiles (Recommended)"),
                (Some(2), "In a folder of their own"),
                (Some(3), "Only in memory")
            ]
        );
        assert_eq!(choices[0].2, "One cache folder and one size limit for both kinds of tile.");
        // "Type something." is the field; "Chat about this" a button that presses its own row.
        assert!(parts.contains(&Part::Field { placeholder: "Something else…", send: Send::Arrow(Hit::Answer) }));
        let foot = parts.last().unwrap();
        assert_eq!(
            foot,
            &Part::Foot {
                left: vec![("Chat about this", Hit::Option(4))],
                text: "",
                right: vec![("Open terminal", Hit::Open), ("Esc to skip", Hit::Escape)],
            }
        );
    }

    /// The permission card showed the command from the transcript and again among the screen's
    /// rows, under two titles. Its title is the prompt's own first row, and the command is there
    /// once.
    #[test]
    fn a_permission_prompt_shows_the_command_once_under_the_prompts_own_title() {
        let (_demo, mut st, id, _) = window(BENCH, shots::PERMISSION);
        let pane = shots::tab(&mut st, id).pane.as_ref().unwrap();
        let parts = parts(&pane.chat, &pane.model, 700.0).expect("the permission card");
        assert!(matches!(&parts[0], Part::Head { title, chip, .. } if title == "Bash command" && chip.is_empty()));
        assert_eq!(parts[1], Part::Rows(vec!["Run the eviction benchmark".into()]));
        assert_eq!(parts[2], Part::Well(vec!["cargo bench -p tiles --bench eviction".into()]));
        let command = parts
            .iter()
            .map(|part| match part {
                Part::Rows(rows) | Part::Well(rows) => rows.iter().filter(|r| r.contains("cargo bench")).count(),
                Part::Head { title, .. } => usize::from(title.contains("cargo bench")),
                _ => 0,
            })
            .sum::<usize>();
        assert_eq!(command, 1, "the command is shown once");
        assert!(matches!(&parts[3], Part::Ask { text, .. } if text == "Do you want to proceed?"));
        // The choices in the CLI's exact words, numbered as its menu numbers them.
        let choices = labels(&parts);
        assert_eq!(choices.len(), 4);
        assert_eq!((choices[0].0, choices[0].1.as_str()), (Some(1), "Yes"));
        assert_eq!((choices[3].0, choices[3].1.as_str()), (Some(4), "No"));
        assert!(choices[1].1.starts_with("Yes, and don't ask again for cargo bench commands in "));
    }

    /// A card that waits for an answer stands where the composer is, as wide as its box. The
    /// composer is not drawn and takes no click; its draft is kept and comes back with it.
    #[test]
    fn a_card_takes_the_composers_place_and_the_draft_comes_back_with_the_composer() {
        use crate::state::Target;
        let (_demo, mut st, id, later) = window(BENCH, shots::PERMISSION);
        chat(&mut st, id).editor.set("a draft to keep".into());
        let mut anim = mdshared_ui::anim::Animator::new();
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        let composer = |st: &crate::state::AppState| {
            st.hits
                .iter()
                .filter(|(_, t)| matches!(t, Target::Chat(Control::Editor | Control::Send | Control::Attach)))
                .count()
        };
        let layout = chat(&mut st, id).layout;
        let card = chat(&mut st, id).question.rect;
        assert!(chat(&mut st, id).question.docked());
        assert_eq!(
            (card.x, card.w, card.bottom()),
            (layout.box_frame.x, layout.box_frame.w, layout.box_frame.bottom())
        );
        assert!(layout.conversation.bottom() <= card.y, "the conversation ends over the card");
        assert_eq!(composer(&st), 0, "the composer is not drawn under a card");
        let yes = Target::Chat(Control::Question(Hit::Option(0)));
        assert!(st.hits.iter().any(|(r, t)| *t == yes && card.contains(r.cx(), r.cy())));
        assert_eq!(chat(&mut st, id).editor.text, "a draft to keep");
        // The prompt is gone: the composer is back, with the draft.
        let idle = shots::show(&mut st, id, shots::IDLE, later + Duration::from_secs(1));
        shots::frame_at(&mut st, &mut anim, idle, WINDOW);
        assert!(!chat(&mut st, id).question.docked());
        assert_eq!(composer(&st), 3);
        assert_eq!(chat(&mut st, id).editor.text, "a draft to keep");
    }

    /// A short window drops the descriptions, then lowers the choices, before the card is cut.
    #[test]
    fn a_short_window_makes_a_card_denser_before_it_cuts_it() {
        let choice = |n: usize| Choice {
            number: Some(n),
            label: format!("Choice {n}"),
            note: "What it means.".into(),
            hit: Hit::Option(n - 1),
            on: false,
        };
        let parts = vec![
            Part::Head { chip: "Size".into(), title: String::new(), icon: "", note: String::new(), close: true },
            ask("Which size?", 600.0, 15.5),
            Part::Choices { rows: (1..=4).map(choice).collect(), enabled: true },
            Part::Foot { left: Vec::new(), text: "", right: vec![("Open terminal", Hit::Open)] },
        ];
        let roomy = height(&parts, Density::Roomy);
        let plain = height(&parts, Density::Plain);
        let tight = height(&parts, Density::Tight);
        assert!(roomy > plain && plain > tight);
        assert_eq!(density_for(&parts, roomy), Density::Roomy);
        assert_eq!(density_for(&parts, roomy - 1.0), Density::Plain);
        assert_eq!(density_for(&parts, plain - 1.0), Density::Tight);
        assert_eq!(density_for(&parts, 10.0), Density::Tight);
    }
}
