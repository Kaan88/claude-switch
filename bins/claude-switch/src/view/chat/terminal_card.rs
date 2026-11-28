//! What Chat cannot read, shown in Chat: while Chat stops because Claude Code may be waiting in
//! the terminal, or refuses a send because it cannot read the screen, a card above the message
//! box shows the part of the live terminal screen that matters, drawn as the terminal draws it
//! (the conversation's grid renderer, from the screen's own cells and colours). It follows the
//! screen while it stays, and goes once Chat can read the screen again, the terminal is in view,
//! or the user dismisses it.
//!
//! It only looks. **Answer in terminal** opens the drawer with the keyboard, as "Open terminal"
//! does; nothing here ever writes to the terminal, and no key (Tab included) acts on the card.
//! The drawer it opened closes by itself once the cause is gone, by the rule the automatic
//! drawer follows (see [`gone`]), unless the user resized it or typed in Chat since.
//!
//! Which rows: the bottom of the screen up to its last row with text, at most [`MAX_ROWS`], from
//! the last row there that starts one of Claude Code's messages or tool calls (`●`), so the card
//! opens on what is being asked and the input under it. Each terminal row stays one row: the
//! text gets smaller to fit the card's width, down to [`MIN_FONT`], and past that the right
//! side is cut (dialog borders reach the terminal's edge and would only wrap into noise).
use std::ops::Range;
use std::time::Instant;

use mdshared_term::{Attrs, Cell, Color, Screen};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::termview::{Metrics, Scheme};
use mdshared_ui::theme::Palette;

use super::{Chat, Drawer, Gate, Reason, submission};
use crate::state::Target;
use crate::view::chat::input::Control;
use crate::view::ui::grid::{FONT_SIZE, Grid, Look};

/// The most terminal rows the card shows.
pub(super) const MAX_ROWS: usize = 14;
/// The smallest text the rows are set in to fit the card's width.
const MIN_FONT: f32 = 9.0;
/// The card's padding, its title row, and the space around the screen's rows.
const PAD: f32 = 12.0;
const HEAD: f32 = 26.0;
const INNER: f32 = 8.0;
/// The most of the conversation the card takes.
const SHARE: f32 = 0.6;
/// The bullet Claude Code starts a message or a tool call with.
pub(super) const BULLETS: [char; 2] = ['●', '⏺'];

/// Refused sends that point to the terminal because Chat cannot read what it shows.
const REFUSALS: [&str; 6] = [
    submission::CHECK,
    submission::NO_QUIET_PROMPT,
    submission::NOT_EMPTY,
    submission::INPUT_HAS_TEXT,
    submission::MENU_OPEN,
    submission::FULLSCREEN,
];

/// Why the card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cause {
    /// The gate's reason: Claude Code may be waiting, the bell rang, a permission prompt Chat
    /// cannot read whole.
    Reason(Reason),
    /// A send refused with these words.
    Refused(&'static str),
}

impl Cause {
    fn label(self) -> &'static str {
        match self {
            Self::Reason(reason) => reason.label(),
            Self::Refused(words) => words,
        }
    }
}

/// The card's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Opens the drawer with the keyboard.
    Answer,
    Dismiss,
}

#[derive(Default)]
pub(super) struct Card {
    /// Why it shows, when it does.
    cause: Option<Cause>,
    /// The cause the user dismissed it for: it stays away until that cause is gone.
    dismissed: Option<Cause>,
    /// When **Answer in terminal** opened the drawer, while it may still close it by itself.
    answered: Option<Instant>,
    /// The rows shown, with their colour codes, as last read from the screen.
    source: String,
    /// How many rows, and the widest row's cells.
    rows: usize,
    cols: usize,
    /// Where the last layout put the card (empty when it is not shown).
    pub(super) rect: Rect,
    /// The rows laid out.
    grid: Option<Laid>,
    look: Look,
}

impl Card {
    /// Shown in the last layout.
    pub(super) fn shown(&self) -> bool {
        self.rect.h > 0.0
    }

    #[cfg(test)]
    pub(super) fn source(&self) -> &str {
        &self.source
    }
}

/// Why the card would show now, before Dismiss: a gate reason that sends the user to the
/// terminal because Chat cannot read the screen, or such a refused send while the screen is not
/// ready; not while the terminal is in view or a question card shows the prompt.
pub(super) fn cause(chat: &Chat) -> Option<Cause> {
    if chat.drawer != Drawer::Closed {
        return None;
    }
    unread(chat)
}

/// [`cause`] whether or not the terminal is in view.
fn unread(chat: &Chat) -> Option<Cause> {
    if chat.question.visible() && !chat.question.only_denials() {
        return None;
    }
    match chat.reason {
        Some(reason @ (Reason::Tool | Reason::Bell | Reason::Permission)) => Some(Cause::Reason(reason)),
        Some(_) => None,
        None => chat
            .error
            .filter(|error| REFUSALS.contains(error) && chat.last_gate != Some(Gate::Ready))
            .map(Cause::Refused),
    }
}

