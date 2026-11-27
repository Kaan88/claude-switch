//! This app's own settings file, and the two keys it writes into the bridge's worker list.
//!
//! The settings file looks like this; every key is optional and a wrong one gives its default:
//!
//! ```json
//! {
//!   "accounts": [{"slug": "main", "name": "Main", "color": [217, 119, 87]},
//!                {"slug": "work", "name": "Work", "color": [74, 144, 226]}],
//!   "current": "main",
//!   "workers": "same" | "other" | {"account": "work"},
//!   "embeddedTerminal": true,
//!   "showArchived": false,
//!   "showWorkers": false,
//!   "terminalScheme": "auto",
//!   "terminalFontSize": 13,
//!   "windowTheme": "system",
//!   "showUsage": true,
//!   "appearance": {"backdrop":"off", "accentSource":"theme", "reduceMotion":false,
//!                  "cornerRadius":8, "softShadows":true, "cursorBlink":false},
//!   "chat": {"chips": ["model", "effort"], "commands": ["/review", "", ""], "answerMenus": true}
//! }
//! ```
//!
//! Folders are not stored: they follow from the slug and [`Roots`].

mod chat;
mod coordination;
mod notifications;
mod terminal;
pub mod theme;
pub use chat::{ChatPrefs, OWN_COMMANDS, QUICK_CHIPS, command_line};
pub use coordination::{sync_workers, usage_enabled, usage_permit, usage_permit_until};
pub use notifications::{Attention, Notifications, WarningWindows, claim_warning, load_warning_windows};
pub use terminal::*;

use crate::accounts::COLORS;
use crate::util::{io_refusal, lock_beside, str_field, write_json};
use crate::{Account, Config, Refusal, Roots, WorkersUse};
use serde_json::{Value, json};

pub const TERMINAL_FONT_MIN: u8 = 9;
pub const TERMINAL_FONT_MAX: u8 = 24;
pub const TERMINAL_FONT_DEFAULT: u8 = 13;

/// Persisted appearance settings, independent of the GUI toolkit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearancePrefs {
    pub backdrop: String,
    pub accent_source: String,
    pub accent_colour: Option<String>,
    pub corner_radius: u8,
    pub soft_shadows: bool,
    pub reduce_motion: bool,
    pub cursor_blink: bool,
    pub cursor_shape: TerminalCursor,
    pub terminal_opacity: u8,
    pub terminal_background: Option<String>,
    pub terminal_foreground: Option<String>,
    pub terminal_accent: Option<String>,
    /// The palette used when the window is dark, and when it is light: ids the app resolves,
    /// keeping an unknown one (from a newer version) as it was.
    pub dark_theme: String,
    pub light_theme: String,
    pub themes: std::collections::BTreeMap<String, theme::NamedTheme>,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            backdrop: "off".into(),
            accent_source: "theme".into(),
            accent_colour: None,
            corner_radius: 8,
            soft_shadows: true,
            reduce_motion: false,
            cursor_blink: false,
            cursor_shape: TerminalCursor::Block,
            terminal_opacity: TERMINAL_OPACITY_MAX as u8,
            terminal_background: None,
            terminal_foreground: None,
            terminal_accent: None,
            dark_theme: "dark".into(),
            light_theme: "light".into(),
            themes: Default::default(),
        }
    }
}

