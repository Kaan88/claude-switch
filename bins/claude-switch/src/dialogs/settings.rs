use super::*;

use super::{opencode_rows, terminal_appearance};
use crate::termtheme::{self, ACCENT_SOURCES, PRESETS, WINDOW_THEMES, preset_index};
use claude_switch_core::WorkersUse;
use claude_switch_core::compact::FocusKind;
use claude_switch_core::config::{TERMINAL_FONT_MAX, TERMINAL_FONT_MIN, TerminalCursor, terminal_opacity};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::colour_picker::parse_hex;
use mdshared_ui::settings::{Form, Outcome, Row, Value};

mod appearance;
mod pages;
mod theme_editor;
mod tiles;
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
const SET_HANDOVER_MODEL: u32 = 96;
const SET_AUTO_HANDOVER: u32 = 97;
const SET_AUTO_PERCENT: u32 = 98;
const SET_CONTINUE_IN: u32 = 120;
/// Per [`FocusKind`]: the text row is `FOCUS_TEXT + 2 * index`, its Reset button the next id.
const FOCUS_TEXT: u32 = 121;
#[cfg(test)]
const SET_DEFAULT_FOCUS: u32 = FOCUS_TEXT;
#[cfg(test)]
const RESET_DEFAULT_FOCUS: u32 = FOCUS_TEXT + 1;

fn focus_ids(kind: FocusKind) -> (u32, u32) {
    let text = FOCUS_TEXT + 2 * kind.index() as u32;
    (text, text + 1)
}

fn focus_kind_of(id: u32) -> Option<(FocusKind, bool)> {
    FocusKind::ALL.into_iter().find_map(|kind| {
        let (text, reset) = focus_ids(kind);
        (id == text || id == reset).then_some((kind, id == reset))
    })
}
const SET_CLASSIC_VIEW: u32 = 99;
const SET_CHAT_READING: u32 = 130;
const AUTO_STEPS: [u8; 5] = [50, 60, 70, 80, 90];

/// The threshold choice: Claude Code's own point first, then percentages (a saved one included).
fn auto_percents(st: &AppState) -> Vec<Option<u8>> {
    let mut steps: Vec<_> = std::iter::once(None).chain(AUTO_STEPS.map(Some)).collect();
    if let Some(saved) = st.config.compact.auto_percent
        && !steps.contains(&Some(saved))
    {
        let at = steps.iter().position(|step| step.is_some_and(|step| step > saved)).unwrap_or(steps.len());
        steps.insert(at, Some(saved));
    }
    steps
}

fn update_handover_prefs(st: &mut AppState, change: impl FnOnce(&mut claude_switch_core::compact::Preferences)) {
    save_handover_prefs(st, change);
    refresh_handover_page(st);
}

fn save_handover_prefs(st: &mut AppState, change: impl FnOnce(&mut claude_switch_core::compact::Preferences)) {
    let result = match &st.source {
        crate::source::Source::Real(roots) => claude_switch_core::config::update_compact(roots, |prefs| {
            change(prefs);
            Ok(())
        })
        .map(|prefs| st.config.compact = prefs),
        crate::source::Source::Demo => {
            change(&mut st.config.compact);
            Ok(())
        }
    };
    if let Err(e) = result {
        st.error("Handover setting was not saved", e.to_string());
    }
}

pub(crate) fn settings(st: &mut AppState, category: Option<&str>) -> Dialog {
    let category = category.or(st.settings_category);
    let index = PAGES.iter().position(|(name, _)| Some(*name) == category).unwrap_or(0);
    prepare_page(st, index);
    st.settings_category = Some(PAGES[index].0);
    Dialog::Settings(Box::new(Settings::new(st, index)))
}

/// Fresh data for a page that shows more than settings: the accounts' agents and the saved
/// handover preferences (another window may have changed them).
fn prepare_page(st: &mut AppState, index: usize) {
    if PAGES[index].0 == HANDOVERS {
        if let crate::source::Source::Real(roots) = &st.source {
            st.config.compact = claude_switch_core::config::load(roots).compact;
        }
        st.load_settings_agents();
    }
}

