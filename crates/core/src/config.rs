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
//!   "appearance": {"backdrop":"off", "accentSource":"system", "reduceMotion":false,
//!                  "cornerRadius":8, "softShadows":true, "cursorBlink":false}
//! }
//! ```
//!
//! Folders are not stored: they follow from the slug and [`Roots`].

use crate::accounts::COLORS;
use crate::util::{io_refusal, read_json, str_field, write_json};
use crate::{Account, Config, Refusal, Roots, WorkersUse};
use serde_json::{Map, Value, json};

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
    pub terminal_background: Option<String>,
    pub terminal_foreground: Option<String>,
    pub terminal_accent: Option<String>,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            backdrop: "off".into(),
            accent_source: "system".into(),
            accent_colour: None,
            corner_radius: 8,
            soft_shadows: true,
            reduce_motion: false,
            cursor_blink: false,
            terminal_background: None,
            terminal_foreground: None,
            terminal_accent: None,
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
            terminal_background: colour("terminalBackground"),
            terminal_foreground: colour("terminalForeground"),
            terminal_accent: colour("terminalAccent"),
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
            "terminalBackground": self.terminal_background,
            "terminalForeground": self.terminal_foreground,
            "terminalAccent": self.terminal_accent,
        })
    }
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

/// Reads [`Roots::config_file`]. A missing or damaged file, or a missing key, gives the defaults:
/// one account (`main`, named "Main", primary, the standard folders), `current` = `main`, workers
/// `Same`, embedded terminal on, archived and worker sessions hidden. The first account's folders always come from
/// `roots`, whatever the file says.
pub fn load(roots: &Roots) -> Config {
    let file = read_json(&roots.config_file()).filter(Value::is_object).unwrap_or(Value::Null);
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
        show_archived,
        show_workers,
        show_hidden: flag("showHidden", false),
        folders_view: flag("foldersView", false),
        terminal_scheme,
        terminal_font_size,
        window_theme,
        appearance: AppearancePrefs::read(&file["appearance"]),
    }
}

/// Writes the settings (temporary file beside it, then rename), creating the folder.
pub fn save(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let accounts: Vec<Value> =
        config.accounts.iter().map(|a| json!({"slug": a.slug, "name": a.name, "color": a.color})).collect();
    let workers = match &config.workers {
        WorkersUse::Same => json!("same"),
        WorkersUse::Other => json!("other"),
        WorkersUse::Account(s) => json!({"account": s}),
    };
    let value = json!({
        "accounts": accounts,
        "current": config.current,
        "workers": workers,
        "embeddedTerminal": config.embedded_terminal,
        "showArchived": config.show_archived,
        "showWorkers": config.show_workers,
        "showHidden": config.show_hidden,
        "foldersView": config.folders_view,
        "terminalScheme": config.terminal_scheme,
        "terminalFontSize": config.terminal_font_size,
        "windowTheme": config.window_theme,
        "appearance": config.appearance.json(),
    });
    write_json(&roots.config_file(), &value)
}

/// Sets `claudeAccounts` (every account's CLI folder, in order, as strings) and `claudeWorkersUse`
/// (`"same"`, `"other"` or a CLI folder) in [`Roots::workers_file`], leaving every other key of that
/// file exactly as it is. A missing file is created holding only the two keys; a file that is not a
/// JSON object is left alone and is [`Refusal::Io`].
pub fn write_workers(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let path = roots.workers_file();
    let mut object = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err(io_refusal("update", &path, &"it is not a JSON object")),
            Err(e) => return Err(io_refusal("update", &path, &e)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(io_refusal("read", &path, &e)),
    };
    let dirs: Vec<Value> =
        config.accounts.iter().map(|a| Value::String(a.cli_dir.to_string_lossy().into_owned())).collect();
    let uses = match &config.workers {
        WorkersUse::Same => "same".to_string(),
        WorkersUse::Other => "other".to_string(),
        WorkersUse::Account(slug) => match config.accounts.iter().find(|a| &a.slug == slug) {
            Some(a) => a.cli_dir.to_string_lossy().into_owned(),
            None => "same".to_string(),
        },
    };
    object.insert("claudeAccounts".into(), Value::Array(dirs));
    object.insert("claudeWorkersUse".into(), Value::String(uses));
    write_json(&path, &Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};

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
        assert!(c.embedded_terminal);
        assert!(!c.show_archived);
        assert!(!c.show_workers);
        assert!(!c.show_hidden);
        assert!(!c.folders_view);
        assert_eq!((c.terminal_scheme.as_str(), c.terminal_font_size, c.window_theme.as_str()), ("auto", 13, "system"));
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
        c.show_archived = true;
        c.show_workers = true;
        c.show_hidden = true;
        c.folders_view = true;
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
            ("off", "system", 8)
        );
        assert!(defaults.soft_shadows && !defaults.cursor_blink && !defaults.reduce_motion);
        put(
            &roots.config_file(),
            r#"{"appearance":{"backdrop":"glass","accentSource":"cover","cornerRadius":"wide",
                "softShadows":null,"reduceMotion":"true","cursorBlink":99,"accentColour":"invalid",
                "terminalBackground":5,"unrecognised":true}}"#,
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
            reduce_motion: true,
            terminal_background: Some(format!("#{:06x}", 12_345)),
            terminal_foreground: Some(format!("#{:06x}", 54_321)),
            terminal_accent: Some(format!("#{:06x}", 22_222)),
        };
        save(&roots, &c).unwrap();
        assert_eq!(load(&roots), c);
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
            r#"{"accounts":[{"slug":"Bad Slug"},{"slug":"work","color":[1,2,300]},{"slug":"work"},
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
