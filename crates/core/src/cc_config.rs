//! Claude Code's own `/config` entries that Settings › Claude Code › Advanced edits, where each
//! is stored, and the careful write that changes one of them.
//!
//! The table was taken from the bundled code of Claude Code [`VERSION`] (the `/config` panel's
//! list and the settings schema); `docs/DESIGN.md` ("Claude Code's /config") has the whole list,
//! the entries left out and why. `tools/cli-compat.ps1` checks that an installed Claude Code still
//! has every key and label here.
//!
//! Two places:
//!
//! - [`Store::Settings`]: the user `settings.json`. Saved in the master account and brought to
//!   every sharing account by [`settings_sync::sync`], as the Claude Code page does.
//! - [`Store::Global`]: the account's `.claude.json`. Claude Code keeps the whole file in memory
//!   and writes it back while it runs and when it exits, so a change is refused while the account
//!   has a running Claude Code (it would be lost). The file is re-read, only the key changes, every
//!   other key is kept, and it is replaced atomically while Claude Code's own lock folder
//!   (`.claude.json.lock`) is held.
//!
//! In both, a value that changed since the page read it is never overwritten: [`save`] compares
//! the key with what the page showed and refuses when it differs.
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::settings_sync::{self, master_account};
use crate::util::{io_refusal, write_json};
use crate::{Account, Config, Refusal, Roots};

/// The Claude Code version the table was taken from.
pub const VERSION: &str = "2.1.332";

/// Where an entry is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    /// The user `settings.json`, shared by the accounts.
    Settings,
    /// The account's `.claude.json`, its own.
    Global,
}

/// A value an entry can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lit {
    Bool(bool),
    Str(&'static str),
}

impl Lit {
    pub fn json(self) -> Value {
        match self {
            Lit::Bool(b) => Value::Bool(b),
            Lit::Str(s) => Value::String(s.to_owned()),
        }
    }
}

/// The control an entry gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A switch with Claude Code's default.
    Switch(bool),
    /// A choice among labelled values, with the index of Claude Code's default; `None` when the
    /// default depends on the plan or on what Claude Code has asked (the key is simply absent).
    Choice { options: &'static [(&'static str, Lit)], default: Option<usize> },
}

/// One `/config` entry.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The key, a dotted path for a nested one (`worktree.baseRef`).
    pub key: &'static str,
    /// The label `/config` shows, for the compatibility check.
    pub cli_label: &'static str,
    /// The label in Switch.
    pub label: &'static str,
    /// What it does, in a short line.
    pub help: &'static str,
    pub store: Store,
    pub kind: Kind,
    /// Keys `/config` removes when it saves this one.
    pub also_remove: &'static [&'static str],
}

const fn entry(key: &'static str, label: &'static str, help: &'static str, store: Store, kind: Kind) -> Entry {
    Entry { key, cli_label: label, label, help, store, kind, also_remove: &[] }
}

/// `e` shown in Switch under a shorter label than `/config`'s.
const fn short(label: &'static str, e: Entry) -> Entry {
    Entry { label, ..e }
}

const fn switch(key: &'static str, label: &'static str, help: &'static str, store: Store, default: bool) -> Entry {
    entry(key, label, help, store, Kind::Switch(default))
}

use Lit::{Bool, Str};
use Store::{Global, Settings};

const ON_OFF: &[(&str, Lit)] = &[("On", Bool(true)), ("Off", Bool(false))];