impl AppearancePrefs {
    fn read(v: &Value) -> Self {
        let d = Self::default();
        let word = |key, choices: &[&str], default: String| {
            str_field(v, key).filter(|s| choices.contains(s)).map_or(default, str::to_owned)
        };
        let flag = |key, default| v.get(key).and_then(Value::as_bool).unwrap_or(default);
        let colour = |key| {
            str_field(v, key)
                .and_then(|s| s.strip_prefix('#'))
                .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(|s| format!("#{}", s.to_ascii_lowercase()))
        };
        Self {
            backdrop: word("backdrop", &["off", "mica", "acrylic"], d.backdrop),
            accent_source: word("accentSource", &["theme", "system", "custom"], d.accent_source),
            accent_colour: colour("accentColour"),
            corner_radius: v.get("cornerRadius").and_then(Value::as_u64).map_or(d.corner_radius, |n| n.min(16) as u8),
            soft_shadows: flag("softShadows", d.soft_shadows),
            reduce_motion: flag("reduceMotion", d.reduce_motion),
            cursor_blink: flag("cursorBlink", d.cursor_blink),
            cursor_shape: str_field(v, "cursorShape").and_then(TerminalCursor::parse).unwrap_or_default(),
            terminal_opacity: v
                .get("terminalOpacity")
                .and_then(Value::as_i64)
                .map_or(d.terminal_opacity, terminal_opacity),
            terminal_background: colour("terminalBackground"),
            terminal_foreground: colour("terminalForeground"),
            terminal_accent: colour("terminalAccent"),
            dark_theme: theme_id(v, "darkTheme").unwrap_or(d.dark_theme),
            light_theme: theme_id(v, "lightTheme").unwrap_or(d.light_theme),
            themes: v["themes"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(id, value)| {
                    let t: theme::NamedTheme = serde_json::from_value(value.clone()).ok()?;
                    (id.starts_with("user-") && valid_theme_id(id) && t.validate().is_ok()).then(|| (id.clone(), t))
                })
                .collect(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "backdrop": self.backdrop,
            "accentSource": self.accent_source,
            "accentColour": self.accent_colour,
            "cornerRadius": self.corner_radius,
            "softShadows": self.soft_shadows,
            "reduceMotion": self.reduce_motion,
            "cursorBlink": self.cursor_blink,
            "cursorShape": self.cursor_shape.key(),
            "terminalOpacity": self.terminal_opacity,
            "terminalBackground": self.terminal_background,
            "terminalForeground": self.terminal_foreground,
            "terminalAccent": self.terminal_accent,
            "darkTheme": self.dark_theme,
            "lightTheme": self.light_theme,
            "themes": self.themes,
        })
    }
}

/// A palette id: a short word of lower-case letters, digits and hyphens.
fn theme_id(v: &Value, key: &str) -> Option<String> {
    str_field(v, key).filter(|s| valid_theme_id(s)).map(str::to_owned)
}

fn valid_theme_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether a slug read from the file can name a folder.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
}

fn read_color(v: Option<&Value>, fallback: [u8; 3]) -> [u8; 3] {
    let Some(items) = v.and_then(Value::as_array).filter(|a| a.len() == 3) else { return fallback };
    let mut out = [0u8; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        match item.as_u64().and_then(|n| u8::try_from(n).ok()) {
            Some(n) => *slot = n,
            None => return fallback,
        }
    }
    out
}

/// Reads [`Roots::config_file`]. A missing file, or a missing key, gives the defaults:
/// one account (`main`, named "Main", primary, the standard folders), `current` = `main`, workers
/// `Same`, embedded terminal on, archived and worker sessions hidden. The first account's folders always come from
/// `roots`, whatever the file says.
pub fn load(roots: &Roots) -> Config {
    load_checked(roots).0
}

fn read_settings(path: &std::path::Path) -> Result<Value, Refusal> {
    crate::util::check_parent(path, false)?;
    crate::util::regular_target(path)?;
    match crate::read_bounded(path, crate::APP_FILE_LIMIT) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| io_refusal("read settings", path, &e))?;
            if !value.is_object()
                || value.get("showUsage").is_some_and(|v| !v.is_boolean())
                || value.get("accounts").is_some_and(|v| !valid_accounts(v))
            {
                return Err(io_refusal("read settings", path, &"invalid settings object"));
            }
            Ok(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(io_refusal("read settings", path, &e)),
    }
}

fn valid_accounts(value: &Value) -> bool {
    let mut slugs = std::collections::HashSet::new();
    value.as_array().is_some_and(|entries| {
        entries.iter().all(|entry| {
            str_field(entry, "slug").is_some_and(|slug| valid_slug(slug) && slugs.insert(slug))
                && entry.get("name").is_none_or(|s| s.as_str().is_some_and(|s| !s.trim().is_empty()))
                && entry.get("color").is_none_or(|c| {
                    c.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(|v| v.as_u64().is_some_and(|n| n <= 255)))
                })
        })
    })
}

/// The second result is true when an existing config could not be trusted.
pub fn load_checked(roots: &Roots) -> (Config, bool) {
    let loaded = read_settings(&roots.config_file());
    let damaged = loaded.is_err();
    (parse(roots, loaded.unwrap_or(Value::Null), damaged), damaged)
}