/// The cause the card showed is gone, by the automatic drawer's rule: Claude Code no longer
/// waits (no reason, or one a card in Chat answers) and no send is refused for the screen.
fn gone(chat: &Chat) -> bool {
    unread(chat).is_none()
        && chat.reason.is_none_or(|r| matches!(r, Reason::Question | Reason::Approval | Reason::Menu | Reason::Slash))
}

/// Closes the drawer **Answer in terminal** opened once its cause is gone, unless the user took
/// it over: closed or resized it, or typed in Chat since. Whether it closed it.
fn settle(chat: &mut Chat) -> bool {
    let Some(at) = chat.terminal_card.answered else { return false };
    if chat.drawer != Drawer::Manual || chat.typed_at.is_some_and(|t| t > at) {
        chat.terminal_card.answered = None;
        return false;
    }
    if !gone(chat) {
        return false;
    }
    chat.terminal_card.answered = None;
    chat.drawer = Drawer::Closed;
    chat.drawer_focus = false;
    true
}

/// The user resized, closed or opened the drawer: it is theirs, and stays open after the cause
/// is gone.
pub(super) fn keep(chat: &mut Chat) {
    chat.terminal_card.answered = None;
}

/// Reads the card's rows from the live screen, each tick: whether anything it shows changed.
pub(super) fn observe(chat: &mut Chat, screen: &Screen) -> bool {
    let closed = settle(chat);
    let cause = cause(chat);
    if cause.is_none() {
        chat.terminal_card.dismissed = None;
    }
    let card = &mut chat.terminal_card;
    let cause = cause.filter(|c| card.dismissed != Some(*c));
    let (source, rows, cols) = match cause {
        Some(_) => encode(screen, rows(screen)),
        None => (String::new(), 0, 0),
    };
    let changed = closed || card.cause != cause || card.source != source;
    (card.cause, card.source, card.rows, card.cols) = (cause, source, rows, cols);
    changed
}

/// A click on one of the card's buttons at `now`. Never writes to the terminal.
pub(super) fn press(chat: &mut Chat, hit: Hit, now: Instant) {
    match hit {
        Hit::Answer => {
            chat.open_drawer();
            chat.terminal_card.answered = Some(now);
        }
        Hit::Dismiss => {
            chat.terminal_card.dismissed = chat.terminal_card.cause;
            chat.terminal_card.cause = None;
        }
    }
}

/// The rows of the screen the card shows (see the module).
pub(super) fn rows(screen: &Screen) -> Range<usize> {
    let blank = |r: usize| screen.line(r).is_none_or(|l| l.cells().iter().all(is_blank));
    let Some(last) = (0..screen.rows()).rev().find(|&r| !blank(r)) else { return 0..0 };
    let top = (last + 1).saturating_sub(MAX_ROWS);
    let starts = |r: usize| BULLETS.contains(&screen.cell(r, 0).ch);
    let mut first = (top..=last).rev().find(|&r| starts(r)).unwrap_or(top);
    while first < last && blank(first) {
        first += 1;
    }
    first..last + 1
}

fn is_blank(c: &Cell) -> bool {
    c.ch == ' ' && c.bg == Color::Default && !c.attrs.contains(Attrs::INVERSE)
}

/// The rows as text with colour codes for every change of colour or attribute, each row without
/// its blank end: the source of the card's grid. Also how many rows, and the widest's cells.
pub(super) fn encode(screen: &Screen, rows: Range<usize>) -> (String, usize, usize) {
    let mut out = String::new();
    let mut widest = 0;
    let count = rows.len();
    for (n, r) in rows.enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let Some(line) = screen.line(r) else { continue };
        let end = line.cells().iter().rposition(|c| !is_blank(c)).map_or(0, |i| i + 1);
        // A wide character's second cell belongs to it.
        let end = if line.cell(end).is_spacer() { end + 1 } else { end };
        widest = widest.max(end);
        let mut pen: Option<(Color, Color, Attrs)> = None;
        for cell in line.cells().iter().take(end).filter(|c| !c.is_spacer()) {
            let look = (cell.fg, cell.bg, cell.attrs);
            if pen != Some(look) {
                out.push_str(&sgr(look));
                pen = Some(look);
            }
            out.push(if cell.ch.is_control() { ' ' } else { cell.ch });
        }
        if pen.is_some() {
            out.push_str("\u{1b}[0m");
        }
    }
    (out, count, widest)
}

