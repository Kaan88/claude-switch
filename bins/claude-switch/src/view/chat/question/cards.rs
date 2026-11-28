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
pub(super) const TIGHT_ROW: f32 = 30.0;
const LIST_GAP: f32 = 10.0;
const FIELD_ROW: f32 = 40.0;
const FIELD_GAP: f32 = 12.0;
/// The foot: plain buttons and what Esc does.
pub(super) const FOOT: f32 = 24.0;
const STATUS_ROW: f32 = 22.0;
/// A row copied from the CLI as text.
const TEXT_ROW: f32 = 20.0;
/// The note under a question cut short.
const CUT_ROW: f32 = 18.0;
const CUT_NOTE: &str = "The rest of the question is in the terminal.";
/// The most lines a question shows, and the fewest it is cut to when the card must be short.
const ASK_LINES: usize = 8;
const ASK_LEAST: usize = 2;
/// The fewest lines the command well and the prompt's rows are cut to.
const WELL_LEAST: usize = 2;
const ROWS_LEAST: usize = 1;
/// How far one wheel notch scrolls a card taller than its room.
pub(super) const WHEEL_STEP: f32 = 48.0;
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
    /// The question, wrapped: measured at the window's scale when the card is laid out. `lines`
    /// of its `total` lines show; a question cut short says so under it.
    Ask {
        text: String,
        size: f32,
        /// One line's height.
        line: f32,
        total: usize,
        lines: usize,
    },
    /// Rows of the CLI as text, one line each.
    Rows(Vec<String>),
    /// Code as the transcript has it (a command, an edit's lines), in a well; `more` of its
    /// lines are not shown, which its last row says.
    Well {
        lines: Vec<String>,
        more: usize,
    },
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
            Part::Ask { line, total, lines, .. } => {
                let cut = if lines < total { CUT_ROW } else { 0.0 };
                (*lines as f32 * line).ceil() + 1.0 + cut + ASK_GAP
            }
            Part::Rows(rows) => rows.len() as f32 * TEXT_ROW + 6.0,
            Part::Well { lines, more } => {
                (lines.len() + usize::from(*more > 0)) as f32 * WELL_LINE + 2.0 * WELL_PAD + WELL_GAP
            }
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

/// The question's text, set as the card sets it: semibold, wrapped at `width`.
fn ask_layout(text: &str, pal: &Palette, scale: f32, width: f32, size: f32) -> TextLayout {
    TextLayout::heading(text, pal, scale, width.max(40.0), size)
}

/// The question's height and one line's height, measured as it is drawn at `scale`. The last
/// few are kept: laying out text is the costly part of a card, which is laid out every frame.
fn measure(text: &str, width: f32, size: f32, scale: f32) -> (f32, f32) {
    type Key = (String, u32, u32, u32);
    thread_local! {
        static MEASURED: std::cell::RefCell<Vec<(Key, (f32, f32))>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = (text.to_owned(), width.to_bits(), size.to_bits(), scale.to_bits());
    MEASURED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, found)) = cache.iter().find(|(k, _)| *k == key) {
            return *found;
        }
        let pal = &mdshared_ui::theme::DARK;
        let height = ask_layout(text, pal, scale, width, size).height();
        let line = ask_layout("Ag", pal, scale, width, size).height().max(1.0);
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((key, (height, line)));
        (height, line)
    })
}

fn ask(text: &str, width: f32, size: f32, scale: f32) -> Part {
    let (height, line) = measure(text, width, size, scale);
    let total = ((height / line).round() as usize).max(1);
    Part::Ask { text: text.to_owned(), size, line, total, lines: total.min(ASK_LINES) }
}