fn parse(roots: &Roots, file: Value, damaged: bool) -> Config {
    let entries: &[Value] = file.get("accounts").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let main_entry = entries.iter().find(|e| str_field(e, "slug") == Some("main"));
    let mut accounts = vec![Account {
        slug: "main".into(),
        name: main_entry.and_then(|e| str_field(e, "name")).unwrap_or("Main").to_string(),
        color: read_color(main_entry.and_then(|e| e.get("color")), COLORS[0]),
        primary: true,
        cli_dir: roots.primary_cli_dir(),
        desktop_dir: roots.primary_desktop_dir(),
    }];
    for entry in entries {
        let Some(slug) = str_field(entry, "slug").filter(|s| valid_slug(s)) else { continue };
        if accounts.iter().any(|a| a.slug == slug) {
            continue;
        }
        let base = roots.accounts_dir().join(slug);
        let fallback = COLORS[accounts.len() % COLORS.len()];
        accounts.push(Account {
            slug: slug.to_string(),
            name: str_field(entry, "name").unwrap_or(slug).to_string(),
            color: read_color(entry.get("color"), fallback),
            primary: false,
            cli_dir: base.join("cli"),
            desktop_dir: base.join("desktop"),
        });
    }
    let known = |s: &str| accounts.iter().any(|a| a.slug == s);
    let current = str_field(&file, "current").filter(|s| known(s)).unwrap_or("main").to_string();
    let workers = match file.get("workers") {
        Some(Value::String(s)) if s == "other" => WorkersUse::Other,
        Some(Value::Object(o)) => match o.get("account").and_then(Value::as_str) {
            Some(s) if known(s) => WorkersUse::Account(s.to_string()),
            _ => WorkersUse::Same,
        },
        _ => WorkersUse::Same,
    };
    let flag = |k: &str, default: bool| file.get(k).and_then(Value::as_bool).unwrap_or(default);
    let embedded_terminal = flag("embeddedTerminal", true);
    let classic_terminal_view = flag("classicTerminalView", true);
    let show_archived = flag("showArchived", false);
    let show_workers = flag("showWorkers", false);
    let terminal_scheme = str_field(&file, "terminalScheme").filter(|s| !s.is_empty()).unwrap_or("auto").to_string();
    let terminal_font_size = file.get("terminalFontSize").map_or(TERMINAL_FONT_DEFAULT, |size| {
        if let Some(n) = size.as_i64() {
            n.clamp(i64::from(TERMINAL_FONT_MIN), i64::from(TERMINAL_FONT_MAX)) as u8
        } else if size.is_u64() {
            TERMINAL_FONT_MAX
        } else {
            TERMINAL_FONT_DEFAULT
        }
    });
    let window_theme =
        str_field(&file, "windowTheme").filter(|s| matches!(*s, "light" | "dark")).unwrap_or("system").to_string();
    Config {
        accounts,
        current,
        workers,
        embedded_terminal,
        classic_terminal_view,
        show_archived,
        show_workers,
        show_usage: !damaged && flag("showUsage", true),
        notifications: Notifications::read(&file["notifications"]),
        show_hidden: flag("showHidden", false),
        sessions_mode: crate::grouping::Mode::ALL
            .into_iter()
            .find(|m| file["sessionsMode"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(m.label())))
            .unwrap_or(if flag("foldersView", false) { crate::grouping::Mode::Projects } else { Default::default() }),
        group_by_folder: flag("groupByFolder", flag("foldersView", false)),
        show_sittings: flag("showSittings", false),
        suggest_topics: flag("suggestTopics", false),
        settings_share: crate::settings_sync::Share::read(&file["settingsShare"]),
        terminal_scheme,
        terminal_font_size,
        window_theme,
        appearance: AppearancePrefs::read(&file["appearance"]),
        chat: ChatPrefs::read(&file["chat"]),
        compact: crate::compact::Preferences::read(&file["compact"]),
        meaning: crate::meaning::Prefs::read(&file["meaning"]),
    }
}

/// Writes the settings (temporary file beside it, then rename), creating the folder.
pub fn save(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let path = roots.config_file();
    let _lock = lock_beside(&path)?;
    crate::util::regular_target(&path)?;
    let current = read_settings(&path)?;
    let mut config = config.clone();
    // Ordinary saves from another window must never restore a forgotten provider consent.
    config.compact = crate::compact::Preferences::read(&current["compact"]);
    config.show_usage = current["showUsage"].as_bool().unwrap_or(true);
    save_value(roots, &config)
}

