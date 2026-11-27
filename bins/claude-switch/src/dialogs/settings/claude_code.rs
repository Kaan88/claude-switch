//! Settings › Claude Code: Claude Code's own settings, saved in the master account and brought to
//! every account that shares settings (`claude_switch_core::settings_sync`). The page reads them
//! when it opens and after each change; nothing here is kept in the app's own config.
use super::{AppState, Dialog, Row};
use claude_switch_core::settings_sync::Setting;
use mdshared_ui::settings::Value;

pub(crate) const CLAUDE_CODE: &str = "Claude Code";
pub(super) const SET_MODEL: u32 = 140;
pub(super) const SET_EFFORT: u32 = 141;
pub(super) const SET_MODE: u32 = 142;
pub(super) const SET_THEME: u32 = 143;
pub(super) const SET_CHROME: u32 = 144;
pub(super) const SET_COMPACT: u32 = 145;
pub(super) const SET_VERBOSE: u32 = 146;

const CHOICES: [(u32, Setting); 4] = [
    (SET_MODEL, Setting::Model),
    (SET_EFFORT, Setting::Effort),
    (SET_MODE, Setting::PermissionMode),
    (SET_THEME, Setting::Theme),
];
/// The switches, each with Claude Code's own default.
const SWITCHES: [(u32, Setting, bool); 3] = [
    (SET_CHROME, Setting::Chrome, false),
    (SET_COMPACT, Setting::AutoCompact, true),
    (SET_VERBOSE, Setting::Verbose, false),
];

pub(super) fn owns(id: u32) -> bool {
    (SET_MODEL..=SET_VERBOSE).contains(&id)
}

/// A choice's options, labels with the value each saves (`None`: the key is removed, so Claude
/// Code's default applies), and the index of the saved one. A saved value the list lacks is
/// added, so it is shown as it is and kept until another is chosen.
fn options(st: &AppState, setting: Setting) -> (Vec<(String, Option<serde_json::Value>)>, usize) {
    let fixed: &[(&str, Option<&str>)] = match setting {
        Setting::Model => &[
            ("Claude Code's default", None),
            ("Opus", Some("opus")),
            ("Sonnet", Some("sonnet")),
            ("Haiku", Some("haiku")),
        ],
        Setting::Effort => &[
            ("The model's default", None),
            ("Low", Some("low")),
            ("Medium", Some("medium")),
            ("High", Some("high")),
            ("Extra high", Some("xhigh")),
            ("Max", Some("max")),
        ],
        Setting::PermissionMode => &[
            ("Claude Code's default", None),
            ("Manual: ask before changes", Some("default")),
            ("Accept edits", Some("acceptEdits")),
            ("Plan", Some("plan")),
            ("Auto", Some("auto")),
        ],
        _ => &[
            ("Claude Code's default", None),
            ("Dark", Some("dark")),
            ("Light", Some("light")),
            ("Follow the system", Some("auto")),
        ],
    };
    let mut list: Vec<(String, Option<serde_json::Value>)> =
        fixed.iter().map(|(label, v)| ((*label).to_owned(), v.map(|v| serde_json::Value::String(v.into())))).collect();
    let saved = st.source.claude_setting(&st.config, setting);
    let index = list.iter().position(|(_, v)| *v == saved).unwrap_or_else(|| {
        let label =
            saved.as_ref().map_or_else(String::new, |v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
        list.push((label, saved));
        list.len() - 1
    });
    (list, index)
}

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let master = claude_switch_core::settings_sync::master_account(&st.config, &st.config.settings_share)
        .map_or("the first account", |a| a.name.as_str());
    let labels = ["Model", "Effort", "Permission mode", "Theme"];
    let mut rows = vec![Row::header("Shared by the accounts")];
    for ((id, setting), label) in CHOICES.into_iter().zip(labels) {
        let (list, index) = options(st, setting);
        let names: Vec<&str> = list.iter().map(|(name, _)| name.as_str()).collect();
        let row = Row::choice(id, label, &names, index);
        rows.push(if id == SET_MODEL {
            row.help(format!("Saved in {master}, shared with the accounts."))
        } else {
            row
        });
    }
    let switches = ["Chrome by default", "Auto-compact", "Verbose output"];
    for ((id, setting, default), label) in SWITCHES.into_iter().zip(switches) {
        let on = st.source.claude_setting(&st.config, setting).and_then(|v| v.as_bool()).unwrap_or(default);
        let row = Row::toggle(id, label, on);
        rows.push(if id == SET_CHROME { row.help("These three apply to new sessions.") } else { row });
    }
    rows
}

/// A change on the page: saved in the master and shared, then the page is read again.
pub(super) fn apply(st: &mut AppState, id: u32, value: Value) {
    let change = match value {
        Value::Choice(i) => CHOICES.iter().find(|(c, _)| *c == id).and_then(|&(_, setting)| {
            let (list, _) = options(st, setting);
            list.into_iter().nth(i).map(|(_, v)| (setting, v))
        }),
        Value::Bool(on) => SWITCHES
            .iter()
            .find(|(s, ..)| *s == id)
            .map(|&(_, setting, _)| (setting, Some(serde_json::Value::Bool(on)))),
        _ => None,
    };
    let Some((setting, value)) = change else { return };
    if let Err(e) = st.source.set_claude_setting(&st.config, setting, value) {
        st.error("Claude Code's setting was not saved", e.to_string());
    }
    let rows = rows(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == CLAUDE_CODE
    {
        settings.form.set_rows(rows);
    }
}
