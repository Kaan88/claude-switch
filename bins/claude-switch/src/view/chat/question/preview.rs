//! The preview of a question's option: the `preview` Claude gave it in the call's input
//! (monospace text such as a mock-up, code or a layout), read from the transcript, never from the
//! screen. It is drawn as the terminal draws text, with the conversation's grid renderer
//! (`view/ui/grid.rs`: the terminal's font, cell grid and theme colours), so box-drawing lines
//! meet and columns line up.
//!
//! The card shows the preview of the option under the pointer, or of the one ↑ and ↓ moved to,
//! beside the choices when the card is wide enough ([`SIDE_MIN`]) and under them otherwise. Its
//! height is bounded ([`MAX_ROWS`] rows, fewer when the window is short) and the wheel scrolls a
//! longer one. A drag selects in it, and Ctrl+C or Copy copies plain text (the grid's own text: a
//! wrapped line comes out as one line). The text gets smaller to fit the pane's width, down to
//! [`MIN_FONT`]; past that, lines wrap as a terminal that wide would show them. Nothing here
//! writes to the terminal.
use claude_switch_core::transcript::QuestionOption;
use mdshared_term::char_width;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::termview::Metrics;

use super::cards::Part;
use super::*;
use crate::view::chat::input::Control;
use crate::view::ui::grid::{FONT_SIZE, Grid, Look};

/// The card's inner width from which the preview stands beside the choices.
pub(super) const SIDE_MIN: f32 = 720.0;
/// The share of the card's width the preview takes beside the choices.
const SIDE_SHARE: f32 = 0.56;
/// Between the choices and the preview beside them.
pub(super) const GAP: f32 = 12.0;
/// The row over the preview: what it shows, and Copy.
const HEADER: f32 = 26.0;
/// Around the grid inside its box.
const INNER: f32 = 8.0;
/// The most rows of a preview shown at once, and the fewest a short window cuts it to.
pub(super) const MAX_ROWS: usize = 16;
pub(super) const LEAST_ROWS: usize = 5;
/// The smallest text a preview is set in to fit its width.
const MIN_FONT: f32 = 9.0;
/// A tab's widest step, for measuring lines.
const TAB: usize = 8;

/// The preview's place as laid out: its width and how many of its rows show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::view::chat) struct Size {
    pub w: f32,
    pub rows: usize,
    /// One row's height.
    pub cell: f32,
}

impl Size {
    pub(super) fn h(&self) -> f32 {
        HEADER + self.rows as f32 * self.cell + 2.0 * INNER
    }
}

/// The previews of the question on the card, laid out for a width and scale.
struct Laid {
    texts: Vec<String>,
    labels: Vec<String>,
    width: f32,
    scale: f32,
    metrics: Metrics,
    /// One per option; `None` for an option without a preview.
    grids: Vec<Option<Grid>>,
}

/// The card's previews: which shows, how far it is scrolled, the selection in it.
#[derive(Default)]
pub(in crate::view::chat) struct State {
    /// The option shown, from the pointer or the keyboard; the first with a preview without.
    at: Option<usize>,
    laid: Option<Laid>,
    look: Look,
    /// Rows scrolled past the top.
    scroll: usize,
    /// A selection in the shown preview's text: where it started and where it ends.
    selection: Option<(usize, usize)>,
    dragging: bool,
    /// Where the last frame drew the shown grid: inside its box, and its top (scrolled).
    view: Rect,
    top: f32,
}

/// How many cells a line takes, a tab its widest step.
fn cells(line: &str) -> usize {
    line.chars().map(|c| if c == '\t' { TAB } else { usize::from(char_width(c)) }).sum()
}

impl State {
    /// The option whose preview shows.
    fn shown(&self) -> Option<usize> {
        let laid = self.laid.as_ref()?;
        self.at.filter(|&at| at < laid.grids.len()).or_else(|| laid.grids.iter().position(Option::is_some))
    }

    fn grid(&self) -> Option<&Grid> {
        let at = self.shown()?;
        self.laid.as_ref()?.grids.get(at)?.as_ref()
    }

    /// Shows option `at`: from the top, with no selection.
    fn show(&mut self, at: usize) {
        if self.shown() != Some(at) {
            self.scroll = 0;
            self.selection = None;
            self.dragging = false;
        }
        self.at = Some(at);
    }

    /// The pointer is over option `at` of the card's list.
    pub(super) fn hover(&mut self, at: usize) {
        if self.laid.as_ref().is_some_and(|laid| at < laid.grids.len()) {
            self.show(at);
        }
    }

    /// ↑ or ↓: the previous or next option's preview. False when the card has no previews.
    pub(super) fn step(&mut self, down: bool) -> bool {
        let Some(count) = self.laid.as_ref().map(|laid| laid.grids.len()) else { return false };
        let at = self.shown().unwrap_or(0);
        let to = if down { (at + 1).min(count.saturating_sub(1)) } else { at.saturating_sub(1) };
        self.show(to);
        true
    }

