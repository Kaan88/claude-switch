//! The effort level a Claude tab started with, read from the same sources as Claude Code and in
//! its order: `CLAUDE_CODE_EFFORT_LEVEL` (which also outranks `/effort`), the `--effort` flag,
//! `effortLevel` in the local, project and user settings, and else the model's default. Read once
//! when the tab opens; nothing is written.
use std::path::{Path, PathBuf};

use claude_switch_core::Launch;

const ENV: &str = "CLAUDE_CODE_EFFORT_LEVEL";
const CONFIG: &str = "CLAUDE_CONFIG_DIR";
const MAX_SETTINGS: u64 = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Start {
    /// The environment variable: no command in the session changes it.
    Env(&'static str),
    /// The flag or a settings file: `/effort` replaces it.
    Set(&'static str),
    /// Nothing set: the model's default.
    #[default]
    Default,
}

fn level(value: &str) -> Option<&'static str> {
    super::commands::EFFORT.into_iter().chain(["auto"]).find(|l| value.trim().eq_ignore_ascii_case(l))
}

fn flag(args: &[String]) -> Option<&'static str> {
    let mut found = None;
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--effort=") {
            found = level(value);
        } else if arg == "--effort" {
            found = args.get(i + 1).and_then(|v| level(v));
        }
    }
    found
}

fn setting(file: &Path) -> Option<&'static str> {
    std::fs::metadata(file).ok().filter(|m| m.is_file() && m.len() <= MAX_SETTINGS)?;
    let text = std::fs::read_to_string(file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["effortLevel"].as_str().and_then(level)
}

/// The level `launch` starts with. `home`: the user's folder, for the first account's settings.
pub(crate) fn starting(launch: &Launch, home: &Path) -> Start {
    let inherited = |name: &str| {
        let removed = launch.env_remove.iter().any(|r| r == name);
        launch.env.get(name).cloned().or_else(|| (!removed).then(|| std::env::var(name).ok()).flatten())
    };
    if let Some(value) = inherited(ENV).as_deref().and_then(level) {
        return Start::Env(value);
    }
    if let Some(value) = flag(&launch.args) {
        return Start::Set(value);
    }
    let config = inherited(CONFIG).map_or_else(|| home.join(".claude"), PathBuf::from);
    let project = launch.cwd.as_ref().map(|cwd| cwd.join(".claude"));
    let files = project
        .iter()
        .flat_map(|p| [p.join("settings.local.json"), p.join("settings.json")])
        .chain([config.join("settings.json")]);
    files.filter_map(|f| setting(&f)).next().map_or(Start::Default, Start::Set)
}

/// Claude Code's default level for `model` (its docs, 2026-11), or `None` for a model it does not
/// name.
pub(super) fn model_default(model: &str) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    if !model.contains("claude") && !["fable", "opus", "sonnet", "haiku"].iter().any(|m| model.contains(m)) {
        return None;
    }
    let newest =
        ["fable-5-2", "fable-5.2", "opus-5-5", "opus-5.5", "sonnet-5-5", "sonnet-5.5", "haiku-5-5", "haiku-5.5"];
    Some(if newest.iter().any(|m| model.contains(m)) {
        "medium"
    } else if model.contains("opus-4-7") || model.contains("opus-4.7") {
        "xhigh"
    } else {
        "high"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(args: &[&str], cwd: Option<&Path>) -> Launch {
        Launch {
            program: "claude".into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            cwd: cwd.map(Path::to_owned),
            env: Default::default(),
            env_remove: vec![ENV.into(), CONFIG.into()],
            place: claude_switch_core::Place::Terminal,
            label: String::new(),
        }
    }

    #[test]
    fn the_starting_level_follows_claude_codes_order() {
        let scratch = crate::testing::Scratch::new("cs-effort");
        let root = scratch.to_path_buf();
        let _ = std::fs::remove_dir_all(&root);
        let (home, project) = (root.join("home"), root.join("project"));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        assert_eq!(starting(&launch(&[], Some(&project)), &home), Start::Default);

        std::fs::write(home.join(".claude/settings.json"), r#"{"effortLevel":"low"}"#).unwrap();
        assert_eq!(starting(&launch(&[], Some(&project)), &home), Start::Set("low"));
        std::fs::write(project.join(".claude/settings.json"), r#"{"effortLevel":"high"}"#).unwrap();
        assert_eq!(starting(&launch(&[], Some(&project)), &home), Start::Set("high"));
        std::fs::write(project.join(".claude/settings.local.json"), r#"{"effortLevel":"XHigh"}"#).unwrap();
        assert_eq!(starting(&launch(&[], Some(&project)), &home), Start::Set("xhigh"));
        assert_eq!(starting(&launch(&["--resume", "x", "--effort", "max"], Some(&project)), &home), Start::Set("max"));
        assert_eq!(starting(&launch(&["--effort=low"], Some(&project)), &home), Start::Set("low"));

        let mut l = launch(&["--effort", "max"], Some(&project));
        l.env.insert(ENV.into(), "medium".into());
        assert_eq!(starting(&l, &home), Start::Env("medium"));
        l.env.insert(ENV.into(), "unset".into());
        assert_eq!(starting(&l, &home), Start::Set("max"), "an unknown value does not count");

        let other = root.join("account");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("settings.json"), r#"{"effortLevel":"medium"}"#).unwrap();
        let mut l = launch(&[], None);
        l.env.insert(CONFIG.into(), other.to_string_lossy().into_owned());
        assert_eq!(starting(&l, &home), Start::Set("medium"), "a further account's own settings");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn model_defaults_follow_the_docs() {
        assert_eq!(model_default("claude-opus-5-5"), Some("medium"));
        assert_eq!(model_default("claude-fable-5-2"), Some("medium"));
        assert_eq!(model_default("claude-opus-4-7"), Some("xhigh"));
        assert_eq!(model_default("claude-sonnet-4-6"), Some("high"));
        assert_eq!(model_default("gpt-something"), None);
    }
}
