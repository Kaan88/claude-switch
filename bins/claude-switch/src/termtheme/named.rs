//! Apply a user's palette by its saved id. Surface colours aren't palette identifiers.
use super::{appearance, palettes};
use claude_switch_core::config::{AppearancePrefs, theme::NamedTheme};
use mdshared_ui::{
    gfx::{Rgb, mix},
    theme::Palette,
};

pub fn selected(prefs: &AppearancePrefs, dark: bool) -> Option<&NamedTheme> {
    let id = if dark { &prefs.dark_theme } else { &prefs.light_theme };
    prefs.themes.get(id).filter(|t| t.dark() == dark)
}

pub fn palette(theme: &NamedTheme, prefs: &AppearancePrefs, system: Option<Rgb>) -> Palette {
    let base = palettes::get(theme.dark(), &theme.base).palette;
    let (bg, text, accent) = (theme.page.0, theme.text.0, theme.accent.0);
    let palette = Palette {
        page: bg,
        title: theme.title.0,
        bar: theme.bar.0,
        pane: theme.pane.0,
        field: theme.field.0,
        popup: theme.popup.0,
        text,
        muted: theme.muted.0,
        faint: mix(bg, theme.muted.0, 0.75),
        accent,
        on_accent: super::opposing(accent),
        selection: theme.selection.0,
        hover: mix(theme.pane.0, text, 0.09),
        press: mix(theme.pane.0, text, 0.16),
        line: mix(bg, text, 0.17),
        popup_line: mix(theme.popup.0, text, 0.22),
        on: mix(theme.pane.0, accent, 0.22),
        ..base
    };
    if prefs.accent_source == "theme" { palette } else { appearance(prefs).palette(&palette, system, None) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::config::theme::Colour;
    #[test]
    fn automatic_resolves_by_id_even_if_the_window_colours_equal_a_builtin() {
        let p = palettes::DARK_THEMES[0].palette;
        let t = NamedTheme {
            name: "Personal".into(),
            base: "dark".into(),
            page: Colour(p.page),
            bar: Colour(p.bar),
            title: Colour(p.title),
            pane: Colour(p.pane),
            field: Colour(p.field),
            popup: Colour(p.popup),
            text: Colour(p.text),
            muted: Colour(p.muted),
            accent: Colour([203, 125, 152]),
            selection: Colour(p.selection),
            terminal_background: Colour([11, 12, 13]),
            terminal_foreground: Colour([190, 191, 192]),
            terminal_selection: Colour([30, 51, 72]),
            ansi: std::array::from_fn(|i| Colour([i as u8 * 7, 4, 8])),
        };
        let a = AppearancePrefs {
            dark_theme: "user-1".into(),
            themes: [("user-1".into(), t.clone())].into(),
            ..Default::default()
        };
        let pal = palette(selected(&a, true).unwrap(), &a, None);
        assert_eq!(pal.accent, t.accent.0);
        assert!(selected(&a, false).is_none());
        let (s, _) = super::super::terminal("auto", &pal, &a, false);
        assert_eq!(s.ansi, t.ansi.map(|c| c.0));
        assert_eq!(
            (s.background, s.foreground, s.selection),
            (t.terminal_background.0, t.terminal_foreground.0, t.terminal_selection.0)
        );
        let preset = super::super::terminal("campbell", &pal, &a, false).0;
        assert_ne!(preset.ansi, s.ansi);
        assert_ne!(super::super::terminal("auto", &pal, &a, true).0.ansi, s.ansi);
    }
}