/// Apply only this window's edits onto the file as it is now, field by field (inside groups such
/// as `appearance` too). Refuse only an edit another window made to the same field.
pub fn save_changes(roots: &Roots, baseline: &Config, edited: &Config) -> Result<Config, Refusal> {
    let path = roots.config_file();
    let _lock = lock_beside(&path)?;
    let disk = read_settings(&path)?;
    let current = config_value(&parse(roots, disk.clone(), false));
    let before = config_value(baseline);
    let after = config_value(edited);
    let mut merged = disk.as_object().cloned().unwrap_or_default();
    let mut conflicts = Vec::new();
    for (key, value) in after.as_object().expect("settings object") {
        if matches!(key.as_str(), "showUsage" | "compact") || before[key] == *value {
            continue;
        }
        // Groups whose raw object may hold keys from a newer version: merge field by field.
        let raw_group = matches!(key.as_str(), "appearance" | "chat");
        let depth = if raw_group { 2 } else { 1 };
        let m = crate::merge::three_way(before.get(key), Some(value), current.get(key), depth);
        conflicts.extend(m.conflicts.into_iter().map(|p| [key.clone()].into_iter().chain(p).collect::<Vec<_>>()));
        if let Some(v) = m.value {
            if raw_group {
                // The typed view omits unrecognised palette versions and newer settings.
                // Apply only the validated delta to the raw object, keeping those bytes' values.
                let raw = merged.entry(key.clone()).or_insert(Value::Null);
                apply_visual_delta(raw, &current[key], &v);
            } else {
                merged.insert(key.clone(), v);
            }
        }
    }
    if let Some(first) = conflicts.first() {
        return Err(Refusal::Invalid(format!(
            "Another Claude Switch window changed the setting “{}” at the same time. That window's value is kept and this window now shows the current settings; change it again if you still want to.",
            first.join(" › ")
        )));
    }
    let merged = Value::Object(merged);
    write_json(&path, &merged)?;
    Ok(parse(roots, merged, false))
}

fn apply_visual_delta(raw: &mut Value, old: &Value, new: &Value) {
    if old == new {
        return;
    }
    if let (Some(before), Some(after)) = (old.as_object(), new.as_object()) {
        if !raw.is_object() {
            *raw = json!({})
        }
        let fields = raw.as_object_mut().unwrap();
        for k in before.keys().filter(|k| !after.contains_key(*k)) {
            fields.remove(k);
        }
        for (k, v) in after {
            if before.get(k) != Some(v) {
                apply_visual_delta(fields.entry(k).or_insert(Value::Null), &old[k], v);
            }
        }
    } else {
        *raw = new.clone();
    }
}

pub fn set_usage(roots: &Roots, enabled: bool) -> Result<(), Refusal> {
    let path = roots.config_file();
    let _lock = lock_beside(&path)?;
    let mut current = read_settings(&path)?;
    current["showUsage"] = json!(enabled);
    write_json(&path, &current)
}

/// Updates handover preferences under the settings lock, without changing any other setting.
pub fn update_compact(
    roots: &Roots,
    update: impl FnOnce(&mut crate::compact::Preferences) -> Result<(), Refusal>,
) -> Result<crate::compact::Preferences, Refusal> {
    let path = roots.config_file();
    let _lock = lock_beside(&path)?;
    crate::util::regular_target(&path)?;
    let mut current = read_settings(&path)?;
    let mut compact = crate::compact::Preferences::read(&current["compact"]);
    update(&mut compact)?;
    current["compact"] = compact.json();
    write_json(&path, &current)?;
    Ok(compact)
}

/// Explicit recovery only. Keep the damaged bytes in a new, exclusively created sibling.
pub fn reset(roots: &Roots) -> Result<Config, Refusal> {
    let path = roots.config_file();
    let _lock = lock_beside(&path)?;
    crate::util::regular_target(&path)?;
    if read_settings(&path).is_ok() {
        return Err(Refusal::Invalid("Settings changed on disk. Restart to use the current settings.".into()));
    }
    let bytes = crate::read_bounded(&path, crate::APP_FILE_LIMIT)
        .map_err(|e| io_refusal("back up settings; move oversized files aside manually", &path, &e))?;
    let serial = crate::usage::now_ms();
    let backup = path.with_file_name(format!("config.broken.{}.{serial}.json", std::process::id()));
    crate::util::write_new_atomic(&backup, &bytes)?;
    let config = parse(roots, Value::Null, true); // Recover preferences without opting back into usage.
    if crate::read_bounded(&path, crate::APP_FILE_LIMIT).ok().as_deref() != Some(bytes.as_slice()) {
        return Err(Refusal::Invalid("Settings changed while being backed up.".into()));
    }
    save_value(roots, &config)?;
    Ok(config)
}

fn save_value(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    write_json(&roots.config_file(), &config_value(config))
}

