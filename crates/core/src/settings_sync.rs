//! Claude Code's settings kept the same in every account that shares them.
//!
//! Each account's CLI folder has its own `settings.json`, and its `.claude.json` holds a few more
//! of the choices `/config` makes. A hard link between the files did not last: Claude Code saves
//! `settings.json` as a new file (`/model` does, for one), which breaks the link without a word.
//! So the files are copied instead, by [`sync`], before every launch and when the app starts:
//!
//! - a file (or preference) that changed in one account since the last sync goes to the others;
//! - when several changed, the master account's version wins, else the most recently written one;
//! - with nothing known about the last sync (the first run), the master's version wins.
//!
//! A file that is not valid JSON is never copied and never replaced, every replaced
//! `settings.json` is backed up first, and from `.claude.json` only the [`PREFS`] are touched,
//! and only when their value differs.
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

use crate::util::{io_refusal, read_json, write_atomic, write_json};
use crate::{Account, Config, Refusal, Roots};

/// The choices `/config` keeps in `.claude.json` rather than `settings.json`, which every sharing
/// account gets too. Account details, counters and per-project state are never among them.
pub const PREFS: [&str; 14] = [
    "claudeInChromeDefaultEnabled",
    "autoCompactEnabled",
    "autoConnectIde",
    "autoInstallIdeExtension",
    "diffTool",
    "editorMode",
    "fileCheckpointingEnabled",
    "messageIdleNotifThresholdMs",
    "preferredNotifChannel",
    "showExpandedTodos",
    "showTurnDuration",
    "terminalProgressBarEnabled",
    "todoFeatureEnabled",
    "verbose",
];

/// How many backups of replaced `settings.json` files are kept.
const BACKUPS: usize = 30;

/// Which accounts share and which one wins a tie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    /// The master account's slug; the first account when unset or unknown.
    pub master: String,
    /// The slugs of accounts that keep settings of their own.
    pub off: Vec<String>,
}

impl Default for Share {
    fn default() -> Self {
        Share { master: "main".into(), off: Vec::new() }
    }
}

impl Share {
    pub(crate) fn read(value: &Value) -> Share {
        let master = value["master"].as_str().filter(|s| !s.is_empty()).unwrap_or("main").to_owned();
        let off = value["off"]
            .as_array()
            .map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        Share { master, off }
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::json!({"master": self.master, "off": self.off})
    }

    /// Whether `slug` takes part. The master always does.
    pub fn shares(&self, slug: &str) -> bool {
        slug == self.master || !self.off.iter().any(|s| s == slug)
    }
}

/// What one sync did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The account whose `settings.json` went to the others, and the slugs it went to.
    pub settings: Option<(String, Vec<String>)>,
    /// The preferences that changed, each with the accounts it was written to.
    pub prefs: Vec<(String, Vec<String>)>,
    /// Accounts whose `settings.json` or `.claude.json` could not be read as JSON, left alone.
    pub unreadable: Vec<String>,
}

/// One account's copy of a value, with when its file was last written.
struct Copy<'a> {
    account: &'a Account,
    value: Option<Value>,
    written: SystemTime,
}

fn state_file(roots: &Roots) -> PathBuf {
    roots.config_file().with_file_name("settings-sync.json")
}

fn backup_dir(roots: &Roots) -> PathBuf {
    roots.config_file().with_file_name("settings-backups")
}