/// The entries, in the order `/config` lists them.
pub const ENTRIES: &[Entry] = &[
    switch("idleCompaction", "Idle compaction", "Compacts a long chat while it sits idle.", Settings, true),
    switch(
        "autoContinueAtUsageLimit",
        "Continue automatically at usage limit",
        "Waits for the limit to reset, then goes on.",
        Settings,
        true,
    ),
    short(
        "On a flagged message",
        entry(
            "switchModelsOnFlag",
            "Switch models when a message is flagged",
            "When safeguards flag one.",
            Settings,
            Kind::Choice { options: &[("Switch models", Bool(true)), ("Ask each time", Bool(false))], default: None },
        ),
    ),
    switch("spinnerTipsEnabled", "Show tips", "Tips beside the spinner.", Settings, true),
    entry(
        "feedbackDrafts",
        "Claude-drafted feedback",
        "Feedback Claude drafts.",
        Settings,
        Kind::Choice {
            options: &[("Notify", Str("notify")), ("Quiet", Str("quiet")), ("Off", Str("off"))],
            default: Some(0),
        },
    ),
    switch("prefersReducedMotion", "Reduce motion", "Fewer animations in the terminal.", Settings, false),
    switch("alwaysThinkingEnabled", "Thinking mode", "Thinking on models that support it.", Settings, true),
    switch("promptSuggestionEnabled", "Prompt suggestions", "Suggests your next prompt.", Settings, true),
    switch("awaySummaryEnabled", "Session recap", "A recap when you come back to a session.", Settings, true),
    switch(
        "fileCheckpointingEnabled",
        "Rewind code (checkpoints)",
        "Keeps edits so /rewind can undo them.",
        Global,
        true,
    ),
    Entry {
        also_remove: &["disableWorkflows"],
        ..entry(
            "enableWorkflows",
            "Dynamic workflows",
            "Multi-agent workflows.",
            Settings,
            Kind::Choice { options: ON_OFF, default: None },
        )
    },
    switch(
        "workflowKeywordTriggerEnabled",
        "Ultracode keyword trigger",
        "\"ultracode\" in a prompt starts a workflow.",
        Settings,
        true,
    ),
    entry(
        "workflowSizeGuideline",
        "Dynamic workflow size",
        "Agents a workflow may start.",
        Global,
        Kind::Choice {
            options: &[
                ("Small", Str("small")),
                ("Medium", Str("medium")),
                ("Large", Str("large")),
                ("Unrestricted", Str("unrestricted")),
            ],
            default: None,
        },
    ),
    Entry {
        also_remove: &["disableArtifact"],
        ..entry(
            "enableArtifact",
            "Artifacts",
            "Pages Claude makes for you.",
            Settings,
            Kind::Choice { options: ON_OFF, default: None },
        )
    },
    switch("terminalProgressBarEnabled", "Terminal progress bar", "Progress in the terminal's tab.", Global, true),
    switch(
        "showStatusInTerminalTab",
        "Show status in terminal tab",
        "The session's state in the tab title.",
        Global,
        false,
    ),
    switch("showTurnDuration", "Show turn duration", "How long each answer took.", Global, true),
    switch(
        "precomputeCompactionEnabled",
        "Precompute compaction",
        "Prepares the summary before it is needed.",
        Settings,
        true,
    ),
    switch("showMessageTimestamps", "Show message timestamps", "A time beside each message.", Global, false),
    entry(
        "timeFormat",
        "Time format",
        "Clock format for times shown.",
        Settings,
        Kind::Choice {
            options: &[
                ("Auto", Str("auto")),
                ("12-hour", Str("12-hour")),
                ("24-hour", Str("24-hour")),
                ("24-hour UTC", Str("24-hour-utc")),
            ],
            default: Some(0),
        },
    ),
    entry(
        "worktree.baseRef",
        "Worktree base ref",
        "Where new worktrees start.",
        Settings,
        Kind::Choice { options: &[("Fresh", Str("fresh")), ("Head", Str("head"))], default: Some(0) },
    ),
    switch("useAutoModeDuringPlan", "Use auto mode during plan", "Plan mode runs in auto mode.", Settings, true),
    switch("respectGitignore", "Respect .gitignore in file picker", "Hides ignored files from @.", Global, true),
    switch("copyFullResponse", "Skip the /copy picker", "/copy copies the whole answer.", Global, false),
    switch("copyOnSelect", "Copy on select", "Selecting text copies it.", Global, true),
    switch("autoScrollEnabled", "Auto-scroll", "Follows new output.", Global, true),
    entry(
        "preferredNotifChannel",
        "Local notifications",
        "How you hear it is done.",
        Global,
        Kind::Choice {
            options: &[
                ("Auto", Str("auto")),
                ("Terminal bell", Str("terminal_bell")),
                ("iTerm2", Str("iterm2")),
                ("iTerm2 and bell", Str("iterm2_with_bell")),
                ("Kitty", Str("kitty")),
                ("Ghostty", Str("ghostty")),
                ("None", Str("notifications_disabled")),
            ],
            default: Some(0),
        },
    ),
    switch("inputNeededNotifEnabled", "Push when actions required", "A phone notice when Claude waits.", Global, false),
    switch("agentPushNotifEnabled", "Push when Claude decides", "Claude may send a phone notice.", Global, false),
    entry(
        "language",
        "Language",
        "Language of Claude's answers.",
        Settings,
        Kind::Choice {
            options: &[
                ("English", Str("english")),
                ("Chinese", Str("chinese")),
                ("French", Str("french")),
                ("German", Str("german")),
                ("Japanese", Str("japanese")),
                ("Korean", Str("korean")),
                ("Portuguese", Str("portuguese")),
                ("Spanish", Str("spanish")),
                ("Turkish", Str("turkish")),
            ],
            default: None,
        },
    ),
    entry(
        "editorMode",
        "Editor mode",
        "Keys in the input box.",
        Global,
        Kind::Choice { options: &[("Normal", Str("normal")), ("Vim", Str("vim"))], default: Some(0) },
    ),
    short(
        "Unanswered questions",
        entry(
            "askUserQuestionTimeout",
            "Question auto-continue timeout",
            "Claude goes on after this.",
            Settings,
            Kind::Choice {
                options: &[("Never", Str("never")), ("60 s", Str("60s")), ("5 min", Str("5m")), ("10 min", Str("10m"))],
                default: Some(0),
            },
        ),
    ),
    entry(
        "modelProposedGoals",
        "Claude-proposed goals",
        "Goals Claude may set.",
        Settings,
        Kind::Choice {
            options: &[("Auto", Str("auto")), ("Always ask", Str("alwaysAsk")), ("Off", Str("disabled"))],
            default: Some(0),
        },
    ),
    switch(
        "externalEditorContext",
        "Show last response in external editor",
        "Ctrl+G opens it with the answer.",
        Global,
        false,
    ),
    switch("prStatusFooterEnabled", "Show PR status footer", "The branch's pull request below input.", Global, true),
    entry(
        "diffTool",
        "Diff tool",
        "Where an IDE shows edits.",
        Global,
        Kind::Choice { options: &[("Auto", Str("auto")), ("Terminal", Str("terminal"))], default: Some(0) },
    ),
    switch("autoConnectIde", "Auto-connect to IDE (external terminal)", "Connects to an open IDE.", Global, false),
    switch("autoInstallIdeExtension", "Auto-install IDE extension", "Installs it in a supported IDE.", Global, true),
    entry(
        "teammateMode",
        "Teammate mode",
        "Where agent teammates run.",
        Global,
        Kind::Choice {
            options: &[
                ("Auto", Str("auto")),
                ("tmux", Str("tmux")),
                ("iTerm2", Str("iterm2")),
                ("In process", Str("in-process")),
            ],
            default: Some(3),
        },
    ),
    short(
        "Remote Control for all",
        entry(
            "remoteControlAtStartup",
            "Enable Remote Control for all sessions",
            "claude.ai can drive sessions.",
            Global,
            Kind::Choice { options: ON_OFF, default: None },
        ),
    ),
    entry(
        "dialogExpiry",
        "Dialog expiry",
        "How long remote dialogs wait.",
        Settings,
        Kind::Choice {
            options: &[("60 s", Str("60s")), ("5 min", Str("5m")), ("10 min", Str("10m")), ("Never", Str("never"))],
            default: None,
        },
    ),
    short(
        "From your other sessions",
        entry(
            "crossSessionInbound",
            "Messages from your other sessions",
            "Messages they send here.",
            Settings,
            Kind::Choice {
                options: &[("Accept", Str("accept")), ("Hold", Str("hold")), ("Refuse", Str("refuse"))],
                default: None,
            },
        ),
    ),
    short(
        "Remote delivery notices",
        entry(
            "remoteMessageReceipts",
            "Delivery notices to and from cloud and Remote Control sessions",
            "Cloud and remote messages.",
            Settings,
            Kind::Choice {
                options: &[("On", Str("on")), ("Always", Str("always")), ("Off", Str("off"))],
                default: Some(0),
            },
        ),
    ),
];