fn config_value(config: &Config) -> Value {
    let accounts: Vec<Value> =
        config.accounts.iter().map(|a| json!({"slug": a.slug, "name": a.name, "color": a.color})).collect();
    let workers = match &config.workers {
        WorkersUse::Same => json!("same"),
        WorkersUse::Other => json!("other"),
        WorkersUse::Account(s) => json!({"account": s}),
    };
    json!({
        "accounts": accounts,
        "current": config.current,
        "workers": workers,
        "embeddedTerminal": config.embedded_terminal,
        "classicTerminalView": config.classic_terminal_view,
        "showArchived": config.show_archived,
        "showWorkers": config.show_workers,
        "showUsage": config.show_usage,
        "notifications": config.notifications.json(),
        "showHidden": config.show_hidden,
        "sessionsMode": config.sessions_mode.label().to_ascii_lowercase(),
        "groupByFolder": config.group_by_folder,
        "showSittings": config.show_sittings,
        "suggestTopics": config.suggest_topics,
        "settingsShare": config.settings_share.json(),
        "terminalScheme": config.terminal_scheme,
        "terminalFontSize": config.terminal_font_size,
        "windowTheme": config.window_theme,
        "appearance": config.appearance.json(),
        "chat": config.chat.json(),
        "compact": config.compact.json(),
        "meaning": config.meaning.json(),
    })
}

#[cfg(test)]
mod tests {
    use super::coordination::write_workers;
    use super::*;
    use crate::util::read_json;
    use crate::util::testing::{TempDir, config_with, put};