/// Fits the parts into `room`: the roomiest density that fits; else low choices, with the
/// command well, the prompt's copied rows and the question cut shorter step by step (each says
/// what it left out), and a panel's rows cut. The choices and the field are never cut: what still
/// does not fit scrolls, with the foot kept in view (`Draw::parts`).
pub(super) fn fit(parts: &mut [Part], room: f32) -> Density {
    if let Some(density) =
        [Density::Roomy, Density::Plain, Density::Tight].into_iter().find(|d| height(parts, *d) <= room)
    {
        return density;
    }
    let density = Density::Tight;
    type Step = fn(&mut Part, usize);
    let well: Step = |part, keep| {
        if let Part::Well { lines, more } = part
            && lines.len() > keep
        {
            *more += lines.len() - keep;
            lines.truncate(keep);
        }
    };
    let rows: Step = |part, keep| {
        if let Part::Rows(rows) = part
            && rows.len() > keep
        {
            rows.drain(..rows.len() - keep);
        }
    };
    let ask: Step = |part, keep| {
        if let Part::Ask { lines, .. } = part {
            *lines = (*lines).min(keep);
        }
    };
    let steps: [(Step, usize); 7] =
        [(well, 4), (rows, 2), (ask, 4), (well, WELL_LEAST), (rows, ROWS_LEAST), (ask, ASK_LEAST), (ask, ASK_LEAST)];
    for (step, keep) in steps {
        parts.iter_mut().for_each(|part| step(part, keep));
        if height(parts, density) <= room {
            return density;
        }
    }
    fit_panel(parts, room, density);
    density
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A question's "Chat about this" row: a button of the foot, not a choice of the list.
fn chat_about(label: &str) -> bool {
    label.trim().eq_ignore_ascii_case("Chat about this")
}

fn status(chat: &Chat) -> Option<Part> {
    match (chat.question.error, chat.question_notice().or_else(|| chat.question.typed_hint())) {
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
    let scale = card.scale();
    match pending(model).map(|p| p.kind) {
        Some(Kind::Question(questions)) => Some(question(chat, questions, width, scale)),
        Some(Kind::Tool(kind)) if card.shown.is_some() => Some(permission(chat, kind, width, scale)),
        _ => card.menu.as_ref().map(|menu| self::menu(chat, menu, width, scale)),
    }
}

/// Claude's question, from the transcript: one question, or a form's question on its page.
fn question(chat: &Chat, questions: Option<&[Question]>, width: f32, scale: f32) -> Vec<Part> {
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
            ask("Claude is asking a question", width, 15.5, scale),
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
        ask(&question.question, width, 15.5, scale),
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
fn permission(chat: &Chat, kind: &ItemKind, width: f32, scale: f32) -> Vec<Part> {
    let card = &chat.question;
    let Some(shown) = card.shown.as_ref() else { return Vec::new() };
    let can_answer = shown.matched && (card.answerable.is_some() || card.failed);
    let close = card.answerable.is_some() && card.step.is_none();
    let (detail, more) = detail(kind);
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
        parts.push(Part::Well { lines: detail, more });
    }
    parts.push(ask(&shown.question, width, 14.0, scale));
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
fn menu(chat: &Chat, menu: &MenuShown, width: f32, scale: f32) -> Vec<Part> {
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
            parts.push(ask(&menu.title, width, 14.0, scale));
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
    let mut parts = vec![head(chip, close), ask(title, width, 15.5, scale)];
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
    pub(super) fn well(&self, p: &mut Painter, r: Rect, lines: &[String], more: usize, colours: look::Diff) {
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
        if more > 0 {
            let cy = r.y + WELL_PAD + WELL_LINE * (lines.len() as f32 + 0.5);
            let note = format!("+{more} more {} in the terminal", if more == 1 { "line" } else { "lines" });
            p.text(Face::Ui, 12.0, r.x + 12.0, cy, &note, pal.muted, (r.w - 24.0).max(0.0));
        }
        p.restore(clip);
    }

    /// Draws a card from its parts, inside the card's box. A card taller than its box scrolls
    /// (the wheel over it), with its foot kept at the bottom; nothing outside what shows takes a
    /// click.
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
        let over = (height(parts, density) - r.h).max(0.0).floor();
        let scroll = chat.question.scroll_parts(over);
        let pinned = over > 0.0 && matches!(parts.last(), Some(Part::Foot { .. }));
        let foot_y = r.bottom() - (INSET - 6.0) - FOOT;
        let body = if pinned { Rect::new(r.x, r.y, r.w, (foot_y - 6.0 - r.y).max(0.0)) } else { r };
        let clip = p.clip(body);
        let first = hits.len();
        let mut y = r.y + INSET - scroll;
        let shown = if pinned { &parts[..parts.len() - 1] } else { parts };
        for part in shown {
            let h = part.height(density);
            if y < body.bottom() && y + h > body.y {
                self.part(p, chat, hits, Rect::new(x, y, w, h), part, density, colours);
            }
            y += h;
        }
        p.restore(clip);
        clip_hits(hits, first, body);
        if pinned {
            // The foot stays in view over the scrolled parts, with a hairline and the scroll's
            // place along the card's right edge.
            let edge = look::edge(pal);
            p.fill(Rect::new(r.x + INSET, foot_y - 6.0, w, p.px()), edge);
            let track = Rect::new(r.right() - 7.0, body.y + 10.0, 3.0, (body.h - 20.0).max(0.0));
            let thumb_h = (track.h * body.h / (body.h + over)).clamp(16.0_f32.min(track.h), track.h);
            let thumb_y = track.y + (track.h - thumb_h) * if over > 0.0 { scroll / over } else { 0.0 };
            p.rbox(Rect::new(track.x, thumb_y, track.w, thumb_h), 1.5, pal.thumb);
            if let Some(part) = parts.last() {
                let foot = Rect::new(x, foot_y, w, FOOT);
                self.part(p, chat, hits, foot, part, density, colours);
            }
        }
    }

    /// One part of a card in its place.
    #[allow(clippy::too_many_arguments)] // drawing call: a place plus what to draw there
    fn part(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        hits: &mut Vec<(Rect, Target)>,
        at: Rect,
        part: &Part,
        density: Density,
        colours: look::Diff,
    ) {
        let pal = self.pal;
        let (x, y, w, h) = (at.x, at.y, at.w, at.h);
        match part {
            Part::Head { chip, title, icon, note, close } => {
                self.head(p, hits, Rect::new(x, y, w, HEAD), chip, (title, icon), note, *close);
            }
            Part::Ask { text, size, line, total, lines } => {
                let shown = (*lines as f32 * line).ceil() + 1.0;
                let clip = p.clip(Rect::new(x, y, w, shown));
                if crate::view::working::spinner_row(text) {
                    // A row the CLI animates keeps its glyph in a fixed slot.
                    crate::view::working::draw_line(p, *size, x, y + shown / 2.0, text, pal.text, w);
                } else {
                    ask_layout(text, pal, p.scale, w, *size).draw(p, x, y, None, pal);
                }
                p.restore(clip);
                if lines < total {
                    p.text(Face::Ui, 12.0, x, y + shown + CUT_ROW / 2.0, CUT_NOTE, pal.muted, w);
                }
            }
            Part::Rows(rows) => {
                for (i, row) in rows.iter().enumerate() {
                    let cy = y + TEXT_ROW * (i as f32 + 0.5);
                    copied_row(p, Face::Ui, 13.5, x, cy, row, look::soft(pal), w);
                }
            }
            Part::Well { lines, more } => self.well(p, Rect::new(x, y, w, h - WELL_GAP), lines, *more, colours),
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
    }
}

/// Every hit pushed since `first` takes clicks only where it shows inside `shown`: one wholly
/// outside goes.
pub(super) fn clip_hits(hits: &mut Vec<(Rect, Target)>, first: usize, shown: Rect) {
    let mut at = first;
    while at < hits.len() {
        let r = hits[at].0;
        let (x0, y0) = (r.x.max(shown.x), r.y.max(shown.y));
        let (x1, y1) = (r.right().min(shown.right()), r.bottom().min(shown.bottom()));
        if x1 > x0 && y1 > y0 {
            hits[at].0 = Rect::new(x0, y0, x1 - x0, y1 - y0);
            at += 1;
        } else {
            hits.remove(at);
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
        assert_eq!(parts[2], Part::Well { lines: vec!["cargo bench -p tiles --bench eviction".into()], more: 0 });
        let command = parts
            .iter()
            .map(|part| match part {
                Part::Rows(rows) | Part::Well { lines: rows, .. } => {
                    rows.iter().filter(|r| r.contains("cargo bench")).count()
                }
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
            ask("Which size?", 600.0, 15.5, 1.0),
            Part::Choices { rows: (1..=4).map(choice).collect(), enabled: true },
            Part::Foot { left: Vec::new(), text: "", right: vec![("Open terminal", Hit::Open)] },
        ];
        let roomy = height(&parts, Density::Roomy);
        let plain = height(&parts, Density::Plain);
        let tight = height(&parts, Density::Tight);
        assert!(roomy > plain && plain > tight);
        let fitted = |room: f32| fit(&mut parts.clone(), room);
        assert_eq!(fitted(roomy), Density::Roomy);
        assert_eq!(fitted(roomy - 1.0), Density::Plain);
        assert_eq!(fitted(plain - 1.0), Density::Tight);
        assert_eq!(fitted(10.0), Density::Tight);
    }

    fn card_hits(st: &crate::state::AppState) -> Vec<(Rect, Hit)> {
        st.hits
            .iter()
            .filter_map(|(r, t)| match t {
                crate::state::Target::Chat(Control::Question(hit)) => Some((*r, *hit)),
                _ => None,
            })
            .collect()
    }

    fn inside(r: Rect, card: Rect) -> bool {
        r.x >= card.x - 0.01
            && r.right() <= card.right() + 0.01
            && r.y >= card.y - 0.01
            && r.bottom() <= card.bottom() + 0.01
    }

    /// The review's case: a permission card with a six-line command needs more than the room a
    /// short window leaves. The well is cut first (and says so), the choices get lower, and what
    /// still does not fit scrolls under a foot that stays in view. Every choice can be reached,
    /// and no target of the card lies outside it.
    #[test]
    fn a_tall_card_in_a_short_window_keeps_every_choice_reachable_and_no_click_outside_it() {
        use crate::view::chat::shots::{LONG_COMMAND, long_permission};
        let short = Size { w: 1100.0, h: 560.0, scale: 1.5 };
        let demo = Transcript::new_showcase(Some((BENCH.0, BENCH.1, LONG_COMMAND))).unwrap();
        let (mut st, id) = shots::chat_window(true, &demo);
        shots::running(&mut st, id, "Waiting");
        let later = shots::show(&mut st, id, &long_permission(), Instant::now() + Duration::from_secs(1));
        let mut anim = mdshared_ui::anim::Animator::new();
        // The second frame is laid out at the window's scale.
        shots::frame_at(&mut st, &mut anim, later, short);
        shots::frame_at(&mut st, &mut anim, later, short);
        let layout = chat(&mut st, id).layout;
        let card = chat(&mut st, id).question.rect;
        let room = ((layout.box_frame.bottom() - layout.conversation.y - 40.0) * CARD_SHARE).floor();
        assert!(card.h > 0.0 && card.h <= room + 0.5 && chat(&mut st, id).question.docked());
        let laid = chat(&mut st, id).question.laid.clone().expect("the parts");
        assert!(
            laid.iter()
                .any(|part| matches!(part, Part::Well { lines, more } if lines.len() < 6 && lines.len() + more == 6)),
            "the well is cut and counts what it left out: {laid:?}"
        );
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..30 {
            let hits = card_hits(&st);
            for (r, hit) in &hits {
                assert!(inside(*r, card), "{hit:?} at {r:?} outside the card {card:?}");
            }
            // The foot stays: Open terminal and Esc whatever the scroll.
            assert!(hits.iter().any(|(_, h)| *h == Hit::Open) && hits.iter().any(|(_, h)| *h == Hit::Escape));
            seen.extend(hits.iter().filter_map(|(_, h)| match h {
                Hit::Option(i) => Some(*i),
                _ => None,
            }));
            assert!(chat(&mut st, id).question_wheel(card.cx(), card.cy(), -1.0), "the card takes the wheel");
            shots::frame_at(&mut st, &mut anim, later, short);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [0, 1, 2, 3], "every choice, No included, is reachable");
    }

    /// A question on a new session was laid out where the greeting stands, in the middle of the
    /// window: the card belongs at the window's foot, where the composer is.
    #[test]
    fn a_card_on_a_new_session_stands_at_the_windows_foot_not_in_the_greetings_place() {
        let demo = Transcript::new_empty().unwrap();
        let mut st = shots::window(true);
        let id = shots::claude_tab(&mut st, "Move the map tiles to a worker thread");
        let mut pane = st.new_session_pane(crate::view::model::Source::File(demo.path().into()), demo.cwd().into());
        pane.chat.scripted = true;
        pane.chat.enabled = true;
        pane.chat.identity_unverified = false;
        pane.chat.awaiting_transcript = true;
        pane.ui.set_chat(true);
        shots::tab(&mut st, id).pane = Some(pane);
        let now = Instant::now() + Duration::from_secs(1);
        let later = shots::show(&mut st, id, shots::IDLE, now);
        shots::frame_at(&mut st, &mut mdshared_ui::anim::Animator::new(), later, WINDOW);
        assert!(chat(&mut st, id).layout.greeting.h > 0.0, "the greeting while nothing asks");
        let later = shots::show(&mut st, id, shots::QUESTION, later);
        shots::frame_at(&mut st, &mut mdshared_ui::anim::Animator::new(), later, WINDOW);
        let pane = shots::tab(&mut st, id).pane.as_ref().unwrap();
        assert!(pane.chat.question.menu.is_some() && !pane.chat.welcome(&pane.model));
        let (layout, card) = (pane.chat.layout, pane.chat.question.rect);
        assert_eq!(layout.greeting.h, 0.0);
        assert!(pane.chat.question.docked() && card.bottom() == layout.box_frame.bottom());
        assert!(layout.composer.bottom() >= WINDOW.h - 0.5, "at the window's foot");
    }

    /// "Run in background" stands on the command's line in the conversation; scrolled out of
    /// view, it had no place at all. The card offers it then.
    #[test]
    fn run_in_background_falls_back_to_the_card_when_its_line_is_out_of_view() {
        use crate::view::chat::shots::{WORKING, WORKING_LINE};
        let (_demo, mut st, id, later) = window(BENCH, WORKING);
        shots::running(&mut st, id, WORKING_LINE);
        let mut anim = mdshared_ui::anim::Animator::new();
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        let background = |st: &crate::state::AppState| card_hits(st).into_iter().any(|(_, h)| h == Hit::Background);
        assert!(chat(&mut st, id).question.offer_inline, "on the command's line");
        assert!(!chat(&mut st, id).question.visible(), "no card for it");
        assert!(background(&st));
        // Scrolled back to the top: the line is out of view, and the card offers it.
        let area = chat(&mut st, id).layout.conversation;
        shots::tab(&mut st, id).pane.as_mut().unwrap().ui.wheel(area.cx(), area.cy(), 5000.0);
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        let card = chat(&mut st, id).question.rect;
        assert!(!chat(&mut st, id).question.offer_inline && card.h > 0.0);
        assert!(card_hits(&st).iter().any(|(r, h)| *h == Hit::Background && inside(*r, card)));
    }

    /// A long question was cut at four lines without a word, measured at scale 1. It is
    /// measured at the window's scale, shows up to eight lines, says when it is cut, and gets
    /// shorter when the card must.
    #[test]
    fn a_long_question_says_when_it_is_cut_and_takes_part_in_the_fitting() {
        let long = "Which of these caches should the elevation tiles share, given that ".repeat(12);
        for scale in [1.0, 1.5, 2.0] {
            let Part::Ask { total, lines, line, .. } = ask(&long, 420.0, 15.5, scale) else { panic!() };
            assert!(total > ASK_LINES && lines == ASK_LINES, "{scale}: {total} {lines}");
            let part = ask(&long, 420.0, 15.5, scale);
            let cut = (lines as f32 * line).ceil() + 1.0 + ASK_GAP;
            assert_eq!(part.height(Density::Roomy), cut + CUT_ROW, "room for the note under it");
        }
        let short = ask("Which size?", 420.0, 15.5, 1.5);
        assert!(matches!(short, Part::Ask { total: 1, lines: 1, .. }));
        assert_eq!(short.height(Density::Roomy), short.height(Density::Tight));
        // A short card: the question gives up lines before anything else is cut.
        let mut parts = vec![
            Part::Head { chip: "Size".into(), title: String::new(), icon: "", note: String::new(), close: true },
            ask(&long, 420.0, 15.5, 1.0),
            Part::Choices {
                rows: vec![Choice {
                    number: Some(1),
                    label: "One".into(),
                    note: String::new(),
                    hit: Hit::Option(0),
                    on: false,
                }],
                enabled: true,
            },
            Part::Foot { left: Vec::new(), text: "", right: vec![("Open terminal", Hit::Open)] },
        ];
        let full = height(&parts, Density::Tight);
        fit(&mut parts, full - 60.0);
        assert!(matches!(parts[1], Part::Ask { lines, .. } if lines < ASK_LINES));
    }

    /// The blocked-calls card used the old 12 px padding and pill buttons inside the raised box:
    /// it has the cards' padding, its calls as the cards' list and plain buttons now.
    #[test]
    fn the_blocked_calls_card_is_laid_out_as_the_other_cards() {
        use crate::view::chat::question::screens::recorded::BLOCKED_ONE;
        let (_demo, mut st, id, later) = window(BENCH, BLOCKED_ONE);
        let mut anim = mdshared_ui::anim::Animator::new();
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        let card = chat(&mut st, id).question.rect;
        assert!(card.h > 0.0 && chat(&mut st, id).question.blocked.is_some());
        let hits = card_hits(&st);
        assert!(!hits.is_empty());
        for (r, hit) in &hits {
            assert!(
                r.x >= card.x + INSET - 8.0 - 0.01 && r.right() <= card.right() - INSET + 8.0 + 0.01,
                "{hit:?} {r:?}"
            );
            if matches!(hit, Hit::Open | Hit::Escape | Hit::Approve | Hit::Retry) {
                assert_eq!(r.h, 24.0, "{hit:?}: a plain button, not the old pill");
            }
        }
    }

    /// Typing while a card stands in the composer's place goes nowhere, as before, but the card
    /// says so once, until it is gone.
    #[test]
    fn a_key_that_goes_nowhere_under_a_card_says_where_typing_goes() {
        let (_demo, mut st, id, later) = window(BENCH, shots::PERMISSION);
        let mut anim = mdshared_ui::anim::Animator::new();
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        chat(&mut st, id).question.note_typed_away();
        shots::frame_at(&mut st, &mut anim, later, WINDOW);
        let laid = chat(&mut st, id).question.laid.clone().unwrap();
        assert!(laid.contains(&Part::Status { text: TYPED_AWAY, warn: false }), "{laid:?}");
        let idle = shots::show(&mut st, id, shots::IDLE, later + Duration::from_secs(1));
        shots::frame_at(&mut st, &mut anim, idle, WINDOW);
        assert!(chat(&mut st, id).question.typed_hint().is_none());
    }
}