/// The value at `key` (a dotted path) of a JSON object, `null` counting as absent.
pub fn get<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut node = root;
    for part in key.split('.') {
        node = node.get(part)?;
    }
    Some(node).filter(|v| !v.is_null())
}

/// Sets or (with `None`) removes `key` in `root`, making the objects on its path. Everything else
/// is kept as it is.
pub fn put(root: &mut Value, key: &str, value: Option<Value>) {
    let parts: Vec<&str> = key.split('.').collect();
    let (last, parents) = parts.split_last().expect("a key");
    let mut node = root;
    for part in parents {
        if !node.get(*part).is_some_and(Value::is_object) {
            if value.is_none() {
                return;
            }
            node[*part] = Value::Object(Map::new());
        }
        node = node.get_mut(*part).expect("just made");
    }
    let Some(map) = node.as_object_mut() else { return };
    match value {
        Some(value) => _ = map.insert((*last).to_owned(), value),
        None => _ = map.remove(*last),
    }
}

/// The account whose file holds `store`'s entries: the master for `settings.json`.
pub fn account_of<'a>(config: &'a Config, store: Store, chosen: &'a Account) -> Option<&'a Account> {
    match store {
        Store::Settings => master_account(config, &config.settings_share),
        Store::Global => Some(chosen),
    }
}

