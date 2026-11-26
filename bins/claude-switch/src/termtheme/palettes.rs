//! The window palettes the Appearance page offers, each with the terminal colours that match it.
//!
//! Everything a theme is lives here, in one table: the toolkit's palettes where it has them
//! (`theme::DARK`, `theme::LIGHT`, `appearance::WARM`, `appearance::COOL`, and the high-contrast
//! pair in `tokens`), three dark palettes of the app's own, and a terminal scheme for each. The
//! toolkit has no dark warm or cool palette and no terminal schemes yet; both could move there.
//! The window's accent is not part of a palette: Claude's clay (or the system's, or a picked
//! colour) is put into whichever palette is chosen by `windowlook::palette`.

use mdshared_ui::appearance::{COOL, WARM};
use mdshared_ui::gfx::rgb;
use mdshared_ui::theme::{DARK, LIGHT, Palette};
use mdshared_ui::tokens::{HIGH_CONTRAST_DARK, HIGH_CONTRAST_LIGHT};

/// A terminal's colours: background, text, cursor, selection and the 16 ANSI colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermColours {
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

pub struct Theme {
    /// The word saved in the config.
    pub id: &'static str,
    /// What the Appearance page calls it.
    pub name: &'static str,
    pub palette: Palette,
    /// What "Automatic" terminal colours are under this palette.
    pub terminal: TermColours,
    /// Text, quiet text and the accent are held to 7:1 rather than 4.5:1.
    pub high_contrast: bool,
}

pub const DARK_ANSI: [u32; 16] = [
    0x161616, 0xcf5252, 0x6eae56, 0xd2a64f, 0x5888cf, 0xab72c4, 0x51aca9, 0xbebebe, 0x767676, 0xff7777, 0x9cdb81,
    0xf7d77b, 0x88b6ff, 0xe19bf2, 0x85dedd, 0xf2f2f2,
];
pub const LIGHT_ANSI: [u32; 16] = [
    0x262626, 0xa92d36, 0x347028, 0x88590a, 0x255da6, 0x873d9b, 0x216a75, 0xd0d0d0, 0x626262, 0xca4148, 0x56883c,
    0xa66f12, 0x397bc0, 0xa655b1, 0x36868c, 0xf5f5f5,
];
const BRIGHT_ANSI: [u32; 16] = [
    0x3a3a3a, 0xff6b6b, 0x7ee07e, 0xffd75f, 0x7fb2ff, 0xe39bff, 0x6fe3e3, 0xe6e6e6, 0x8a8a8a, 0xff9a9a, 0xa8f5a8,
    0xffe9a0, 0xa9ccff, 0xf0c2ff, 0xa0f5f5, 0xffffff,
];
const DEEP_ANSI: [u32; 16] = [
    0x000000, 0x9a0000, 0x0a5c0a, 0x6b4500, 0x0037a6, 0x6e1a87, 0x005f66, 0xbfbfbf, 0x4d4d4d, 0xc00000, 0x2e7d32,
    0x8a5a00, 0x1f5fd6, 0x8e24aa, 0x00838f, 0xe8e8e8,
];

const fn term(background: u32, foreground: u32, selection: u32, ansi: [u32; 16]) -> TermColours {
    TermColours { background, foreground, cursor: foreground, selection, ansi }
}

pub const DARK_TERMINAL: TermColours = term(0x101010, 0xd6d6d6, 0x284a70, DARK_ANSI);
pub const LIGHT_TERMINAL: TermColours = term(0xfcfcfc, 0x262626, 0xb8d5f5, LIGHT_ANSI);

/// Near black: the neutral dark one step deeper, for OLED screens and dim rooms.
const BLACK: Palette = Palette {
    page: rgb(0x0d0d0d),
    title: rgb(0x171717),
    bar: rgb(0x131313),
    pane: rgb(0x111111),
    text: rgb(0xe8e8e8),
    muted: rgb(0xa0a0a0),
    faint: rgb(0x707070),
    hover: rgb(0x262626),
    press: rgb(0x303030),
    on: rgb(0x242a38),
    line: rgb(0x272727),
    field: rgb(0x0a0a0a),
    popup: rgb(0x1a1a1a),
    popup_line: rgb(0x343434),
    ..DARK
};

/// Warm dark: brown-greys and parchment text.
const WARM_DARK: Palette = Palette {
    page: rgb(0x1f1b18),
    title: rgb(0x2b2622),
    bar: rgb(0x27221e),
    pane: rgb(0x241f1b),
    text: rgb(0xece4da),
    muted: rgb(0xb0a597),
    faint: rgb(0x80766a),
    hover: rgb(0x38312b),
    press: rgb(0x433a33),
    on: rgb(0x3e3833),
    line: rgb(0x3a332c),
    field: rgb(0x1a1714),
    popup: rgb(0x2a2420),
    popup_line: rgb(0x463d35),
    warn_bg: rgb(0x3d3020),
    ..DARK
};

