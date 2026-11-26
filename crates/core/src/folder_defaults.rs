//! Per-folder launch preference stored beside other app-owned folder marks.

use serde_json::{Value, json};
use std::path::Path;

use crate::{App, Config, marks::Marks, sessions::folder_key};

pub(crate) fn decode(value: &Value) -> Option<(&str, App)> {
    let account = value["account"].as_str().filter(|s| !s.is_empty())?;
    let app = match value["app"].as_str()? {
        "cli" => App::Cli,
        "desktop" => App::Desktop,
        _ => return None,
    };
    Some((account, app))
}

pub(crate) fn encode(value: Option<(&str, App)>) -> Value {
    value.map_or(
        Value::Null,
        |(account, app)| json!({"account": account, "app": if app == App::Cli { "cli" } else { "desktop" }}),
    )
}

/// A missing account falls back to the selected account, without changing the remembered app.
pub fn resolve<'a>(marks: &Marks, config: &'a Config, cwd: &Path, fallback: App) -> (&'a str, App) {
    let selected = crate::account(config, &config.current).ok().or_else(|| config.accounts.first());
    let selected = selected.map_or(config.current.as_str(), |a| a.slug.as_str());
    let Some((account, app)) = marks.folder_default(&folder_key(cwd)) else { return (selected, fallback) };
    (crate::account(config, account).map_or(selected, |a| a.slug.as_str()), app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::TempDir;

    #[test]
    fn saved_defaults_are_per_folder_and_clearable() {
        let temp = TempDir::new("folder-defaults");
        let roots = temp.roots();
        let mut config = crate::config::load(&roots);
        let cwd = temp.0.join("project");
        let key = folder_key(&cwd);
        let mut marks = Marks::default();
        assert!(marks.set_folder_default(&key, Some(("removed", App::Desktop))));
        marks.save(&roots).unwrap();
        let mut marks = Marks::load(&roots).unwrap();
        assert_eq!(marks.folder_default(&key), Some(("removed", App::Desktop)));
        assert_eq!(resolve(&marks, &config, &cwd, App::Cli), (config.current.as_str(), App::Desktop));
        let mut added = config.accounts[0].clone();
        added.slug = "removed".into();
        config.accounts.push(added);
        assert_eq!(resolve(&marks, &config, &cwd, App::Cli), ("removed", App::Desktop));
        assert_eq!(resolve(&marks, &config, &temp.0, App::Cli), (config.current.as_str(), App::Cli));
        assert!(marks.set_folder_default(&key, None));
        assert_eq!(resolve(&marks, &config, &cwd, App::Cli), (config.current.as_str(), App::Cli));
    }
}
