//! The palette picker: a grid of cards, each a small window drawn in the palette it offers.
//!
//! The form owns scrolling and Tab stops. Each line of the grid is one form choice row (its
//! label, if any, above the cards) followed by empty spacer rows that give the cards their
//! height, so the rows after the grid start below it. Left/Right and Home/End address the same
//! choices as the mouse; Up/Down move between lines as between any rows.
use super::{Form, Row};
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb, mix};
use mdshared_ui::keys::KeyName;
use mdshared_ui::overlay::Bubble;
use mdshared_ui::settings::{Outcome, Value};
use mdshared_ui::theme::Palette;

pub(super) const DARK: u32 = 20;
pub(super) const LIGHT: u32 = 21;
const EXTRA: u32 = 4000;
/// Cards per line, on every page width Settings allows (its rows are 550 to 576 px wide).
pub(super) const ACROSS: usize = 5;
pub(super) const MORE_TERMINAL: u32 = 386;

/// The form's label inset, which the grid's left and right edges follow.
const INSET: f32 = 12.0;
/// Between cards, across and down.
const GAP: f32 = 12.0;
pub(super) const CARD_H: f32 = 86.0;
const CARD_RADIUS: f32 = 6.0;
/// The card's border round the preview.
const PAD: f32 = 5.0;
const PREVIEW_H: f32 = 54.0;
pub(super) const NAME_SIZE: f32 = 12.0;
/// Room for a line's label above its cards.
const LABEL_H: f32 = 32.0;
/// The toolkit's choice row and the single-page form's heading, which the spacers are.
const ROW_H: f32 = 34.0;
const SPACER_H: f32 = 32.0;

#[derive(Clone, Copy)]
pub(super) enum Family {
    Window(bool),
    Terminal,
}

pub(super) fn row_id(dark: bool, group: usize) -> u32 {
    match group {
        0 => {
            if dark {
                DARK
            } else {
                LIGHT
            }
        }
        n => EXTRA + 2 * n as u32 + u32::from(!dark),
    }
}

pub(super) fn group(id: u32) -> Option<(Family, usize)> {
    match id {
        DARK => Some((Family::Window(true), 0)),
        LIGHT => Some((Family::Window(false), 0)),
        super::SET_TERM_COLOURS => Some((Family::Terminal, 0)),
        MORE_TERMINAL => Some((Family::Terminal, 1)),
        n if n >= EXTRA + 2 => Some((Family::Window(n.is_multiple_of(2)), ((n - EXTRA) / 2) as usize)),
        _ => None,
    }
}

/// One line of the grid: its choice row and the spacers that make room for the cards.
pub(super) fn line(id: u32, label: &str, names: &[&str], value: usize) -> Vec<Row> {
    let top = if label.is_empty() { 0.0 } else { LABEL_H };
    let spacers = ((top + CARD_H + GAP - ROW_H) / SPACER_H).ceil() as usize;
    let mut rows = vec![Row::choice(id, label, names, value)];
    rows.extend(std::iter::repeat_with(|| Row::header("")).take(spacers));
    rows
}

/// A line as laid out: the form row it starts at, its whole height and where its cards start.
struct Line<'a> {
    id: u32,
    label: &'a str,
    names: &'a [String],
    rect: Rect,
    cards_y: f32,
    /// An ordinary row follows: the line takes the hairline the form draws between rows.
    rule: bool,
}

impl Line<'_> {
    fn card(&self, k: usize) -> Rect {
        let w = (self.rect.w - 2.0 * INSET - GAP * (ACROSS - 1) as f32) / ACROSS as f32;
        let x = (self.rect.x + INSET + k as f32 * (w + GAP)).round();
        let right = (self.rect.x + INSET + k as f32 * (w + GAP) + w).round();
        Rect::new(x, self.cards_y, right - x, CARD_H)
    }
}

fn lines(form: &Form) -> impl Iterator<Item = Line<'_>> {
    let rows = form.rows();
    rows.iter().enumerate().filter_map(move |(i, row)| {
        let Row::Choice { id, label, options, .. } = row else { return None };
        group(*id)?;
        let spacers = rows[i + 1..].iter().take_while(|r| matches!(r, Row::Header(t) if t.is_empty())).count();
        let first = form.rect_of_row(i);
        let last = form.rect_of_row(i + spacers);
        let rect = Rect::new(first.x, first.y, first.w, last.bottom() - first.y);
        let top = if label.is_empty() { 0.0 } else { LABEL_H };
        let rule = match rows.get(i + spacers + 1) {
            Some(Row::Choice { id, label, .. }) if group(*id).is_some() => !label.is_empty(),
            Some(Row::Header(_)) | None => false,
            Some(_) => true,
        };
        Some(Line { id: *id, label, names: options, rect, cards_y: first.y + top, rule })
    })
}