/// The code that sets exactly these colours and attributes.
fn sgr((fg, bg, attrs): (Color, Color, Attrs)) -> String {
    let mut codes = vec!["0".to_owned()];
    for (flag, code) in [
        (Attrs::BOLD, "1"),
        (Attrs::DIM, "2"),
        (Attrs::ITALIC, "3"),
        (Attrs::UNDERLINE, "4"),
        (Attrs::BLINK, "5"),
        (Attrs::INVERSE, "7"),
        (Attrs::HIDDEN, "8"),
        (Attrs::STRIKE, "9"),
    ] {
        if attrs.contains(flag) {
            codes.push(code.to_owned());
        }
    }
    for (color, base) in [(fg, 38), (bg, 48)] {
        match color {
            Color::Default => {}
            Color::Indexed(n) => codes.push(format!("{base};5;{n}")),
            Color::Rgb(r, g, b) => codes.push(format!("{base};2;{r};{g};{b}")),
        }
    }
    format!("\u{1b}[{}m", codes.join(";"))
}

/// The card's rows laid out, and for what: the source, the display scale, the width.
struct Laid {
    source: String,
    scale: f32,
    width: f32,
    metrics: Metrics,
    grid: Grid,
}

/// Places the card above the message box, taking its room from the conversation, and lays out
/// its rows for the card's width at `scale` (again only when the rows, the width or the scale
/// changed).
pub(super) fn place(chat: &mut Chat, scale: f32) {
    let card = &mut chat.terminal_card;
    let conversation = &mut chat.layout.conversation;
    let frame = chat.layout.box_frame;
    card.rect = Rect::default();
    if card.cause.is_none() || frame.w <= 0.0 {
        return;
    }
    let width = (frame.w - 2.0 * PAD - 2.0 * INNER).max(1.0);
    let fresh = card.grid.as_ref().is_some_and(|l| l.source == card.source && l.scale == scale && l.width == width);
    if !fresh {
        let metrics = fit(&mut card.look, card.cols, width, scale);
        let grid = Grid::with_cols(&card.source, metrics, card.cols);
        card.grid = Some(Laid { source: card.source.clone(), scale, width, metrics, grid });
    }
    let Some(laid) = &card.grid else { return };
    let chrome = 2.0 * PAD + HEAD + 2.0 * INNER;
    let rows_h = if card.rows == 0 { 0.0 } else { laid.grid.height() };
    let h = (chrome + rows_h).min((conversation.h * SHARE).floor()).max(0.0);
    if h < chrome + laid.metrics.cell_h().min(rows_h) {
        return;
    }
    card.rect = Rect::new(frame.x, conversation.bottom() - h - 8.0, frame.w, h);
    conversation.h = (conversation.h - h - 16.0).max(0.0);
}

/// The cells of the largest text size from [`FONT_SIZE`] down to [`MIN_FONT`] whose `cols`
/// cells fit `width`.
fn fit(look: &mut Look, cols: usize, width: f32, scale: f32) -> Metrics {
    let mut size = FONT_SIZE;
    loop {
        let metrics = look.font.metrics(size, scale);
        if cols as f32 * metrics.cell_w() <= width || size <= MIN_FONT {
            return metrics;
        }
        size -= 0.5;
    }
}

/// Draws the card where [`place`] put it, in the terminal theme's `scheme`.
pub(super) fn draw(
    chat: &mut Chat,
    p: &mut Painter,
    pal: &Palette,
    scheme: Scheme,
    hover: Option<Target>,
    hits: &mut Vec<(Rect, Target)>,
) {
    let card = &mut chat.terminal_card;
    let (Some(cause), r) = (card.cause, card.rect) else { return };
    if r.h <= 0.0 {
        return;
    }
    crate::view::look::raised_box(p, pal, r, true);
    // The buttons at the right of the title row, the words before them.
    let mut x = r.right() - PAD;
    let mut button = |p: &mut Painter, label: &str, hit: Hit, w: f32| {
        x -= w;
        let b = Rect::new(x, r.y + PAD, w, HEAD - 4.0);
        x -= 6.0;
        let target = Target::Chat(Control::Screen(hit));
        super::paint::button(p, pal, b, label, hover == Some(target), false);
        hits.push((b, target));
    };
    button(p, "Dismiss", Hit::Dismiss, 70.0);
    button(p, "Answer in terminal", Hit::Answer, 136.0);
    let words = (x - r.x - PAD - 6.0).max(0.0);
    p.text(Face::Semibold, 12.0, r.x + PAD, r.y + PAD + (HEAD - 4.0) / 2.0, cause.label(), pal.text, words);
    // The screen's rows on the terminal's own background; the bottom rows when not all fit.
    let screen = Rect::new(r.x + PAD, r.y + PAD + HEAD, r.w - 2.0 * PAD, r.h - 2.0 * PAD - HEAD);
    p.frame(screen, 6.0, scheme.background, pal.line);
    let Some(Laid { grid, .. }) = &card.grid else { return };
    let inside = Rect::new(screen.x + INNER, screen.y + INNER, screen.w - 2.0 * INNER, screen.h - 2.0 * INNER);
    card.look.scheme = Some(scheme);
    card.look.loud = true;
    let clip = p.clip(inside);
    let top = inside.y - (grid.height() - inside.h).max(0.0);
    grid.draw(p, (inside.x, top), inside, None, pal, &mut card.look);
    p.restore(clip);
}

#[cfg(test)]
mod tests;