/// The file of `store` in `account`.
pub fn file(roots: &Roots, account: &Account, store: Store) -> PathBuf {
    match store {
        Store::Settings => account.cli_dir.join("settings.json"),
        Store::Global => crate::accounts::claude_json(roots, account),
    }
}

/// A file as a JSON object: `Ok(None)` when it does not exist, an error when it is not an object.
pub fn read(path: &Path) -> Result<Option<Value>, Refusal> {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v @ Value::Object(_)) => Ok(Some(v)),
            _ => Err(Refusal::Invalid(format!(
                "{} is not valid JSON, so it was left as it is. Fix it or remove it first.",
                path.display()
            ))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_refusal("read", path, &e)),
    }
}

/// Claude Code's lock for its config file (a folder beside it, as `proper-lockfile` makes it),
/// held while the file is replaced, and removed when dropped.
struct ConfigLock(PathBuf);

impl ConfigLock {
    fn take(path: &Path) -> Result<ConfigLock, Refusal> {
        let mut lock = path.as_os_str().to_owned();
        lock.push(".lock");
        let lock = PathBuf::from(lock);
        match std::fs::create_dir(&lock) {
            Ok(()) => Ok(ConfigLock(lock)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(Refusal::Invalid(format!(
                "Claude Code is saving {} right now; try again in a moment.",
                path.display()
            ))),
            Err(e) => Err(io_refusal("lock", path, &e)),
        }
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Saves `value` (or, with `None`, removes the key: Claude Code's default) for `entry`.
///
/// `expected` is the value the page showed. When the file holds another one now, nothing is
/// written and the error says so. `settings.json` goes to the master account and on to the
/// sharing accounts; `.claude.json` goes to `account` only, and is refused while that account has
/// a running Claude Code.
pub fn save(
    roots: &Roots,
    config: &Config,
    account: &Account,
    entry: &Entry,
    expected: Option<&Value>,
    value: Option<Value>,
) -> Result<(), Refusal> {
    let account = account_of(config, entry.store, account)
        .ok_or_else(|| Refusal::Invalid("There is no account to save Claude Code's settings in.".into()))?;
    let path = file(roots, account, entry.store);
    let _lock = match entry.store {
        Store::Global => {
            let running = crate::running::scan(roots, config);
            if running.iter().any(|r| r.account == account.slug) {
                return Err(Refusal::Invalid(format!(
                    "{} has Claude Code running, which would write its .claude.json back. Close the account's \
                     sessions to change this.",
                    account.name
                )));
            }
            Some(ConfigLock::take(&path)?)
        }
        Store::Settings => None,
    };
    let mut root = read(&path)?.unwrap_or_else(|| Value::Object(Map::new()));
    if get(&root, entry.key) != expected.filter(|v| !v.is_null()) {
        return Err(Refusal::Invalid(format!(
            "\"{}\" was changed elsewhere since this page was read, so it was not saved. The page shows the \
             value it has now.",
            entry.label
        )));
    }
    put(&mut root, entry.key, value);
    for key in entry.also_remove {
        put(&mut root, key, None);
    }
    match entry.store {
        Store::Settings => {
            settings_sync::backup(roots, account, &path)?;
            write_json(&path, &root)?;
            settings_sync::sync(roots, config, &config.settings_share).map(|_| ())
        }
        Store::Global => write_json(&path, &root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::read_json;
    use crate::util::testing::{TempDir, config_with, put as put_file};
    use serde_json::json;

    fn by_key(key: &str) -> &'static Entry {
        ENTRIES.iter().find(|e| e.key == key).unwrap()
    }

    #[test]
    fn the_table_has_one_entry_per_key_with_a_valid_default() {
        let mut keys: Vec<_> = ENTRIES.iter().map(|e| e.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ENTRIES.len());
        for e in ENTRIES {
            if let Kind::Choice { options, default } = e.kind {
                assert!(default.is_none_or(|d| d < options.len()), "{}", e.key);
                assert!(options.len() >= 2, "{}", e.key);
            }
            // The Claude Code page has these; the advanced page must not write them twice.
            assert!(
                !["model", "effortLevel", "theme", "verbose", "autoCompactEnabled", "claudeInChromeDefaultEnabled"]
                    .contains(&e.key)
            );
        }
    }

    #[test]
    fn a_settings_entry_goes_to_the_master_and_on_and_unknown_keys_stay() {
        let t = TempDir::new("ccc-settings");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let [main, work] = [&config.accounts[0], &config.accounts[1]];
        put_file(&main.cli_dir.join("settings.json"), r#"{"someday":{"x":1},"worktree":{"symlinkDirectories":["a"]}}"#);
        let base = by_key("worktree.baseRef");
        save(&roots, &config, work, base, None, Some(json!("head"))).unwrap();
        let main_now = read_json(&main.cli_dir.join("settings.json")).unwrap();
        assert_eq!(main_now["worktree"], json!({"symlinkDirectories": ["a"], "baseRef": "head"}));
        assert_eq!(main_now["someday"], json!({"x": 1}));
        assert_eq!(read_json(&work.cli_dir.join("settings.json")).unwrap(), main_now, "shared");

        // Reset: the key goes, its neighbours stay.
        save(&roots, &config, main, base, Some(&json!("head")), None).unwrap();
        let main_now = read_json(&main.cli_dir.join("settings.json")).unwrap();
        assert_eq!(main_now["worktree"], json!({"symlinkDirectories": ["a"]}));

        // Workflows: /config drops the old switch with it.
        put_file(&main.cli_dir.join("settings.json"), r#"{"disableWorkflows":true}"#);
        save(&roots, &config, main, by_key("enableWorkflows"), None, Some(json!(true))).unwrap();
        assert_eq!(read_json(&main.cli_dir.join("settings.json")).unwrap(), json!({"enableWorkflows": true}));
    }

    #[test]
    fn a_value_changed_elsewhere_is_never_overwritten() {
        let t = TempDir::new("ccc-stale");
        let roots = t.roots();
        let config = config_with(&roots, &[]);
        let main = &config.accounts[0];
        let json_path = crate::accounts::claude_json(&roots, main);
        put_file(&json_path, r#"{"showTurnDuration":false,"numStartups":4}"#);
        // The page showed "absent" but the file says false now.
        let err = save(&roots, &config, main, by_key("showTurnDuration"), None, Some(json!(true))).unwrap_err();
        assert!(err.to_string().contains("changed elsewhere"), "{err}");
        assert_eq!(read_json(&json_path).unwrap()["showTurnDuration"], false);
        save(&roots, &config, main, by_key("showTurnDuration"), Some(&json!(false)), Some(json!(true))).unwrap();
        assert_eq!(read_json(&json_path).unwrap(), json!({"showTurnDuration": true, "numStartups": 4}));

        put_file(&main.cli_dir.join("settings.json"), r#"{"timeFormat":"24-hour"}"#);
        let err = save(&roots, &config, main, by_key("timeFormat"), Some(&json!("auto")), None).unwrap_err();
        assert!(err.to_string().contains("changed elsewhere"));
    }

    #[test]
    fn claude_json_is_refused_while_the_account_runs_claude_code_or_holds_its_lock() {
        let t = TempDir::new("ccc-running");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let [main, work] = [&config.accounts[0], &config.accounts[1]];
        let work_json = crate::accounts::claude_json(&roots, work);
        put_file(&work_json, r#"{"numStartups":2}"#);
        // This test's own process stands in for a running Claude Code of Work.
        let pid = std::process::id();
        put_file(
            &work.cli_dir.join("sessions").join(format!("{pid}.json")),
            &json!({"pid": pid, "sessionId": "0b4f5a64-1c0e-4a43-9a43-1f4c2b1f8f10", "cwd": "C:\\x"}).to_string(),
        );
        let entry = by_key("copyOnSelect");
        let err = save(&roots, &config, work, entry, None, Some(json!(false))).unwrap_err();
        assert!(err.to_string().contains("Work has Claude Code running"), "{err}");
        assert_eq!(read_json(&work_json).unwrap(), json!({"numStartups": 2}));
        // Another account is free; a held lock refuses, and the lock is gone after a save.
        let main_json = crate::accounts::claude_json(&roots, main);
        put_file(&main_json, "{}");
        std::fs::create_dir(main_json.with_extension("json.lock")).unwrap();
        assert!(
            save(&roots, &config, main, entry, None, Some(json!(false))).unwrap_err().to_string().contains("try again")
        );
        std::fs::remove_dir(main_json.with_extension("json.lock")).unwrap();
        save(&roots, &config, main, entry, None, Some(json!(false))).unwrap();
        assert_eq!(read_json(&main_json).unwrap(), json!({"copyOnSelect": false}));
        assert!(!main_json.with_extension("json.lock").exists());
        // A file that is not JSON is never replaced.
        put_file(&main_json, "{ nope");
        assert!(save(&roots, &config, main, entry, Some(&json!(false)), None).is_err());
        assert_eq!(std::fs::read_to_string(&main_json).unwrap(), "{ nope");
    }
}
