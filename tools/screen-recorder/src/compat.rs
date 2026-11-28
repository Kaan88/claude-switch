//! `recorder compat`: records every script in `scripts/` from the installed Claude Code, has
//! Chat's own readers check each recording against `expect/` (the app's ignored `cli_compat`
//! test), and writes a report. One worker does the work; the screen (`tui`) or plain lines show it.
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::record::{Abort, Helper, Launch, Recorder};

const USAGE: &str = "\
recorder compat [options]     (see README.md)
  --config-dir DIR        the account to record with (its CLAUDE_CONFIG_DIR); without it the
                          signed-in account that was used least recently
  --api-key-env NAME      record with the API key in this variable and a scratch config folder
  --model NAME            default haiku
  --only a,b              only these scripts
  --out DIR               where recordings and the report go
  --retries N             record a script again when its states do not match (default 1)
  --only-if-new-version   do nothing when this Claude Code version was already checked
  --plain | --ci          plain lines instead of the screen (also when output is not a terminal)
  --repo DIR              the claude-switch checkout (default: the one this was built from)";

/// Dollars per million tokens (input, output) by model family; a cache write costs 1.25 times
/// the input price and a cache read a tenth of it. Rough on purpose: the estimate only counts
/// what the transcripts hold.
const PRICES: [(&str, f64, f64); 5] =
    [("haiku-5", 0.10, 0.50), ("haiku-4", 1.0, 5.0), ("sonnet", 2.0, 10.0), ("opus", 4.0, 20.0), ("", 0.10, 0.50)];
/// How many run folders are kept under the default place.
const KEPT_RUNS: usize = 5;

pub struct Options {
    pub repo: PathBuf,
    pub config_dir: Option<PathBuf>,
    pub api_key_env: Option<String>,
    pub model: String,
    pub only: Vec<String>,
    pub out: Option<PathBuf>,
    pub retries: u32,
    pub only_if_new: bool,
    pub plain: bool,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        repo: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        config_dir: None,
        api_key_env: None,
        model: "haiku".to_owned(),
        only: Vec::new(),
        out: None,
        retries: 1,
        only_if_new: false,
        plain: !std::io::stdout().is_terminal(),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--config-dir" => o.config_dir = Some(value()?.into()),
            "--api-key-env" => o.api_key_env = Some(value()?),
            "--model" => o.model = value()?,
            "--only" => o.only = value()?.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
            "--out" => o.out = Some(value()?.into()),
            "--retries" => o.retries = value()?.parse().map_err(|_| "--retries needs a number".to_owned())?,
            "--repo" => o.repo = value()?.into(),
            "--only-if-new-version" => o.only_if_new = true,
            "--plain" | "--ci" => o.plain = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

/// One script with what its `#:` header lines say.
#[derive(Clone)]
pub struct Script {
    pub name: String,
    pub body: String,
    /// More arguments for `claude`; `{scripts}` is the scripts folder.
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// About how long it takes, for the time left.
    pub seconds: u64,
}

fn scripts(dir: &Path) -> Result<Vec<Script>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    files.sort();
    let folder = dir.to_string_lossy().replace('\\', "/");
    let mut out = Vec::new();
    for file in files {
        let body = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut script = Script {
            name: file.file_stem().unwrap().to_string_lossy().into_owned(),
            args: Vec::new(),
            cols: 100,
            rows: 40,
            seconds: 45,
            body: String::new(),
        };
        for line in body.lines().filter_map(|l| l.strip_prefix("#:")) {
            let (key, value) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            let number = |what: &str| value.trim().parse::<u64>().map_err(|_| format!("{}: {what}", file.display()));
            match key {
                "args" => script.args = value.split_whitespace().map(|a| a.replace("{scripts}", &folder)).collect(),
                "seconds" => script.seconds = number("#: seconds N")?,
                "size" => {
                    let (cols, rows) =
                        value.trim().split_once(' ').ok_or(format!("{}: #: size C R", file.display()))?;
                    script.cols = cols.parse().map_err(|_| "columns".to_owned())?;
                    script.rows = rows.parse().map_err(|_| "rows".to_owned())?;
                }
                _ => return Err(format!("{}: unknown header `#: {key}`", file.display())),
            }
        }
        script.body = body;
        out.push(script);
    }
    Ok(out)
}

#[derive(Clone, PartialEq)]
pub enum Status {
    Queued,
    Recording,
    Checking,
    Pass,
    Fail,
    Skipped(String),
    Error(String),
}

#[derive(Clone)]
pub struct Diff {
    pub key: String,
    pub expected: String,
    pub got: String,
}

#[derive(Clone)]
pub struct StateResult {
    pub name: String,
    pub diffs: Vec<Diff>,
}

pub struct Row {
    pub script: Script,
    pub status: Status,
    pub attempt: u32,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
    pub states: Vec<StateResult>,
    /// Why the recording ended early, if it did.
    pub note: String,
}

impl Row {
    pub fn finished(&self) -> bool {
        !matches!(self.status, Status::Queued | Status::Recording | Status::Checking)
    }

    pub fn failed(&self) -> Vec<&StateResult> {
        self.states.iter().filter(|s| !s.diffs.is_empty()).collect()
    }
}

#[derive(Clone, Default)]
pub struct Cost {
    pub requests: usize,
    pub dollars: f64,
}

pub struct Shared {
    pub rows: Vec<Row>,
    pub queue: VecDeque<usize>,
    /// What the worker is doing, for the header.
    pub phase: String,
    pub version: String,
    pub last_tested: Option<String>,
    pub account: String,
    pub model: String,
    pub started: Instant,
    /// How long the run took once every script was done.
    pub took: Option<Duration>,
    pub cost: Cost,
    pub out: PathBuf,
    pub report: Option<PathBuf>,
    /// Why nothing (more) can run.
    pub fatal: Option<String>,
    /// Nothing queued and nothing running: the report is written.
    pub done: bool,
    /// Lines for the plain output.
    pub lines: Vec<String>,
    /// The app's test program, once built.
    pub check: Option<PathBuf>,
}

