use super::*;
use crate::termtheme::{self, ACCENT_SOURCES, PRESETS, WINDOW_THEMES, preset_index};
use claude_switch_core::WorkersUse;
use claude_switch_core::config::{TERMINAL_FONT_MAX, TERMINAL_FONT_MIN};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::colour_picker::parse_hex;
use mdshared_ui::settings::{Outcome, Row, Value};

const SET_WORKERS: u32 = 1;
const SET_EMBEDDED: u32 = 2;
const SET_ARCHIVED: u32 = 3;
const SET_WORKERS_SHOWN: u32 = 4;
const SET_THEME: u32 = 5;
const SET_TERM_COLOURS: u32 = 6;
const SET_TERM_FONT: u32 = 7;
const SET_BACKDROP: u32 = 8;
const SET_ACCENT: u32 = 9;
const SET_ACCENT_COLOUR: u32 = 10;
const SET_REDUCE_MOTION: u32 = 11;
const SET_CORNERS: u32 = 12;
const SET_SHADOWS: u32 = 13;
const SET_CURSOR_BLINK: u32 = 14;
const SET_TERM_BACKGROUND: u32 = 15;
const SET_TERM_FOREGROUND: u32 = 16;
const SET_TERM_ACCENT: u32 = 17;
const SET_HIDDEN: u32 = 18;

pub fn settings(st: &AppState) -> Dialog {
    let mut options = vec!["Same account as the session".to_string(), "The other account".to_string()];
    options.extend(st.config.accounts.iter().map(|a| format!("Always {}", a.name)));
    let value = match &st.config.workers {
        WorkersUse::Same => 0,
        WorkersUse::Other => 1,
        WorkersUse::Account(slug) => st.config.accounts.iter().position(|a| &a.slug == slug).map_or(0, |k| k + 2),
    };
    let refs: Vec<&str> = options.iter().map(String::as_str).collect();
    let themes = WINDOW_THEMES.map(|(_, name)| name);
    let theme = WINDOW_THEMES.iter().position(|(id, _)| *id == st.config.window_theme).unwrap_or(0);
    let scheme_names: Vec<_> = PRESETS.iter().map(|preset| preset.name).collect();
    let mut theme_row = Row::choice(SET_THEME, "Theme", &themes, theme);
    if st.forced_dark.is_some() {
        theme_row = theme_row.help("The --theme launch flag overrides this setting.");
    }
    let a = &st.config.appearance;
    let auto = termtheme::scheme("auto", st.pal());
    let accent = ACCENT_SOURCES.iter().position(|(id, _)| *id == a.accent_source).unwrap_or(1);
    let accent_names = ACCENT_SOURCES.map(|(_, label)| label);
    let colour = |id, label, saved: &Option<String>, fallback| {
        Row::text(id, label, termtheme::swatch(saved.as_deref(), fallback))
    };
    let rows = vec![
        Row::header("Appearance"),
        theme_row,
        Row::choice(
            SET_BACKDROP,
            Backdrop::TITLE,
            &Backdrop::LABELS,
            Backdrop::parse(&a.backdrop).unwrap_or_default().index(),
        )
        .help("Windows 11, for the title bar. Terminal text stays opaque."),
        Row::choice(SET_ACCENT, "Accent source", &accent_names, accent),
        colour(SET_ACCENT_COLOUR, "Window accent", &a.accent_colour, st.pal().accent)
            .help("Used when Accent source is Custom."),
        Row::toggle(SET_REDUCE_MOTION, "Reduce motion", a.reduce_motion)
            .help("Windows’ reduced-motion preference also applies."),
        Row::number(SET_CORNERS, "Dialog and menu corners", i32::from(a.corner_radius), 0, 16).unit("px"),
        Row::toggle(SET_SHADOWS, "Soft shadows", a.soft_shadows),
        Row::header("Terminal"),
        Row::choice(SET_TERM_COLOURS, "Terminal colours", &scheme_names, preset_index(&st.config.terminal_scheme)),
        Row::number(
            SET_TERM_FONT,
            "Terminal font size",
            i32::from(st.config.terminal_font_size),
            i32::from(TERMINAL_FONT_MIN),
            i32::from(TERMINAL_FONT_MAX),
        )
        .unit("px"),
        Row::toggle(SET_CURSOR_BLINK, "Blink terminal cursor", a.cursor_blink)
            .help("Follows the program and system when enabled."),
        Row::header("Custom colours"),
        colour(SET_TERM_BACKGROUND, "Terminal background", &a.terminal_background, auto.background)
            .help("Select Custom in Terminal colours."),
        colour(SET_TERM_FOREGROUND, "Terminal text", &a.terminal_foreground, auto.foreground)
            .help("Adjusted to at least 7:1 contrast."),
        colour(SET_TERM_ACCENT, "Terminal accent", &a.terminal_accent, st.pal().accent)
            .help("Tints the selection, cursor and active tab mark."),
        Row::header("Workers"),
        Row::choice(SET_WORKERS, "Account the bridge's workers use", &refs, value),
        Row::header("Sessions"),
        Row::toggle(SET_EMBEDDED, "Open CLI sessions in a tab of this window", st.config.embedded_terminal)
            .help("Off: they open in Windows Terminal, or a console window without it."),
        Row::toggle(SET_ARCHIVED, "Show archived sessions", st.config.show_archived),
        Row::toggle(SET_WORKERS_SHOWN, "Show sessions started by tools", st.config.show_workers)
            .help("Sessions that tools and subagents started through the SDK, not you."),
        Row::toggle(SET_HIDDEN, "Show hidden", st.config.show_hidden)
            .help("Include hidden sessions and folders and enable the Hidden filter."),
    ];
    let mut form = Form::new("Settings", rows);
    for id in [SET_ACCENT_COLOUR, SET_TERM_BACKGROUND, SET_TERM_FOREGROUND, SET_TERM_ACCENT] {
        form.show_as_colour(id);
    }
    Dialog::Settings(Box::new(form))
}