fn written(path: &Path) -> SystemTime {
    std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The copy that goes to the others, if any: see the module's rules.
fn winner<'a>(copies: &'a [Copy<'a>], last: Option<&Value>, master: &str) -> Option<&'a Copy<'a>> {
    // A missing file or key is no change to spread: it takes the value the others have.
    let changed: Vec<&Copy> =
        copies.iter().filter(|c| c.value.is_some() && (last.is_none() || c.value.as_ref() != last)).collect();
    let master_copy = copies.iter().find(|c| c.account.slug == master);
    match (last, changed.as_slice()) {
        (_, []) => None,
        (None, _) => {
            master_copy.filter(|c| c.value.is_some()).or_else(|| changed.iter().copied().find(|c| c.value.is_some()))
        }
        (Some(_), [one]) => Some(*one),
        (Some(_), several) => several
            .iter()
            .copied()
            .find(|c| c.account.slug == master)
            .or_else(|| several.iter().copied().max_by_key(|c| c.written)),
    }
    .filter(|c| c.value.is_some())
}

/// Brings every sharing account's settings to the same state (module docs). Errors writing one
/// account do not stop the others; the first is returned after all were tried.
pub fn sync(roots: &Roots, config: &Config, share: &Share) -> Result<Report, Refusal> {
    let master = if config.accounts.iter().any(|a| a.slug == share.master) { share.master.as_str() } else { "main" };
    let members: Vec<&Account> = config.accounts.iter().filter(|a| a.slug == master || share.shares(&a.slug)).collect();
    let mut report = Report::default();
    if members.len() < 2 {
        return Ok(report);
    }
    let state = read_json(&state_file(roots)).unwrap_or(Value::Null);
    let mut first_error = None;
    let mut note = |r: Result<(), Refusal>| {
        if let Err(e) = r
            && first_error.is_none()
        {
            first_error = Some(e);
        }
    };

    // settings.json
    let mut copies = Vec::new();
    for account in &members {
        let path = account.cli_dir.join("settings.json");
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) if value.is_object() => {
                    copies.push(Copy { account, value: Some(value), written: written(&path) })
                }
                _ => report.unreadable.push(account.slug.clone()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                copies.push(Copy { account, value: None, written: SystemTime::UNIX_EPOCH })
            }
            Err(_) => report.unreadable.push(account.slug.clone()),
        }
    }
    let last_settings = state.get("settings").filter(|v| v.is_object());
    let mut synced_settings = last_settings.cloned();
    if let Some(win) = winner(&copies, last_settings, master) {
        let value = win.value.clone().unwrap();
        let mut to = Vec::new();
        for copy in copies.iter().filter(|c| c.value.as_ref() != Some(&value)) {
            let path = copy.account.cli_dir.join("settings.json");
            let result = backup(roots, copy.account, &path).and_then(|()| write_json(&path, &value));
            if result.is_ok() {
                to.push(copy.account.slug.clone());
            }
            note(result);
        }
        if !to.is_empty() {
            report.settings = Some((win.account.slug.clone(), to));
        }
        synced_settings = Some(value);
    }

    // The preferences in .claude.json.
    let mut files: Vec<(&Account, PathBuf, Map<String, Value>)> = Vec::new();
    for account in &members {
        let path = crate::accounts::claude_json(roots, account);
        // No .claude.json yet: Claude Code makes it on first use; nothing to bring in line.
        if let Ok(bytes) = std::fs::read(&path) {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => files.push((account, path, map)),
                _ => report.unreadable.push(account.slug.clone()),
            }
        }
    }
    let last_prefs = state.get("prefs").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut synced_prefs = last_prefs.clone();
    let mut changed_files = vec![false; files.len()];
    for key in PREFS {
        let copies: Vec<Copy> = files
            .iter()
            .map(|(account, path, map)| Copy { account, value: map.get(key).cloned(), written: written(path) })
            .collect();
        if copies.iter().all(|c| c.value.is_none()) {
            continue;
        }
        let last = last_prefs.get(key);
        let Some(win) = winner(&copies, last, master) else { continue };
        let value = win.value.clone().unwrap();
        let mut to = Vec::new();
        for (i, (account, _, map)) in files.iter_mut().enumerate() {
            if map.get(key) != Some(&value) {
                map.insert(key.to_owned(), value.clone());
                changed_files[i] = true;
                to.push(account.slug.clone());
            }
        }
        if !to.is_empty() {
            report.prefs.push((key.to_owned(), to));
        }
        synced_prefs.insert(key.to_owned(), value);
    }
    for (i, (account, path, map)) in files.into_iter().enumerate() {
        if !changed_files[i] {
            continue;
        }
        // Read again just before the write: Claude Code may have saved meanwhile. Only the
        // preferences change; everything else is taken from the file as it is now.
        let result = match read_json(&path) {
            Some(Value::Object(mut now)) => {
                for key in PREFS {
                    if let Some(value) = map.get(key) {
                        now.insert(key.to_owned(), value.clone());
                    }
                }
                write_json(&path, &Value::Object(now))
            }
            _ => Err(Refusal::Invalid(format!("{}'s .claude.json could not be read again.", account.name))),
        };
        note(result);
    }

    let state = serde_json::json!({"settings": synced_settings, "prefs": synced_prefs});
    note(write_json(&state_file(roots), &state));
    match first_error {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

/// Keeps a copy of `path` before it is replaced; the newest [`BACKUPS`] are kept.
pub(crate) fn backup(roots: &Roots, account: &Account, path: &Path) -> Result<(), Refusal> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    backup_bytes(roots, account, "settings.json", &bytes)
}

