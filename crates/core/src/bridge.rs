//! The OpenCode bridge's jobs (`claude-flash-bridge`, installed at
//! `~/.claude/mcp-servers/opencode-agent`), read from the state folder its server instances share:
//! `%TEMP%\opencode-agent`. Each job has `<id>.json`, its state, which the bridge replaces whole
//! (a temporary file renamed over it), and `<id>.jsonl`, the worker's raw event stream, which only
//! grows. Switch only reads them: it never writes, renames or deletes a file of the bridge's.
//!
//! The state's fields, as the bridge writes them: `id`, `owner` (the pid of the server instance
//! that runs it), `status` (`queued`, `running`, `done`, `failed`, `cancelled`, `timeout`),
//! `started`, `running` and `finished` (ms since 1970), `directory`, `label`, `worker`, `model`,
//! `lastTool`, `inFlight` (`{label, since}` or null), `waiting` (why it is queued), `childPid` (the
//! worker process it started), `lastEvent`. A job whose owner has exited stays `running` or
//! `queued` in its file; the bridge then calls it lost, and so does Switch ("owner gone").

pub mod log;
pub mod process;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::util::str_field;

/// The largest job file read: the bridge caps a job's reply at 20 000 characters.
pub const MAX_JOB_FILE: usize = 1 << 20;

/// The bridge's state folder: `opencode-agent` in Node's temporary folder (`TEMP`, then `TMP`).
pub fn folder() -> PathBuf {
    let temp = ["TEMP", "TMP"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    temp.join("opencode-agent")
}

/// The job id of a state file's name (`job_<base36>_<n>[-label].json`), as the bridge checks it.
pub fn job_id(name: &str) -> Option<&str> {
    let id = name.strip_suffix(".json")?;
    let rest = id.strip_prefix("job_")?;
    let (instance, rest) = rest.split_once('_')?;
    let (number, label) = rest.split_once('-').map_or((rest, None), |(n, l)| (n, Some(l)));
    let lower = |s: &str, extra: &[u8]| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || extra.contains(&b))
    };
    let ok = lower(instance, b"")
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && label.is_none_or(|l| lower(l, b"-"));
    ok.then_some(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    TimedOut,
    /// A status this version does not know.
    Other(String),
}

impl State {
    fn parse(status: &str) -> Self {
        match status {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "timeout" => Self::TimedOut,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn word(&self) -> &str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::TimedOut => "Timed out",
            Self::Other(s) => s,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub label: Option<String>,
    pub worker: Option<String>,
    pub model: Option<String>,
    pub directory: PathBuf,
    pub state: State,
    /// Its server instance has exited while the file still says queued or running: the job can
    /// no longer finish (the bridge calls it lost).
    pub owner_gone: bool,
    pub started_ms: u64,
    pub running_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub last_event_ms: Option<u64>,
    pub last_tool: Option<String>,
    /// The tool call under way, as the bridge labels it.
    pub in_flight: Option<String>,
    /// Why a queued job waits.
    pub waiting: Option<String>,
    pub owner: Option<u32>,
    pub child_pid: Option<u32>,
    /// When the bridge last saved the state (the file's modification time, ms since 1970).
    pub saved_ms: u64,
    /// The job's event stream: `<id>.jsonl` beside the state, never a path the file names.
    pub log: PathBuf,
}

impl Job {
    /// Queued or running by its file, whether or not its owner is still there.
    pub fn pending(&self) -> bool {
        matches!(self.state, State::Queued | State::Running)
    }

    /// Queued or running, with its owner there.
    pub fn live(&self) -> bool {
        self.pending() && !self.owner_gone
    }

    /// What the list says it is.
    pub fn state_word(&self) -> &str {
        if self.pending() && self.owner_gone { "Owner gone" } else { self.state.word() }
    }

    /// Its name: the label it was given, else its id.
    pub fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }

    /// How long it has run (or waited, while queued), at `now_ms`: up to its end once ended,
    /// and up to its last saved state once its owner is gone.
    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        let from = self.running_ms.unwrap_or(self.started_ms);
        let to = match (self.finished_ms, self.owner_gone) {
            (Some(end), _) => end,
            (None, true) => self.last_event_ms.unwrap_or(self.saved_ms).max(self.saved_ms),
            (None, false) => now_ms,
        };
        to.saturating_sub(from)
    }

    /// Reads a state file's JSON. `None` when it is not a job's state.
    pub fn parse(id: &str, bytes: &[u8], dir: &Path, saved_ms: u64) -> Option<Self> {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        if str_field(&v, "id") != Some(id) {
            return None;
        }
        let ms = |key: &str| v.get(key).and_then(Value::as_u64).filter(|&t| t > 0);
        let pid = |key: &str| v.get(key).and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|&p| p > 0);
        let text = |key: &str| str_field(&v, key).map(|s| plain(s, 200)).filter(|s| !s.is_empty());
        Some(Self {
            id: id.to_owned(),
            label: text("label"),
            worker: text("worker"),
            model: text("model"),
            directory: str_field(&v, "directory").map(|s| PathBuf::from(plain(s, MAX_PATH))).unwrap_or_default(),
            state: State::parse(str_field(&v, "status").unwrap_or("")),
            owner_gone: false,
            started_ms: ms("started").unwrap_or(saved_ms),
            running_ms: ms("running"),
            finished_ms: ms("finished"),
            last_event_ms: ms("lastEvent"),
            last_tool: text("lastTool"),
            in_flight: v.get("inFlight").and_then(|f| str_field(f, "label")).map(|s| plain(s, 200)),
            waiting: text("waiting"),
            owner: pid("owner"),
            child_pid: pid("childPid"),
            saved_ms,
            log: dir.join(format!("{id}.jsonl")),
        })
    }

    /// Reads a job's state file now (bounded).
    pub fn read(dir: &Path, id: &str) -> io::Result<Self> {
        let path = dir.join(format!("{id}.json"));
        let bytes = crate::read_bounded(&path, MAX_JOB_FILE)?;
        let saved = std::fs::metadata(&path)?.modified().map(unix_ms).unwrap_or(0);
        Self::parse(id, &bytes, dir, saved)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a job's state"))
    }
}