/// What a settings form's event gave.
pub fn settings_outcome(st: &mut AppState, o: Outcome) {
    match o {
        Outcome::Closed => st.dialog = None,
        Outcome::Changed { id, value } => {
            match (id, value) {
                (SET_WORKERS, Value::Choice(i)) => {
                    st.config.workers = match i {
                        0 => WorkersUse::Same,
                        1 => WorkersUse::Other,
                        k => st
                            .config
                            .accounts
                            .get(k - 2)
                            .map_or(WorkersUse::Same, |a| WorkersUse::Account(a.slug.clone())),
                    };
                    if let Err(e) = st.source.write_workers(&st.config) {
                        st.error("The workers' setting was not written", e.to_string());
                    }
                }
                (SET_EMBEDDED, Value::Bool(b)) => st.config.embedded_terminal = b,
                (SET_ARCHIVED, Value::Bool(b)) => {
                    st.config.show_archived = b;
                    st.rebuild_view();
                }
                (SET_WORKERS_SHOWN, Value::Bool(b)) => {
                    st.config.show_workers = b;
                    st.rebuild_view();
                }
                (SET_HIDDEN, Value::Bool(b)) => {
                    st.config.show_hidden = b;
                    if !b && st.filter == crate::list::model::Filter::Hidden {
                        st.filter = Default::default();
                    }
                    st.rebuild_view();
                    if st.selected_session().is_some_and(|s| !st.listed(s)) {
                        st.select_first();
                    }
                }
                (SET_THEME, Value::Choice(i)) if i < WINDOW_THEMES.len() => {
                    st.config.window_theme = WINDOW_THEMES[i].0.into();
                }
                (SET_TERM_COLOURS, Value::Choice(i)) if i < PRESETS.len() => {
                    st.config.terminal_scheme = PRESETS[i].id.into();
                }
                (SET_TERM_FONT, Value::Number(n)) => {
                    st.config.terminal_font_size =
                        n.clamp(i32::from(TERMINAL_FONT_MIN), i32::from(TERMINAL_FONT_MAX)) as u8;
                }
                (SET_BACKDROP, Value::Choice(i)) => {
                    st.config.appearance.backdrop = Backdrop::from_index(i).key().into()
                }
                (SET_ACCENT, Value::Choice(i)) if i < ACCENT_SOURCES.len() => {
                    if ACCENT_SOURCES[i].0 == "custom" && st.config.appearance.accent_colour.is_none() {
                        st.config.appearance.accent_colour = Some(termtheme::hex(st.pal().accent));
                    }
                    st.config.appearance.accent_source = ACCENT_SOURCES[i].0.into();
                }
                (SET_REDUCE_MOTION, Value::Bool(b)) => st.config.appearance.reduce_motion = b,
                (SET_SHADOWS, Value::Bool(b)) => st.config.appearance.soft_shadows = b,
                (SET_CURSOR_BLINK, Value::Bool(b)) => st.config.appearance.cursor_blink = b,
                (SET_CORNERS, Value::Number(n)) => st.config.appearance.corner_radius = n.clamp(0, 16) as u8,
                (id, Value::Text(text)) => {
                    let a = &mut st.config.appearance;
                    let field = match id {
                        SET_ACCENT_COLOUR => &mut a.accent_colour,
                        SET_TERM_BACKGROUND => &mut a.terminal_background,
                        SET_TERM_FOREGROUND => &mut a.terminal_foreground,
                        SET_TERM_ACCENT => &mut a.terminal_accent,
                        _ => return,
                    };
                    let Some(colour) = parse_hex(&text) else { return };
                    *field = Some(termtheme::hex(colour));
                }
                _ => return,
            }
            st.retheme();
            st.save_config();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, term::Term, winfile::WindowFile};

    #[test]
    fn appearance_changes_apply_to_hidden_tabs_and_the_flag_still_wins() {
        for forced in [None, Some(true)] {
            let mut st = AppState::new(Source::Demo, forced, WindowFile::default());
            settings_outcome(&mut st, Outcome::Changed { id: SET_THEME, value: Value::Choice(1) });
            assert_eq!(st.config.window_theme, "light");
            assert_eq!(st.pal().dark, forced.unwrap_or(false));
            settings_outcome(
                &mut st,
                Outcome::Changed { id: SET_TERM_COLOURS, value: Value::Choice(preset_index("black")) },
            );
            assert_eq!(st.config.terminal_scheme, "black");

            let ids = ["a", "b"].map(|label| {
                let new =
                    crate::tabs::NewTab { label: label.into(), color: st.pal().muted, session: None, account: None };
                st.tabs.open(new, Term::empty(st.grid))
            });
            st.show_sessions();
            let area = Rect::new(0.0, 80.0, 760.0, 400.0);
            st.layout_terminals(area, area, 1.0);
            let before = st.grid;
            settings_outcome(&mut st, Outcome::Changed { id: SET_TERM_FONT, value: Value::Number(24) });
            st.layout_terminals(area, area, 1.0);
            assert!(st.grid.0 < before.0 && st.grid.1 < before.1);
            for id in ids {
                let screen = &st.tabs.get(id).unwrap().term.screen;
                assert_eq!((screen.cols(), screen.rows()), (usize::from(st.grid.0), usize::from(st.grid.1)));
            }
        }
    }
}