fn refresh_handover_page(st: &mut AppState) {
    let rows = handovers(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == HANDOVERS
    {
        settings.form.set_rows(rows);
    }
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
                prepare_page(st, index);
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
        // Changes stay in the picker until Apply, Enter or a press outside; Escape restores.
        form.show_as_colour_with_confirmation(id);
    }
    form
}

fn colour(id: u32, label: &str, saved: &Option<String>, fallback: mdshared_ui::gfx::Rgb) -> Row {
    Row::text(id, label, termtheme::swatch(saved.as_deref(), fallback))
}

fn terminal(st: &AppState) -> Vec<Row> {
    let a = &st.config.appearance;
    let auto = termtheme::terminal("auto", st.pal(), a, st.high_contrast).0;
    let selected = preset_index(&st.config.terminal_scheme);
    let mut rows: Vec<_> = PRESETS
        .chunks(tiles::ACROSS)
        .enumerate()
        .map(|(i, chunk)| {
            let (id, label) = if i == 0 { (SET_TERM_COLOURS, "Terminal colours") } else { (tiles::MORE_TERMINAL, "") };
            let names: Vec<_> = chunk.iter().map(|s| s.name).collect();
            Row::choice(id, label, &names, if selected / tiles::ACROSS == i { selected % tiles::ACROSS } else { 0 })
        })
        .collect();
    rows.extend([
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
    ]);
    rows
}