/// The card under the pointer: its row's id and its index in that row.
pub(super) fn hit(form: &Form, x: f32, y: f32) -> Option<(u32, usize)> {
    if form.popup_open() || !form.body_rect().contains(x, y) {
        return None;
    }
    lines(form).find_map(|l| (0..l.names.len()).find(|&k| l.card(k).contains(x, y)).map(|k| (l.id, k)))
}

/// Whether the pointer is on a grid line, cards or not. The form's own control for the row is
/// under the cards and must not take the press.
pub(super) fn covers(form: &Form, x: f32, y: f32) -> bool {
    !form.popup_open() && form.body_rect().contains(x, y) && lines(form).any(|l| l.rect.contains(x, y))
}

/// Scrolls the focused grid line wholly into view: the form reveals only its first row.
pub(super) fn reveal(form: &mut Form) {
    let Some(id) = form.focus_id() else { return };
    let Some(rect) = lines(form).find(|l| l.id == id).map(|l| l.rect) else { return };
    let body = form.body_rect();
    let by = if rect.y < body.y {
        rect.y - body.y
    } else if rect.bottom() > body.bottom() {
        (rect.bottom() - body.bottom()).min(rect.y - body.y)
    } else {
        return;
    };
    form.wheel(body.x, body.y, by);
}

/// Each card on the page with its row id and index, for tests.
#[cfg(test)]
pub(super) fn cards(form: &Form) -> Vec<(u32, usize, Rect, Rect)> {
    lines(form).flat_map(|l| (0..l.names.len()).map(move |k| (l.id, k, l.card(k), l.rect))).collect()
}

/// Where a card's name is drawn and how wide it may be, for tests.
#[cfg(test)]
pub(super) fn name_room(card: Rect) -> Rect {
    name_rect(card)
}

fn name_rect(card: Rect) -> Rect {
    let top = card.y + PAD + PREVIEW_H;
    Rect::new(card.x + 6.0, top, card.w - 12.0, card.bottom() - top)
}

pub(super) fn key(form: &mut Form, config: &claude_switch_core::Config, key: &KeyName) -> Option<Outcome> {
    let id = form.focus_id()?;
    let (family, group) = group(id)?;
    let row = form.rows().iter().find(|r| r.id() == Some(id))?;
    let Row::Choice { options, .. } = row else { return None };
    let n = options.len();
    // A Form choice has a value even on a row that doesn't contain the selected palette.
    let selected = super::appearance::selected_index(config, family).checked_sub(group * ACROSS).filter(|&i| i < n);
    let next = match key {
        KeyName::Enter | KeyName::Space if selected.is_none() => return Some(Outcome::Redraw),
        KeyName::Right | KeyName::Enter | KeyName::Space => selected.map_or(0, |i| (i + 1) % n),
        KeyName::Left => selected.map_or(n - 1, |i| (i + n - 1) % n),
        KeyName::Home => 0,
        KeyName::End => n - 1,
        _ => return None,
    };
    form.set_value(id, Value::Choice(next));
    Some(Outcome::Changed { id, value: Value::Choice(next) })
}

/// The colours a card shows. A window palette fills a small window: title bar, side pane, text,
/// an accent button and a terminal with a few ANSI colours. A terminal preset fills the whole
/// preview with its terminal.
pub(super) struct Sample {
    pub page: Rgb,
    pub bar: Rgb,
    pub pane: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub terminal: Rgb,
    pub ink: Rgb,
    /// Red, green, yellow, blue and magenta.
    pub ansi: [Rgb; 5],
    pub terminal_only: bool,
    pub selected: bool,
}

/// Red, green, yellow, blue and magenta of a terminal's sixteen.
pub(super) fn five(ansi: &[Rgb; 16]) -> [Rgb; 5] {
    [ansi[1], ansi[2], ansi[3], ansi[4], ansi[5]]
}