impl Shared {
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |f: fn(&Status) -> bool| self.rows.iter().filter(|r| f(&r.status)).count();
        (
            count(|s| *s == Status::Pass),
            count(|s| matches!(s, Status::Fail | Status::Error(_))),
            count(|s| matches!(s, Status::Skipped(_))),
        )
    }

    /// Seconds the queued and running scripts are expected to take still.
    pub fn left(&self) -> u64 {
        self.rows
            .iter()
            .map(|row| match row.status {
                Status::Queued => row.script.seconds,
                Status::Recording | Status::Checking => {
                    row.script.seconds.saturating_sub(row.started.map_or(0, |s| s.elapsed().as_secs()))
                }
                _ => 0,
            })
            .sum()
    }
}

/// The lock, also after a thread panicked while it held it: the run's state is only shown and
/// reported, and the screen and the clean-up must still get to it.
pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Run {
    pub shared: Arc<Mutex<Shared>>,
    pub live: Arc<Mutex<Vec<String>>>,
    /// Ends the script being recorded.
    pub stop: Arc<AtomicBool>,
    /// Ends the worker once it is idle.
    pub quit: Arc<AtomicBool>,
}

/// Where everything of one run lives, and how `claude` is started.
struct Setup {
    options: Options,
    /// `%TEMP%\cs-compat`: the work folder and, with an API key, the config folder.
    scratch: PathBuf,
    work: PathBuf,
    /// The transcripts of the work folder, under the account's `projects`.
    project: PathBuf,
    env: Vec<(String, String)>,
    base: PathBuf,
}

pub fn mmss(time: Duration) -> String {
    format!("{}:{:02}", time.as_secs() / 60, time.as_secs() % 60)
}

/// The folder Claude Code keeps a work folder's transcripts in: every character that is not a
/// letter or digit becomes `-`.
fn project_name(work: &Path) -> String {
    work.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn local_data() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("claude-switch-cli-compat")
}

/// `YYYYMMDD-HHMMSS` (UTC) from the clock.
fn stamp() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86400, secs % 86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn claude_version() -> Result<String, String> {
    let output = if cfg!(windows) {
        Command::new("cmd").args(["/c", "claude", "--version"]).output()
    } else {
        Command::new("claude").arg("--version").output()
    };
    let output = output.map_err(|e| format!("claude --version: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().next().unwrap_or_default();
    if version.is_empty() {
        Err("claude --version printed nothing: is Claude Code installed?".to_owned())
    } else {
        Ok(version.to_owned())
    }
}

/// The signed-in accounts on this machine: `~/.claude` and `~/.claude-accounts/<name>/cli`, each
/// with when its `projects` folder was last written.
fn accounts() -> Vec<(PathBuf, SystemTime)> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) else {
        return Vec::new();
    };
    let mut dirs = vec![home.join(".claude")];
    if let Ok(entries) = std::fs::read_dir(home.join(".claude-accounts")) {
        dirs.extend(entries.filter_map(Result::ok).map(|e| e.path().join("cli")));
    }
    dirs.into_iter()
        .filter(|dir| dir.join(".credentials.json").is_file())
        .map(|dir| {
            let newest = std::fs::read_dir(dir.join("projects"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (dir, newest)
        })
        .collect()
}

/// "idiot" for `…\.claude-accounts\idiot\cli`, else the folder's own name.
fn account_name(dir: &Path) -> String {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match dir.parent() {
        Some(parent) if name(dir) == "cli" => name(parent),
        _ => name(dir),
    }
}

fn setup(options: Options) -> Result<(Setup, String), String> {
    let scratch = std::env::temp_dir().join("cs-compat");
    let work = scratch.join("work");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let (config, account, mut env) = if let Some(name) = &options.api_key_env {
        let key = std::env::var(name).ok().filter(|k| !k.trim().is_empty()).ok_or(format!("{name} is not set"))?;
        let key = key.trim().to_owned();
        // A config folder of its own that says: the first-run screens are done, and this key
        // (Claude Code keeps its last 20 characters) may be used. Without it the CLI asks.
        let config = scratch.join("config");
        std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
        let tail: String = key.chars().skip(key.chars().count().saturating_sub(20)).collect();
        let json = serde_json::json!({
            "hasCompletedOnboarding": true,
            "customApiKeyResponses": { "approved": [tail], "rejected": [] },
        });
        std::fs::write(config.join(".claude.json"), json.to_string()).map_err(|e| e.to_string())?;
        (config, "API key".to_owned(), vec![("ANTHROPIC_API_KEY".to_owned(), key)])
    } else {
        let config = match &options.config_dir {
            Some(dir) => dir.clone(),
            None => {
                let mut found = accounts();
                found.sort_by_key(|(_, used)| *used);
                found.into_iter().next().map(|(dir, _)| dir).ok_or("no signed-in account found: pass --config-dir")?
            }
        };
        if !config.is_dir() {
            return Err(format!("{} is not a folder", config.display()));
        }
        let name = account_name(&config);
        (config, name, Vec::new())
    };
    env.push(("CLAUDE_CONFIG_DIR".to_owned(), config.to_string_lossy().into_owned()));
    let project = config.join("projects").join(project_name(&work));
    let base = local_data();
    Ok((Setup { options, scratch, work, project, env, base }, account))
}

/// Folders of an account in which Claude Code keeps files named after a session.
const SESSION_FOLDERS: [&str; 5] = ["plans", "session-env", "file-history", "todos", "tasks"];

/// Removes the scratch folder and what the run left in the account's folder: the transcripts
/// (only the `projects` folder named after the scratch work folder) and the files named after
/// those sessions (a plan, a session's environment and file history).
fn clean(setup: &Setup) {
    if setup.project.file_name().is_some_and(|n| n.to_string_lossy().ends_with("-cs-compat-work")) {
        let sessions: Vec<String> = std::fs::read_dir(&setup.project)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|id| id.len() == 36)
            .collect();
        if let Some(config) = setup.project.parent().and_then(Path::parent) {
            for folder in SESSION_FOLDERS {
                for entry in std::fs::read_dir(config.join(folder)).into_iter().flatten().filter_map(Result::ok) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if sessions.iter().any(|id| name.contains(id.as_str())) {
                        let path = entry.path();
                        let _ =
                            if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&setup.project);
    }
    let _ = std::fs::remove_dir_all(&setup.scratch);
}

/// What the transcripts of the scratch project say the run used.
fn cost(project: &Path) -> Cost {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(project, &mut files);
    // A message is written once per content block, each time with its usage so far: the last wins.
    let mut messages: HashMap<String, f64> = HashMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for line in text.lines() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let message = &entry["message"];
            let (Some(id), Some(usage)) = (message["id"].as_str(), message["usage"].as_object()) else { continue };
            let model = message["model"].as_str().unwrap_or_default();
            let (_, input, output) = PRICES.iter().find(|(family, ..)| model.contains(family)).unwrap();
            let tokens = |key: &str| usage.get(key).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
            let dollars = (tokens("input_tokens") * input
                + tokens("cache_creation_input_tokens") * input * 1.25
                + tokens("cache_read_input_tokens") * input * 0.1
                + tokens("output_tokens") * output)
                / 1e6;
            messages.insert(id.to_owned(), dollars);
        }
    }
    Cost { requests: messages.len(), dollars: messages.values().sum::<f64>().max(0.0) }
}

/// Runs a command to its end with its output in `log` (its standard output in `out` when one is
/// given), unless asked to stop.
fn run_logged(mut command: Command, log: &Path, out: Option<&Path>, stop: &AtomicBool) -> Result<bool, String> {
    let create = |path: &Path| std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()));
    let errors = create(log)?;
    let output = match out {
        Some(path) => create(path)?,
        None => errors.try_clone().map_err(|e| e.to_string())?,
    };
    let name = command.get_program().to_string_lossy().into_owned();
    let mut child =
        command.stdin(Stdio::null()).stdout(output).stderr(errors).spawn().map_err(|e| format!("{name}: {e}"))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.success());
        }
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The API key goes to `claude` alone: never to cargo, the build scripts and proc macros it
/// runs, or the check's test program.
fn without_key(command: &mut Command, setup: &Setup) {
    command.env_remove("ANTHROPIC_API_KEY");
    if let Some(name) = &setup.options.api_key_env {
        command.env_remove(name);
    }
}