/// Keeps `bytes` as a backup named after `name` (`settings.json`, `claude.json`); the newest
/// [`BACKUPS`] of each name are kept.
pub(crate) fn backup_bytes(roots: &Roots, account: &Account, name: &str, bytes: &[u8]) -> Result<(), Refusal> {
    let dir = backup_dir(roots);
    std::fs::create_dir_all(&dir).map_err(|e| io_refusal("create the folder", &dir, &e))?;
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    write_atomic(&dir.join(format!("{stamp}-{}-{name}", account.slug)), bytes)?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    let suffix = format!("-{name}");
    names.retain(|n| n.ends_with(&suffix));
    names.sort_by_key(|n| n.split('-').next().and_then(|s| s.parse::<u128>().ok()).unwrap_or(0));
    for old in &names[..names.len().saturating_sub(BACKUPS)] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};
    use serde_json::json;

    fn settings(account: &Account) -> Value {
        read_json(&account.cli_dir.join("settings.json")).unwrap_or(Value::Null)
    }

    fn write(account: &Account, value: Value) {
        put(&account.cli_dir.join("settings.json"), &value.to_string());
        // Distinct write times, newest last, whatever the file system's resolution.
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    #[test]
    fn the_master_wins_the_first_sync_then_a_change_anywhere_goes_everywhere() {
        let t = TempDir::new("sync-settings");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work"), ("home", "Home")]);
        let [main, work, home] = [&config.accounts[0], &config.accounts[1], &config.accounts[2]];
        let share = Share::default();
        write(main, json!({"model": "opus", "enabledPlugins": {"switch-handover@x": true}}));
        write(work, json!({"model": "sonnet"}));
        let report = sync(&roots, &config, &share).unwrap();
        assert_eq!(report.settings, Some(("main".into(), vec!["work".into(), "home".into()])));
        assert!([work, home].iter().all(|a| settings(a) == settings(main)));
        assert_eq!(std::fs::read_dir(backup_dir(&roots)).unwrap().count(), 1, "work's own file was kept");

        // `/effort` in Work: Work's file goes to the others.
        write(work, json!({"model": "opus", "effortLevel": "high", "enabledPlugins": {"switch-handover@x": true}}));
        assert_eq!(sync(&roots, &config, &share).unwrap().settings.unwrap().0, "work");
        assert_eq!(settings(main)["effortLevel"], "high");
        assert_eq!(settings(home)["effortLevel"], "high");

        // Nothing changed: nothing is written.
        assert_eq!(sync(&roots, &config, &share).unwrap(), Report::default());

        // Two changed at once: the master's wins.
        write(home, json!({"model": "haiku"}));
        write(main, json!({"model": "fable"}));
        sync(&roots, &config, &share).unwrap();
        assert!([work, home].iter().all(|a| settings(a)["model"] == "fable"));
    }

    #[test]
    fn an_account_switched_off_keeps_its_own_and_a_broken_file_is_neither_copied_nor_replaced() {
        let t = TempDir::new("sync-off");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work"), ("home", "Home")]);
        let [main, work, home] = [&config.accounts[0], &config.accounts[1], &config.accounts[2]];
        let share = Share { master: "work".into(), off: vec!["home".into(), "work".into()] };
        assert!(share.shares("work"), "the master always shares");
        write(main, json!({"model": "opus"}));
        write(work, json!({"model": "sonnet"}));
        write(home, json!({"model": "haiku"}));
        sync(&roots, &config, &share).unwrap();
        assert_eq!(settings(main)["model"], "sonnet", "the master's");
        assert_eq!(settings(home)["model"], "haiku", "switched off");

        put(&work.cli_dir.join("settings.json"), "{ not json");
        let report = sync(&roots, &config, &share).unwrap();
        assert_eq!(report.unreadable, ["work"]);
        assert_eq!(std::fs::read_to_string(work.cli_dir.join("settings.json")).unwrap(), "{ not json");
        assert_eq!(settings(main)["model"], "sonnet");
    }

    #[test]
    fn only_the_listed_preferences_of_claude_json_are_shared() {
        let t = TempDir::new("sync-prefs");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let [main, work] = [&config.accounts[0], &config.accounts[1]];
        let main_json = crate::accounts::claude_json(&roots, main);
        let work_json = crate::accounts::claude_json(&roots, work);
        put(
            &main_json,
            &json!({"claudeInChromeDefaultEnabled": true, "numStartups": 5, "oauthAccount": {"a": 1}}).to_string(),
        );
        put(
            &work_json,
            &json!({"claudeInChromeDefaultEnabled": false, "numStartups": 2, "oauthAccount": {"b": 2}}).to_string(),
        );
        let report = sync(&roots, &config, &Share::default()).unwrap();
        assert_eq!(report.prefs, [("claudeInChromeDefaultEnabled".to_owned(), vec!["work".to_owned()])]);
        let work_now = read_json(&work_json).unwrap();
        assert_eq!(work_now["claudeInChromeDefaultEnabled"], true);
        assert_eq!(work_now["numStartups"], 2, "its own counters");
        assert_eq!(work_now["oauthAccount"], json!({"b": 2}), "its own login");

        // Turned off in Work: off everywhere.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut w = read_json(&work_json).unwrap();
        w["claudeInChromeDefaultEnabled"] = json!(false);
        put(&work_json, &w.to_string());
        sync(&roots, &config, &Share::default()).unwrap();
        assert_eq!(read_json(&main_json).unwrap()["claudeInChromeDefaultEnabled"], false);
    }
}