pub(super) fn draw(
    p: &mut Painter,
    pal: &Palette,
    form: &Form,
    hover: Option<(u32, usize)>,
    focus: bool,
    sample: impl Fn(Family, usize) -> Option<Sample>,
) {
    // The confirming picker has already been drawn by Form on top of its rows.
    let picker = form.colour_open().map(|_| p.snapshot(form.picker_rect().inset(-16.0)));
    let body = form.body_rect();
    let clip = p.clip(body);
    let mut tip = None;
    for l in lines(form) {
        if l.rect.bottom() <= body.y || l.rect.y >= body.bottom() {
            continue;
        }
        let Some((family, group)) = group(l.id) else { continue };
        // The form's row (its hover fill and the dropdown it would draw) is under the grid.
        p.fill(l.rect, pal.popup);
        p.text(Face::Ui, 13.5, l.rect.x + INSET, l.rect.y + ROW_H / 2.0, l.label, pal.text, l.rect.w - 2.0 * INSET);
        if l.rule {
            let px = p.px().max(1.0);
            let r = l.rect;
            p.fill_a(Rect::new(r.x + INSET, r.bottom() - px, r.w - 2.0 * INSET, px), pal.popup_line, 0.6);
        }
        let focused = focus && form.focus_id() == Some(l.id);
        let mut any_selected = false;
        for (k, name) in l.names.iter().enumerate() {
            let Some(s) = sample(family, group * ACROSS + k) else { continue };
            let card = l.card(k);
            let hovered = hover == Some((l.id, k));
            any_selected |= s.selected;
            if s.selected && focused {
                mdshared_ui::focus::draw_ring(p, pal, card, CARD_RADIUS);
            }
            if s.selected {
                p.rbox(card, CARD_RADIUS, pal.accent);
                p.rbox(card.inset(2.0), CARD_RADIUS - 2.0, if hovered { mix(pal.on, pal.text, 0.06) } else { pal.on });
            } else if hovered {
                p.frame(card, CARD_RADIUS, pal.hover, mix(pal.popup_line, pal.text, 0.35));
            } else {
                p.frame(card, CARD_RADIUS, pal.popup, pal.popup_line);
            }
            let preview = Rect::new(card.x + PAD, card.y + PAD, card.w - 2.0 * PAD, PREVIEW_H);
            preview_of(p, pal, preview, &s);
            if s.selected {
                badge(p, pal, preview);
            }
            let room = name_rect(card);
            let (face, colour) = if s.selected { (Face::Semibold, pal.text) } else { (Face::Ui, pal.text) };
            let shown = p.elide(face, NAME_SIZE, name, room.w);
            let w = p.measure(face, NAME_SIZE, &shown).min(room.w);
            p.text(face, NAME_SIZE, room.cx() - w / 2.0, room.cy(), &shown, colour, room.w + 1.0);
            if hovered && shown != *name {
                tip = Some((name.clone(), card));
            }
        }
        if focused && !any_selected {
            // The selection is on another line: ring the whole line.
            let first = l.card(0);
            let last = l.card(l.names.len() - 1);
            let all = Rect::new(first.x, first.y, last.right() - first.x, CARD_H);
            mdshared_ui::focus::draw_ring(p, pal, all, CARD_RADIUS);
        }
    }
    p.restore(clip);
    if let Some(saved) = picker {
        saved.restore(p.bmp);
    }
    if let Some((name, card)) = tip {
        let (w, h) = (p.bmp.width as f32 / p.scale, p.bmp.height as f32 / p.scale);
        Bubble::tip(p, &name, card, w, h).draw(p, pal);
    }
}

/// The selected card's check, at the preview's top right.
fn badge(p: &mut Painter, pal: &Palette, preview: Rect) {
    let d = 16.0;
    let r = Rect::new(preview.right() - d - 3.0, preview.y + 3.0, d, d);
    p.rbox(r.inset(-1.5), d / 2.0 + 1.5, pal.on);
    p.rbox(r, d / 2.0, pal.accent);
    p.icon(mdshared_ui::glyph::CHECK, 10.0, r.cx(), r.cy(), pal.on_accent);
}

/// A box with only its top (`top`) or bottom corners rounded, to fill inside a rounded preview.
fn half_round(p: &mut Painter, r: Rect, radius: f32, c: Rgb, top: bool) {
    p.rbox(r, radius, c);
    let square =
        if top { Rect::new(r.x, r.y + radius, r.w, r.h - radius) } else { Rect::new(r.x, r.y, r.w, r.h - radius) };
    p.fill(square, c);
}