fn sessions(st: &AppState) -> Vec<Row> {
    vec![
        Row::toggle(SET_EMBEDDED, "Open CLI sessions in a tab of this window", st.config.embedded_terminal)
            .help("Off: they open in a separate console window."),
        Row::toggle(SET_CLASSIC_VIEW, "Classic terminal view for Claude tabs", st.config.classic_terminal_view)
            .help("Chat cannot send while the CLI draws its full-screen view. Applies to new tabs."),
        Row::choice(
            SET_CHAT_READING,
            "Chat reading width",
            &["Narrow", "Medium", "Wide", "Full"],
            crate::view::Reading::ALL
                .iter()
                .position(|r| *r == crate::view::Reading::from_key(&st.win_file.chat_reading))
                .unwrap_or(1),
        )
        .help("The Markdown viewer has its own width control."),
        Row::toggle(SET_ARCHIVED, "Show sessions archived in the desktop app", st.config.show_archived)
            .help("Claude Desktop's own archive. Switch's Archive is separate."),
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

const NO_AGENT: &str = "None (plain Claude)";

/// The agent choice's names: `None` first, then the accounts' agents, then a saved name that no
/// account has (shown as missing; launches check it again for their own account and folder).
fn agent_names(st: &AppState) -> Vec<Option<String>> {
    let saved = st.config.compact.default_agent.as_deref();
    let mut names: Vec<_> =
        std::iter::once(None).chain(st.compact.settings_agents.iter().map(|(name, _)| Some(name.clone()))).collect();
    if let Some(saved) = saved
        && !names.iter().any(|name| name.as_deref() == Some(saved))
    {
        names.push(Some(saved.to_owned()));
    }
    names
}

/// The models this window last listed, with the saved one first if the list lacks it.
fn model_options(st: &AppState) -> Vec<String> {
    let mut models = st.listed_models().map(<[String]>::to_vec).unwrap_or_default();
    let saved = &st.config.compact.model;
    if !saved.is_empty() && !models.contains(saved) {
        models.insert(0, saved.clone());
    }
    models
}

fn handovers(st: &AppState) -> Vec<Row> {
    let saved = st.config.compact.default_agent.as_deref();
    let names = agent_names(st);
    let labels: Vec<String> = names
        .iter()
        .map(|name| match name {
            None => NO_AGENT.to_owned(),
            Some(name) => st
                .compact
                .settings_agents
                .iter()
                .find(|(found, _)| found == name)
                .map_or_else(|| format!("{name} (missing)"), |(_, label)| label.clone()),
        })
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let value = names.iter().position(|name| name.as_deref() == saved).unwrap_or(0);
    let missing = saved.is_some_and(|saved| !st.compact.settings_agents.iter().any(|(name, _)| name == saved));
    let agent_help = if missing {
        "Missing in every account: launches skip it, with a notice."
    } else {
        "The dialog also offers each project's own agents."
    };
    let model_ids = model_options(st);
    let models: Vec<&str> = if model_ids.is_empty() {
        vec!["Chosen at the first handover"]
    } else {
        model_ids.iter().map(String::as_str).collect()
    };
    let model_value = model_ids.iter().position(|m| *m == st.config.compact.model).unwrap_or(0);
    let percents = auto_percents(st);
    let percent_labels: Vec<String> = percents
        .iter()
        .map(|p| p.map_or_else(|| "Where Claude Code would compact".to_owned(), |p| format!("{p}%")))
        .collect();
    let percent_labels: Vec<&str> = percent_labels.iter().map(String::as_str).collect();
    let percent_value = percents.iter().position(|p| *p == st.config.compact.auto_percent).unwrap_or(0);
    let prefs = &st.config.compact;
    let focus_rows = FocusKind::ALL.into_iter().flat_map(|kind| {
        let (text, reset) = focus_ids(kind);
        let label = match kind {
            FocusKind::Default => "Default focus text",
            FocusKind::Coordination => "Focus for coordination",
            FocusKind::NonCode => "Focus for non-code work",
        };
        let edited = prefs.focus[kind.index()].is_some();
        let help = match (kind, edited) {
            (FocusKind::Default, false) => "Built in. When you give none.",
            (FocusKind::Default, true) => "Yours. When you give none.",
            (FocusKind::Coordination, false) => "Built in. Sessions run by agents.",
            (FocusKind::Coordination, true) => "Yours. Sessions run by agents.",
            (FocusKind::NonCode, false) => "Built in. Sessions with no code edits.",
            (FocusKind::NonCode, true) => "Yours. Sessions with no code edits.",
        };
        [Row::text(text, label, prefs.focus_text(kind)).help(help), Row::action(reset, label, "Reset")]
    });
    let mut rows = vec![
        Row::choice(SET_CONTINUE_IN, "Where it continues", &["This tab", "A new tab"], usize::from(prefs.new_tab))
            .help("This tab: /clear, then the handover as the first message."),
        Row::choice(SET_HANDOVER_MODEL, "Summary written by", &models, model_value)
            .help("The OpenCode model that writes the handover."),
    ];
    rows.extend(focus_rows);
    rows.extend([
        Row::choice(SET_HANDOVER_AGENT, "New chat starts as", &refs, value).help(if prefs.new_tab {
            agent_help
        } else {
            "New tabs only. This tab keeps its agent."
        }),
        Row::header("Instead of Claude Code's compaction"),
        Row::choice(
            SET_AUTO_HANDOVER,
            "Hand over automatically",
            &claude_switch_core::compact::AutoHandover::OPTIONS.map(|(_, _, label)| label),
            st.config.compact.auto.index(),
        )
        .help("Tabs started from now on, same account."),
        Row::choice(SET_AUTO_PERCENT, "When the context reaches", &percent_labels, percent_value)
            .help("After the turn ends. The old chat stays open."),
        Row::action(FORGET_HANDOVER_CONSENTS, "OpenCode provider consents", "Forget consents"),
    ]);
    rows
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
        Outcome::Changed { id: SET_CHAT_READING, value: Value::Choice(i) } => {
            if let Some(reading) = crate::view::Reading::ALL.get(i) {
                st.win_file.chat_reading = reading.key().to_owned();
                st.save_window();
            }
        }
        Outcome::Action(id) if theme_editor::action(st, id) => {}
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
        Outcome::Changed { id: SET_HANDOVER_AGENT, value: Value::Choice(i) } if !st.config_damaged => {
            let name = agent_names(st).get(i).cloned().flatten().unwrap_or_default();
            if let Err(e) = set_handover_agent(st, &name) {
                st.error("Default agent was not saved", e.to_string());
            }
            refresh_handover_page(st);
        }
        Outcome::Changed { id: SET_AUTO_HANDOVER, value: Value::Choice(i) } if !st.config_damaged => {
            if let Some(&(mode, _, _)) = claude_switch_core::compact::AutoHandover::OPTIONS.get(i) {
                update_handover_prefs(st, |prefs| prefs.auto = mode);
            }
        }
        Outcome::Changed { id: SET_CONTINUE_IN, value: Value::Choice(i) } if !st.config_damaged => {
            update_handover_prefs(st, |prefs| prefs.new_tab = i == 1);
        }
        Outcome::Changed { id, value: Value::Text(text) }
            if !st.config_damaged && focus_kind_of(id).is_some_and(|(_, reset)| !reset) =>
        {
            let Some((kind, _)) = focus_kind_of(id) else { return };
            // Saved as typed; the page is not rebuilt, so the field keeps its caret.
            save_handover_prefs(st, |prefs| prefs.set_focus(kind, &text));
        }
        Outcome::Action(id) if !st.config_damaged && focus_kind_of(id).is_some_and(|(_, reset)| reset) => {
            let Some((kind, _)) = focus_kind_of(id) else { return };
            update_handover_prefs(st, |prefs| prefs.set_focus(kind, ""));
        }
        Outcome::Changed { id: SET_AUTO_PERCENT, value: Value::Choice(i) } if !st.config_damaged => {
            if let Some(&percent) = auto_percents(st).get(i) {
                update_handover_prefs(st, |prefs| prefs.auto_percent = percent);
            }
        }
        Outcome::Changed { id: SET_HANDOVER_MODEL, value: Value::Choice(i) } if !st.config_damaged => {
            let Some(model) = model_options(st).into_iter().nth(i) else { return };
            let result = match &st.source {
                crate::source::Source::Real(roots) => claude_switch_core::config::update_compact(roots, |prefs| {
                    prefs.model = model;
                    Ok(())
                })
                .map(|prefs| st.config.compact = prefs),
                crate::source::Source::Demo => {
                    st.config.compact.model = model;
                    Ok(())
                }
            };
            if let Err(e) = result {
                st.error("Handover model was not saved", e.to_string());
            }
            refresh_handover_page(st);
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
                (SET_CLASSIC_VIEW, Value::Bool(b)) => st.config.classic_terminal_view = b,
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
    fn keyboard_enters_an_unselected_palette_row_from_either_end() {
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        for (id, name) in [("user-1", "Evening"), ("user-2", "Midnight")] {
            let mut t = theme_editor::from_base(&termtheme::palettes::DARK_THEMES[0]);
            t.name = name.into();
            st.config.appearance.themes.insert(id.into(), t);
        }
        for (page, second, expected) in [
            ("Appearance", tiles::row_id(true, 1), ["user-1", "user-2", "user-2"]),
            (TERMINAL, tiles::MORE_TERMINAL, ["campbell", "vintage", "custom"]),
        ] {
            st.dialog = Some(settings(&mut st, Some(page)));
            let selected = |st: &AppState| {
                if page == TERMINAL {
                    st.config.terminal_scheme.clone()
                } else {
                    st.config.appearance.dark_theme.clone()
                }
            };
            let before = selected(&st);
            for _ in 0..3 {
                navigate(&mut st, KeyName::Tab);
                if current(&st).form.focus_id() == Some(second) {
                    break;
                }
            }
            assert_eq!(current(&st).form.focus_id(), Some(second));
            navigate(&mut st, KeyName::Enter);
            navigate(&mut st, KeyName::Space);
            assert_eq!(selected(&st), before);
            navigate(&mut st, KeyName::Right);
            assert_eq!(selected(&st), expected[0]);
            navigate(&mut st, KeyName::Right);
            assert_eq!(selected(&st), expected[1]);
            // The first row is now unselected, even though its Form choice still has a value.
            assert!(super::super::key(
                &mut st,
                Instant::now(),
                &KeyName::Tab,
                Mods { shift: true, ..Default::default() },
                None
            ));
            navigate(&mut st, KeyName::Home);
            navigate(&mut st, KeyName::Tab);
            navigate(&mut st, KeyName::Left);
            assert_eq!(selected(&st), expected[2]);
        }
    }

    #[test]
    fn default_accent_keeps_clay_on_builtins_and_uses_the_named_themes_accent() {
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        let mut theme = theme_editor::from_base(&termtheme::palettes::DARK_THEMES[0]);
        let saved = [90, 180, 220];
        theme.accent.0 = saved;
        st.config.appearance.themes.insert("user-1".into(), theme);
        let change = |st: &mut AppState, id, i| settings_outcome(st, Outcome::Changed { id, value: Value::Choice(i) });
        let default = ACCENT_SOURCES.iter().position(|(_, label)| *label == "Default").unwrap();
        change(&mut st, SET_ACCENT, default);
        assert_eq!(st.config.appearance.accent_source, "theme");
        assert_eq!(st.pal().accent, mdshared_ui::gfx::rgb(0xd97757));
        change(&mut st, tiles::row_id(true, 1), 0);
        assert_eq!(st.pal().accent, saved);
    }

    #[test]
    fn the_two_archives_have_distinct_names() {
        let st = AppState::new(Source::Demo, None, WindowFile::default());
        let rows = sessions(&st);
        let desktop = rows.iter().find(|r| r.id() == Some(SET_ARCHIVED)).unwrap();
        assert!(matches!(desktop, Row::Toggle { label, help, .. }
            if label == "Show sessions archived in the desktop app"
                && help == "Claude Desktop's own archive. Switch's Archive is separate."));
        assert_eq!(crate::list::model::Filter::Archived.label(), "Archived here");
    }

    #[test]
    fn chat_width_setting_is_independent_of_markdown_and_round_trips() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.session_view.markdown.reading = crate::view::Reading::Narrow;
        settings_outcome(&mut st, Outcome::Changed { id: SET_CHAT_READING, value: Value::Choice(2) });
        assert_eq!(st.win_file.chat_reading, "wide");
        assert_eq!(st.win_file.markdown_reading, "narrow");
        let restored = crate::winfile::parse(&crate::winfile::to_text(&st.win_file));
        assert_eq!(restored.chat_reading, "wide");
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
                theme_editor::EDIT_DARK,
                theme_editor::EDIT_LIGHT,
                theme_editor::IMPORT,
            ],
            &[
                SET_TERM_COLOURS,
                tiles::MORE_TERMINAL,
                SET_TERM_FONT,
                terminal_appearance::CURSOR,
                SET_CURSOR_BLINK,
                terminal_appearance::OPACITY,
                SET_RESTORE_TABS,
                SET_TERM_BACKGROUND,
                SET_TERM_FOREGROUND,
                SET_TERM_ACCENT,
            ],
            &[
                SET_EMBEDDED,
                SET_CLASSIC_VIEW,
                SET_CHAT_READING,
                SET_ARCHIVED,
                SET_WORKERS_SHOWN,
                SET_HIDDEN,
                SET_ATTENTION,
                SET_SUGGEST_TOPICS,
            ],
            &[SET_USAGE, SET_USAGE_WARNINGS],
            &[
                SET_CONTINUE_IN,
                SET_HANDOVER_MODEL,
                SET_DEFAULT_FOCUS,
                RESET_DEFAULT_FOCUS,
                focus_ids(FocusKind::Coordination).0,
                focus_ids(FocusKind::Coordination).1,
                focus_ids(FocusKind::NonCode).0,
                focus_ids(FocusKind::NonCode).1,
                SET_HANDOVER_AGENT,
                SET_AUTO_HANDOVER,
                SET_AUTO_PERCENT,
                FORGET_HANDOVER_CONSENTS,
            ],
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
    fn handover_agent_is_a_choice_of_real_agents_with_a_missing_saved_name_marked() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.config.compact.set_default_agent("deepseek-flash").unwrap();
        st.dialog = Some(settings(&mut st, Some(HANDOVERS)));
        let labels = |st: &AppState| match current(st).form.rows().iter().find(|r| r.id() == Some(SET_HANDOVER_AGENT)) {
            Some(Row::Choice { label, options, value, .. }) => (label.clone(), options.clone(), *value),
            _ => panic!("agent row must be a choice"),
        };
        let (label, options, value) = labels(&st);
        assert_eq!(label, "New chat starts as");
        assert_eq!(options[0], NO_AGENT);
        assert!(options.iter().any(|o| o.starts_with("main-reviewer (")), "{options:?}");
        assert_eq!(options[value], "deepseek-flash (missing)");
        let main = options.iter().position(|o| o.starts_with("main-reviewer")).unwrap();
        settings_outcome(&mut st, Outcome::Changed { id: SET_HANDOVER_AGENT, value: Value::Choice(main) });
        assert_eq!(st.config.compact.default_agent.as_deref(), Some("main-reviewer"));
        let (_, options, value) = labels(&st);
        assert!(!options.iter().any(|o| o.contains("missing")), "the page is rebuilt");
        assert!(options[value].starts_with("main-reviewer"));
        settings_outcome(&mut st, Outcome::Changed { id: SET_HANDOVER_AGENT, value: Value::Choice(0) });
        assert!(st.config.compact.default_agent.is_none());
        assert_eq!(current(&st).category(), HANDOVERS);
    }

    #[test]
    fn where_it_continues_and_the_default_focus_are_saved_and_reset() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.dialog = Some(settings(&mut st, Some(HANDOVERS)));
        let ids: Vec<_> = current(&st).form.rows().iter().filter_map(Row::id).collect();
        assert_eq!(ids[..2], [SET_CONTINUE_IN, SET_HANDOVER_MODEL], "where it continues comes first");
        assert_eq!(current(&st).form.value(SET_CONTINUE_IN), Some(Value::Choice(0)), "this tab by default");
        settings_outcome(&mut st, Outcome::Changed { id: SET_CONTINUE_IN, value: Value::Choice(1) });
        assert!(st.config.compact.new_tab);
        let Some(Value::Text(text)) = current(&st).form.value(SET_DEFAULT_FOCUS) else { panic!("a text row") };
        assert_eq!(text, FocusKind::Default.built_in());
        settings_outcome(
            &mut st,
            Outcome::Changed { id: SET_DEFAULT_FOCUS, value: Value::Text("Keep the plan.".into()) },
        );
        assert_eq!(st.config.compact.focus_text(FocusKind::Default), "Keep the plan.");
        settings_outcome(&mut st, Outcome::Action(RESET_DEFAULT_FOCUS));
        assert_eq!(st.config.compact.focus_text(FocusKind::Default), FocusKind::Default.built_in());
        let Some(Value::Text(text)) = current(&st).form.value(SET_DEFAULT_FOCUS) else { panic!("a text row") };
        assert_eq!(text, FocusKind::Default.built_in(), "the field shows the reset text");
        let (coordination, reset) = focus_ids(FocusKind::Coordination);
        settings_outcome(&mut st, Outcome::Changed { id: coordination, value: Value::Text("Workers first.".into()) });
        assert_eq!(st.config.compact.focus_text(FocusKind::Coordination), "Workers first.");
        assert_eq!(st.config.compact.focus_text(FocusKind::Default), FocusKind::Default.built_in(), "only that kind");
        settings_outcome(&mut st, Outcome::Action(reset));
        assert_eq!(st.config.compact.focus_text(FocusKind::Coordination), FocusKind::Coordination.built_in());
        let Some(Value::Text(text)) = current(&st).form.value(focus_ids(FocusKind::NonCode).0) else { panic!() };
        assert!(text.starts_with("No code here."));
    }

    #[test]
    fn summary_model_row_names_the_saved_model_and_offers_listed_ones() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.config.compact.model = "deepseek/deepseek-flash".into();
        st.dialog = Some(settings(&mut st, Some(HANDOVERS)));
        let row = current(&st).form.rows().iter().find(|r| r.id() == Some(SET_HANDOVER_MODEL)).cloned();
        let Some(Row::Choice { label, options, .. }) = row else { panic!("model row must be a choice") };
        assert_eq!(label, "Summary written by");
        assert_eq!(options, ["deepseek/deepseek-flash"]);
        settings_outcome(&mut st, Outcome::Changed { id: SET_HANDOVER_MODEL, value: Value::Choice(3) });
        assert_eq!(st.config.compact.model, "deepseek/deepseek-flash", "an index outside the list changes nothing");
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
        assert_eq!(current(&st).form.focus_id(), Some(SET_CLASSIC_VIEW));
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).form.focus_id(), Some(SET_CHAT_READING));
        navigate(&mut st, KeyName::Down);
        assert_eq!(current(&st).form.focus_id(), Some(SET_ARCHIVED));
        let archived = st.config.show_archived;
        navigate(&mut st, KeyName::Space);
        assert_ne!(st.config.show_archived, archived);
        navigate(&mut st, KeyName::Up);
        navigate(&mut st, KeyName::Up);
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
