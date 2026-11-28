//! What Settings › Claude Code › Status shows, read from files only (Claude Code is never run):
//! the installed version, the account's sign-in, the settings files in effect, its MCP servers
//! and memory file, and a few counts from its prompt history and transcripts.
//!
//! Every read is bounded: the prompt history's last [`HISTORY_TAIL`] bytes, the last
//! [`TRANSCRIPT_TAIL`] bytes of at most [`MODEL_SESSIONS`] transcripts, and files of the
//! account's folder up to [`util::APP_FILE_LIMIT`](crate::util::APP_FILE_LIMIT).
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::util::{APP_FILE_LIMIT, read_bounded};
use crate::{Account, Roots};

/// How much of `history.jsonl` (one line per prompt) is read, from its end.
pub const HISTORY_TAIL: u64 = 8 * 1024 * 1024;
/// How much of each transcript is read, from its end, for the models used.
pub const TRANSCRIPT_TAIL: u64 = 64 * 1024;
/// How many of the newest sessions are read for the models used.
pub const MODEL_SESSIONS: usize = 40;

/// How the account signs in to Claude Code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignIn {
    /// A claude.ai login (`.credentials.json` and `oauthAccount`).
    ClaudeAccount,
    /// An `apiKeyHelper` in `settings.json`.
    KeyHelper,
    /// An API key Claude Code saved in `.claude.json` (`primaryApiKey`).
    ApiKey,
    None,
}

impl SignIn {
    pub fn label(self) -> &'static str {
        match self {
            SignIn::ClaudeAccount => "Claude account",
            SignIn::KeyHelper => "API key helper",
            SignIn::ApiKey => "API key",
            SignIn::None => "Not signed in",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The newest version the native installer keeps (`~/.local/share/claude/versions`).
    pub installed: Option<String>,
    pub sign_in: SignIn,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
    /// The keys of the user `settings.json`; `None` without one, `Some(Err)` when unreadable.
    pub user_settings: Option<Result<usize, ()>>,
    /// A machine-wide `managed-settings.json` exists.
    pub managed: Option<PathBuf>,
    /// User-scope MCP servers in `.claude.json`: name and transport.
    pub mcp: Vec<(String, String)>,
    /// Lines of the user memory file `CLAUDE.md`, when there is one.
    pub memory: Option<usize>,
}

fn str_at(v: &Value, path: &[&str]) -> Option<String> {
    let mut node = v;
    for key in path {
        node = node.get(key)?;
    }
    node.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn json(path: &Path) -> Option<Result<Value, ()>> {
    match read_bounded(path, APP_FILE_LIMIT) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(|_| ())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Err(())),
    }
}

/// The newest version folder's name, by its numbers.
fn newest_version(dir: &Path) -> Option<String> {
    let numbers = |name: &str| -> Option<Vec<u64>> { name.split('.').map(|p| p.parse().ok()).collect() };
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some((numbers(&name)?, name)))
        .max()
        .map(|(_, name)| name)
}

/// The machine-wide managed settings file Claude Code reads on Windows, when it exists.
fn managed_settings() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramFiles")?;
    Some(PathBuf::from(base).join("ClaudeCode").join("managed-settings.json")).filter(|p| p.is_file())
}

