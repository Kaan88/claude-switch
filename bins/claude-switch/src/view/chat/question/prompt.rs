//! A permission prompt answered from the screen alone: one the transcript's pending call does not
//! cover (a subagent's, one of several calls of a message, a prompt that does not name the call).
//! The card shows the prompt as Claude Code draws it, from its own top rule down to its question:
//! what is asked for and by whom, the command or file, the question and the choices (Claude Code
//! 2.1.332, the `perm_agent_*` and `perm_two_*` fixtures).
//!
//! Nothing is approved that the card did not show. A choice takes a click only once every row has
//! been in view (a long prompt scrolls) for a moment, and each key is written only while the
//! screen still shows exactly those rows, that question and those choices: the arrows first, then
//! Enter once the highlight is on the chosen row. A prompt whose top is above the terminal's
//! screen is shown as far as it can be read and left to the terminal. A decline with text stays in
//! the terminal too: how the comment field looks on the screen is not recorded.
use super::*;

/// How long the card must have shown a prompt before a choice is taken: a second click meant for
/// the prompt before it must not answer the next one.
pub(super) const SEEN: Duration = Duration::from_millis(500);
pub(super) const TOP_HIDDEN: &str =
    "The top of this request is above the terminal's screen, so Chat cannot show all of it. Answer it in the terminal.";
pub(super) const TOO_WIDE: &str = "A row of this request does not fit the card. Answer it in the terminal.";
pub(super) const READ_ALL: &str = "Scroll to the end of the request to answer it.";
pub(super) const NOT_SEEN: &str = "This request has only just appeared. Read it, then answer.";
/// The rows the wheel and the More button move by.
const WHEEL: usize = 3;
const MONO: f32 = 11.5;
/// A monospace character's width before the first paint measured it.
const MONO_W: f32 = 7.0;
/// The fewest characters a wrapped row holds, whatever the width.
const MIN_CHARS: usize = 24;

/// A permission prompt as last read from the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Prompt {
    /// Its rows above the question, as drawn: all of them from the top rule down when `whole`,
    /// else the last ones the screen shows.
    pub rows: Vec<String>,
    pub question: String,
    pub choices: Vec<String>,
    /// The prompt's own top rule is on the screen: `rows` is all of it.
    pub whole: bool,
}

pub(super) fn shown(screen: &Screen) -> Option<Prompt> {
    let prompt = screens::permission(screen)?;
    let rows = screens::permission_rows(screen, &prompt);
    let whole = rows.is_some();
    let choices = prompt.menu.texts();
    Some(Prompt { rows: rows.unwrap_or(prompt.body), question: prompt.question, choices, whole })
}

/// The screen still shows this prompt, row for row: its menu, for the highlight.
pub(super) fn still_shown(want: &Prompt, screen: &Screen) -> Option<screens::Menu> {
    let prompt = screens::permission(screen)?;
    let rows = screens::permission_rows(screen, &prompt)?;
    (want.whole && rows == want.rows && prompt.question == want.question && prompt.menu.texts() == want.choices)
        .then_some(prompt.menu)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// The prompt's first row: what is asked for, and by whom.
    Head(String),
    Text(String),
    Rule,
}

/// How the card shows the prompt: its rows wrapped at the card's width, how far it is scrolled
/// and what it has shown so far.
#[derive(Clone, Debug, Default)]
pub(super) struct View {
    lines: Vec<Line>,
    scroll: usize,
    /// How many lines the card has room for.
    room: usize,
    /// How many of the lines have been in view.
    revealed: usize,
    /// When the card first painted this prompt.
    painted: Option<Instant>,
    /// A line was wider than the card even after wrapping: part of it was not shown.
    clipped: bool,
    /// The monospace character width the painter measured (0 before the first paint).
    mono: f32,
    /// The card is short: its choices are drawn low, so that they and its foot stay in it.
    tight: bool,
}

impl View {
    /// Wraps the prompt's rows for a card `width` wide.
    fn flow(&mut self, prompt: &Prompt, width: f32) {
        let per = ((width / if self.mono > 0.0 { self.mono } else { MONO_W }).floor() as usize).max(MIN_CHARS);
        let mut lines = Vec::new();
        for (i, row) in prompt.rows.iter().enumerate() {
            if screens::is_rule(row) {
                lines.push(Line::Rule);
                continue;
            }
            let chars: Vec<char> = row.chars().collect();
            for part in chars.chunks(per) {
                let text: String = part.iter().collect();
                lines.push(if i == 0 { Line::Head(text) } else { Line::Text(text) });
            }
        }
        if self.lines != lines {
            self.lines = lines;
            self.scroll = 0;
            self.revealed = 0;
        }
    }