    /// No previews: the card shows none.
    pub(super) fn clear(&mut self) {
        self.laid = None;
        self.view = Rect::default();
        self.dragging = false;
    }

    /// The shown preview's plain text, or the selected part of it.
    pub(in crate::view::chat) fn copy_text(&self, selected: bool) -> Option<String> {
        let text = self.grid()?.text();
        if !selected {
            return Some(text.to_owned());
        }
        let (a, b) = self.selection?;
        let range = a.min(b)..a.max(b).min(text.len());
        (!range.is_empty()).then(|| text.get(range).map(str::to_owned)).flatten()
    }

    fn place_at(&self, x: f32, y: f32) -> Option<usize> {
        let grid = self.grid()?;
        Some(grid.at((x - self.view.x).max(0.0), (y - self.top).max(0.0)))
    }

    /// A press in the preview: a selection starts there.
    pub(in crate::view::chat) fn press(&mut self, x: f32, y: f32) {
        self.selection = self.place_at(x, y).map(|at| (at, at));
        self.dragging = self.selection.is_some();
    }

    /// The pointer moved with the button down: true while a selection in the preview follows it.
    pub(in crate::view::chat) fn drag_to(&mut self, x: f32, y: f32) -> bool {
        if !self.dragging {
            return false;
        }
        // Past the top or the bottom the preview scrolls on, a row at a time.
        let rows = self.grid().map_or(0, |g| (g.height() / self.cell()).round() as usize);
        let shown = ((self.view.h / self.cell()).floor() as usize).max(1);
        if y < self.view.y && self.scroll > 0 {
            self.scroll -= 1;
        } else if y > self.view.bottom() && self.scroll + shown < rows {
            self.scroll += 1;
        }
        let y = y.clamp(self.view.y, self.view.bottom() - 1.0);
        if let (Some(at), Some(sel)) = (self.place_at(x, y), self.selection.as_mut()) {
            sel.1 = at;
        }
        true
    }

    /// The button went up: true when a selection in the preview was being made.
    pub(in crate::view::chat) fn release(&mut self, point: Option<(f32, f32)>) -> bool {
        if let Some((x, y)) = point {
            self.drag_to(x, y);
        }
        std::mem::take(&mut self.dragging)
    }

    fn cell(&self) -> f32 {
        self.laid.as_ref().map_or(16.0, |laid| laid.metrics.cell_h()).max(1.0)
    }

    /// The wheel over the preview: true when it scrolled (a preview longer than its box).
    pub(in crate::view::chat) fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if !self.view.contains(x, y) {
            return false;
        }
        let rows = self.grid().map_or(0, |g| (g.height() / self.cell()).round() as usize);
        let shown = ((self.view.h / self.cell()).floor() as usize).max(1);
        let over = rows.saturating_sub(shown);
        if over == 0 {
            return false;
        }
        let step = 3;
        self.scroll = if dy < 0.0 { (self.scroll + step).min(over) } else { self.scroll.saturating_sub(step) };
        true
    }
}

/// The largest text from [`FONT_SIZE`] down to [`MIN_FONT`] whose `widest` cells fit `width`.
fn fit(look: &mut Look, widest: usize, width: f32, scale: f32) -> Metrics {
    let mut size = FONT_SIZE;
    loop {
        let metrics = look.font.metrics(size, scale);
        if widest as f32 * metrics.cell_w() <= width || size <= MIN_FONT {
            return metrics;
        }
        size -= 0.5;
    }
}