/// `cargo test --no-run` for the app, with the lock file as it is and without the API key.
fn build_command(setup: &Setup) -> Command {
    let mut command = Command::new("cargo");
    command.arg("test").arg("--locked").arg("--manifest-path").arg(setup.options.repo.join("Cargo.toml"));
    command.args(["-p", "claude-switch", "--no-run", "--message-format=json-render-diagnostics"]);
    without_key(&mut command, setup);
    command
}

/// Builds the app's tests and gives the test program: the check runs it directly, so each check
/// takes a moment and a source file changed meanwhile cannot break a run half way.
fn build_check(setup: &Setup, out: &Path, stop: &AtomicBool) -> Result<PathBuf, String> {
    let command = build_command(setup);
    let (log, messages) = (out.join("build.log"), out.join("build.json"));
    let failed = || format!("the check does not build (see {}):\n{}", log.display(), tail(&log, 8));
    if !run_logged(command, &log, Some(&messages), stop)? {
        return Err(failed());
    }
    let text = std::fs::read_to_string(&messages).unwrap_or_default();
    let _ = std::fs::remove_file(&messages);
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["profile"]["test"] == true)
        .filter(|m| m["target"]["name"] == "claude-switch")
        .filter_map(|m| m["executable"].as_str().map(PathBuf::from))
        .next_back()
        .ok_or_else(failed)
}

