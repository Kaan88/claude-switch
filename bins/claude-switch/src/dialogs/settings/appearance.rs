//! Appearance's palette library. A saved id selects both the window and its automatic
//! terminal colours; names are captions and can change independently.
use super::{
    AppState, Form, Row, SET_ACCENT, SET_ACCENT_COLOUR, SET_BACKDROP, SET_CORNERS, SET_REDUCE_MOTION, SET_SHADOWS,
    SET_THEME, colour, theme_editor, tiles,
};
use crate::termtheme::{ACCENT_SOURCES, WINDOW_THEMES, named, palettes};
use claude_switch_core::config::AppearancePrefs;
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::gfx::{Painter, mix, rgb};

#[cfg(test)]
pub(super) use tiles::{DARK as SET_DARK_THEME, LIGHT as SET_LIGHT_THEME};

fn entries(prefs: &AppearancePrefs, dark: bool) -> impl Iterator<Item = (&str, &str)> {
    palettes::list(dark)
        .iter()
        .map(|t| (t.id, t.name))
        .chain(prefs.themes.iter().filter(move |(_, t)| t.dark() == dark).map(|(id, t)| (id.as_str(), t.name.as_str())))
}

pub(super) fn selected_index(config: &claude_switch_core::Config, family: tiles::Family) -> usize {
    match family {
        tiles::Family::Terminal => super::preset_index(&config.terminal_scheme),
        tiles::Family::Window(dark) => {
            let a = &config.appearance;
            let saved = if dark { &a.dark_theme } else { &a.light_theme };
            entries(a, dark).position(|(id, _)| id == saved).unwrap_or(0)
        }
    }
}

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let themes = WINDOW_THEMES.map(|(_, name)| name);
    let theme = WINDOW_THEMES.iter().position(|(id, _)| *id == st.config.window_theme).unwrap_or(0);
    let mut theme_row = Row::choice(SET_THEME, "Theme", &themes, theme);
    if st.forced_dark.is_some() {
        theme_row = theme_row.help("The --theme launch flag overrides this setting.");
    } else if st.high_contrast {
        theme_row = theme_row.help("Windows high contrast is on and takes priority.");
    }
    let a = &st.config.appearance;
    let mut rows = vec![theme_row];
    for dark in [true, false] {
        let saved = if dark { &a.dark_theme } else { &a.light_theme };
        let entries: Vec<_> = entries(a, dark).collect();
        for (group, slice) in entries.chunks(tiles::ACROSS).enumerate() {
            let label = match (dark, group) {
                (true, 0) => "Dark palette",
                (false, 0) => "Light palette",
                _ => "",
            };
            let names: Vec<_> = slice.iter().map(|e| e.1).collect();
            rows.extend(tiles::line(
                tiles::row_id(dark, group),
                label,
                &names,
                slice.iter().position(|e| e.0 == saved).unwrap_or(0),
            ));
        }
    }
    let accent = ACCENT_SOURCES.iter().position(|(id, _)| *id == a.accent_source).unwrap_or(0);
    rows.extend([
        Row::choice(
            SET_BACKDROP,
            Backdrop::TITLE,
            &Backdrop::LABELS,
            Backdrop::parse(&a.backdrop).unwrap_or_default().index(),
        )
        .help("Windows 11 22H2 or later. Terminal opacity is set separately."),
        Row::choice(SET_ACCENT, "Accent source", &ACCENT_SOURCES.map(|(_, name)| name), accent)
            .help("Default: clay for built-ins; saved accent for named themes."),
        colour(SET_ACCENT_COLOUR, "Window accent", &a.accent_colour, st.pal().accent)
            .help("Used when Accent source is Custom."),
        Row::toggle(SET_REDUCE_MOTION, "Reduce motion", a.reduce_motion)
            .help("Windows’ reduced-motion preference also applies."),
        Row::number(SET_CORNERS, "Dialog and menu corners", i32::from(a.corner_radius), 0, 16).unit("px"),
        Row::toggle(SET_SHADOWS, "Soft shadows", a.soft_shadows),
        Row::header("Theme library"),
        Row::action(theme_editor::EDIT_DARK, "Selected dark palette", "Customize…"),
        Row::action(theme_editor::EDIT_LIGHT, "Selected light palette", "Customize…"),
        Row::action(theme_editor::IMPORT, "Open a theme file", "Import…"),
    ]);
    rows
}