    fn last_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.room)
    }

    fn scroll_by(&mut self, down: bool, rows: usize) {
        let to = if down { self.scroll + rows } else { self.scroll.saturating_sub(rows) };
        self.scroll = to.min(self.last_scroll());
    }

    /// The card has shown every line.
    fn read_all(&self) -> bool {
        !self.lines.is_empty() && self.revealed >= self.lines.len()
    }

    /// The card paints its lines from `scroll` now.
    fn show(&mut self, now: Instant) {
        self.scroll = self.scroll.min(self.last_scroll());
        self.revealed = self.revealed.max((self.scroll + self.room).min(self.lines.len()));
        self.painted.get_or_insert(now);
    }
}

impl Card {
    /// The card answers the permission prompt on the screen from the screen alone.
    pub(in crate::view::chat) fn prompt_answers(&self) -> bool {
        self.prompt.as_ref().is_some_and(|prompt| prompt.whole)
    }

    /// The wheel over the card scrolls a prompt longer than the card.
    pub(in crate::view::chat) fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if self.prompt.is_none() || !self.rect.contains(x, y) {
            return false;
        }
        self.prompt_view.scroll_by(dy < 0.0, WHEEL);
        true
    }
}

impl Chat {
    /// The working line's words while the card shows a permission prompt read from the screen:
    /// Claude Code waits for the user whatever the transcript says (a subagent's prompt shows
    /// after the main turn ended).
    pub(in crate::view) fn prompt_waiting(&self) -> Option<&'static str> {
        self.question.prompt_answers().then_some(APPROVING)
    }

    /// The wheel over the card: true when the card took it.
    pub(in crate::view::chat) fn question_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        self.question.previews.wheel(x, y, dy) || self.question.wheel(x, y, dy) || self.question.wheel_card(x, y, dy)
    }
}

/// The padding of the well that holds the prompt's rows.
const WELL: (f32, f32) = (12.0, 8.0);

/// The width the prompt's rows are wrapped at in a card `card_w` wide.
pub(super) fn text_width(card_w: f32) -> f32 {
    (card_w - 2.0 * cards::INSET - 2.0 * WELL.0).max(1.0)
}

/// The card's height around the prompt's own rows: its head, the well's padding, the question,
/// the choices, a row for a notice and the foot.
fn chrome(prompt: &Prompt, tight: bool) -> f32 {
    let head = cards::INSET + cards::HEAD + 10.0;
    let well = 2.0 * WELL.1 + 12.0;
    let question = LINE + 10.0;
    let row = if tight { cards::TIGHT_ROW } else { cards::CHOICE_ROW };
    let choices = prompt.choices.len() as f32 * row + 10.0;
    head + well + question + choices + STATUS + cards::FOOT + cards::INSET - 6.0
}

/// The card's whole height for the prompt wrapped at `width`.
pub(super) fn height(card: &mut Card, width: f32) -> f32 {
    let Some(prompt) = &card.prompt else { return 0.0 };
    card.prompt_view.flow(prompt, width);
    chrome(prompt, false) + card.prompt_view.lines.len() as f32 * LINE
}

/// The card got its rectangle: how many lines fit. A short card draws its choices low first, so
/// that every choice and the foot keep their place in it while the rows scroll.
pub(super) fn fit(card: &mut Card) {
    let Some(prompt) = &card.prompt else { return };
    let h = card.rect.h - card.denied_row;
    let tight = h < chrome(prompt, false) + 3.0 * LINE;
    let room = (h - chrome(prompt, tight)) / LINE;
    card.prompt_view.tight = tight;
    card.prompt_view.room = room.floor().max(0.0) as usize;
}

/// A click in the card. A choice goes only for a prompt the card has shown whole, and only while
/// the screen still shows that prompt.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit, shown: Prompt) -> bool {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let target = match hit {
        Hit::More => {
            let room = chat.question.prompt_view.room.max(1);
            chat.question.prompt_view.scroll_by(true, room);
            return true;
        }
        Hit::Option(target) if target < shown.choices.len() => Some(target),
        Hit::Escape => None,
        _ => return false,
    };
    if !shown.whole {
        chat.question.error = Some(TOP_HIDDEN);
        return true;
    }
    if target.is_some() {
        let view = &chat.question.prompt_view;
        let seen = view.painted.is_some_and(|at| now.saturating_duration_since(at) >= SEEN);
        let refused = match () {
            _ if view.clipped => Some(TOO_WIDE),
            _ if !view.read_all() => Some(READ_ALL),
            _ if !seen => Some(NOT_SEEN),
            _ => None,
        };
        if let Some(refused) = refused {
            chat.question.error = Some(refused);
            return true;
        }
    }
    if busy(chat) {
        return true;
    }
    let subject = Subject::Prompt(shown);
    let Some(target) = target else { return escape(tab, now, None, subject) };
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let from = super::read(&subject, &tab.term.screen, None).and_then(|(menu, ..)| menu.highlight());
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