fn tail(log: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Chat's readers over one script's recordings: the app's `cli_compat` test, and its summary.
fn check(setup: &Setup, run: &Run, out: &Path, script: &str) -> Result<Vec<StateResult>, String> {
    let summary = out.join(script).join("check.json");
    let log = out.join(script).join("check.log");
    let _ = std::fs::remove_file(&summary);
    let program = lock(&run.shared).check.clone().ok_or("the check was not built")?;
    let mut command = Command::new(program);
    without_key(&mut command, setup);
    command.args(["cli_compat", "--ignored", "--nocapture"]);
    command.env("CS_COMPAT_RECORDINGS", out);
    command.env("CS_COMPAT_EXPECT", setup.options.repo.join("tools/screen-recorder/expect"));
    command.env("CS_COMPAT_ONLY", script).env("CS_COMPAT_SUMMARY", &summary);
    run_logged(command, &log, None, &run.stop)?;
    let text = std::fs::read_to_string(&summary)
        .map_err(|_| format!("the check did not run (see {}):\n{}", log.display(), tail(&log, 6)))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", summary.display()))?;
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    Ok(json["states"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|state| StateResult {
            name: text(&state["state"]),
            diffs: state["diffs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| Diff { key: text(&d["key"]), expected: text(&d["expected"]), got: text(&d["got"]) })
                .collect(),
        })
        .collect())
}

fn say(run: &Run, line: String) {
    lock(&run.shared).lines.push(line);
}

/// Records one script and checks it; the row holds the result.
fn run_script(setup: &Setup, run: &Run, i: usize) {
    let (script, out, n, total) = {
        let mut shared = lock(&run.shared);
        shared.phase = "Recording".to_owned();
        let row = &mut shared.rows[i];
        row.status = Status::Recording;
        row.attempt += 1;
        row.started = Some(Instant::now());
        row.took = None;
        row.states.clear();
        row.note.clear();
        let script = row.script.clone();
        // Its place among the scripts this run records.
        let wanted = |r: &Row| !matches!(&r.status, Status::Skipped(why) if why == "not selected");
        let n = shared.rows[..=i].iter().filter(|r| wanted(r)).count();
        (script, shared.out.clone(), n, shared.rows.iter().filter(|r| wanted(r)).count())
    };
    let name = script.name.clone();
    say(run, format!("[{n}/{total}] {name}: recording"));
    lock(&run.live).clear();
    let finish = |status: Status, states: Vec<StateResult>, note: String| {
        let mut shared = lock(&run.shared);
        let row = &mut shared.rows[i];
        row.took = row.started.map(|s| s.elapsed());
        (row.status, row.states, row.note) = (status, states, note);
    };
    // An empty work folder and an empty folder for the recordings, each time.
    let _ = std::fs::remove_dir_all(&setup.work);
    let dir = out.join(&name);
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&setup.work).and_then(|()| std::fs::create_dir_all(&dir)) {
        return finish(Status::Error(format!("scratch folders: {e}")), Vec::new(), String::new());
    }
    let mut args = vec!["--model".to_owned(), setup.options.model.clone()];
    args.extend(script.args.iter().cloned());
    let launch = Launch {
        program: "claude",
        cwd: &setup.work,
        cols: script.cols,
        rows: script.rows,
        args: &args,
        env: &setup.env,
        plain: false,
    };
    let mut recorder = match Recorder::spawn(&launch, dir) {
        Ok(recorder) => recorder,
        Err(e) => return finish(Status::Error(format!("could not start claude: {e}")), Vec::new(), String::new()),
    };
    recorder.strict = true;
    recorder.stop = Some(run.stop.clone());
    recorder.live = Some(run.live.clone());
    recorder.project = Some(setup.project.clone());
    // The app's own code for a branch: its test program, which never sees the key.
    let remove = std::iter::once("ANTHROPIC_API_KEY".to_owned()).chain(setup.options.api_key_env.clone()).collect();
    recorder.helper = lock(&run.shared).check.clone().map(|program| Helper { program, remove });
    let note = match recorder.run(&script.body) {
        Ok(()) => String::new(),
        Err(Abort::Stopped) => return finish(Status::Skipped("stopped".to_owned()), Vec::new(), String::new()),
        Err(why @ (Abort::Start(_) | Abort::Script(_) | Abort::Io(_))) => {
            return finish(Status::Error(why.to_string()), Vec::new(), String::new());
        }
        Err(why @ Abort::Timeout(_)) => why.to_string(),
    };
    {
        let mut shared = lock(&run.shared);
        shared.rows[i].status = Status::Checking;
        shared.phase = "Checking".to_owned();
        shared.cost = cost(&setup.project);
    }
    match check(setup, run, &out, &name) {
        Ok(states) => {
            let bad = states.iter().filter(|s| !s.diffs.is_empty()).count();
            for state in &states {
                if state.diffs.is_empty() {
                    say(run, format!("ok        {name}/{}", state.name));
                }
                for d in &state.diffs {
                    say(
                        run,
                        format!(
                            "MISMATCH  {name}/{}  {}: expected {:?}, got {:?}",
                            state.name, d.key, d.expected, d.got
                        ),
                    );
                }
            }
            if !note.is_empty() {
                say(run, format!("          {name}: {note}"));
            }
            say(run, format!("[{n}/{total}] {name}: {} of {} states as expected", states.len() - bad, states.len()));
            finish(if bad == 0 { Status::Pass } else { Status::Fail }, states, note);
        }
        Err(why) if why == "stopped" => finish(Status::Skipped(why), Vec::new(), note),
        Err(why) => {
            say(run, format!("[{n}/{total}] {name}: {why}"));
            finish(Status::Error(why), Vec::new(), note);
        }
    }
}

/// The report goes into an issue and the run's summary on GitHub, and what it quotes comes from
/// outside (the screens read, error text, log tails): none of it may end a code span or block,
/// start Markdown of its own or mention anyone.
mod markdown {
    /// The longest run of backticks in `text`.
    fn ticks(text: &str) -> usize {
        text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
    }

    /// `text` as inline code that nothing in it can end: a fence longer than any backtick run in
    /// it, and its line breaks shown as ⏎. GitHub mentions no one inside code.
    pub fn code(text: &str) -> String {
        if text.is_empty() {
            return "(nothing)".to_owned();
        }
        let text = text.replace("\r\n", "⏎").replace(['\n', '\r'], "⏎");
        let fence = "`".repeat(ticks(&text) + 1);
        // Padded on both sides: CommonMark takes one space off each, so text that starts or
        // ends with a backtick or a space stays as it is.
        format!("{fence} {text} {fence}")
    }

    /// `text` as a fenced block, each line indented by `indent` (to sit under a list item).
    pub fn block(text: &str, indent: &str) -> String {
        let fence = "`".repeat(ticks(text).max(2) + 1);
        let mut s = format!("{indent}{fence}text\n");
        for line in text.lines() {
            s.push_str(indent);
            s.push_str(line);
            s.push('\n');
        }
        s.push_str(indent);
        s.push_str(&fence);
        s.push('\n');
        s
    }

    /// `text` as plain words on one line: Markdown's marks escaped and every `@` kept from
    /// mentioning anyone.
    pub fn plain(text: &str) -> String {
        let mut s = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '\n' | '\r' => s.push(' '),
                '@' => s.push_str("@\u{200b}"),
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '#' | '!' | '~' | '&' => {
                    s.push('\\');
                    s.push(c);
                }
                c => s.push(c),
            }
        }
        s
    }
}

