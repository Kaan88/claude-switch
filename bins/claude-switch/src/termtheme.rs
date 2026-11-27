//! Terminal colours are independent of the window chrome, except in Match window.

use claude_switch_core::config::AppearancePrefs;
use mdshared_ui::appearance::{AccentFrom, Appearance, Speed};
use mdshared_ui::colour_picker::{HexForm, parse_hex};
use mdshared_ui::cover::contrast;
use mdshared_ui::gfx::{Rgb, mix, rgb};
use mdshared_ui::termview::Scheme;
use mdshared_ui::theme::Palette;
use mdshared_ui::tokens::legible;

pub mod named;
pub mod palettes;
use palettes::{DARK_ANSI, DARK_TERMINAL, LIGHT_ANSI, LIGHT_TERMINAL, TermColours};

pub const WINDOW_THEMES: [(&str, &str); 3] = [("system", "System"), ("light", "Light"), ("dark", "Dark")];
pub const ACCENT_SOURCES: [(&str, &str); 3] = [("theme", "Default"), ("system", "System"), ("custom", "Custom")];
pub const CUSTOM: &str = "custom";
const BLACK: Rgb = rgb(0x000000);
const WHITE: Rgb = rgb(0xffffff);

pub fn appearance(prefs: &AppearancePrefs) -> Appearance {
    let mut look = Appearance::DEFAULT;
    look.accent = match prefs.accent_source.as_str() {
        "custom" => prefs.accent_colour.as_deref().and_then(AccentFrom::from_name).unwrap_or(AccentFrom::Theme),
        "system" => AccentFrom::System,
        _ => AccentFrom::Theme,
    };
    look.radius = f32::from(prefs.corner_radius);
    look.shadow = if prefs.soft_shadows { 1.0 } else { 0.0 };
    look.motion = if prefs.reduce_motion { Speed::Off } else { Speed::Fluent };
    look.sane()
}

pub fn hex(colour: Rgb) -> String {
    HexForm::DEFAULT.format(colour)
}

/// A terminal swatch's saved value, or the corresponding colour of Automatic.
pub fn swatch(saved: Option<&str>, fallback: Rgb) -> String {
    saved.map_or_else(|| hex(fallback), str::to_owned)
}

pub fn window_dark(id: &str) -> Option<bool> {
    match id {
        "light" => Some(false),
        "dark" => Some(true),
        _ => None,
    }
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    colors: Colors,
}

#[derive(Clone, Copy)]
enum Colors {
    /// The window palette's own terminal colours.
    Automatic,
    Window,
    Fixed(TermColours),
}

const fn fixed(
    id: &'static str,
    name: &'static str,
    background: u32,
    foreground: u32,
    cursor: u32,
    selection: u32,
    ansi: [u32; 16],
) -> Preset {
    Preset { id, name, colors: Colors::Fixed(TermColours { background, foreground, cursor, selection, ansi }) }
}

// Published foreground, background, cursor and ANSI values for Campbell, Vintage and Tango:
// https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalSettingsModel/defaults.json
// Selection tints are app-specific to keep the selected text readable.
const CAMPBELL: [u32; 16] = [
    0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc, 0x767676, 0xe74856, 0x16c60c,
    0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
];
const VINTAGE: [u32; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0, 0x808080, 0xff0000, 0x00ff00,
    0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
];
const TANGO: [u32; 16] = [
    0x000000, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, 0x555753, 0xef2929, 0x8ae234,
    0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
];

const DARK: Preset = Preset { id: "dark", name: "Dark", colors: Colors::Fixed(DARK_TERMINAL) };
const LIGHT: Preset = Preset { id: "light", name: "Light", colors: Colors::Fixed(LIGHT_TERMINAL) };

pub const PRESETS: &[Preset] = &[
    Preset { id: "auto", name: "Automatic", colors: Colors::Automatic },
    Preset { id: "match-window", name: "Match window", colors: Colors::Window },
    DARK,
    fixed("black", "Black", 0x000000, 0xd6d6d6, 0xd6d6d6, 0x284a70, DARK_ANSI),
    LIGHT,
    fixed("campbell", "Campbell", 0x0c0c0c, 0xcccccc, 0xffffff, 0x284a70, CAMPBELL),
    fixed("vintage", "Vintage", 0x000000, 0xc0c0c0, 0xffffff, 0x284564, VINTAGE),
    fixed("tango-dark", "Tango Dark", 0x000000, 0xd3d7cf, 0xffffff, 0x315372, TANGO),
    fixed("tango-light", "Tango Light", 0xffffff, 0x555753, 0x000000, 0xc2d9f2, TANGO),
    Preset { id: CUSTOM, name: "Custom", colors: Colors::Automatic },
];

pub fn preset_index(id: &str) -> usize {
    PRESETS.iter().position(|p| p.id == id).unwrap_or(0)
}