impl Draw<'_> {
    /// A permission prompt read from the screen: its rows word for word in a well, the question,
    /// the choices.
    pub(super) fn prompt(&self, p: &mut Painter, chat: &mut Chat, card: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some(prompt) = chat.question.prompt.clone() else { return };
        let notice = chat.question.error.or_else(|| chat.question_notice()).or_else(|| chat.question.typed_hint());
        // Esc declines without reading on: the prompt's footer always offers it.
        let close = chat.question.prompt_answers() && chat.question.step.is_none();
        let (x, w) = (card.x + cards::INSET, (card.w - 2.0 * cards::INSET).max(1.0));
        let mut y = card.y + cards::INSET;
        let title = ("Claude Code asks for permission", crate::view::look::TERMINAL);
        self.head(p, hits, Rect::new(x, y, w, cards::HEAD), "", title, "", close);
        y += cards::HEAD + 10.0;
        let view = &mut chat.question.prompt_view;
        view.mono = p.measure(Face::Mono, MONO, "MMMMMMMMMM") / 10.0;
        view.show(self.now);
        let shown = view.lines.len().saturating_sub(view.scroll).min(view.room);
        let well = Rect::new(x, y, w, shown as f32 * LINE + 2.0 * WELL.1);
        p.frame(well, 8.0, if pal.dark { pal.field } else { pal.bar }, crate::view::look::edge(pal));
        let inner = Rect::new(well.x + WELL.0, well.y + WELL.1, (well.w - 2.0 * WELL.0).max(1.0), well.h);
        let mut clipped = false;
        let mut ly = inner.y;
        for line in view.lines.iter().skip(view.scroll).take(view.room) {
            let cy = ly + LINE / 2.0;
            ly += LINE;
            let (face, text, size) = match line {
                Line::Rule => {
                    p.fill(Rect::new(inner.x, cy, inner.w, p.px()), pal.line);
                    continue;
                }
                Line::Head(text) => (Face::Semibold, text, 12.5),
                Line::Text(text) => (Face::Mono, text, MONO),
            };
            clipped |= p.measure(face, size, text) > inner.w;
            p.text(face, size, inner.x, cy, text, pal.text, inner.w);
        }
        view.clipped = clipped;
        let more = view.lines.len().saturating_sub(view.scroll + view.room);
        let can_answer = prompt.whole && view.read_all() && !view.clipped;
        let status = notice.or(match () {
            _ if !prompt.whole => Some(TOP_HIDDEN),
            _ if view.clipped => Some(TOO_WIDE),
            _ if !view.read_all() => Some(READ_ALL),
            _ => None,
        });
        y = well.bottom() + 12.0;
        p.text(Face::Semibold, 14.0, x, y + LINE / 2.0, &prompt.question, pal.text, w);
        y += LINE + 10.0;
        // Nothing is pre-selected: the terminal's highlight is not shown as a choice.
        let rows: Vec<cards::Choice> = prompt
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
        let density = if chat.question.prompt_view.tight { cards::Density::Tight } else { cards::Density::Plain };
        y += self.choices(p, hits, x, y, w, &rows, can_answer, density) + 10.0;
        if let Some(status) = status {
            let ink = if chat.question.error.is_some() { pal.warn } else { pal.muted };
            p.text(Face::Ui, 12.0, x, y + STATUS / 2.0 - 2.0, status, ink, w);
        }
        y += STATUS;
        // Always: a decline with text is given in the terminal.
        let more_label = format!("{more} more ▾");
        let mut right = Vec::new();
        if more > 0 {
            right.push((more_label.as_str(), Hit::More));
        }
        right.push(("Open terminal", Hit::Open));
        if close {
            right.push(("Esc to cancel", Hit::Escape));
        }
        self.foot(p, hits, Rect::new(x, y, w, cards::FOOT), &[], "", &right);
    }
}

#[cfg(test)]
impl Card {
    /// For tests: the card painted its prompt at `at`, with room for every line.
    pub(super) fn prompt_painted(&mut self, at: Instant) {
        let width = text_width(752.0);
        if let Some(prompt) = &self.prompt {
            self.prompt_view.flow(prompt, width);
        }
        self.prompt_view.room = self.prompt_view.lines.len();
        self.prompt_view.show(at);
    }
}

#[cfg(test)]
mod tests;
