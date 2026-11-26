use super::*;

use super::{opencode_rows, terminal_appearance};
use crate::termtheme::{self, ACCENT_SOURCES, PRESETS, WINDOW_THEMES, preset_index};
use claude_switch_core::WorkersUse;
use claude_switch_core::config::{TERMINAL_FONT_MAX, TERMINAL_FONT_MIN, TerminalCursor, terminal_opacity};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::colour_picker::parse_hex;
use mdshared_ui::settings::{Form, Outcome, Row, Value};

mod appearance;
mod pages;
pub(crate) use pages::Settings;

pub(crate) const TERMINAL: &str = "Terminal";
pub(crate) const OPENCODE: &str = "OpenCode";
pub(super) const HANDOVERS: &str = "Handovers";

type Page = (&'static str, fn(&AppState) -> Vec<Row>);
// In display order. Adding a category only needs a label and a row builder here.
const PAGES: &[Page] = &[
    ("Appearance", appearance::rows),
    (TERMINAL, terminal),
    ("Sessions", sessions),
    ("Usage", usage),
    (HANDOVERS, handovers),
    (OPENCODE, opencode),
    ("Notification area", notifications),
];

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
const SET_USAGE: u32 = 19;
const SET_ATTENTION: u32 = 71;
const SET_USAGE_WARNINGS: u32 = 72;
const SET_TRAY: u32 = 73;
const SET_CLOSE_TO_TRAY: u32 = 74;
const SET_RESTORE_TABS: u32 = 87;
const FORGET_HANDOVER_CONSENTS: u32 = 90;
const SET_SUGGEST_TOPICS: u32 = 91;
const SYNC_WORKERS: u32 = 94;
const SET_HANDOVER_AGENT: u32 = 95;

pub(crate) fn settings(st: &mut AppState, category: Option<&str>) -> Dialog {
    let category = category.or(st.settings_category);
    let index = PAGES.iter().position(|(name, _)| Some(*name) == category).unwrap_or(0);
    st.settings_category = Some(PAGES[index].0);
    Dialog::Settings(Box::new(Settings::new(st, index)))
}

pub(crate) enum SettingsEvent {
    Form(Outcome),
    Page(usize),
}

pub(crate) fn settings_event(st: &mut AppState, event: SettingsEvent) {
    match event {
        SettingsEvent::Form(outcome) => settings_outcome(st, outcome),
        SettingsEvent::Page(index) => {
            if let Some((name, _)) = PAGES.get(index)
                && let Some(Dialog::Settings(mut settings)) = st.dialog.take_if(|d| matches!(d, Dialog::Settings(_)))
            {
                settings.show_page(st, index);
                st.settings_category = Some(name);
                st.dialog = Some(Dialog::Settings(settings));
            }
        }
    }
}

fn page_form(st: &AppState, index: usize) -> Form {
    let (name, rows) = PAGES[index];
    let mut form = Form::new(name, rows(st)).single_page();
    for id in [SET_ACCENT_COLOUR, SET_TERM_BACKGROUND, SET_TERM_FOREGROUND, SET_TERM_ACCENT] {
        form.show_as_colour(id);
    }
    form
}

fn colour(id: u32, label: &str, saved: &Option<String>, fallback: mdshared_ui::gfx::Rgb) -> Row {
    Row::text(id, label, termtheme::swatch(saved.as_deref(), fallback))
}

fn terminal(st: &AppState) -> Vec<Row> {
    let a = &st.config.appearance;
    let auto = termtheme::scheme("auto", st.pal());
    let scheme_names: Vec<_> = PRESETS.iter().map(|preset| preset.name).collect();
    vec![
        Row::choice(SET_TERM_COLOURS, "Terminal colours", &scheme_names, preset_index(&st.config.terminal_scheme)),
        Row::number(
            SET_TERM_FONT,
            "Terminal font size",
            i32::from(st.config.terminal_font_size),
            i32::from(TERMINAL_FONT_MIN),
            i32::from(TERMINAL_FONT_MAX),
        )
        .unit("px"),
        terminal_appearance::cursor_row(a),
        Row::toggle(SET_CURSOR_BLINK, "Blink", a.cursor_blink).help("Follows the program and system when enabled."),
        terminal_appearance::opacity_row(a, st.high_contrast, st.window_focused),
        Row::choice(
            SET_RESTORE_TABS,
            "Reopen terminal tabs on startup",
            &crate::winfile::RestoreTabs::LABELS,
            st.win_file.restore_tabs.index(),
        )
        .help("Claude tabs only. Always still honors resume warnings."),
        Row::header("Custom colours"),
        colour(SET_TERM_BACKGROUND, "Terminal background", &a.terminal_background, auto.background)
            .help("Select Custom in Terminal colours."),
        colour(SET_TERM_FOREGROUND, "Terminal text", &a.terminal_foreground, auto.foreground)
            .help("Adjusted to at least 7:1 contrast."),
        colour(SET_TERM_ACCENT, "Terminal accent", &a.terminal_accent, st.pal().accent)
            .help("Tints the selection, cursor and active tab mark."),
    ]
}

fn sessions(st: &AppState) -> Vec<Row> {
    vec![
        Row::toggle(SET_EMBEDDED, "Open CLI sessions in a tab of this window", st.config.embedded_terminal)
            .help("Off: they open in a separate console window."),
        Row::toggle(SET_ARCHIVED, "Show archived sessions", st.config.show_archived),
        Row::toggle(SET_WORKERS_SHOWN, "Show sessions started by tools", st.config.show_workers)
            .help("Sessions that tools and subagents started through the SDK, not you."),
        Row::toggle(SET_HIDDEN, "Show hidden", st.config.show_hidden)
            .help("Include hidden sessions and folders and enable the Hidden filter."),
        Row::choice(
            SET_ATTENTION,
            "Background session alerts",
            &claude_switch_core::config::Attention::OPTIONS.map(|(_, _, label)| label),
            st.config.notifications.attention.index(),
        )
        .help("Toast mode needs the tray; otherwise flashes."),
        Row::toggle(SET_SUGGEST_TOPICS, "Include topics when suggesting names", st.config.suggest_topics)
            .help("Shares existing topic labels only when you confirm Suggest names. No extra request."),
    ]
}

fn usage(st: &AppState) -> Vec<Row> {
    vec![
        Row::toggle(SET_USAGE, "Show usage", st.config.show_usage)
            .help("Read each account’s login token to ask Anthropic for subscription usage."),
        Row::toggle(SET_USAGE_WARNINGS, "Warn at 80% and 95% usage", st.config.notifications.usage_warnings),
    ]
}

fn handovers(st: &AppState) -> Vec<Row> {
    // The existing name dialog edits a draft and validates it on OK, not on every keystroke.
    let current = st.config.compact.default_agent.as_deref().unwrap_or("Default (Claude)");
    let label =
        current.char_indices().nth(26).map_or_else(|| current.to_owned(), |(end, _)| format!("{}…", &current[..end]));
    vec![
        Row::action(SET_HANDOVER_AGENT, "Default CLI agent", &label),
        Row::action(FORGET_HANDOVER_CONSENTS, "OpenCode provider consents", "Forget consents"),
    ]
}

pub(super) fn set_handover_agent(st: &mut AppState, name: &str) -> Result<(), claude_switch_core::Refusal> {
    if let crate::source::Source::Real(roots) = &st.source {
        st.config.compact = claude_switch_core::config::update_compact(roots, |prefs| prefs.set_default_agent(name))?;
    } else {
        st.config.compact.set_default_agent(name)?;
    }
    Ok(())
}

fn opencode(st: &AppState) -> Vec<Row> {
    let mut options = vec!["Same account as the session".to_string(), "The other account".to_string()];
    options.extend(st.config.accounts.iter().map(|a| format!("Always {}", a.name)));
    let value = match &st.config.workers {
        WorkersUse::Same => 0,
        WorkersUse::Other => 1,
        WorkersUse::Account(slug) => st.config.accounts.iter().position(|a| &a.slug == slug).map_or(0, |k| k + 2),
    };
    let refs: Vec<&str> = options.iter().map(String::as_str).collect();
    [Row::choice(SET_WORKERS, "Account the bridge's workers use", &refs, value)]
        .into_iter()
        .chain(opencode_rows::rows(st))
        .chain([Row::action(SYNC_WORKERS, "Sync bridge account settings", "Sync")])
        .collect()
}

fn notifications(st: &AppState) -> Vec<Row> {
    vec![
        Row::toggle(SET_TRAY, "Show tray icon", st.config.notifications.tray_icon),
        Row::toggle(SET_CLOSE_TO_TRAY, "Close to tray", st.config.notifications.close_to_tray)
            .help("Requires the tray icon. Use Quit from its menu to exit."),
    ]
}

/// What a settings form's event gave.
fn settings_outcome(st: &mut AppState, o: Outcome) {
    match o {
        Outcome::Closed => st.dialog = None,
        Outcome::Changed { id: SET_RESTORE_TABS, value: Value::Choice(i) } => {
            if let Some(&mode) = crate::winfile::RestoreTabs::ALL.get(i) {
                st.win_file.restore_tabs = mode;
                if mode == crate::winfile::RestoreTabs::Never {
                    st.dismiss_tab_restore();
                }
                st.save_window();
            }
        }
        Outcome::Action(FORGET_HANDOVER_CONSENTS) => st.forget_handover_consents(),
        Outcome::Action(SET_HANDOVER_AGENT) if !st.config_damaged => {
            if let crate::source::Source::Real(roots) = &st.source {
                st.config.compact = claude_switch_core::config::load(roots).compact;
            }
            st.dialog = Some(name_dialog(
                "Default handover agent (CLI)",
                st.config.compact.default_agent.as_deref().unwrap_or_default(),
                NameFor::HandoverAgent,
            ));
        }
        Outcome::Action(SYNC_WORKERS) => st.sync_worker_accounts(),
        Outcome::Action(opencode_rows::REFRESH) => st.refresh_opencode(),
        Outcome::Action(opencode_rows::RESTART) => st.ask_restart_opencode(),
        Outcome::Action(opencode_rows::STUDIO) => st.open_config_studio(),
        Outcome::Changed { id, value } => {
            if st.config_damaged {
                st.dialog = None;
                return;
            }
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
                }
                (SET_EMBEDDED, Value::Bool(b)) => st.config.embedded_terminal = b,
                (SET_SUGGEST_TOPICS, Value::Bool(b)) => st.config.suggest_topics = b,
                (SET_USAGE, Value::Bool(b)) => {
                    if let crate::source::Source::Real(roots) = &st.source
                        && let Err(error) = claude_switch_core::config::set_usage(roots, b)
                    {
                        st.error("Usage setting was not saved", error.to_string());
                        return;
                    }
                    st.apply_committed_usage(b);
                }
                (SET_ATTENTION, Value::Choice(i)) if i < claude_switch_core::config::Attention::OPTIONS.len() => {
                    st.config.notifications.attention = claude_switch_core::config::Attention::OPTIONS[i].0;
                }
                (SET_USAGE_WARNINGS, Value::Bool(b)) => st.config.notifications.usage_warnings = b,
                (SET_TRAY, Value::Bool(b)) => st.config.notifications.tray_icon = b,
                (SET_CLOSE_TO_TRAY, Value::Bool(b)) => st.config.notifications.close_to_tray = b,
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
                (id, Value::Choice(i)) if appearance::choose(st, id, i) => {}
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
                (terminal_appearance::CURSOR, Value::Choice(i)) if i < TerminalCursor::OPTIONS.len() => {
                    st.config.appearance.cursor_shape = TerminalCursor::OPTIONS[i].0;
                }
                (terminal_appearance::OPACITY, Value::Number(n)) => {
                    st.config.appearance.terminal_opacity = terminal_opacity(i64::from(n));
                }
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

    fn current(st: &AppState) -> &Settings {
        let Some(Dialog::Settings(settings)) = &st.dialog else { panic!("Settings must be open") };
        settings
    }

    fn navigate(st: &mut AppState, key: KeyName) {
        assert!(super::super::key(st, Instant::now(), &key, Mods::default(), None));
    }

    #[test]
    fn every_setting_has_exactly_one_page() {
        let st = AppState::new(Source::Demo, None, WindowFile::default());
        let expected: &[&[u32]] = &[
            &[
                SET_THEME,
                appearance::SET_DARK_THEME,
                appearance::SET_LIGHT_THEME,
                SET_BACKDROP,
                SET_ACCENT,
                SET_ACCENT_COLOUR,
                SET_REDUCE_MOTION,
                SET_CORNERS,
                SET_SHADOWS,
            ],
            &[
                SET_TERM_COLOURS,
                SET_TERM_FONT,
                terminal_appearance::CURSOR,
                SET_CURSOR_BLINK,
                terminal_appearance::OPACITY,
                SET_RESTORE_TABS,
                SET_TERM_BACKGROUND,
                SET_TERM_FOREGROUND,
                SET_TERM_ACCENT,
            ],
            &[SET_EMBEDDED, SET_ARCHIVED, SET_WORKERS_SHOWN, SET_HIDDEN, SET_ATTENTION, SET_SUGGEST_TOPICS],
            &[SET_USAGE, SET_USAGE_WARNINGS],
            &[SET_HANDOVER_AGENT, FORGET_HANDOVER_CONSENTS],
            &[SET_WORKERS, opencode_rows::REFRESH, opencode_rows::RESTART, opencode_rows::STUDIO, SYNC_WORKERS],
            &[SET_TRAY, SET_CLOSE_TO_TRAY],
        ];
        assert_eq!(PAGES.len(), expected.len());
        for (index, ids) in expected.iter().enumerate() {
            let form = page_form(&st, index);
            assert_eq!(form.rows().iter().filter_map(Row::id).collect::<Vec<_>>(), *ids);
        }
        assert_eq!(
            expected.iter().flat_map(|ids| ids.iter()).collect::<std::collections::HashSet<_>>().len(),
            expected.iter().map(|ids| ids.len()).sum::<usize>(),
        );
    }

    #[test]
    fn handover_agent_edit_commits_on_ok_reports_invalid_input_and_can_clear() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        let now = Instant::now();
        let open = |st: &mut AppState| settings_outcome(st, Outcome::Action(SET_HANDOVER_AGENT));
        st.dialog = Some(settings(&mut st, Some(HANDOVERS)));
        open(&mut st);
        super::super::text_input(&mut st, "bad name");
        assert!(st.config.compact.default_agent.is_none());
        super::super::click(&mut st, now, OK, 0.0);
        assert!(matches!(&st.dialog, Some(Dialog::Name { error: Some(_), .. })));
        super::super::click(&mut st, now, CANCEL, 0.0);
        assert_eq!(current(&st).category(), HANDOVERS);
        open(&mut st);
        super::super::text_input(&mut st, "my-plugin:review");
        super::super::click(&mut st, now, OK, 0.0);
        assert_eq!(st.config.compact.default_agent.as_deref(), Some("my-plugin:review"));
        assert_eq!(current(&st).category(), HANDOVERS);
        open(&mut st);
        if let Some(Dialog::Name { field, .. }) = &mut st.dialog {
            field.select_all();
        }
        super::super::key(&mut st, now, &KeyName::Backspace, Mods::default(), None);
        super::super::click(&mut st, now, OK, 0.0);
        assert!(st.config.compact.default_agent.is_none());
    }

    #[test]
    fn arrows_and_tab_separate_navigation_from_controls_and_reopen_remembers_page() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.dialog = Some(settings(&mut st, None));
        assert_eq!(current(&st).category(), "Appearance");
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).category(), TERMINAL);
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).category(), "Sessions");
        assert!(current(&st).form.value(SET_TERM_FONT).is_none());
        navigate(&mut st, KeyName::Tab);
        assert_eq!(current(&st).form.focus_id(), Some(SET_EMBEDDED));
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).form.focus_id(), Some(SET_ARCHIVED));
        let archived = st.config.show_archived;
        navigate(&mut st, KeyName::Space);
        assert_ne!(st.config.show_archived, archived);
        navigate(&mut st, KeyName::Up);
        assert_eq!(current(&st).category(), "Sessions");
        assert_eq!(current(&st).form.focus_id(), Some(SET_EMBEDDED));
        super::super::key(&mut st, Instant::now(), &KeyName::Tab, Mods { shift: true, ..Mods::default() }, None);
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).category(), "Usage");
        navigate(&mut st, KeyName::Escape);
        assert!(st.dialog.is_none());
        st.dialog = Some(settings(&mut st, None));
        assert_eq!(current(&st).category(), "Usage");
        st.opencode_settings(true);
        assert_eq!(current(&st).category(), OPENCODE);
        navigate(&mut st, KeyName::Escape);
        st.dialog = Some(settings(&mut st, None));
        assert_eq!(current(&st).category(), OPENCODE);
    }

    #[test]
    fn switching_pages_reads_the_merged_config_without_writing_navigation_or_republishing_preferences() {
        use claude_switch_core::{Roots, config};

        let folder = std::env::temp_dir().join(format!("cs-settings-pages-test-{}", std::process::id()));
        let roots = Roots { home: folder.join("home"), appdata: folder.join("appdata") };
        let mut st = AppState::new(Source::Real(roots.clone()), None, WindowFile::default());
        st.dialog = Some(settings(&mut st, Some("Sessions")));
        navigate(&mut st, KeyName::Down);
        navigate(&mut st, KeyName::Up);
        assert!(!roots.config_file().exists(), "navigation must not save");

        let mut elsewhere = config::load(&roots);
        elsewhere.terminal_font_size = 21;
        config::save(&roots, &elsewhere).unwrap();
        config::set_usage(&roots, false).unwrap();
        config::update_compact(&roots, |compact| {
            compact.model = "fixture/model".into();
            compact.consents.insert("fixture".into());
            Ok(())
        })
        .unwrap();

        // An independent field is merged; the next page is built from the merged values.
        navigate(&mut st, KeyName::Tab);
        navigate(&mut st, KeyName::Space);
        assert!(!config::load(&roots).embedded_terminal);
        st.dialog = Some(settings(&mut st, Some(TERMINAL)));
        assert_eq!(current(&st).form.value(SET_TERM_FONT), Some(Value::Number(21)));
        navigate(&mut st, KeyName::Down);
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).form.value(SET_USAGE), Some(Value::Bool(false)));
        let compact = config::load(&roots).compact;
        assert_eq!(compact.model, "fixture/model");
        assert!(compact.consents.contains("fixture"));

        // A subsequent conflicting write still uses the window baseline and is refused.
        elsewhere = config::load(&roots);
        elsewhere.terminal_font_size = 22;
        config::save(&roots, &elsewhere).unwrap();
        st.dialog = Some(settings(&mut st, Some(TERMINAL)));
        navigate(&mut st, KeyName::Tab);
        navigate(&mut st, KeyName::Down);
        navigate(&mut st, KeyName::Left);
        assert_eq!(config::load(&roots).terminal_font_size, 22);
        assert_eq!(st.config.terminal_font_size, 22);
    }

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