/// The run's outcome in one line, as plain text.
fn verdict(shared: &Shared) -> String {
    let (_, fail, skipped) = shared.counts();
    let states: usize = shared.rows.iter().map(|r| r.states.len()).sum();
    let bad: usize = shared.rows.iter().map(|r| r.failed().len()).sum();
    match (&shared.fatal, fail) {
        (Some(why), _) => format!("The check could not run: {}", why.lines().next().unwrap_or_default()),
        (None, 0) if skipped == 0 => format!("All {states} states read as expected."),
        (None, 0) => format!("{states} states read as expected; {skipped} scripts were skipped."),
        (None, _) => {
            format!("{bad} of {states} states do not read as expected ({fail} of {} scripts).", shared.rows.len())
        }
    }
}

fn report(shared: &Shared) -> String {
    use markdown::{block, code, plain};
    let (pass, fail, skipped) = shared.counts();
    let bad: usize = shared.rows.iter().map(|r| r.failed().len()).sum();
    let mut s = format!("# Chat and Claude Code {}\n\n", plain(&shared.version));
    let _ = writeln!(s, "**{}**\n", plain(&verdict(shared)));
    if let Some(why) = shared.fatal.as_ref().filter(|why| why.lines().count() > 1) {
        let _ = writeln!(s, "{}", block(why, ""));
    }
    let _ = writeln!(
        s,
        "- Claude Code: {} (last checked: {})",
        plain(&shared.version),
        plain(shared.last_tested.as_deref().unwrap_or("never"))
    );
    let _ = writeln!(s, "- Account: {} · model: {}", plain(&shared.account), plain(&shared.model));
    let took = shared.took.unwrap_or_else(|| shared.started.elapsed());
    let _ = writeln!(
        s,
        "- Time: {} · model requests in the transcripts: {} · estimated cost: ${:.3}",
        mmss(took),
        shared.cost.requests,
        shared.cost.dollars
    );
    let _ = writeln!(s, "- Scripts: {pass} passed, {fail} failed, {skipped} skipped");
    let _ = writeln!(s, "- Recordings: {}", code(&shared.out.display().to_string()));
    if bad > 0 || shared.rows.iter().any(|r| matches!(r.status, Status::Error(_))) {
        s.push_str("\n## What no longer matches\n\n");
        for row in &shared.rows {
            let name = &row.script.name;
            if let Status::Error(why) = &row.status {
                let _ = write!(s, "- {}: could not record or check it\n\n{}", code(name), block(why, "  "));
            }
            for state in row.failed() {
                let (at, screen) = (format!("{name}/{}", state.name), format!("{name}/{}.txt", state.name));
                let _ = writeln!(s, "- {} (screen: {})", code(&at), code(&screen));
                for d in &state.diffs {
                    let _ = writeln!(s, "  - {}: expected {}, read {}", code(&d.key), code(&d.expected), code(&d.got));
                }
            }
            if !row.note.is_empty() {
                let screen = format!("{name}/aborted.txt");
                let _ = write!(
                    s,
                    "- {}: ended early (screen: {})\n\n{}",
                    code(name),
                    code(&screen),
                    block(&row.note, "  ")
                );
            }
        }
    }
    s.push_str("\n## Scripts\n\n| Script | Result | States | Tries | Time |\n| --- | --- | --- | --- | --- |\n");
    for row in &shared.rows {
        let result = match &row.status {
            Status::Pass => "pass".to_owned(),
            Status::Fail => "FAIL".to_owned(),
            Status::Skipped(why) => format!("skipped ({})", plain(why)),
            Status::Error(_) => "ERROR".to_owned(),
            _ => "not run".to_owned(),
        };
        let ok = row.states.len() - row.failed().len();
        let time = row.took.map(mmss).unwrap_or_default();
        let name = plain(&row.script.name);
        let _ = writeln!(s, "| {name} | {result} | {ok}/{} | {} | {time} |", row.states.len(), row.attempt);
    }
    s.push_str(
        "\nHow to read this and what to do next: `tools/screen-recorder/README.md`, \"After a Claude Code update\".\n",
    );
    s
}

fn summary(shared: &Shared) -> serde_json::Value {
    let (pass, fail, skipped) = shared.counts();
    let states: Vec<_> = shared
        .rows
        .iter()
        .flat_map(|row| {
            row.states.iter().map(|state| {
                let diffs: Vec<_> = state
                    .diffs
                    .iter()
                    .map(|d| serde_json::json!({ "key": d.key, "expected": d.expected, "got": d.got }))
                    .collect();
                serde_json::json!({ "script": row.script.name, "state": state.name, "ok": diffs.is_empty(), "diffs": diffs })
            })
        })
        .collect();
    serde_json::json!({
        "claude_code": shared.version,
        "last_checked": shared.last_tested,
        "scripts": { "passed": pass, "failed": fail, "skipped": skipped },
        "fatal": shared.fatal,
        "requests": shared.cost.requests,
        "estimated_cost_usd": shared.cost.dollars,
        "states": states,
    })
}

/// Everything queued is done: the report, the summary, and the version as checked when the whole
/// set ran.
fn wrap_up(setup: &Setup, run: &Run) {
    let mut shared = lock(&run.shared);
    shared.cost = cost(&setup.project);
    shared.took = Some(shared.started.elapsed());
    shared.phase = "Done".to_owned();
    let path = shared.out.join("report.md");
    let _ = std::fs::write(&path, report(&shared));
    let _ = std::fs::write(shared.out.join("summary.json"), serde_json::to_string_pretty(&summary(&shared)).unwrap());
    shared.report = Some(path);
    let (_, fail, skipped) = shared.counts();
    if shared.fatal.is_none() && skipped == 0 && !run.stop.load(Ordering::Relaxed) {
        let _ = std::fs::create_dir_all(&setup.base);
        let line = format!("{}\n{} failed, checked {} UTC\n", shared.version, fail, stamp());
        let _ = std::fs::write(setup.base.join("last-checked.txt"), line);
    }
    shared.done = true;
}

