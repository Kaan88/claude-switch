//! Appearance's palette library. A saved id selects both the window and its automatic
//! terminal colours; names are captions and can change independently.
use super::{
    AppState, Form, Row, SET_ACCENT, SET_ACCENT_COLOUR, SET_BACKDROP, SET_CORNERS, SET_REDUCE_MOTION, SET_SHADOWS,
    SET_THEME, colour, theme_editor, tiles,
};
use crate::termtheme::{ACCENT_SOURCES, WINDOW_THEMES, named, palettes};
use claude_switch_core::config::AppearancePrefs;
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::gfx::{Painter, rgb};

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
            rows.push(Row::choice(
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
                bar: s.ansi[4],
                text: s.foreground,
                accent: mark,
                terminal: s.selection,
                ink: s.selection_text.unwrap_or(s.foreground),
                selected: index == super::preset_index(&st.config.terminal_scheme),
            });
        };
        let (id, _) = entries(a, dark).nth(index)?;
        let system = (a.accent_source == "system").then_some(st.pal().accent);
        let (pal, terminal, ink) = if let Some(t) = a.themes.get(id) {
            (named::palette(t, a, system), t.terminal_background.0, t.terminal_foreground.0)
        } else {
            let t = palettes::get(dark, id);
            (
                crate::windowlook::palette(&t.palette, &st.appearance, system),
                rgb(t.terminal.background),
                rgb(t.terminal.foreground),
            )
        };
        let selected = index == if dark { dark_index } else { light_index };
        Some(tiles::Sample {
            page: pal.page,
            text: pal.text,
            bar: pal.bar,
            accent: pal.accent,
            terminal,
            ink,
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
        gfx::{Gfx, Rect},
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
                let i = page.form.rows().iter().position(|r| r.id() == Some(id)).unwrap();
                let r = page.form.rect_of_row(i);
                let mut spans = vec![0; palettes::list(dark).len()];
                for x in (r.x as i32)..r.right() as i32 {
                    if let Some((_, k)) = tiles::hit(&page.form, x as f32, r.cy()) {
                        spans[k] += 1;
                    }
                }
                for (t, w) in palettes::list(dark).iter().zip(spans) {
                    assert!(p.measure(mdshared_ui::gfx::Face::Ui, 11.5, t.name) + 5.0 <= w as f32, "{}", t.name);
                }
                let x = r.right() - 20.0;
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
}