/// The account's status. Never fails: what cannot be read is left out.
pub fn status(roots: &Roots, account: &Account) -> Status {
    let global = json(&crate::accounts::claude_json(roots, account)).and_then(Result::ok).unwrap_or(Value::Null);
    let settings = json(&account.cli_dir.join("settings.json"));
    let logged_in = account.cli_dir.join(".credentials.json").exists() && global.get("oauthAccount").is_some();
    let sign_in = if logged_in {
        SignIn::ClaudeAccount
    } else if settings.as_ref().and_then(|s| s.as_ref().ok()).is_some_and(|s| s.get("apiKeyHelper").is_some()) {
        SignIn::KeyHelper
    } else if global.get("primaryApiKey").is_some() {
        SignIn::ApiKey
    } else {
        SignIn::None
    };
    let mcp = global
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|servers| {
            servers
                .iter()
                .map(|(name, s)| {
                    let kind = str_at(s, &["type"])
                        .unwrap_or_else(|| if s.get("url").is_some() { "http" } else { "stdio" }.to_owned());
                    (name.clone(), kind)
                })
                .collect()
        })
        .unwrap_or_default();
    let memory = read_bounded(&account.cli_dir.join("CLAUDE.md"), APP_FILE_LIMIT)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).lines().count());
    Status {
        installed: newest_version(&roots.home.join(".local").join("share").join("claude").join("versions")),
        sign_in,
        email: str_at(&global, &["oauthAccount", "emailAddress"]),
        organization: str_at(&global, &["oauthAccount", "organizationName"]),
        plan: str_at(&global, &["oauthAccount", "planDisplayName"]),
        user_settings: settings.map(|s| s.map(|v| v.as_object().map_or(0, |m| m.len()))),
        managed: managed_settings(),
        mcp,
        memory,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Prompts in the history read.
    pub prompts: usize,
    /// Sessions those prompts went to.
    pub sessions: usize,
    /// Local days with at least one prompt.
    pub days: usize,
    /// Folders the prompts were typed in.
    pub folders: usize,
    /// The history was longer than what was read: the counts are of its newest part.
    pub partial: bool,
    /// Models that answered in the newest sessions, with how many of those sessions each did,
    /// most first.
    pub models: Vec<(String, usize)>,
    /// How many sessions were read for `models`.
    pub model_sessions: usize,
}

/// The last `max` bytes of a file, from the first whole line, and whether it was cut.
fn tail(path: &Path, max: u64) -> Option<(Vec<u8>, bool)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).ok()?;
    if start > 0 {
        let first = bytes.iter().position(|&b| b == b'\n').map_or(bytes.len(), |i| i + 1);
        bytes.drain(..first);
    }
    Some((bytes, start > 0))
}

