//! Colours and marks the conversation, the composer and the cards share. Every colour is made
//! from the window's palette, so a theme the user picked or made carries through.
use mdshared_ui::gfx::{Painter, Rect, Rgb, mix};
use mdshared_ui::theme::Palette;

/// The padding of the composer and of a card: their text starts where the reading column's does.
pub(super) const INSET: f32 = 16.0;
/// The corner radius of the composer and of a card in its place.
pub(super) const BOX_RADIUS: f32 = 18.0;

/// An arrow pointing up: Send.
pub(super) const ARROW_UP: &str = "M8 13V3.5M3.8 7.7L8 3.5l4.2 4.2";

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

/// A round button's quiet fill on `ground`.
pub(super) fn quiet_fill(pal: &Palette, ground: Rgb) -> Rgb {
    mix(ground, pal.text, 0.12)
}

/// A raised box: the composer's, and a card's in its place.
pub(super) fn raised_box(p: &mut Painter, pal: &Palette, r: Rect, strong: bool) {
    p.shadow_with(r, BOX_RADIUS, pal.dark, 0.7);
    let border = mix(edge(pal), pal.text, if strong { 0.38 } else { 0.12 });
    p.frame(r, BOX_RADIUS, raised(pal), border);
}

/// A round button with an icon: `fill` under `ink`.
pub(super) fn round_icon(p: &mut Painter, r: Rect, fill: Rgb, icon: &str, size: f32, ink: Rgb) {
    p.rbox(r, r.h / 2.0, fill);
    p.icon(icon, size, r.cx(), r.cy(), ink);
}