fn preview_of(p: &mut Painter, pal: &Palette, r: Rect, s: &Sample) {
    // The outline is the page's, so a light preview reads on a light card and a dark on a dark.
    p.frame(r, 4.0, s.page, pal.popup_line);
    let inner = r.inset(1.0);
    if s.terminal_only {
        return terminal_of(p, inner, s);
    }
    // Title bar, with the current tab in the page's colour and an accent mark.
    let bar = Rect::new(inner.x, inner.y, inner.w, 10.0);
    half_round(p, bar, 3.0, s.bar, true);
    p.rbox(Rect::new(inner.x + 4.0, inner.y + 3.0, 22.0, 7.0), 1.5, s.page);
    p.fill(Rect::new(inner.x + 4.0, inner.y + 3.0, 22.0, 1.5), s.accent);
    p.fill(Rect::new(inner.x + 29.0, inner.y + 5.5, 12.0, 1.5), s.muted);
    // Side pane: a list with its current entry.
    let side = Rect::new(inner.x, bar.bottom(), (inner.w * 0.3).round(), inner.bottom() - bar.bottom());
    half_round(p, Rect::new(side.x, side.y, side.w + 3.0, side.h), 3.0, s.pane, false);
    p.fill(Rect::new(side.right(), side.y, 3.0, side.h), s.page);
    p.fill(Rect::new(side.x + 2.0, side.y + 4.0, side.w - 4.0, 6.0), mix(s.pane, s.text, 0.12));
    p.fill(Rect::new(side.x + 2.0, side.y + 4.0, 1.5, 6.0), s.accent);
    p.fill(Rect::new(side.x + 5.0, side.y + 6.0, side.w - 11.0, 2.0), s.text);
    for (k, w) in [(0, 0.6), (1, 0.45), (2, 0.55)] {
        p.fill(Rect::new(side.x + 5.0, side.y + 14.0 + 6.0 * k as f32, ((side.w - 8.0) * w).round(), 2.0), s.muted);
    }
    // The page: a heading, a line of quiet text and an accent button.
    let x = side.right() + 5.0;
    let w = inner.right() - 4.0 - x;
    p.fill(Rect::new(x, side.y + 4.0, (w * 0.55).round(), 3.0), s.text);
    p.fill(Rect::new(x, side.y + 10.0, (w * 0.8).round(), 2.0), s.muted);
    p.rbox(Rect::new(x, side.y + 15.0, 16.0, 6.0), 2.0, s.accent);
    // A terminal: a line of its text and its colours.
    let term = Rect::new(x, side.y + 24.0, w, inner.bottom() - 3.0 - (side.y + 24.0));
    p.rbox(term, 2.0, s.terminal);
    p.fill(Rect::new(term.x + 3.0, term.y + 3.0, (term.w * 0.45).round(), 2.0), s.ink);
    ansi_row(p, term.x + 3.0, term.y + 8.0, (term.w - 6.0).min(40.0), &s.ansi);
}

fn terminal_of(p: &mut Painter, inner: Rect, s: &Sample) {
    let x = inner.x + 6.0;
    let w = inner.w - 12.0;
    // A prompt, a command, its output, a selected line and the colours.
    p.fill(Rect::new(x, inner.y + 7.0, 4.0, 2.5), s.accent);
    p.fill(Rect::new(x + 7.0, inner.y + 7.0, (w * 0.5).round(), 2.5), s.text);
    p.fill(Rect::new(x, inner.y + 14.0, (w * 0.75).round(), 2.0), s.muted);
    p.fill(Rect::new(x - 2.0, inner.y + 19.0, (w * 0.6).round() + 4.0, 6.0), s.terminal);
    p.fill(Rect::new(x, inner.y + 21.0, (w * 0.45).round(), 2.0), s.ink);
    p.fill(Rect::new(x, inner.y + 30.0, (w * 0.35).round(), 2.0), s.muted);
    ansi_row(p, x, inner.bottom() - 13.0, w.min(60.0), &s.ansi);
}

/// Five colour chips across `w`.
fn ansi_row(p: &mut Painter, x: f32, y: f32, w: f32, ansi: &[Rgb; 5]) {
    let step = w / 5.0;
    for (k, c) in ansi.iter().enumerate() {
        p.rbox(Rect::new((x + k as f32 * step).round(), y, (step - 2.0).max(2.0), 5.0), 1.0, *c);
    }
}
