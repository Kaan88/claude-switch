//! A portable palette contains colours and a display name, never other settings or paths.

use crate::Refusal;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Colour(pub [u8; 3]);

impl TryFrom<String> for Colour {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let hex = s
            .strip_prefix('#')
            .filter(|s| s.len() == 6 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or("Expected a #RRGGBB colour")?;
        let n = u32::from_str_radix(hex, 16).map_err(|_| "Invalid colour")?.to_be_bytes();
        Ok(Self([n[1], n[2], n[3]]))
    }
}

impl From<Colour> for String {
    fn from(Colour([r, g, b]): Colour) -> Self {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedTheme {
    pub name: String,
    pub base: String,
    pub page: Colour,
    pub title: Colour,
    pub bar: Colour,
    pub pane: Colour,
    pub field: Colour,
    pub popup: Colour,
    pub text: Colour,
    pub muted: Colour,
    pub accent: Colour,
    pub selection: Colour,
    pub terminal_background: Colour,
    pub terminal_foreground: Colour,
    pub terminal_selection: Colour,
    pub ansi: [Colour; 16],
}

impl NamedTheme {
    pub fn validate(&self) -> Result<(), Refusal> {
        if self.name.trim().is_empty() || self.name.chars().count() > 48 || self.name.chars().any(char::is_control) {
            return Err(Refusal::Invalid("Use a theme name of 1–48 characters.".into()));
        }
        if !matches!(
            self.base.as_str(),
            "dark"
                | "black"
                | "warm-dark"
                | "cool-dark"
                | "contrast-dark"
                | "light"
                | "warm-light"
                | "cool-light"
                | "contrast-light"
        ) {
            return Err(Refusal::Invalid("Unrecognised base palette.".into()));
        }
        Ok(())
    }

    pub fn dark(&self) -> bool {
        matches!(self.base.as_str(), "dark" | "black" | "warm-dark" | "cool-dark" | "contrast-dark")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    theme: NamedTheme,
}

pub fn import(path: &Path) -> Result<NamedTheme, Refusal> {
    crate::util::regular_target(path)?;
    let bytes = crate::read_bounded(path, 16384).map_err(|e| Refusal::Io(format!("Cannot read theme: {e}")))?;
    let file: File =
        serde_json::from_slice(&bytes).map_err(|e| Refusal::Invalid(format!("Invalid theme file: {e}")))?;
    if file.version != 1 {
        return Err(Refusal::Invalid("Unsupported theme file version.".into()));
    }
    file.theme.validate()?;
    Ok(file.theme)
}

pub fn export(path: &Path, theme: &NamedTheme) -> Result<(), Refusal> {
    theme.validate()?;
    if !path.is_absolute()
        || !path.extension().is_some_and(|s| s.eq_ignore_ascii_case("json"))
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::Normal(s) if s.to_string_lossy().contains([':', '\0'])))
    {
        return Err(Refusal::Invalid("Choose a new .json file for the theme.".into()));
    }
    let bytes = serde_json::to_vec_pretty(&File { version: 1, theme: theme.clone() }).expect("theme JSON");
    crate::util::write_new_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load, save_changes};
    use crate::util::testing::{TempDir, put};
    use serde_json::json;

    fn sample() -> NamedTheme {
        let mut data = json!({"base":"black","name":"Night study"});
        data["ansi"] = json!(vec!["#020304"; 16]);
        for key in [
            "page",
            "title",
            "bar",
            "pane",
            "field",
            "popup",
            "text",
            "muted",
            "accent",
            "selection",
            "terminalBackground",
            "terminalForeground",
            "terminalSelection",
        ] {
            data[key] = json!("#aabbcc");
        }
        serde_json::from_value(data).unwrap()
    }

    #[test]
    fn theme_file_is_only_the_named_palette_and_invalid_imports_do_not_replace_it() {
        let dir = TempDir::new("named-theme");
        let path = dir.0.join("night.theme.json");
        let t = sample();
        export(&path, &t).unwrap();
        assert_eq!(import(&path).unwrap(), t);
        assert!(export(&path, &t).is_err(), "never overwrite");
        let mut bad: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(bad.as_object().unwrap().len(), 2);
        bad["theme"]["ansi"][15] = json!("#zz0000");
        put(&path, &bad.to_string());
        assert!(import(&path).is_err());
        put(&path, &" ".repeat(16385));
        assert!(import(&path).is_err());
    }
    #[test]
    fn adding_a_palette_to_old_settings_keeps_both_windows_changes() {
        let dir = TempDir::new("theme-migration");
        let roots = dir.roots();
        put(
            &roots.config_file(),
            r#"{"appearance":{"darkTheme":"black","lightTheme":"future-palette","accentSource":"system","terminalOpacity":75,"futureOption":true,"themes":{"future-palette":{"format":9}}}}"#,
        );
        let old = load(&roots);
        let mut a = old.clone();
        let mut b = old.clone();
        a.appearance.themes.insert("user-1".into(), sample());
        a.appearance.dark_theme = "user-1".into();
        b.appearance.corner_radius = 11;
        save_changes(&roots, &old, &b).unwrap();
        save_changes(&roots, &old, &a).unwrap();
        let current = load(&roots);
        a.appearance.corner_radius = 11;
        assert_eq!(current, a);
        assert_eq!(current.appearance.light_theme, "future-palette");
        assert_eq!(current.appearance.terminal_opacity, 75);
        let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(roots.config_file()).unwrap()).unwrap();
        assert_eq!(disk["appearance"]["futureOption"], true);
        assert_eq!(disk["appearance"]["themes"]["future-palette"]["format"], 9);
    }
}