fn worker(setup: &Setup, run: &Run) {
    // The check's test program first: a build error is found before any model request is made.
    {
        lock(&run.shared).phase = "Building the check (cargo test --no-run)".to_owned();
        say(run, "building the check".to_owned());
        let out = lock(&run.shared).out.clone();
        let built = build_check(setup, &out, &run.stop);
        let mut shared = lock(&run.shared);
        match built {
            Ok(program) => shared.check = Some(program),
            Err(why) => {
                shared.queue.clear();
                for row in shared.rows.iter_mut().filter(|r| r.status == Status::Queued) {
                    row.status = Status::Skipped("the check did not build".to_owned());
                }
                shared.fatal = Some(why);
            }
        }
    }
    let mut started = false;
    loop {
        let next = lock(&run.shared).queue.pop_front();
        let Some(i) = next else {
            if !lock(&run.shared).done {
                wrap_up(setup, run);
            }
            if run.quit.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if run.stop.load(Ordering::Relaxed) {
            lock(&run.shared).rows[i].status = Status::Skipped("stopped".to_owned());
            continue;
        }
        run_script(setup, run, i);
        let mut shared = lock(&run.shared);
        match shared.rows[i].status.clone() {
            // Claude Code that does not start for the first script will not start for the rest.
            Status::Error(why) if !started => {
                for row in shared.rows.iter_mut().filter(|r| r.status == Status::Queued) {
                    row.status = Status::Skipped("Claude Code did not start".to_owned());
                }
                shared.queue.clear();
                shared.fatal = Some(why);
            }
            Status::Fail if shared.rows[i].attempt <= setup.options.retries => {
                // The model does not always do what a prompt asks: once more before it counts.
                shared.rows[i].status = Status::Queued;
                shared.queue.push_front(i);
                let name = shared.rows[i].script.name.clone();
                shared.lines.push(format!("          {name}: recording it once more"));
                started = true;
            }
            _ => started = true,
        }
    }
    clean(setup);
}

/// 0: every state as expected (or nothing to do); 1: something does not match; 2: could not run.
pub fn main(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let options = match options(args) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("{why}\n{USAGE}");
            return 2;
        }
    };
    let fail = |why: String| {
        eprintln!("{why}");
        2
    };
    let version = match claude_version() {
        Ok(version) => version,
        Err(why) => return fail(why),
    };
    let last_tested = std::fs::read_to_string(local_data().join("last-checked.txt"))
        .ok()
        .and_then(|text| text.lines().next().map(str::to_owned));
    if options.only_if_new && last_tested.as_deref() == Some(version.as_str()) {
        println!("Claude Code {version} was already checked: nothing to do.");
        return 0;
    }
    let dir = options.repo.join("tools/screen-recorder/scripts");
    let all = match scripts(&dir) {
        Ok(all) => all,
        Err(why) => return fail(why),
    };
    if let Some(unknown) = options.only.iter().find(|name| all.iter().all(|s| &s.name != *name)) {
        return fail(format!("no script named {unknown} in {}", dir.display()));
    }
    // A script that does not read ends the run before any model request.
    for script in &all {
        if let Err(why) = crate::record::script_reads(&script.body) {
            return fail(format!("{}: {why}", script.name));
        }
    }
    let (plain, model, only) = (options.plain, options.model.clone(), options.only.clone());
    let out = options.out.clone();
    let (setup, account) = match setup(options) {
        Ok(found) => found,
        Err(why) => return fail(why),
    };
    let out = out.unwrap_or_else(|| {
        // Under the default place only the newest runs are kept.
        let runs = setup.base.join("runs");
        let mut old: Vec<PathBuf> =
            std::fs::read_dir(&runs).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()).collect();
        old.sort();
        for dir in old.iter().rev().skip(KEPT_RUNS - 1) {
            let _ = std::fs::remove_dir_all(dir);
        }
        runs.join(stamp())
    });
    if let Err(e) = std::fs::create_dir_all(&out) {
        return fail(format!("{}: {e}", out.display()));
    }
    let rows: Vec<Row> = all
        .into_iter()
        .map(|script| {
            let wanted = only.is_empty() || only.contains(&script.name);
            let status = if wanted { Status::Queued } else { Status::Skipped("not selected".to_owned()) };
            Row { script, status, attempt: 0, started: None, took: None, states: Vec::new(), note: String::new() }
        })
        .collect();
    let queue = (0..rows.len()).filter(|&i| rows[i].status == Status::Queued).collect();
    let header = format!(
        "Claude Code {version} (last checked: {}) · account {account} · model {model}",
        last_tested.as_deref().unwrap_or("never")
    );
    let run = Run {
        shared: Arc::new(Mutex::new(Shared {
            rows,
            queue,
            phase: "Starting".to_owned(),
            version,
            last_tested,
            account,
            model,
            started: Instant::now(),
            took: None,
            cost: Cost::default(),
            out,
            report: None,
            fatal: None,
            done: false,
            lines: vec![header],
            check: None,
        })),
        live: Arc::new(Mutex::new(Vec::new())),
        stop: Arc::new(AtomicBool::new(false)),
        quit: Arc::new(AtomicBool::new(plain)),
    };
    let worked = std::thread::scope(|scope| {
        let work = scope.spawn(|| worker(&setup, &run));
        if plain {
            let mut printed = 0;
            while !work.is_finished() {
                let lines: Vec<String> = lock(&run.shared).lines[printed..].to_vec();
                printed += lines.len();
                lines.iter().for_each(|line| println!("{line}"));
                std::thread::sleep(Duration::from_millis(100));
            }
            lock(&run.shared).lines[printed..].iter().for_each(|line| println!("{line}"));
        } else if let Err(e) = crate::tui::run(&run, || work.is_finished()) {
            eprintln!("the screen failed: {e}");
            run.stop.store(true, Ordering::Relaxed);
            run.quit.store(true, Ordering::Relaxed);
        }
        work.join()
    });
    if let Err(panic) = worked {
        // The worker stopped half way (its panic is printed above): the scratch folder and the
        // account's transcripts of the run still go, and nothing counts as checked.
        clean(&setup);
        let why =
            panic.downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| panic.downcast_ref::<String>().cloned());
        eprintln!("The recorder stopped unexpectedly: {}", why.unwrap_or_default());
        return 2;
    }
    let shared = lock(&run.shared);
    let (pass, fail, skipped) = shared.counts();
    if plain {
        println!();
    }
    println!("{}", verdict(&shared));
    println!(
        "{pass} passed, {fail} failed, {skipped} skipped · {} · about ${:.3}",
        mmss(shared.took.unwrap_or_default()),
        shared.cost.dollars
    );
    if let Some(path) = &shared.report {
        println!("Report: {}", path.display());
    }
    match (&shared.fatal, fail) {
        (Some(_), _) => 2,
        (None, 0) => 0,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_work_folder_names_its_transcript_folder_as_claude_code_does() {
        let work = Path::new(r"C:\Users\some.one\AppData\Local\Temp\cs-compat\work");
        assert_eq!(project_name(work), "C--Users-some-one-AppData-Local-Temp-cs-compat-work");
    }

    #[test]
    fn an_account_is_named_by_its_folder() {
        assert_eq!(account_name(Path::new("C:/Users/u/.claude-accounts/work/cli")), "work");
        assert_eq!(account_name(Path::new("C:/Users/u/.claude")), ".claude");
    }

    #[test]
    fn every_script_has_its_expectations_and_a_header_that_reads() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let all = scripts(&here.join("scripts")).expect("the scripts");
        assert!(all.len() >= 11);
        for script in &all {
            assert!(here.join("expect").join(format!("{}.expect", script.name)).is_file(), "{}", script.name);
            assert!(script.body.lines().any(|l| l == "start"), "{}", script.name);
            crate::record::script_reads(&script.body).unwrap_or_else(|why| panic!("{}: {why}", script.name));
            assert!(script.args.iter().all(|a| !a.contains("{scripts}")), "{}", script.name);
        }
        let deny = all.iter().find(|s| s.name == "deny").expect("deny");
        let settings = deny.args.last().expect("its settings file");
        assert!(Path::new(settings).is_file(), "{settings}");
    }

    fn api_setup(name: &str) -> Setup {
        let mut options = options(&["--api-key-env".to_owned(), name.to_owned()]).unwrap();
        options.repo = PathBuf::from("repo");
        let none = PathBuf::new();
        Setup { options, scratch: none.clone(), work: none.clone(), project: none.clone(), env: Vec::new(), base: none }
    }

    #[test]
    fn the_api_key_never_reaches_cargo_or_the_check() {
        let setup = api_setup("CS_RECORDER_TEST_KEY");
        let removed = |command: &Command| -> Vec<String> {
            let envs = command.get_envs().filter(|(_, value)| value.is_none());
            envs.map(|(key, _)| key.to_string_lossy().to_ascii_uppercase()).collect()
        };
        let build = build_command(&setup);
        let args: Vec<_> = build.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--locked".to_owned()), "{args:?}");
        for wanted in ["ANTHROPIC_API_KEY", "CS_RECORDER_TEST_KEY"] {
            assert!(removed(&build).contains(&wanted.to_owned()), "cargo keeps {wanted}");
        }
        let mut check = Command::new("check");
        without_key(&mut check, &setup);
        assert_eq!(removed(&check).len(), 2);
    }

    fn workflow() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/cli-compat.yml");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn in_the_workflow_only_the_recording_step_has_the_key_and_the_check_is_built_before_it() {
        let text = workflow();
        let given: Vec<usize> =
            text.match_indices("ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}").map(|m| m.0).collect();
        assert_eq!(given.len(), 1, "one step has the key");
        let built = text.find("cargo test --locked -p claude-switch --no-run").expect("a step that builds the check");
        assert!(built < given[0], "the check is built before the key is there");
        let recording = &text[given[0]..];
        let step_end = recording.find("\n      - ").unwrap_or(recording.len());
        assert!(recording[..step_end].contains("recorder.exe compat"), "the key goes to the recording step");
    }

    /// What GitHub would read as Markdown outside code: the text left once fenced blocks and
    /// code spans are taken out.
    fn outside_code(markdown: &str) -> String {
        let run = |s: &str| s.chars().take_while(|&c| c == '`').count();
        let mut out = String::new();
        let mut open: Option<usize> = None;
        for line in markdown.lines() {
            let start = line.trim_start();
            match open {
                Some(n) if run(start) >= n && start.trim_matches('`').trim().is_empty() => open = None,
                Some(_) => {}
                None if run(start) >= 3 => open = Some(run(start)),
                None => {
                    let mut rest = line;
                    while let Some(at) = rest.find('`') {
                        out.push_str(&rest[..at]);
                        let n = run(&rest[at..]);
                        let after = &rest[at + n..];
                        // The closing run is exactly as long as the opening one.
                        let close = (0..after.len()).find(|&i| {
                            after.is_char_boundary(i) && run(&after[i..]) == n && (i == 0 || !after[..i].ends_with('`'))
                        });
                        let Some(close) = close else { panic!("an unclosed code span: {line}") };
                        rest = &after[close + n..];
                    }
                    out.push_str(rest);
                    out.push('\n');
                }
            }
        }
        assert!(open.is_none(), "an unclosed fence");
        out
    }

    #[test]
    fn the_report_quotes_what_it_read_so_that_nothing_in_it_is_markdown_or_a_mention() {
        let script = |name: &str| Script {
            name: name.to_owned(),
            body: String::new(),
            args: Vec::new(),
            cols: 100,
            rows: 40,
            seconds: 1,
        };
        let row = |name: &str, status: Status| Row {
            script: script(name),
            status,
            attempt: 1,
            started: None,
            took: None,
            states: Vec::new(),
            note: String::new(),
        };
        let mut failed = row("ask", Status::Fail);
        let got = "x\n@octocat ```";
        let diff = Diff { key: "menu".into(), expected: "a`b".into(), got: got.into() };
        failed.states.push(StateResult { name: "open".into(), diffs: vec![diff] });
        failed.note = "ended: ``\n# @someone".into();
        let error = row("deny", Status::Error("the check does not build:\n```\n@octocat look | here".into()));
        let shared = Shared {
            rows: vec![failed, error, row("plain", Status::Skipped("@me *now*".into()))],
            queue: VecDeque::new(),
            phase: String::new(),
            version: "2.1.400".into(),
            last_tested: None,
            account: "@team".into(),
            model: "haiku".into(),
            started: Instant::now(),
            took: Some(Duration::from_secs(61)),
            cost: Cost::default(),
            out: PathBuf::from("C:/runs/a`b"),
            report: None,
            fatal: None,
            done: true,
            lines: Vec::new(),
            check: None,
        };
        let report = report(&shared);
        assert!(report.contains("  - ` menu `: expected `` a`b ``, read ```` x⏎@octocat ``` ````\n"), "{report}");
        assert!(
            report.contains("  ````text\n  the check does not build:\n  ```\n  @octocat look | here\n  ````\n"),
            "{report}"
        );
        assert!(report.contains("  ```text\n  ended: ``\n  # @someone\n  ```\n"), "{report}");
        assert!(report.contains("- Account: @\u{200b}team"), "{report}");
        assert!(report.contains("| plain | skipped (@\u{200b}me \\*now\\*) |"), "{report}");
        let words = outside_code(&report);
        assert!(!words.contains("octocat") && !words.contains("someone"), "{words}");
        let mentions = words.match_indices('@').filter(|(at, _)| !words[at + 1..].starts_with('\u{200b}')).count();
        assert_eq!(mentions, 0, "{words}");
    }

    #[test]
    fn in_the_workflow_the_issue_and_the_marker_name_the_installed_version_and_no_issue_is_made_twice() {
        let text = workflow();
        let check = &text[text.find("\n  check:").expect("the check job")..];
        // Past the first job only the step that compares the installed version reads the one found.
        let newest: Vec<&str> =
            check.lines().filter(|l| l.contains("needs.version.outputs.version")).map(str::trim).collect();
        assert_eq!(newest, ["NEWEST: ${{ needs.version.outputs.version }}"]);
        for named in [
            "name: cli-compat-${{ steps.installed.outputs.version }}",
            "VERSION: ${{ steps.installed.outputs.version }}",
            "VERSION: ${{ needs.check.outputs.installed }}",
            "key: cli-compat-${{ needs.check.outputs.installed }}",
        ] {
            assert!(check.contains(named), "{named}");
        }
        assert!(check.contains("gh issue list --state all"), "closed issues are looked up too");
        assert!(check.contains("gh issue comment"), "a closed one gets the report as a comment");
    }

    /// The steps of a job, each as its own text, from `- name:` / `- uses:` to the next one.
    fn steps(job: &str) -> Vec<&str> {
        let mut starts: Vec<usize> = job.match_indices("\n      - ").map(|(at, _)| at).collect();
        starts.push(job.len());
        starts.windows(2).map(|w| &job[w[0]..w[1]]).collect()
    }

    #[test]
    fn in_the_workflow_a_version_already_checked_is_not_recorded_again_unless_started_by_hand() {
        let text = workflow();
        let at = |what: &str| text.find(what).unwrap_or_else(|| panic!("{what}"));
        let check = &text[at("\n  check:")..at("\n  remember:")];
        let remember = &text[at("\n  remember:")..];
        let steps = steps(check);
        let index = |name: &str| steps.iter().position(|s| s.contains(name)).unwrap_or_else(|| panic!("{name}"));
        // Once installed, the installed version's marker is looked for, as the marker is saved.
        let seen = index("name: Look for the marker of the installed version");
        assert!(index("name: Installed version") < seen && seen < index("name: Decide"));
        assert!(steps[seen].contains("key: cli-compat-${{ steps.installed.outputs.version }}"));
        assert!(steps[seen].contains("lookup-only: true"));
        assert!(remember.contains("key: cli-compat-${{ needs.check.outputs.installed }}"));
        // Saved on Linux, looked for on Windows too: every lookup and the save are cross-OS.
        assert_eq!(text.matches("enableCrossOsArchive: true").count(), 3);
        // Started by hand it records whatever the cache says.
        let decide = steps[index("name: Decide")];
        assert!(decide.contains("$env:GITHUB_EVENT_NAME -ne 'workflow_dispatch' -and $env:SEEN -eq 'true'"));
        // Nothing costly runs, and nothing fails or is remembered, when it does not record.
        for name in ["Set up Rust stable", "Build the recorder", "Build the check", "Record every script"] {
            assert!(steps[index(name)].contains("if: steps.decide.outputs.record == 'true'"), "{name}");
        }
        assert!(steps[index("name: Fail when")].contains("if: steps.decide.outputs.record == 'true' &&"));
        assert!(check.contains("recorded: ${{ steps.decide.outputs.record }}"));
        assert!(remember.contains("if: needs.check.result == 'success' && needs.check.outputs.recorded == 'true'"));
    }

    #[test]
    fn the_stamp_is_a_date_and_a_time() {
        let stamp = stamp();
        assert_eq!((stamp.len(), stamp.as_bytes()[8]), (15, b'-'));
        assert!(stamp.starts_with("20"));
    }
}