    #[test]
    fn handover_agent_loads_valid_names_only_and_is_saved_without_republishing_other_preferences() {
        let t = TempDir::new("handover-agent");
        let roots = t.roots();
        let stale = load(&roots);
        assert_eq!(stale.compact.default_agent, None);
        update_compact(&roots, |p| {
            p.model = "fixture/model".into();
            p.consents.insert("fixture".into());
            p.set_default_agent(" plugin:nested:review_2 ")
        })
        .unwrap();
        let after = load(&roots);
        assert_eq!(after.compact.agent_for(None), Some("plugin:nested:review_2"));
        assert!(update_compact(&roots, |p| p.set_default_agent("plugin:--flag")).is_err());
        assert_eq!(load(&roots).compact, after.compact);
        let mut edited = stale.clone();
        edited.show_hidden = !stale.show_hidden;
        assert_eq!(save_changes(&roots, &stale, &edited).unwrap().compact, after.compact);
        update_compact(&roots, |p| p.set_default_agent(" ")).unwrap();
        let after = load(&roots);
        assert_eq!(after.compact.default_agent, None);
        assert_eq!(after.compact.model, "fixture/model");
        assert!(after.compact.consents.contains("fixture"));
        let path = roots.config_file();
        let mut on_disk = read_json(&path).unwrap();
        on_disk["compact"]["defaultAgent"] = json!("invalid agent");
        write_json(&path, &on_disk).unwrap();
        assert_eq!(load(&roots).compact, after.compact);
    }
    #[test]
    fn damaged_settings_disable_usage_and_reset_preserves_exact_bytes() {
        let t = TempDir::new("settings-recovery");
        let roots = t.roots();
        let path = roots.config_file();
        assert!(load_checked(&roots).0.show_usage);
        for damaged in ["not JSON", "null", r#"{"accounts":{}}"#, r#"{"showUsage":"false"}"#] {
            put(&path, damaged);
            let (cfg, broken) = load_checked(&roots);
            assert!(broken && !cfg.show_usage);
            assert!(save(&roots, &cfg).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), damaged);
        }
        let old = std::fs::read(&path).unwrap();
        let cfg = reset(&roots).unwrap();
        assert!(!cfg.show_usage && !load_checked(&roots).1);
        let backups = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("config.broken."))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(backups[0].path()).unwrap(), old);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let t = TempDir::new("cfg-default");
        let roots = t.roots();
        let c = load(&roots);
        assert_eq!(c.accounts.len(), 1);
        let main = &c.accounts[0];
        assert_eq!((main.slug.as_str(), main.name.as_str(), main.primary), ("main", "Main", true));
        assert_eq!(main.cli_dir, roots.primary_cli_dir());
        assert_eq!(main.desktop_dir, roots.primary_desktop_dir());
        assert_eq!(c.current, "main");
        assert_eq!(c.workers, WorkersUse::Same);
        assert!(c.embedded_terminal && c.classic_terminal_view);
        assert!(!c.show_archived);
        assert!(!c.show_workers);
        assert!(!c.show_hidden);
        assert_eq!(c.sessions_mode, crate::grouping::Mode::Recent);
        assert!(!c.group_by_folder && !c.show_sittings && !c.suggest_topics);
        assert_eq!((c.terminal_scheme.as_str(), c.terminal_font_size, c.window_theme.as_str()), ("auto", 13, "system"));
    }

    #[test]
    fn folders_view_migrates_without_affecting_an_explicit_new_mode() {
        use crate::grouping::Mode;
        let t = TempDir::new("cfg-list-modes");
        let roots = t.roots();
        let file = roots.config_file();
        put(&file, r#"{"foldersView":true}"#);
        let legacy = load(&roots);
        assert_eq!(legacy.sessions_mode, Mode::Projects);
        assert!(legacy.group_by_folder);
        save(&roots, &legacy).unwrap();
        assert_eq!(load(&roots), legacy);
        put(&file, r#"{"foldersView":true,"sessionsMode":"Topics","groupByFolder":false}"#);
        let modern = load(&roots);
        assert_eq!(modern.sessions_mode, Mode::Topics);
        assert!(!modern.group_by_folder);
    }

    #[test]
    fn settings_come_back_as_they_were_saved() {
        let t = TempDir::new("cfg-roundtrip");
        let roots = t.roots();
        let mut c = config_with(&roots, &[("work", "Work")]);
        c.accounts[0].name = "Home".into();
        c.current = "work".into();
        c.workers = WorkersUse::Account("work".into());
        c.embedded_terminal = false;
        c.classic_terminal_view = false;
        c.show_archived = true;
        c.show_workers = true;
        c.show_hidden = true;
        c.sessions_mode = crate::grouping::Mode::Threads;
        c.group_by_folder = true;
        c.show_sittings = true;
        c.suggest_topics = true;
        c.terminal_scheme = "campbell".into();
        c.terminal_font_size = 21;
        c.window_theme = "light".into();
        save(&roots, &c).unwrap();
        assert_eq!(load(&roots), c);
        let text = std::fs::read_to_string(roots.config_file()).unwrap();
        assert!(text.contains("\"showWorkers\": true") || text.contains("\"showWorkers\":true"), "{text}");
        c.workers = WorkersUse::Other;
        save(&roots, &c).unwrap();
        assert_eq!(load(&roots).workers, WorkersUse::Other);
    }

    #[test]
    fn appearance_settings_accept_old_and_future_files_and_bound_font_size() {
        let t = TempDir::new("cfg-appearance");
        let roots = t.roots();
        let file = roots.config_file();
        for (json, size) in [
            (r#"{"futureKey":true}"#, 13),
            (r#"{"terminalFontSize":-1}"#, 9),
            (r#"{"terminalFontSize":100}"#, 24),
            (r#"{"terminalFontSize":18446744073709551615}"#, 24),
            (r#"{"terminalFontSize":"11","terminalScheme":42,"windowTheme":"sepia"}"#, 13),
        ] {
            put(&file, json);
            let c = load(&roots);
            assert_eq!(
                (c.terminal_scheme.as_str(), c.terminal_font_size, c.window_theme.as_str()),
                ("auto", size, "system")
            );
        }
        put(&file, r#"{"terminalScheme":"future-scheme","windowTheme":"dark"}"#);
        let c = load(&roots);
        save(&roots, &c).unwrap();
        assert_eq!(c.terminal_scheme, "future-scheme");
        assert_eq!(c.window_theme, "dark");
        assert_eq!(load(&roots), c);
    }

    #[test]
    fn visual_preferences_default_validate_and_round_trip() {
        let t = TempDir::new("cfg-visual");
        let roots = t.roots();
        let defaults = AppearancePrefs::default();
        assert_eq!(load(&roots).appearance, defaults);
        assert_eq!(
            (defaults.backdrop.as_str(), defaults.accent_source.as_str(), defaults.corner_radius),
            ("off", "theme", 8)
        );
        assert!(defaults.soft_shadows && !defaults.cursor_blink && !defaults.reduce_motion);
        assert_eq!((defaults.cursor_shape, defaults.terminal_opacity), (TerminalCursor::Block, 100));
        put(
            &roots.config_file(),
            r#"{"appearance":{"backdrop":"glass","accentSource":"cover","cornerRadius":"wide",
                "softShadows":null,"reduceMotion":"true","cursorBlink":99,"accentColour":"invalid",
                 "cursorShape":"beam","terminalOpacity":"80","terminalBackground":5,"unrecognised":true,
                 "darkTheme":"Black","lightTheme":"../x"}}"#,
        );
        assert_eq!(load(&roots).appearance, defaults);
        put(&roots.config_file(), r#"{"appearance":{"cornerRadius":999}}"#);
        assert_eq!(load(&roots).appearance.corner_radius, 16);
        let mut c = load(&roots);
        c.appearance = AppearancePrefs {
            backdrop: "acrylic".into(),
            accent_source: "custom".into(),
            accent_colour: Some(format!("#{:06x}", 32_408)),
            corner_radius: 0,
            soft_shadows: false,
            cursor_blink: true,
            cursor_shape: TerminalCursor::Underline,
            terminal_opacity: 80,
            reduce_motion: true,
            terminal_background: Some(format!("#{:06x}", 12_345)),
            terminal_foreground: Some(format!("#{:06x}", 54_321)),
            terminal_accent: Some(format!("#{:06x}", 22_222)),
            dark_theme: "warm-dark".into(),
            light_theme: "from-a-newer-version".into(),
            themes: Default::default(),
        };
        save(&roots, &c).unwrap();
        assert_eq!(load(&roots), c);
    }

    #[test]
    fn chat_settings_load_older_files_and_keep_newer_keys_through_a_merged_save() {
        let t = TempDir::new("cfg-chat");
        let roots = t.roots();
        put(&roots.config_file(), r#"{"showHidden":true,"terminalFontSize":15}"#);
        let old = load(&roots);
        assert_eq!(old.chat, ChatPrefs::default(), "a file from before the Chat page");
        assert!(old.show_hidden);
        put(
            &roots.config_file(),
            r#"{"futureTop":1,"chat":{"chips":["clear","future-chip"],"answerMenus":false,"futureKey":"kept"}}"#,
        );
        let start = load(&roots);
        assert_eq!(start.chat.chips, ["clear", "future-chip"]);
        let mut edited = start.clone();
        edited.chat.set_command(0, "/review --strict");
        edited.chat.answer_menus = true;
        let saved = save_changes(&roots, &start, &edited).unwrap();
        assert_eq!(saved.chat, edited.chat);
        let raw = read_json(&roots.config_file()).unwrap();
        assert_eq!(raw["chat"]["futureKey"], "kept");
        assert_eq!(raw["futureTop"], 1);
        assert_eq!(raw["chat"]["chips"], json!(["clear", "future-chip"]));
    }

    #[test]
    fn terminal_opacity_is_clamped_and_quantized_when_read() {
        for (raw, expected) in [(i64::MIN, 60), (62, 60), (63, 65), (97, 95), (i64::MAX, 100)] {
            assert_eq!(AppearancePrefs::read(&json!({ "terminalOpacity": raw })).terminal_opacity, expected);
        }
    }

    #[test]
    fn show_usage_defaults_on_and_can_be_disabled_persistently() {
        let t = TempDir::new("cfg-usage");
        let roots = t.roots();
        assert!(load(&roots).show_usage);
        set_usage(&roots, false).unwrap();
        let stale_off = load(&roots);
        assert!(!stale_off.show_usage);
        set_usage(&roots, true).unwrap();
        save(&roots, &stale_off).unwrap();
        assert!(load(&roots).show_usage);
    }

    #[test]
    fn two_windows_change_different_fields_of_one_group_and_only_the_same_field_is_refused() {
        let t = TempDir::new("cfg-two-windows");
        let roots = t.roots();
        let start = load(&roots);
        let mut one = start.clone();
        one.appearance.reduce_motion = !start.appearance.reduce_motion;
        let one_saved = save_changes(&roots, &start, &one).unwrap();
        let mut two = start.clone();
        two.appearance.soft_shadows = !start.appearance.soft_shadows;
        let saved = save_changes(&roots, &start, &two).expect("another field of appearance");
        assert_eq!(saved.appearance.reduce_motion, one.appearance.reduce_motion, "the other window's edit is kept");
        assert_eq!(saved.appearance.soft_shadows, two.appearance.soft_shadows);
        let mut font = one_saved.clone();
        font.terminal_font_size = 20;
        save_changes(&roots, &one_saved, &font).expect("one window, again");
        let mut other = saved.clone();
        other.terminal_font_size = 11;
        let e = save_changes(&roots, &saved, &other).expect_err("both set the font size").to_string();
        assert!(e.contains("terminalFontSize") && !e.contains("Restart"), "{e}");
        assert_eq!(load(&roots).terminal_font_size, 20);
    }

    #[test]
    fn account_add_and_remove_survive_an_unrelated_stale_font_save() {
        let t = TempDir::new("cfg-cross-window-accounts");
        let roots = t.roots();
        let main_only = load(&roots);
        let mut with_work = main_only.clone();
        let work = crate::accounts::add(&roots, &mut with_work, "Work").unwrap();
        let with_work = save_changes(&roots, &main_only, &with_work).unwrap();
        let mut font = main_only.clone();
        font.terminal_font_size = 18;
        let saved = save_changes(&roots, &main_only, &font).unwrap();
        assert!(saved.accounts.iter().any(|a| a.slug == work.slug));
        let mut removed = with_work.clone();
        crate::accounts::remove(&mut removed, &work.slug).unwrap();
        save_changes(&roots, &with_work, &removed).unwrap();
        font = saved.clone();
        font.terminal_font_size = 19;
        let saved = save_changes(&roots, &saved, &font).unwrap();
        assert_eq!(saved.accounts.len(), 1);
        assert_eq!(saved.terminal_font_size, 19);
        assert!(usage_permit(&roots, &work.cli_dir).is_none());
    }

    #[test]
    fn stale_settings_save_does_not_reverse_an_existing_usage_opt_out() {
        let t = TempDir::new("cfg-stale-usage");
        let roots = t.roots();
        let mut stale = load(&roots);
        put(&roots.config_file(), r#"{"showUsage":false}"#);
        stale.show_workers = true;
        save(&roots, &stale).unwrap();
        assert!(!load(&roots).show_usage);
        assert!(load(&roots).show_workers);
    }

    #[test]
    fn a_damaged_file_or_wrong_keys_give_the_defaults_for_those_keys() {
        let t = TempDir::new("cfg-damaged");
        let roots = t.roots();
        put(&roots.config_file(), "{oops");
        let c = load(&roots);
        assert_eq!((c.accounts.len(), c.current.as_str(), c.embedded_terminal), (1, "main", true));
        put(
            &roots.config_file(),
            r#"{"accounts":[{"slug":"work"},
                {"slug":"main","name":"Home"}],
               "current":"nobody","workers":{"account":"nobody"},"embeddedTerminal":"yes","showArchived":true}"#,
        );
        let c = load(&roots);
        assert_eq!(c.accounts.iter().map(|a| a.slug.as_str()).collect::<Vec<_>>(), ["main", "work"]);
        assert_eq!(c.accounts[0].name, "Home");
        assert_eq!(c.accounts[0].cli_dir, roots.primary_cli_dir());
        assert_eq!(c.accounts[1].name, "work");
        assert_eq!(c.accounts[1].color, COLORS[1]);
        assert_eq!(c.accounts[1].cli_dir, roots.accounts_dir().join("work").join("cli"));
        assert_eq!(c.current, "main");
        assert_eq!(c.workers, WorkersUse::Same);
        assert!(c.embedded_terminal);
        assert!(c.show_archived);
    }

    #[test]
    fn write_workers_keeps_foreign_keys() {
        let t = TempDir::new("cfg-workers");
        let roots = t.roots();
        let mut c = config_with(&roots, &[("work", "Work")]);
        c.workers = WorkersUse::Account("work".into());
        put(&roots.workers_file(), r#"{"workers":[{"name":"x","n":1.5}],"claudeWorkersUse":"old","z":null}"#);
        write_workers(&roots, &c).unwrap();
        let v = read_json(&roots.workers_file()).unwrap();
        assert_eq!(v["workers"], json!([{"name": "x", "n": 1.5}]));
        assert_eq!(v["z"], Value::Null);
        let dirs: Vec<String> = c.accounts.iter().map(|a| a.cli_dir.to_string_lossy().into_owned()).collect();
        assert_eq!(v["claudeAccounts"], json!(dirs));
        assert_eq!(v["claudeWorkersUse"], json!(dirs[1]));
        assert_eq!(v.as_object().unwrap().len(), 4);
    }

    #[test]
    fn write_workers_creates_a_missing_file_and_refuses_a_non_object() {
        let t = TempDir::new("cfg-workers-new");
        let roots = t.roots();
        let c = load(&roots);
        write_workers(&roots, &c).unwrap();
        let v = read_json(&roots.workers_file()).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert_eq!(v["claudeWorkersUse"], json!("same"));
        put(&roots.workers_file(), "[1,2]");
        assert!(matches!(write_workers(&roots, &c), Err(Refusal::Io(_))));
        assert_eq!(std::fs::read_to_string(roots.workers_file()).unwrap(), "[1,2]");
    }
}