/// A Claude Code setting the app's own page edits. It is saved in the master account and goes
/// to every sharing account through [`sync`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Model,
    Effort,
    PermissionMode,
    Theme,
    Chrome,
    AutoCompact,
    Verbose,
}

impl Setting {
    /// Where it lives: in `settings.json` (true) or `.claude.json`, and its path there.
    fn place(self) -> (bool, &'static [&'static str]) {
        match self {
            Setting::Model => (true, &["model"]),
            Setting::Effort => (true, &["effortLevel"]),
            Setting::PermissionMode => (true, &["permissions", "defaultMode"]),
            Setting::Theme => (true, &["theme"]),
            Setting::Chrome => (false, &["claudeInChromeDefaultEnabled"]),
            Setting::AutoCompact => (false, &["autoCompactEnabled"]),
            Setting::Verbose => (false, &["verbose"]),
        }
    }
}

/// The account whose settings win a tie: the chosen master, or the first account.
pub fn master_account<'a>(config: &'a Config, share: &Share) -> Option<&'a Account> {
    config.accounts.iter().find(|a| a.slug == share.master).or_else(|| config.accounts.iter().find(|a| a.primary))
}

fn file_of(roots: &Roots, account: &Account, in_settings: bool) -> PathBuf {
    if in_settings { account.cli_dir.join("settings.json") } else { crate::accounts::claude_json(roots, account) }
}

