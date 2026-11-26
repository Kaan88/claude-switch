//! A synchronous OpenCode V2 CLI client. Call it from a worker in GUI applications.
mod process;

use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const REQUEST: Duration = Duration::from_secs(20);
const SERVICE: Duration = Duration::from_secs(60);

/// Finds a native executable; never returns a PATH batch shim.
pub fn find_program() -> Option<PathBuf> {
    discover(
        std::env::var_os("OPENCODE_EXE").map(PathBuf::from),
        std::env::var_os("PATH").as_deref(),
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).as_deref().map(Path::new),
        std::env::var_os("APPDATA").as_deref().map(Path::new),
        std::env::var_os("LOCALAPPDATA").as_deref().map(Path::new),
    )
}

fn discover(
    override_path: Option<PathBuf>,
    paths: Option<&OsStr>,
    home: Option<&Path>,
    appdata: Option<&Path>,
    local: Option<&Path>,
) -> Option<PathBuf> {
    let native = |p: &Path| {
        p.is_absolute()
            && (!cfg!(windows) || p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
            && p.is_file()
    };
    if let Some(path) = override_path {
        return native(&path).then_some(path);
    }
    let exe = if cfg!(windows) { "opencode.exe" } else { "opencode" };
    let npm = |dir: &Path| dir.join("node_modules").join("@opencode").join("cli").join("bin").join(exe);
    paths
        .into_iter()
        .flat_map(std::env::split_paths)
        .flat_map(|p| [npm(&p), p.join(exe)])
        .chain(
            home.into_iter()
                .flat_map(|p| ["scoop/apps/nodejs/current/bin", "scoop/persist/nodejs/bin"].map(|s| npm(&p.join(s)))),
        )
        .chain(appdata.into_iter().map(|p| npm(&p.join("npm"))))
        .chain(local.into_iter().map(|p| p.join("Programs/opencode").join(exe)))
        .chain(home.into_iter().flat_map(|p| [p.join(".opencode/bin").join(exe), p.join(".local/bin").join(exe)]))
        .find(|p| native(p))
}

/// Captures stdout and stderr. Returns trimmed stdout, or a diagnostic on failure.
/// Do not pass secrets as arguments; use `run_prompt` for private text.
pub fn run(mut command: Command, args: &[&str], timeout: Duration) -> Result<String, String> {
    command.args(args);
    let output = process::capture(command, None, false, || false, timeout)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Err(format!("opencode {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// None if no executable was discovered.
    pub path: Option<PathBuf>,
    /// None if the executable did not return a version.
    pub version: Option<String>,
    /// None if the service did not return a recognized session map.
    pub running: Option<usize>,
}

/// Reports whether the discovered executable and its already-running service answer.
pub fn status(command: impl Fn() -> Option<Command>) -> Status {
    let Some(version_command) = command() else { return Status::default() };
    let path = Some(PathBuf::from(version_command.get_program()));
    let version = run(version_command, &["--version"], REQUEST).ok().filter(|s| !s.is_empty());
    let running = service_sessions(&command);
    Status { path, version, running }
}

/// Checks the service before asking the API, which may otherwise auto-start OpenCode.
pub fn service_sessions(command: impl Fn() -> Option<Command>) -> Option<usize> {
    let endpoint = run(command()?, &["service", "status"], REQUEST).ok()?;
    if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
        return None;
    }
    active_sessions(command()?)
}

/// Studio-compatible direct API probe; the CLI may start its service when needed.
pub fn active_sessions(command: Command) -> Option<usize> {
    active(command, REQUEST).ok()
}

fn active(command: Command, timeout: Duration) -> Result<usize, String> {
    let text = run(command, &["api", "session.active"], timeout)?;
    let value: Value = serde_json::from_str(&text).map_err(|_| "Invalid OpenCode session response.")?;
    let data = value["data"].as_object().ok_or("Invalid OpenCode session response.")?;
    Ok(data.values().filter(|session| session["type"] == "running").count())
}

/// Restarts the service, falling back to stop/start, and returns the verified running count.
pub fn restart_service(command: impl Fn() -> Command) -> Result<usize, String> {
    if run(command(), &["service", "restart"], SERVICE).is_err() {
        let _ = run(command(), &["service", "stop"], Duration::from_secs(30));
        run(command(), &["service", "start"], SERVICE)?;
    }
    active(command(), SERVICE).map_err(|e| format!("OpenCode restarted but does not answer: {e}"))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Model {
    pub provider: String,
    pub model: String,
}

impl Model {
    pub fn parse(id: &str) -> Option<Self> {
        let (provider, model) = id.split_once('/')?;
        let valid =
            |s: &str| !s.is_empty() && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control());
        (valid(provider) && model.split('/').all(valid))
            .then(|| Self { provider: provider.into(), model: model.into() })
    }

    pub fn id(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

pub fn models(mut command: Command, timeout: Duration, cancelled: impl Fn() -> bool) -> Result<Vec<Model>, String> {
    command.arg("models");
    let mut models: Vec<_> =
        private(command, None, timeout, cancelled)?.lines().filter_map(|s| Model::parse(s.trim())).collect();
    models.sort_unstable();
    models.dedup();
    if models.is_empty() { Err("OpenCode did not list any models.".into()) } else { Ok(models) }
}

/// Runs one noninteractive prompt on stdin. Additional CLI options are caller policy.
/// Never surfaces stderr, which may contain private prompt text.
pub fn run_prompt(
    mut command: Command,
    model: &str,
    options: &[&str],
    prompt: &str,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    Model::parse(model).ok_or("Choose a valid OpenCode provider/model.")?;
    command.args(["run", "--model", model]).args(options);
    private(command, Some(prompt), timeout, cancelled)
}

fn private(
    command: Command,
    prompt: Option<&str>,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<String, String> {
    let output = process::capture(command, prompt, true, cancelled, timeout)?;
    if !output.status.success() {
        return Err("OpenCode ended with an error. Check the installation and model in OpenCode.".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "OpenCode did not return UTF-8 text.".into())
}

#[cfg(test)]
mod tests;