/// Cool blue-grey dark: slate surfaces and blue-white text.
const COOL_DARK: Palette = Palette {
    page: rgb(0x1b2028),
    title: rgb(0x252b35),
    bar: rgb(0x212731),
    pane: rgb(0x1f242d),
    text: rgb(0xe3e8ef),
    muted: rgb(0x9ea9b8),
    faint: rgb(0x6e7988),
    hover: rgb(0x2f3743),
    press: rgb(0x39424f),
    on: rgb(0x2c3c55),
    line: rgb(0x323a46),
    field: rgb(0x171b22),
    popup: rgb(0x252c37),
    popup_line: rgb(0x3d4654),
    ..DARK
};

const fn theme(id: &'static str, name: &'static str, palette: Palette, terminal: TermColours) -> Theme {
    Theme { id, name, palette, terminal, high_contrast: false }
}

/// The dark palettes, in the order the Appearance page lists them. The first is the default.
pub const DARK_THEMES: &[Theme] = &[
    theme("dark", "Neutral", DARK, DARK_TERMINAL),
    theme("black", "Near black", BLACK, term(0x050505, 0xdadada, 0x284a70, DARK_ANSI)),
    theme("warm-dark", "Warm", WARM_DARK, term(0x171411, 0xe2d8cc, 0x5a4632, DARK_ANSI)),
    theme("cool-dark", "Blue-grey", COOL_DARK, term(0x141920, 0xd8dee8, 0x2d4c72, DARK_ANSI)),
    Theme {
        id: "contrast-dark",
        name: "High contrast",
        palette: HIGH_CONTRAST_DARK,
        terminal: term(0x000000, 0xffffff, 0x1f4f8f, BRIGHT_ANSI),
        high_contrast: true,
    },
];

/// The light palettes, in the order the Appearance page lists them. The first is the default.
pub const LIGHT_THEMES: &[Theme] = &[
    theme("light", "Paper", LIGHT, LIGHT_TERMINAL),
    theme("warm-light", "Warm", WARM, term(0xfbf7f0, 0x2a241f, 0xdcc3a0, LIGHT_ANSI)),
    theme("cool-light", "Cool", COOL, term(0xf6f9fc, 0x17202b, 0xb0cbea, LIGHT_ANSI)),
    Theme {
        id: "contrast-light",
        name: "High contrast",
        palette: HIGH_CONTRAST_LIGHT,
        terminal: term(0xffffff, 0x000000, 0xb3d1ff, DEEP_ANSI),
        high_contrast: true,
    },
];

/// The dark or light list.
pub fn list(dark: bool) -> &'static [Theme] {
    if dark { DARK_THEMES } else { LIGHT_THEMES }
}

/// The saved id's place in the dark or light list; an unknown or empty id is the first.
pub fn index(dark: bool, id: &str) -> usize {
    list(dark).iter().position(|t| t.id == id).unwrap_or(0)
}

/// The theme the saved id names in the dark or light list (the first for anything else).
pub fn get(dark: bool, id: &str) -> &'static Theme {
    &list(dark)[index(dark, id)]
}

/// The theme a window palette was made from, whatever accent was put into it: found by its
/// surfaces. A palette from none of them (the system's contrast colours) gives the plain dark or
/// light theme.
pub fn of(pal: &Palette) -> &'static Theme {
    let all = list(pal.dark);
    all.iter()
        .find(|t| t.palette.page == pal.page && t.palette.bar == pal.bar && t.palette.text == pal.text)
        .unwrap_or(&all[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_ui::cover::contrast;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_every_palette_is_found_again_from_its_colours() {
        let mut ids = HashSet::new();
        for dark in [true, false] {
            for (i, t) in list(dark).iter().enumerate() {
                assert!(ids.insert(t.id), "duplicate {}", t.id);
                assert_eq!(t.palette.dark, dark, "{}", t.id);
                assert_eq!(index(dark, t.id), i);
                assert!(std::ptr::eq(of(&t.palette), t), "{}", t.id);
            }
            assert_eq!(index(dark, "no-such-theme"), 0);
            assert_eq!(get(dark, "").id, list(dark)[0].id);
        }
        // A saved dark id is not found in the light list.
        assert_eq!(get(false, "black").id, "light");
    }

    #[test]
    fn text_reads_on_every_surface_and_terminal_matches_the_palette() {
        for t in DARK_THEMES.iter().chain(LIGHT_THEMES) {
            let p = &t.palette;
            let (text, muted) = if t.high_contrast { (7.0, 7.0) } else { (7.0, 4.5) };
            for surface in [p.page, p.title, p.bar, p.pane, p.field, p.popup] {
                assert!(contrast(p.text, surface) >= text, "{} text", t.id);
                assert!(contrast(p.muted, surface) >= muted, "{} muted", t.id);
            }
            assert!(contrast(p.text, p.hover) >= 4.5 && contrast(p.text, p.press) >= 4.5, "{} hover", t.id);
            let term = t.terminal;
            let (bg, fg) = (rgb(term.background), rgb(term.foreground));
            // Dark windows get dark terminals and light windows light ones.
            assert_eq!(contrast(bg, rgb(0)) < contrast(bg, rgb(0xffffff)), t.palette.dark, "{}", t.id);
            assert!(contrast(fg, bg) >= if t.high_contrast { 15.0 } else { 10.0 }, "{} terminal text", t.id);
        }
    }
}