/// The setting's value in the master account, if it is set.
pub fn get(roots: &Roots, config: &Config, setting: Setting) -> Option<Value> {
    let account = master_account(config, &config.settings_share)?;
    let (in_settings, path) = setting.place();
    let mut value = &read_json(&file_of(roots, account, in_settings))?;
    for key in path {
        value = value.get(key)?;
    }
    Some(value.clone()).filter(|v| !v.is_null())
}

/// Sets (or, with `None`, removes) the setting in the master account, then brings the sharing
/// accounts in line. A file that is not a JSON object is refused, never replaced.
pub fn set(roots: &Roots, config: &Config, setting: Setting, value: Option<Value>) -> Result<Report, Refusal> {
    let account = master_account(config, &config.settings_share)
        .ok_or_else(|| Refusal::Invalid("There is no account to save Claude Code's settings in.".into()))?;
    let (in_settings, path) = setting.place();
    let file = file_of(roots, account, in_settings);
    let mut root = match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                return Err(Refusal::Invalid(format!(
                    "{} is not valid JSON, so it was left as it is. Fix it or remove it first.",
                    file.display()
                )));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => return Err(io_refusal("read", &file, &e)),
    };
    let (last, parents) = path.split_last().expect("a path");
    let mut node = &mut root;
    for key in parents {
        if !node.get(*key).is_some_and(Value::is_object) {
            node[*key] = Value::Object(Map::new());
        }
        node = node.get_mut(*key).expect("just made");
    }
    let map = node.as_object_mut().expect("an object");
    match value {
        Some(value) => _ = map.insert((*last).to_owned(), value),
        None => _ = map.shift_remove(*last),
    }
    if in_settings {
        backup(roots, account, &file)?;
    }
    write_json(&file, &root)?;
    sync(roots, config, &config.settings_share)
}

#[cfg(test)]
mod setting_tests {
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};
    use serde_json::json;

    #[test]
    fn a_setting_is_saved_in_the_master_and_reaches_every_sharing_account() {
        let t = TempDir::new("sync-set");
        let roots = t.roots();
        let mut config = config_with(&roots, &[("work", "Work"), ("home", "Home")]);
        config.settings_share = Share { master: "work".into(), off: vec!["home".into()] };
        let [main, work, home] = [&config.accounts[0], &config.accounts[1], &config.accounts[2]];
        put(&home.cli_dir.join("settings.json"), r#"{"model":"haiku"}"#);
        put(&crate::accounts::claude_json(&roots, main), r#"{"numStartups":3}"#);
        put(&crate::accounts::claude_json(&roots, work), r#"{"numStartups":9}"#);

        set(&roots, &config, Setting::PermissionMode, Some(json!("acceptEdits"))).unwrap();
        set(&roots, &config, Setting::Model, Some(json!("opus"))).unwrap();
        set(&roots, &config, Setting::Chrome, Some(json!(true))).unwrap();
        assert_eq!(get(&roots, &config, Setting::Model), Some(json!("opus")));
        let main_settings = read_json(&main.cli_dir.join("settings.json")).unwrap();
        assert_eq!(main_settings["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(main_settings["model"], "opus");
        assert_eq!(read_json(&work.cli_dir.join("settings.json")).unwrap(), main_settings);
        assert_eq!(read_json(&home.cli_dir.join("settings.json")).unwrap()["model"], "haiku", "keeps its own");
        let main_json = read_json(&crate::accounts::claude_json(&roots, main)).unwrap();
        assert_eq!(
            (main_json["claudeInChromeDefaultEnabled"].clone(), main_json["numStartups"].clone()),
            (json!(true), json!(3))
        );

        // Back to Claude Code's default: the key goes, everywhere.
        set(&roots, &config, Setting::Model, None).unwrap();
        assert_eq!(get(&roots, &config, Setting::Model), None);
        assert!(read_json(&main.cli_dir.join("settings.json")).unwrap().get("model").is_none());

        put(&work.cli_dir.join("settings.json"), "{ broken");
        assert!(set(&roots, &config, Setting::Theme, Some(json!("light"))).is_err());
        assert_eq!(std::fs::read_to_string(work.cli_dir.join("settings.json")).unwrap(), "{ broken");
    }
}