pub fn scheme(id: &str, pal: &Palette) -> Scheme {
    let mut s = Scheme::from_palette(pal);
    let preset = &PRESETS[preset_index(id)];
    let colors = match preset.colors {
        Colors::Automatic => Colors::Fixed(palettes::of(pal).terminal),
        _ => preset.colors,
    };
    match colors {
        Colors::Fixed(TermColours { background, foreground, cursor, selection, ansi }) => {
            s.background = rgb(background);
            s.foreground = rgb(foreground);
            s.cursor = rgb(cursor);
            s.selection = rgb(selection);
            s.ansi = ansi.map(rgb);
        }
        Colors::Window => {
            s.background = pal.bar;
            let tint = mix(pal.bar, pal.selection, 0.40);
            // High-contrast bars are black or white; a 40% tint does not show there.
            s.selection = if contrast(tint, pal.bar) >= 1.4 { tint } else { pal.selection };
        }
        Colors::Automatic => unreachable!("automatic resolves to a fixed preset"),
    }
    s.cursor_text = s.background;
    // The toolkit replaces the cell background on selection but otherwise keeps the resolved
    // glyph colour (which may be the old background after SGR 7). Use a readable pair instead.
    s.selection_text = Some(s.foreground);
    s
}

/// The scheme and its tab mark. The renderer consumes these cached values, not the form's text.
pub fn terminal(id: &str, pal: &Palette, prefs: &AppearancePrefs, high_contrast: bool) -> (Scheme, Rgb) {
    let mut s = scheme(id, pal);
    if !high_contrast
        && (preset_index(id) == 0 || id == CUSTOM)
        && let Some(theme) = named::selected(prefs, pal.dark)
    {
        s.background = theme.terminal_background.0;
        s.foreground = theme.terminal_foreground.0;
        s.ansi = theme.ansi.map(|c| c.0);
        s.selection = theme.terminal_selection.0;
        s.cursor = s.foreground;
        s.cursor_text = s.background;
        s.selection_text =
            Some(if contrast(s.foreground, s.selection) >= 4.5 { s.foreground } else { opposing(s.selection) });
    }
    let read = |v: &Option<String>, default| v.as_deref().and_then(parse_hex).unwrap_or(default);
    let mut accent = pal.accent;
    if id == CUSTOM {
        (s.background, s.foreground) = readable_pair(
            read(&prefs.terminal_background, s.background),
            read(&prefs.terminal_foreground, s.foreground),
        );
        let ansi = if contrast(s.background, WHITE) > contrast(s.background, BLACK) { DARK_ANSI } else { LIGHT_ANSI };
        s.ansi = ansi.map(rgb);
        if !high_contrast {
            accent = read(&prefs.terminal_accent, accent);
        }
        s.cursor = legible(accent, s.background, 4.5);
        s.cursor_text = if contrast(s.background, s.cursor) >= 4.5 { s.background } else { opposing(s.cursor) };
    }
    let mark = legible(accent, s.background, 3.0);
    if prefs.accent_source != "theme" || id == CUSTOM || high_contrast {
        let tint = mix(s.background, mark, 0.55);
        s.selection = if contrast(tint, s.background) >= 1.4 { tint } else { mark };
        s.selection_text =
            Some(if contrast(s.foreground, s.selection) >= 4.5 { s.foreground } else { opposing(s.selection) });
    }
    (s, mark)
}

fn opposing(c: Rgb) -> Rgb {
    if contrast(c, BLACK) >= contrast(c, WHITE) { BLACK } else { WHITE }
}

fn readable_pair(mut bg: Rgb, fg: Rgb) -> (Rgb, Rgb) {
    let edge = opposing(bg);
    if contrast(bg, edge) < 7.0 {
        // Mid-range grounds can fail against both black and white; move the ground only then.
        bg = legible(bg, edge, 7.0);
    }
    (bg, legible(fg, bg, 7.0))
}