/// The longest folder shown: Windows' long-path limit.
const MAX_PATH: usize = 1024;

/// A text of the job file as the window may draw it: one line, no control characters, at most
/// `max` characters.
fn plain(text: &str, max: usize) -> String {
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    crate::util::squeeze(&text, max)
}

pub fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Reads the state folder again and again, parsing only the files that changed since.
#[derive(Default)]
pub struct Scanner {
    files: HashMap<String, (u64, SystemTime, Option<Job>)>,
}

impl Scanner {
    /// Every job in `dir`: the pending ones, then the `ended` most recent others; each group
    /// newest first. `alive(pid, started_ms)` says whether a pending job's owner is still there.
    pub fn scan(&mut self, dir: &Path, ended: usize, alive: impl Fn(u32, u64) -> bool) -> io::Result<Vec<Job>> {
        let mut seen = HashMap::with_capacity(self.files.len());
        for entry in std::fs::read_dir(dir)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(job_id) else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let (len, modified) = (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            let job = match self.files.remove(id) {
                Some((l, m, job)) if l == len && m == modified => job,
                _ => (len <= MAX_JOB_FILE as u64)
                    .then(|| std::fs::read(entry.path()).ok())
                    .flatten()
                    .and_then(|bytes| Job::parse(id, &bytes, dir, unix_ms(modified))),
            };
            seen.insert(id.to_owned(), (len, modified, job));
        }
        self.files = seen;
        let (mut pending, mut done): (Vec<Job>, Vec<Job>) =
            self.files.values().filter_map(|(_, _, job)| job.clone()).partition(Job::pending);
        for job in &mut pending {
            job.owner_gone = job.owner.is_none_or(|pid| !alive(pid, job.started_ms));
        }
        pending.sort_by(|a, b| b.started_ms.cmp(&a.started_ms).then_with(|| b.id.cmp(&a.id)));
        done.sort_by(|a, b| {
            let end = |j: &Job| j.finished_ms.unwrap_or(j.saved_ms);
            end(b).cmp(&end(a)).then_with(|| b.id.cmp(&a.id))
        });
        done.truncate(ended);
        pending.extend(done);
        Ok(pending)
    }
}

/// Why a job cannot be cancelled from here, in words for the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The file says it is not queued or running.
    NotRunning(String),
    /// Queued: no worker process yet. Only the bridge can take it out of its queue.
    Queued,
    /// The file names no worker process.
    NoProcess,
    /// The process it names has ended.
    Ended(u32),
    /// The pid now belongs to another program: Windows gave it out again.
    NotTheJobs(u32),
    /// The process cannot be looked at (another user's, or an elevated one).
    Unknown(u32),
    /// It runs as another user, or elevated while Switch is: Switch does not end it.
    Protected(u32),
    /// It could not be ended (access denied).
    Denied(u32, String),
    /// The state file could not be read.
    Unreadable(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning(state) => write!(f, "The job is no longer running ({}).", state.to_lowercase()),
            Self::Queued => f.write_str(
                "The job is still queued and has no process to end. Cancel it from Claude (opencode_cancel), which can take it out of the bridge's queue.",
            ),
            Self::NoProcess => f.write_str("The job's file names no worker process, so there is nothing to end."),
            Self::Ended(pid) => write!(f, "Its process ({pid}) has already ended."),
            Self::NotTheJobs(pid) => write!(
                f,
                "Process {pid} is no longer the job's: Windows has given its number to another program. Nothing was ended."
            ),
            Self::Unknown(pid) => write!(
                f,
                "Process {pid} could not be checked (it may belong to another user or run elevated), so it was not ended."
            ),
            Self::Protected(pid) => write!(
                f,
                "Process {pid} runs as another user or elevated, so Switch does not end it. Nothing was ended."
            ),
            Self::Denied(pid, why) => write!(f, "Process {pid} could not be ended: {why}"),
            Self::Unreadable(why) => write!(f, "The job's file could not be read: {why}"),
        }
    }
}

/// What a cancel ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub pid: u32,
    /// Processes ended, the worker's own included.
    pub count: usize,
    /// Its owner was there: the bridge sees the worker end and marks the job itself.
    pub owner_there: bool,
}

/// Ends the job's worker process tree, after reading its state again: only while the file says
/// it is running, and only when the pid it names is still the job's (`process::is_jobs`).
pub fn cancel(dir: &Path, id: &str, alive: impl Fn(u32, u64) -> bool) -> Result<Ended, Refused> {
    let job = Job::read(dir, id).map_err(|e| Refused::Unreadable(e.to_string()))?;
    match job.state {
        State::Running => {}
        State::Queued => return Err(Refused::Queued),
        ref other => return Err(Refused::NotRunning(other.word().to_owned())),
    }
    let pid = job.child_pid.ok_or(Refused::NoProcess)?;
    let owner_there = job.owner.is_some_and(|owner| alive(owner, job.started_ms));
    let count = process::end_job_tree(&job)?;
    Ok(Ended { pid, count, owner_there })
}

#[cfg(test)]
#[path = "bridge/tests.rs"]
mod tests;