pub(super) fn choose(st: &mut AppState, id: u32, index: usize) -> bool {
    let Some((family, group)) = tiles::group(id) else { return false };
    let tiles::Family::Window(dark) = family else {
        let Some(preset) = super::PRESETS.get(group * tiles::ACROSS + index) else { return false };
        st.config.terminal_scheme = preset.id.into();
        return true;
    };
    let a = &mut st.config.appearance;
    let Some(key) = entries(a, dark).nth(group * tiles::ACROSS + index).map(|e| e.0.to_owned()) else { return false };
    *if dark { &mut a.dark_theme } else { &mut a.light_theme } = key;
    true
}

pub(super) fn draw(p: &mut Painter, st: &AppState, form: &Form, hover: Option<(u32, usize)>, focus: bool) {
    let a = &st.config.appearance;
    let dark_index = selected_index(&st.config, tiles::Family::Window(true));
    let light_index = selected_index(&st.config, tiles::Family::Window(false));
    tiles::draw(p, st.pal(), form, hover, focus, |family, index| {
        let tiles::Family::Window(dark) = family else {
            let preset = super::PRESETS.get(index)?;
            let (s, mark) = crate::termtheme::terminal(preset.id, st.pal(), a, st.high_contrast);
            return Some(tiles::Sample {
                page: s.background,
                bar: s.background,
                pane: s.background,
                text: s.foreground,
                muted: mix(s.background, s.foreground, 0.55),
                accent: mark,
                terminal: s.selection,
                ink: s.selection_text.unwrap_or(s.foreground),
                ansi: tiles::five(&s.ansi),
                terminal_only: true,
                selected: index == super::preset_index(&st.config.terminal_scheme),
            });
        };
        let (id, _) = entries(a, dark).nth(index)?;
        let system = (a.accent_source == "system").then_some(st.pal().accent);
        let (pal, terminal, ink, ansi) = if let Some(t) = a.themes.get(id) {
            (
                named::palette(t, a, system),
                t.terminal_background.0,
                t.terminal_foreground.0,
                tiles::five(&t.ansi.map(|c| c.0)),
            )
        } else {
            let t = palettes::get(dark, id);
            (
                crate::windowlook::palette(&t.palette, &st.appearance, system),
                rgb(t.terminal.background),
                rgb(t.terminal.foreground),
                tiles::five(&t.terminal.ansi.map(rgb)),
            )
        };
        let selected = index == if dark { dark_index } else { light_index };
        Some(tiles::Sample {
            page: pal.page,
            bar: pal.title,
            pane: pal.pane,
            text: pal.text,
            muted: pal.muted,
            accent: pal.accent,
            terminal,
            ink,
            ansi,
            terminal_only: false,
            selected,
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use mdshared_text::Bitmap;
    use mdshared_ui::{
        anim::Animator,
        gfx::{Face, Gfx, Rect},
        keys::{KeyName, Mods},
        settings::{Outcome, Value},
    };
    use std::time::Instant;

    #[test]
    fn named_swatches_draw_at_minimum_size_and_mouse_and_keyboard_choose_the_same_palette() {
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        let mut gfx = Gfx::new();
        for scale in [1.0, 1.5] {
            let win = Rect::new(0.0, 0.0, 760.0, 480.0);
            let mut bmp = Bitmap::new((760.0 * scale) as u32, (480.0 * scale) as u32);
            let mut p = Painter::new(&mut bmp, &mut gfx, scale);
            let mut page = super::super::Settings::new(&st, 0);
            let mut anim = Animator::new();
            anim.begin_frame(Instant::now());
            page.draw(&mut p, &st, win, 1.0, &mut anim);
            if scale == 1.0
                && let Some(dir) = std::env::var_os("CS_THEME_SHOTS")
            {
                p.bmp.write_png(&std::path::Path::new(&dir).join("appearance.png")).unwrap();
            }
            for (id, dark) in [(SET_DARK_THEME, true), (SET_LIGHT_THEME, false)] {
                let line: Vec<_> = tiles::cards(&page.form).into_iter().filter(|c| c.0 == id).collect();
                assert_eq!(line.len(), palettes::list(dark).len());
                let r = line.last().unwrap().2;
                let x = r.right() - 20.0;
                page.form.wheel(r.x, r.y, r.y - page.form.body_rect().y - 40.0);
                let r = tiles::cards(&page.form).into_iter().find(|c| c.0 == id && c.1 == line.len() - 1).unwrap().2;
                page.press(x, r.cy(), Mods::default(), 1);
                let k = palettes::list(dark).len() - 1;
                assert!(
                    matches!(page.release(x,r.cy()),super::super::SettingsEvent::Form(Outcome::Changed{id:found,value:Value::Choice(n)}) if found==id&&n==k)
                );
                assert!(
                    matches!(page.key(&st.config,&KeyName::Home,Mods::default(),None),super::super::SettingsEvent::Form(Outcome::Changed{id:found,value:Value::Choice(0)}) if found==id)
                );
            }
        }
        assert!(choose(&mut st, SET_DARK_THEME, 2));
        st.retheme();
        assert_eq!(st.pal().page, palettes::DARK_THEMES[2].palette.page);
        assert!(choose(&mut st, SET_LIGHT_THEME, 2));
        st.forced_dark = Some(false);
        st.retheme();
        assert_eq!(st.term_colours.0.background, rgb(palettes::LIGHT_THEMES[2].terminal.background));
        assert!(!choose(&mut st, SET_DARK_THEME, 99));
    }

    /// A custom theme as the theme editor would save it, on `base`'s colours.
    fn custom(name: &str, dark: bool) -> claude_switch_core::config::theme::NamedTheme {
        use claude_switch_core::config::theme::Colour;
        let t = &palettes::list(dark)[2];
        let p = t.palette;
        claude_switch_core::config::theme::NamedTheme {
            name: name.into(),
            base: t.id.into(),
            page: Colour(p.page),
            bar: Colour(p.bar),
            title: Colour(p.title),
            pane: Colour(p.pane),
            field: Colour(p.field),
            popup: Colour(p.popup),
            text: Colour(p.text),
            muted: Colour(p.muted),
            accent: Colour([0x2f, 0x9e, 0x8f]),
            selection: Colour(p.selection),
            terminal_background: Colour(rgb(t.terminal.background)),
            terminal_foreground: Colour(rgb(t.terminal.foreground)),
            terminal_selection: Colour(rgb(t.terminal.selection)),
            ansi: t.terminal.ansi.map(|c| Colour(rgb(c))),
        }
    }

    /// Every card on the Appearance page, at the narrowest and a wide Settings, at each display
    /// scale, in the dark and the light window, with custom themes wrapping a second line: names
    /// are whole (or elided only when a custom name is too long), cards keep to their line and
    /// never reach the next setting, and the dark and light cards share one column grid. With
    /// `CS_THEME_SHOTS` set, each frame is saved there as a PNG.
    #[test]
    fn palette_cards_keep_names_whole_and_stay_inside_their_line_at_every_width_and_scale() {
        let shots = std::env::var_os("CS_THEME_SHOTS").map(std::path::PathBuf::from);
        let mut gfx = Gfx::new();
        for (dark, custom_themes) in [(true, false), (false, false), (true, true), (false, true)] {
            let mut st = AppState::new(Source::Demo, Some(dark), Default::default());
            if custom_themes {
                let a = &mut st.config.appearance;
                for (k, name) in ["Solarized night", "Evening on the porch with a long theme name"].iter().enumerate() {
                    a.themes.insert(format!("user-d{k}"), custom(name, true));
                    a.themes.insert(format!("user-l{k}"), custom(name, false));
                }
                a.dark_theme = "user-d1".into();
                a.light_theme = "user-l0".into();
                st.retheme();
            }
            for (w, h) in [(760.0, 480.0), (760.0, 720.0), (1280.0, 800.0)] {
                for scale in [1.0f32, 1.25, 1.5, 2.0] {
                    let win = Rect::new(0.0, 0.0, w, h);
                    let mut bmp = Bitmap::new((w * scale) as u32, (h * scale) as u32);
                    let mut p = Painter::new(&mut bmp, &mut gfx, scale);
                    let mut page = super::super::Settings::new(&st, 0);
                    let mut anim = Animator::new();
                    anim.begin_frame(Instant::now());
                    // Hover a dark card and give the keyboard the light line.
                    page.draw(&mut p, &st, win, 1.0, &mut anim);
                    // With custom themes that is the long name, whose tooltip shows.
                    let hovered = tiles::cards(&page.form)[if custom_themes { 6 } else { 1 }].2;
                    page.pointer_move(hovered.cx(), hovered.cy());
                    page.key(&st.config, &KeyName::Tab, Mods::default(), None);
                    page.form.focus_row(SET_LIGHT_THEME);
                    page.draw(&mut p, &st, win, 1.0, &mut anim);
                    if let Some(dir) = &shots {
                        let name = format!(
                            "palettes-{}{}-{w}x{h}-{}pct.png",
                            if dark { "dark" } else { "light" },
                            if custom_themes { "-custom" } else { "" },
                            (scale * 100.0) as u32
                        );
                        p.bmp.write_png(&dir.join(name)).unwrap();
                    }
                    let form = &page.form;
                    let cards = tiles::cards(form);
                    let expected = (if custom_themes { 2 } else { 0 }) * 2 + 9;
                    assert_eq!(cards.len(), expected);
                    for (id, k, card, line) in &cards {
                        let at = format!("{w}x{h} @{scale}: row {id} card {k}");
                        // Inside its line, which ends where the next row starts.
                        assert!(card.x >= line.x && card.right() <= line.right(), "{at}");
                        assert!(card.y >= line.y && card.bottom() + 8.0 <= line.bottom(), "{at}");
                        let i = form.rows().iter().position(|r| r.id() == Some(*id)).unwrap();
                        let next =
                            form.rows()[i + 1..].iter().position(|r| !matches!(r, Row::Header(t) if t.is_empty()));
                        if let Some(n) = next {
                            assert!(form.rect_of_row(i + 1 + n).y >= line.bottom() - 0.01, "{at}");
                        }
                        // Cards of a line don't touch.
                        if let Some((_, _, after, _)) = cards.iter().find(|c| c.0 == *id && c.1 == k + 1) {
                            assert!(after.x - card.right() >= 10.0, "{at}");
                        }
                        // The name is whole, or elided only if it is a long custom one.
                        let room = tiles::name_room(*card);
                        assert!(room.x >= card.x && room.right() <= card.right() && room.bottom() <= card.bottom());
                        assert!(room.h >= 1.6 * tiles::NAME_SIZE, "{at}: room for descenders");
                        let Row::Choice { options, .. } = &form.rows()[i] else { unreachable!() };
                        let name = &options[*k];
                        let fits = p.measure(Face::Semibold, tiles::NAME_SIZE, name) <= room.w;
                        assert!(fits || name.chars().count() > 20, "{at}: {name} is clipped");
                    }
                    if h == 480.0 {
                        // At the smallest window, Tab to the light line brings all its cards
                        // into view, not only the label row the form itself reveals.
                        let mut page = super::super::Settings::new(&st, 0);
                        page.draw(&mut p, &st, win, 1.0, &mut anim);
                        page.key(&st.config, &KeyName::Tab, Mods::default(), None);
                        for _ in 0..40 {
                            if page.form.focus_id() == Some(SET_LIGHT_THEME) {
                                break;
                            }
                            page.key(&st.config, &KeyName::Tab, Mods::default(), None);
                        }
                        assert_eq!(page.form.focus_id(), Some(SET_LIGHT_THEME));
                        let body = page.form.body_rect();
                        for (id, _, card, _) in tiles::cards(&page.form) {
                            if id == SET_LIGHT_THEME {
                                assert!(card.y >= body.y && card.bottom() <= body.bottom(), "{w}x{h} @{scale}");
                            }
                        }
                    }
                    if w == 760.0 && h == 720.0 && !custom_themes {
                        terminal_page(&st, &mut p, win, shots.as_deref(), dark, scale);
                    }
                    // One column grid for dark and light.
                    let column = |id: u32, k: usize| cards.iter().find(|c| c.0 == id && c.1 == k).map(|c| c.2);
                    for k in 0..4 {
                        let (d, l) = (column(SET_DARK_THEME, k).unwrap(), column(SET_LIGHT_THEME, k).unwrap());
                        assert_eq!((d.x, d.w, d.h), (l.x, l.w, l.h));
                    }
                }
            }
        }
    }

    /// The Terminal page's presets use the same cards: whole names, nothing past the line.
    fn terminal_page(
        st: &AppState,
        p: &mut Painter,
        win: Rect,
        shots: Option<&std::path::Path>,
        dark: bool,
        scale: f32,
    ) {
        let index = super::super::PAGES.iter().position(|(name, _)| *name == super::super::TERMINAL).unwrap();
        let mut page = super::super::Settings::new(st, index);
        let mut anim = Animator::new();
        anim.begin_frame(Instant::now());
        page.draw(p, st, win, 1.0, &mut anim);
        if let Some(dir) = shots {
            let name = format!(
                "terminal-{}-{}x{}-{}pct.png",
                if dark { "dark" } else { "light" },
                win.w,
                win.h,
                (scale * 100.0) as u32
            );
            p.bmp.write_png(&dir.join(name)).unwrap();
        }
        let cards = tiles::cards(&page.form);
        assert_eq!(cards.len(), super::super::PRESETS.len());
        for (id, k, card, line) in cards {
            assert!(card.bottom() + 8.0 <= line.bottom() && card.right() <= line.right());
            let i = page.form.rows().iter().position(|r| r.id() == Some(id)).unwrap();
            let Row::Choice { options, .. } = &page.form.rows()[i] else { unreachable!() };
            assert!(
                p.measure(Face::Semibold, tiles::NAME_SIZE, &options[k]) <= tiles::name_room(card).w,
                "{}",
                options[k]
            );
        }
    }
}
