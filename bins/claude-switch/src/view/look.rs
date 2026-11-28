//! Colours and marks the conversation, the composer and the cards share. Every colour is made
//! from the window's palette, so a theme the user picked or made carries through.
use mdshared_ui::gfx::{Painter, Rect, Rgb, mix};
use mdshared_ui::theme::Palette;
use mdshared_ui::tokens::legible;

/// The padding of the composer and of a card: their text starts where the reading column's does.
pub(super) const INSET: f32 = 16.0;
/// The corner radius of the composer and of a card in its place.
pub(super) const BOX_RADIUS: f32 = 18.0;

/// An arrow pointing up: Send.
pub(super) const ARROW_UP: &str = "M8 13V3.5M3.8 7.7L8 3.5l4.2 4.2";

/// A terminal window: a command.
pub(super) const TERMINAL: &str =
    "M2 3h12a.5 .5 0 0 1 .5 .5v9a.5 .5 0 0 1-.5 .5H2a.5 .5 0 0 1-.5-.5v-9A.5 .5 0 0 1 2 3zM4.5 6.5l2 1.5-2 1.5M8 10h3";
/// A fork in a line: a branch of the conversation.
pub(super) const BRANCH: &str = "M5 2.5v8M5 10.5a1.5 1.5 0 1 0 0 3a1.5 1.5 0 1 0 0-3zM11 3a1.5 1.5 0 1 0 0 3a1.5 1.5 0 1 0 0-3zM11 6c0 2.6-6 2-6 4.5";
/// A person: an agent.
pub(super) const AGENT: &str =
    "M5.5 5.5a2.5 2.5 0 1 0 5 0a2.5 2.5 0 1 0-5 0zM3 13.5c.6-2.4 2.5-3.7 5-3.7s4.400 1.3 5 3.7";
/// The Claude mark: kept for the working line.
pub(super) const MARK: &str = "✳︎";

/// The composer and the cards: a surface one step above the page.
pub(super) fn raised(pal: &Palette) -> Rgb {
    if pal.dark { mix(pal.page, pal.text, 0.06) } else { pal.field }
}

/// The hairline around a raised surface and between its rows.
pub(super) fn edge(pal: &Palette) -> Rgb {
    if pal.dark { mix(pal.line, pal.text, 0.07) } else { pal.line }
}

/// The user's bubble: the page a step lighter in a dark theme, a step darker in a light one.
pub(super) fn bubble(pal: &Palette) -> Rgb {
    mix(pal.page, pal.text, if pal.dark { 0.10 } else { 0.065 })
}

/// Text between the body and the muted colour: a step's target, a card's description.
pub(super) fn soft(pal: &Palette) -> Rgb {
    mix(pal.text, pal.muted, 0.55)
}

/// The colours of added and removed lines: the terminal theme's green and red, given by the
/// window, so a diff here reads as one does in the terminal beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diff {
    pub add: Rgb,
    pub del: Rgb,
}

impl Diff {
    /// From the terminal's sixteen colours (2 is green, 1 is red).
    pub fn from_ansi(ansi: &[Rgb; 16]) -> Self {
        Self { add: ansi[2], del: ansi[1] }
    }

    /// Without a terminal theme: the accent for added lines and the palette's danger colour for
    /// removed ones.
    pub(super) fn fallback(pal: &Palette) -> Self {
        Self { add: pal.accent, del: pal.danger }
    }

    /// Both colours readable as text on `ground`.
    pub(super) fn on(self, ground: Rgb) -> Self {
        Self { add: legible(self.add, ground, 4.5), del: legible(self.del, ground, 4.5) }
    }
}

/// A round button's quiet fill on `ground`.
pub(super) fn quiet_fill(pal: &Palette, ground: Rgb) -> Rgb {
    mix(ground, pal.text, 0.12)
}

/// A raised box: the composer's, and a card's in its place.
pub(super) fn raised_box(p: &mut Painter, pal: &Palette, r: Rect, strong: bool) {
    // Quiet, as in the desktop app: no shadow in a dark theme (the lighter box stands out by
    // itself) and only a breath of one in a light theme; in a dark theme an edge only just lighter
    // than the box (a light theme keeps its hairline, or a white box would vanish).
    if !pal.dark {
        p.shadow_with(r, BOX_RADIUS, false, 0.25);
    }
    // Focus (`strong`) only firms the edge a little.
    let border = match (pal.dark, strong) {
        (true, false) => mix(raised(pal), pal.text, 0.05),
        (true, true) => mix(raised(pal), pal.text, 0.12),
        (false, false) => edge(pal),
        (false, true) => mix(edge(pal), pal.text, 0.15),
    };
    p.frame(r, BOX_RADIUS, raised(pal), border);
}

/// The raised box with an edge in `colour`, two device pixels wide: the box belongs to what that
/// colour stands for (a subagent).
pub(super) fn edged_box(p: &mut Painter, pal: &Palette, r: Rect, colour: Rgb) {
    p.rbox(r.inset(-1.0), BOX_RADIUS + 1.0, colour);
    p.frame(r, BOX_RADIUS, raised(pal), colour);
}

/// A round button with an icon: `fill` under `ink`.
pub(super) fn round_icon(p: &mut Painter, r: Rect, fill: Rgb, icon: &str, size: f32, ink: Rgb) {
    p.rbox(r, r.h / 2.0, fill);
    p.icon(icon, size, r.cx(), r.cy(), ink);
}