/// A per-tab palette for the toolkit's active tab, which takes its fill from `bar`.
pub fn active_tab_palette(pal: &Palette, (background, foreground, accent): (Rgb, Rgb, Rgb)) -> Palette {
    Palette {
        bar: background,
        text: foreground,
        muted: mix(foreground, background, 0.22),
        accent,
        hover: mix(background, foreground, 0.12),
        press: mix(background, foreground, 0.2),
        ..*pal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_ui::theme::{DARK as DARK_WINDOW, LIGHT as LIGHT_WINDOW};
    use std::collections::HashSet;

    fn luminance(c: Rgb) -> f64 {
        let [r, g, b] = c.map(|v| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn presets_have_unique_ids_and_readable_distinct_colours() {
        let mut ids = HashSet::new();
        for preset in PRESETS {
            assert!(ids.insert(preset.id), "duplicate id {}", preset.id);
            for t in palettes::DARK_THEMES.iter().chain(palettes::LIGHT_THEMES) {
                let pal = crate::windowlook::palette(&t.palette, &Appearance::DEFAULT, None);
                let s = scheme(preset.id, &pal);
                assert!(contrast(s.foreground, s.background) >= 7.0, "default text: {}", preset.id);
                assert!(contrast(s.selection, s.background) >= 1.4, "selection: {}", preset.id);
                assert!(contrast(s.selection_text.unwrap(), s.selection) >= 4.5, "selected text: {}", preset.id);
                for (i, a) in s.ansi.iter().enumerate() {
                    for (j, b) in s.ansi.iter().enumerate().skip(i + 1) {
                        let squared: i32 = (0..3).map(|k| (i32::from(a[k]) - i32::from(b[k])).pow(2)).sum();
                        assert!(squared >= 8 * 8, "{} ANSI {i} and {j} are too close", preset.id);
                    }
                }
            }
        }
    }

    #[test]
    fn automatic_takes_each_palettes_own_terminal_whatever_the_accent() {
        for t in palettes::DARK_THEMES.iter().chain(palettes::LIGHT_THEMES) {
            for accent in ["theme", "system", "custom"] {
                let prefs = AppearancePrefs {
                    accent_source: accent.into(),
                    accent_colour: Some("#2080c0".into()),
                    ..AppearancePrefs::default()
                };
                let pal = crate::windowlook::palette(&t.palette, &appearance(&prefs), Some(rgb(0x30a050)));
                assert_eq!(scheme("auto", &pal).background, rgb(t.terminal.background), "{}", t.id);
            }
        }
    }

    #[test]
    fn automatic_and_unknown_follow_the_window_and_match_uses_the_bar() {
        for (pal, id) in [(DARK_WINDOW, "dark"), (LIGHT_WINDOW, "light")] {
            let want = scheme(id, &pal);
            assert_eq!(scheme("auto", &pal), want);
            assert_eq!(scheme("future-scheme", &pal), want);
            assert_eq!(scheme("match-window", &pal).background, pal.bar);
        }
        assert!(luminance(scheme("auto", &DARK_WINDOW).background) < luminance(DARK_WINDOW.bar));
        assert_eq!(scheme("black", &LIGHT_WINDOW).background, rgb(0x000000));
    }

    #[test]
    fn custom_colours_keep_high_contrast_even_for_identical_midtones() {
        use claude_switch_core::config::AppearancePrefs;
        for i in (0..=255_u8).step_by(3) {
            let sample = [i; 3];
            let prefs = AppearancePrefs {
                terminal_background: Some(hex(sample)),
                terminal_foreground: Some(hex(sample)),
                terminal_accent: Some(hex(sample)),
                ..AppearancePrefs::default()
            };
            for pal in [DARK_WINDOW, LIGHT_WINDOW] {
                let (s, mark) = terminal(CUSTOM, &pal, &prefs, false);
                assert!(contrast(s.background, s.foreground) >= 7.0, "{s:?}");
                assert!(contrast(s.background, s.selection) >= 1.4, "{s:?}");
                assert!(contrast(s.selection, s.selection_text.unwrap()) >= 4.5, "{s:?}");
                assert!(contrast(s.cursor, s.cursor_text) >= 4.5, "{s:?}");
                assert!(contrast(mark, s.background) >= 3.0, "{s:?}");
            }
        }
        let prefs = AppearancePrefs {
            terminal_background: Some(hex(BLACK)),
            terminal_foreground: Some(hex(WHITE)),
            ..AppearancePrefs::default()
        };
        let (s, _) = terminal(CUSTOM, &LIGHT_WINDOW, &prefs, false);
        assert_eq!((s.background, s.foreground), (BLACK, WHITE), "a readable choice is kept");
    }

    #[test]
    fn motion_stops_at_rest_and_either_reduction_setting_snaps() {
        use mdshared_ui::anim::{Animator, document};
        use std::time::{Duration, Instant};

        for (reduce_motion, system_reduced) in [(false, false), (true, false), (false, true)] {
            let prefs = AppearancePrefs { reduce_motion, ..AppearancePrefs::default() };
            let mut anim = Animator::new();
            anim.set_policy(appearance(&prefs).motion.policy());
            anim.set_system_reduced(system_reduced);
            let now = Instant::now();
            anim.begin_frame(now);
            let value = anim.value_from("enter", 0.0_f32, 1.0, document::MENU.enter);
            let reduced = reduce_motion || system_reduced;
            assert_eq!(value, if reduced { 1.0 } else { 0.0 });
            assert_eq!(anim.end_frame().is_none(), reduced);
            for i in 1..=20 {
                anim.begin_frame(now + Duration::from_millis(i * 16));
                anim.value_from("enter", 0.0_f32, 1.0, document::MENU.enter);
                if i == 20 {
                    assert!(anim.end_frame().is_none());
                } else {
                    anim.end_frame();
                }
            }
        }
    }
}