/// Counts from the account's prompt history, and the models of the newest sessions in it whose
/// transcript `transcript` finds. `offset_minutes` makes days local.
pub fn stats(account: &Account, offset_minutes: i32, transcript: impl Fn(&str) -> Option<PathBuf>) -> Stats {
    let mut stats = Stats::default();
    let Some((bytes, partial)) = tail(&account.cli_dir.join("history.jsonl"), HISTORY_TAIL) else { return stats };
    stats.partial = partial;
    let (mut sessions, mut days, mut folders) = (Vec::new(), HashSet::new(), HashSet::new());
    for line in bytes.split(|&b| b == b'\n') {
        let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
        let Some(ms) = v.get("timestamp").and_then(Value::as_i64) else { continue };
        stats.prompts += 1;
        days.insert((ms + i64::from(offset_minutes) * 60_000).div_euclid(86_400_000));
        if let Some(folder) = v.get("project").and_then(Value::as_str) {
            folders.insert(folder.to_owned());
        }
        if let Some(id) = v.get("sessionId").and_then(Value::as_str) {
            // Newest last; a session seen again moves to the end.
            sessions.retain(|s: &String| s != id);
            sessions.push(id.to_owned());
        }
    }
    (stats.sessions, stats.days, stats.folders) = (sessions.len(), days.len(), folders.len());
    let mut models: BTreeMap<String, usize> = BTreeMap::new();
    for id in sessions.iter().rev().filter_map(|id| transcript(id)).take(MODEL_SESSIONS) {
        let Some((bytes, _)) = tail(&id, TRANSCRIPT_TAIL) else { continue };
        stats.model_sessions += 1;
        let mut seen = HashSet::new();
        for line in bytes.split(|&b| b == b'\n') {
            // Only assistant lines name a model; skip the rest without parsing them.
            if !line.windows(9).any(|w| w == b"assistant") {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
            if let Some(model) = str_at(&v, &["message", "model"]).filter(|m| !m.starts_with('<')) {
                seen.insert(model);
            }
        }
        for model in seen {
            *models.entry(model).or_default() += 1;
        }
    }
    let mut models: Vec<_> = models.into_iter().collect();
    models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    stats.models = models;
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, config_with, put};
    use serde_json::json;

    #[test]
    fn status_reads_the_account_files_without_opening_credentials() {
        let t = TempDir::new("cc-status");
        let roots = t.roots();
        let config = config_with(&roots, &[("work", "Work")]);
        let work = &config.accounts[1];
        assert_eq!(status(&roots, work).sign_in, SignIn::None);
        put(&work.cli_dir.join(".credentials.json"), "not json: never read");
        put(
            &crate::accounts::claude_json(&roots, work),
            &json!({
                "oauthAccount": {"emailAddress": "w@example.invalid", "organizationName": "Org", "planDisplayName": "Max"},
                "mcpServers": {"files": {"command": "x"}, "web": {"type": "http", "url": "https://example.invalid"}}
            })
            .to_string(),
        );
        put(&work.cli_dir.join("settings.json"), r#"{"model":"opus","theme":"dark"}"#);
        put(&work.cli_dir.join("CLAUDE.md"), "one\ntwo\n");
        for v in ["2.1.9", "2.1.10", "notes"] {
            std::fs::create_dir_all(roots.home.join(".local/share/claude/versions").join(v)).unwrap();
        }
        let s = status(&roots, work);
        assert_eq!(s.sign_in, SignIn::ClaudeAccount);
        assert_eq!(s.installed.as_deref(), Some("2.1.10"));
        assert_eq!((s.email.as_deref(), s.plan.as_deref()), (Some("w@example.invalid"), Some("Max")));
        assert_eq!(s.user_settings, Some(Ok(2)));
        assert_eq!(s.mcp, [("files".to_owned(), "stdio".to_owned()), ("web".to_owned(), "http".to_owned())]);
        assert_eq!(s.memory, Some(2));
    }

    #[test]
    fn stats_count_the_history_and_the_models_of_the_newest_sessions() {
        let t = TempDir::new("cc-stats");
        let roots = t.roots();
        let config = config_with(&roots, &[]);
        let main = &config.accounts[0];
        let day = 86_400_000_i64;
        let lines: Vec<String> =
            [(0, "a", "C:\\x"), (1000, "a", "C:\\x"), (day + 5, "b", "C:\\y"), (2 * day, "a", "C:\\x")]
                .iter()
                .map(|(ms, id, folder)| {
                    json!({"display": "hi", "timestamp": ms, "project": folder, "sessionId": id}).to_string()
                })
                .chain(["not json".to_owned()])
                .collect();
        put(&main.cli_dir.join("history.jsonl"), &lines.join("\n"));
        let dir = t.0.join("projects");
        put(
            &dir.join("a.jsonl"),
            &[
                json!({"type": "assistant", "message": {"model": "claude-opus-5-5"}}),
                json!({"type": "assistant", "message": {"model": "<synthetic>"}}),
                json!({"type": "assistant", "message": {"model": "claude-opus-5-5"}}),
            ]
            .map(|v| v.to_string())
            .join("\n"),
        );
        put(&dir.join("b.jsonl"), &json!({"type": "assistant", "message": {"model": "claude-haiku-5-5"}}).to_string());
        let s = stats(main, 0, |id| Some(dir.join(format!("{id}.jsonl"))));
        assert_eq!((s.prompts, s.sessions, s.days, s.folders, s.partial), (4, 2, 3, 2, false));
        assert_eq!(s.models, [("claude-haiku-5-5".to_owned(), 1), ("claude-opus-5-5".to_owned(), 1)]);
        assert_eq!(s.model_sessions, 2);
        // Days are local: a second either side of midnight UTC is one day a minute east of it.
        let around = [day - 1000, day + 1000].map(|ms| json!({"timestamp": ms}).to_string()).join(
            "
",
        );
        put(&main.cli_dir.join("history.jsonl"), &around);
        assert_eq!((stats(main, 0, |_| None).days, stats(main, 1, |_| None).days), (2, 1));
    }
}