/// Lays out the previews of `options` (the question the card shows) for a card `width` wide at
/// `scale`, where the parts hold a [`Part::Preview`]: beside the choices (the part before it
/// then holds its size, and the preview part goes) or under them. Without a preview to show the
/// preview part goes. Grids are made again only when the texts, the width or the scale changed.
pub(super) fn lay(card: &mut Card, parts: &mut Vec<Part>, width: f32, scale: f32, options: &[QuestionOption]) {
    let Some(at) = parts.iter().position(|p| matches!(p, Part::Preview(_))) else {
        card.previews.clear();
        return;
    };
    if options.iter().all(|o| o.preview.is_empty()) {
        parts.remove(at);
        card.previews.clear();
        return;
    }
    let side = width >= SIDE_MIN;
    let pane_w = if side { (width * SIDE_SHARE).floor() } else { width };
    let grid_w = (pane_w - 2.0 * INNER).max(1.0);
    let state = &mut card.previews;
    let same_texts = state.laid.as_ref().is_some_and(|laid| laid.texts.iter().eq(options.iter().map(|o| &o.preview)));
    let fresh = same_texts && state.laid.as_ref().is_some_and(|laid| laid.width == grid_w && laid.scale == scale);
    if !same_texts {
        // Another question: from its first preview, at the top.
        state.at = None;
        state.scroll = 0;
        state.selection = None;
        state.dragging = false;
    }
    if !fresh {
        let texts: Vec<String> = options.iter().map(|o| o.preview.clone()).collect();
        let widest = texts.iter().flat_map(|t| t.lines()).map(cells).max().unwrap_or(0);
        let metrics = fit(&mut state.look, widest, grid_w, scale);
        let grids = texts.iter().map(|t| (!t.is_empty()).then(|| Grid::new(t, metrics, grid_w))).collect();
        let labels = options.iter().map(|o| o.label.clone()).collect();
        state.laid = Some(Laid { texts, labels, width: grid_w, scale, metrics, grids });
    }
    let Some(laid) = state.laid.as_ref() else { return };
    let cell = laid.metrics.cell_h().max(1.0);
    // As tall as the tallest preview (up to the bound): the card does not jump as the pointer
    // moves from one option to the next.
    let rows = laid.grids.iter().flatten().map(|g| (g.height() / cell).round() as usize).max().unwrap_or(1);
    let size = Size { w: pane_w, rows: rows.clamp(1, MAX_ROWS), cell };
    if side {
        parts.remove(at);
        if let Some(Part::Choices { side, .. }) =
            parts[..at].iter_mut().rev().find(|p| matches!(p, Part::Choices { .. }))
        {
            *side = Some(size);
        }
    } else {
        parts[at] = Part::Preview(size);
    }
}

impl Draw<'_> {
    /// The preview pane in `r` (its header and its box), `rows` rows of the grid high.
    pub(super) fn preview(
        &self,
        p: &mut Painter,
        chat: &mut Chat,
        hits: &mut Vec<(Rect, Target)>,
        r: Rect,
        size: &Size,
    ) {
        let pal = self.pal;
        let state = &mut chat.question.previews;
        let Some(shown) = state.shown() else { return };
        let label = state.laid.as_ref().and_then(|laid| laid.labels.get(shown).cloned()).unwrap_or_default();
        let cy = r.y + HEADER / 2.0 - 2.0;
        let copy_w = self.plain(p, hits, r.right() + 8.0, cy, "Copy", Hit::CopyPreview, false);
        let word = "Preview";
        let word_w = p.measure(Face::Ui, 11.5, word);
        p.text(Face::Ui, 11.5, r.x, cy, word, pal.muted, word_w + 2.0);
        let x = r.x + word_w + 8.0;
        p.text(Face::Semibold, 12.5, x, cy, &label, pal.text, (r.right() - copy_w - x - 4.0).max(0.0));
        let frame = Rect::new(r.x, r.y + HEADER, r.w, size.rows as f32 * size.cell + 2.0 * INNER);
        p.frame(frame, 6.0, self.scheme.background, pal.line);
        let inside =
            Rect::new(frame.x + INNER, frame.y + INNER, (frame.w - 2.0 * INNER).max(0.0), size.rows as f32 * size.cell);
        let target = Target::Chat(Control::Question(Hit::Preview));
        hits.push((frame, target));
        let State { laid, look, scroll, selection, view, top, .. } = state;
        let Some(grid) = laid.as_ref().and_then(|laid| laid.grids.get(shown)).and_then(Option::as_ref) else {
            *view = Rect::default();
            p.text(
                Face::Ui,
                12.0,
                inside.x,
                inside.y + size.cell / 2.0,
                "No preview for this choice.",
                pal.muted,
                inside.w,
            );
            return;
        };
        let rows = (grid.height() / size.cell).round() as usize;
        let over = rows.saturating_sub(size.rows);
        *scroll = (*scroll).min(over);
        *view = inside;
        *top = inside.y - *scroll as f32 * size.cell;
        look.scheme = Some(self.scheme);
        look.loud = true;
        let clip = p.clip(inside);
        let sel = selection.map(|(a, b)| a.min(b)..a.max(b));
        grid.draw(p, (inside.x, *top), inside, sel, pal, look);
        p.restore(clip);
        if over > 0 {
            // Where the shown rows are in the whole preview, along the box's right edge.
            let track = Rect::new(frame.right() - 5.0, frame.y + 4.0, 2.5, (frame.h - 8.0).max(0.0));
            let thumb_h = (track.h * size.rows as f32 / rows as f32).clamp(12.0_f32.min(track.h), track.h);
            let thumb_y = track.y + (track.h - thumb_h) * (*scroll as f32 / over as f32);
            p.rbox(Rect::new(track.x, thumb_y, track.w, thumb_h), 1.25, pal.thumb);
        }
    }
}
