//! The Workers tab: the OpenCode bridge's jobs, live from its state folder
//! (`claude_switch_core::bridge`), beside Sessions while that folder exists. On the left the jobs
//! (pending first), on the right the selected job's facts, Cancel, and its output streamed from the
//! tail of its event log. Reads happen off the window's thread, once a second while the tab shows.
//! Nothing here writes the bridge's files; Cancel ends the worker's process tree after a confirm,
//! only while the pid its file names is still the job's.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use claude_switch_core::bridge::{self, Ended, Job as BridgeJob, Refused, Scanner, log};
use mdshared_app::job::Job;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::infobar::{InfoBar, Severity, draw_infobar};
use mdshared_ui::keys::KeyName;
use mdshared_ui::overlay::draw_dialog_button;
use mdshared_ui::splitter::Axis;
use mdshared_ui::theme::Palette;

use crate::dialogs::{Dialog, Pending};
use crate::list::now_ms;
use crate::state::{AppState, Target};

/// How often the jobs and the shown output are read while the tab shows.
pub const POLL: Duration = Duration::from_secs(1);
/// How many ended jobs the list keeps under the pending ones.
const ENDED: usize = 150;
const ROW_H: f32 = 62.0;
const HEAD_H: f32 = 46.0;
const PAD: f32 = 24.0;
const LABEL_W: f32 = 104.0;

type ScanResult = io::Result<Vec<BridgeJob>>;

#[derive(Default)]
pub struct WorkersTab {
    /// The state folder read; a made-up one in the demo.
    pub dir: PathBuf,
    demo: Option<DemoFolder>,
    scanner: Arc<Mutex<Scanner>>,
    scan: Option<Job<ScanResult>>,
    pub jobs: Vec<BridgeJob>,
    /// The folder was read at least once.
    pub scanned: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    list_area: Rect,
    list_scroll: f32,
    list_max: f32,
    /// The details pane scrolls when the window is too short for the facts and the output.
    details_area: Rect,
    details_scroll: f32,
    details_max: f32,
    output: Option<Output>,
    cancel: Option<(String, Job<Result<Ended, Refused>>)>,
    /// What the last cancel did, for the job it was for.
    notice: Option<(String, Severity, &'static str, String)>,
}

/// The selected job's output: its event log's tail, read off the window's thread.
struct Output {
    id: String,
    /// The tail, while no read holds it.
    tail: Option<log::Tail>,
    reading: Option<Job<(log::Tail, io::Result<bool>)>>,
    error: Option<String>,
    /// What is drawn: a copy of the tail's entries, and their height at `width`, by push number.
    entries: VecDeque<log::Entry>,
    heights: VecDeque<f32>,
    first: u64,
    resets: u64,
    width: f32,
    cut: bool,
    area: Rect,
    offset: f32,
    max: f32,
    follow: bool,
}

impl Output {
    fn new(job: &BridgeJob) -> Self {
        Self {
            id: job.id.clone(),
            tail: Some(log::Tail::new(&job.log)),
            reading: None,
            error: None,
            entries: VecDeque::new(),
            heights: VecDeque::new(),
            first: 0,
            resets: 0,
            width: 0.0,
            cut: false,
            area: Rect::default(),
            offset: 0.0,
            max: 0.0,
            follow: true,
        }
    }

    /// Starts a read of what was appended, unless one runs.
    fn read(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        let Some(mut tail) = self.tail.take() else { return };
        match Job::start(wake, move |_, _| {
            let result = tail.read();
            (tail, result)
        }) {
            Ok(job) => self.reading = Some(job),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Takes a finished read: true when what is drawn changed.
    fn take(&mut self) -> bool {
        let Some(job) = self.reading.as_ref().filter(|j| !j.running()) else { return false };
        let Some((tail, result)) = job.take() else { return false };
        self.reading = None;
        let error = result
            .as_ref()
            .err()
            .map(|e| if e.kind() == io::ErrorKind::NotFound { "No output yet.".to_owned() } else { e.to_string() });
        let changed = matches!(result, Ok(true)) || error != self.error;
        self.error = error;
        if matches!(result, Ok(true)) {
            self.sync(&tail);
        }
        let behind = tail.behind();
        self.tail = Some(tail);
        changed || behind
    }

    /// Copies the tail's entries; keeps the heights of those already measured.
    fn sync(&mut self, tail: &log::Tail) {
        let first = tail.pushed - tail.entries.len() as u64;
        if tail.resets != self.resets {
            self.resets = tail.resets;
            self.entries.clear();
            self.heights.clear();
            self.first = first;
        }
        while self.first < first && !self.entries.is_empty() {
            self.entries.pop_front();
            self.heights.pop_front();
            self.first += 1;
        }
        self.first = first;
        let have = self.entries.len();
        self.entries.extend(tail.entries.iter().skip(have).cloned());
        self.cut = tail.cut;
    }

    fn wheel(&mut self, dy: f32) -> bool {
        let before = self.offset;
        self.offset = (self.offset - dy).clamp(0.0, self.max);
        self.follow = self.offset >= self.max - 1.0;
        self.offset != before
    }
}

impl WorkersTab {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, ..Default::default() }
    }

    /// The demo's: made-up jobs in a temporary folder of their own, which goes with the tab.
    pub fn demo() -> Self {
        match DemoFolder::write() {
            Ok(demo) => Self { dir: demo.0.clone(), demo: Some(demo), ..Default::default() },
            Err(e) => Self { error: Some(e.to_string()), ..Default::default() },
        }
    }

    /// The folder exists: the tab belongs in the strip.
    pub fn available(&self) -> bool {
        self.demo.is_some() || self.dir.is_dir()
    }

    fn selected_job(&self) -> Option<&BridgeJob> {
        let id = self.selected.as_deref()?;
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Reads the folder and the shown output again (each unless a read runs).
    pub fn poll(&mut self, wake: impl Fn() + Send + Sync + Clone + 'static) {
        if self.scan.is_none() {
            let (dir, scanner, demo) = (self.dir.clone(), self.scanner.clone(), self.demo.is_some());
            self.scan = Job::start(wake.clone(), move |_, _| {
                let mut scanner = scanner.lock().unwrap_or_else(|e| e.into_inner());
                if demo {
                    scanner.scan(&dir, ENDED, |pid, _| pid != DEMO_GONE)
                } else {
                    scanner.scan(&dir, ENDED, bridge::process::owner_alive)
                }
            })
            .ok();
        }
        if let Some(output) = &mut self.output {
            output.read(wake);
        }
    }

    /// Takes what the reads delivered: true when the tab needs drawing again.
    pub fn take(&mut self) -> bool {
        let mut changed = false;
        if let Some(job) = self.scan.as_ref().filter(|j| !j.running()) {
            let result = job.take();
            self.scan = None;
            changed = true;
            match result {
                Some(Ok(jobs)) => {
                    self.jobs = jobs;
                    self.error = None;
                }
                Some(Err(e)) => self.error = Some(e.to_string()),
                None => {}
            }
            self.scanned = true;
            if self.selected_job().is_none() {
                self.selected = self.jobs.first().map(|j| j.id.clone());
            }
            self.follow_selection();
        }
        if let Some(output) = &mut self.output {
            changed |= output.take();
        }
        if let Some((id, job)) = self.cancel.as_ref().filter(|(_, j)| !j.running()) {
            let id = id.clone();
            let result = job.take();
            self.cancel = None;
            self.notice = Some(match result {
                Some(Ok(ended)) => (id, Severity::Info, "Job cancelled", ended_text(&ended)),
                Some(Err(why)) => (id, Severity::Warning, "Not cancelled", why.to_string()),
                None => (id, Severity::Warning, "Not cancelled", "The cancel did not finish.".into()),
            });
            changed = true;
        }
        changed
    }

    /// The output shown is the selected job's.
    fn follow_selection(&mut self) {
        let Some(job) = self.selected_job() else {
            self.output = None;
            return;
        };
        if self.output.as_ref().is_none_or(|o| o.id != job.id) {
            self.output = Some(Output::new(job));
        }
    }

    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_owned());
            self.details_scroll = 0.0;
            self.follow_selection();
        }
    }

    /// Up and Down move the selection; true when the key was the tab's.
    fn step(&mut self, forward: bool) -> bool {
        let at = self.selected.as_deref().and_then(|id| self.jobs.iter().position(|j| j.id == id));
        let to = match at {
            Some(i) if forward => (i + 1).min(self.jobs.len().saturating_sub(1)),
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        let Some(id) = self.jobs.get(to).map(|j| j.id.clone()) else { return false };
        self.select(&id);
        // Keep it in view.
        let top = to as f32 * ROW_H;
        let view = (self.list_area.h - HEAD_H).max(ROW_H);
        self.list_scroll = self.list_scroll.clamp((top + ROW_H - view).max(0.0), top);
        true
    }

    /// The wheel: the output scrolls first, and at its end the details around it.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if let Some(output) = self.output.as_mut().filter(|o| o.area.contains(x, y))
            && output.wheel(dy)
        {
            return true;
        }
        if self.details_area.contains(x, y) {
            let before = self.details_scroll;
            self.details_scroll = (self.details_scroll - dy).clamp(0.0, self.details_max);
            return self.details_scroll != before;
        }
        if self.list_area.contains(x, y) {
            self.list_scroll = (self.list_scroll - dy).clamp(0.0, self.list_max);
            return true;
        }
        false
    }
}

fn ended_text(ended: &Ended) -> String {
    let under = match ended.count.saturating_sub(1) {
        0 => String::new(),
        1 => " and 1 process under it".into(),
        n => format!(" and {n} processes under it"),
    };
    let after = if ended.owner_there {
        "The bridge sees its worker end and marks the job ended."
    } else {
        "Its server is gone, so its file still says running."
    };
    format!("Ended the worker (process {}){under}. {after}", ended.pid)
}

/// The demo's owner pid that is "gone".
const DEMO_GONE: u32 = 999_999;

/// The demo's made-up state folder, under the temporary folder, removed when the tab goes.
struct DemoFolder(PathBuf);

impl Drop for DemoFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl DemoFolder {
    fn write() -> io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let dir = loop {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("claude-switch-demo-workers-{}-{serial}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let demo = Self(dir);
        demo_jobs(&demo.0, now_ms())?;
        Ok(demo)
    }
}

/// Writes made-up jobs and logs into `dir`, as if the bridge had run them around `now`.
pub(crate) fn demo_jobs(dir: &Path, now: u64) -> io::Result<()> {
    use serde_json::json;
    let me = std::process::id();
    let jobs = [
        (
            "job_k3x_4-tile-cache",
            "running",
            "flash",
            "deepseek/deepseek-v4-flash",
            "trail-planner",
            95,
            me,
            None,
            "bash",
        ),
        ("job_k3x_3-readme", "running", "claude-sonnet", "claude-code/sonnet", "notes", 610, DEMO_GONE, None, "Edit"),
        ("job_k3x_5-lint", "queued", "flash", "deepseek/deepseek-v4-flash", "pixel-forge", 20, me, None, ""),
        (
            "job_k3x_2-search-fuzz",
            "done",
            "flash",
            "deepseek/deepseek-v4-flash",
            "pixel-forge",
            3_400,
            me,
            Some(2_900),
            "read",
        ),
        ("job_k3x_1-crlf", "failed", "pro", "deepseek/deepseek-v4-pro", "notes", 7_800, me, Some(7_000), "grep"),
    ];
    for (id, status, worker, model, folder, ago_s, owner, ended_ago_s, tool) in jobs {
        let started = now - ago_s * 1000;
        let label = id.split_once('-').map_or("", |(_, l)| l).replace('-', " ");
        let mut state = json!({
            "id": id, "owner": owner, "status": status, "started": started,
            "directory": format!("C:\\Users\\demo\\Desktop\\{folder}"), "label": label,
            "worker": worker, "model": model, "lastTool": tool, "files": [], "reply": "", "ok": status == "done",
        });
        if status != "queued" {
            state["running"] = json!(started + 2_000);
            state["childPid"] = json!(40_000 + ago_s);
            state["lastEvent"] = json!(now - 4_000);
        } else {
            state["waiting"] = json!("2 jobs run at once on this machine");
        }
        if let Some(ago) = ended_ago_s {
            state["finished"] = json!(now - ago * 1000);
        }
        if status == "running" && owner == me {
            state["inFlight"] = json!({"label": "bash cargo test -p tiles", "since": now - 9_000});
        }
        std::fs::write(dir.join(format!("{id}.json")), state.to_string())?;
    }
    let opencode = [
        json!({"type":"step_start","part":{"type":"step-start"}}),
        json!({"type":"text","part":{"type":"text","text":"I'll find where tiles are evicted, then run the cache tests."}}),
        json!({"type":"tool_use","part":{"tool":"grep","state":{"status":"completed","title":"evict","input":{"pattern":"evict"}}}}),
        json!({"type":"tool_use","part":{"tool":"read","state":{"status":"completed","title":"src/tiles/cache.rs","input":{}}}}),
        json!({"type":"text","part":{"type":"text","text":"Eviction picks the tile with the oldest file time. Two tiles written in the same tick get the same time on Windows, so the order is not exact. A use counter fixes that."}}),
        json!({"type":"tool_use","part":{"tool":"edit","state":{"status":"completed","title":"src/tiles/cache.rs","input":{}}}}),
        json!({"type":"tool_use","part":{"tool":"bash","state":{"status":"error","input":{"command":"cargo test -p tiles cache"},"error":"1 test failed"}}}),
        json!({"type":"text","part":{"type":"text","text":"One test still expects file times. I'll update it and run the suite again."}}),
        json!({"type":"tool_use","part":{"tool":"bash","state":{"status":"running","input":{"command":"cargo test -p tiles"}}}}),
    ];
    let claude = [
        json!({"type":"system","subtype":"init","model":"claude-sonnet"}),
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"I'll rewrite the quick start, then add one example per endpoint."},{"type":"tool_use","name":"Read","input":{"file_path":"README.md"}}]}}),
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"README.md"}}]}}),
    ];
    let lines = |events: &[serde_json::Value]| events.iter().map(|e| format!("{e}\n")).collect::<String>();
    std::fs::write(dir.join("job_k3x_4-tile-cache.jsonl"), lines(&opencode))?;
    std::fs::write(dir.join("job_k3x_3-readme.jsonl"), lines(&claude))?;
    std::fs::write(dir.join("job_k3x_2-search-fuzz.jsonl"), lines(&opencode[..6]))?;
    std::fs::write(dir.join("job_k3x_1-crlf.jsonl"), lines(&opencode[..3]))?;
    Ok(())
}

impl AppState {
    /// Puts the Workers tab in the strip while the bridge's folder exists.
    pub fn check_workers_folder(&mut self) -> bool {
        let there = self.workers.available();
        let shown = self.tabs.workers_shown();
        if there == self.tabs.workers_tab {
            return false;
        }
        self.tabs.set_workers_tab(there);
        if shown && !there {
            self.rebuild_view();
        }
        true
    }

    /// The demo's Workers tab, shown: made-up jobs (`--demo-jobs`).
    pub fn start_demo_workers(&mut self) {
        self.workers = WorkersTab::demo();
        self.check_workers_folder();
        self.tabs.activate(crate::tabs::WORKERS);
    }

    /// The real bridge's folder, at start (the demo keeps its own or none).
    pub fn start_workers(&mut self) {
        if !self.source.is_demo() {
            self.workers = WorkersTab::new(bridge::folder());
        }
        self.check_workers_folder();
        self.poll_workers();
    }

    /// The feed timer's turn for the Workers tab: read again while it shows.
    pub fn poll_workers(&mut self) -> bool {
        if !self.tabs.workers_shown() {
            return false;
        }
        let wake = Arc::new(self.wake_fn());
        self.workers.poll(move || wake());
        true
    }

    pub fn take_workers(&mut self) -> bool {
        self.workers.take()
    }

    pub fn workers_key(&mut self, name: &KeyName) -> bool {
        match name {
            KeyName::Up => self.workers.step(false),
            KeyName::Down => self.workers.step(true),
            _ => false,
        }
    }

    /// A click on a row or on Cancel.
    pub fn workers_activate(&mut self, t: Target) {
        match t {
            Target::Worker(i) => {
                if let Some(id) = self.workers.jobs.get(i).map(|j| j.id.clone()) {
                    self.workers.select(&id);
                }
            }
            Target::WorkerCancel => self.ask_cancel_worker(),
            _ => {}
        }
    }

    fn ask_cancel_worker(&mut self) {
        let Some(job) = self.workers.selected_job() else { return };
        let id = job.id.clone();
        let refused = |why: Refused| Some((id.clone(), Severity::Warning, "Not cancelled", why.to_string()));
        if job.state == bridge::State::Queued {
            self.workers.notice = refused(Refused::Queued);
            return;
        }
        let Some(pid) = job.child_pid else {
            self.workers.notice = refused(Refused::NoProcess);
            return;
        };
        let gone = if job.owner_gone {
            " Its server is gone, so its file will still say running."
        } else {
            " The bridge then marks it ended."
        };
        self.dialog = Some(Dialog::Confirm {
            title: "Cancel this job?".into(),
            text: format!(
                "Ends the worker of \"{}\" (process {pid}) and the processes it started, if that process is still the job's. Edits it already made stay on disk.{gone}",
                job.name()
            ),
            ok: "End the job".into(),
            then: Pending::CancelWorker(id),
        });
    }

    /// After the confirm: ends the job's process tree on a worker thread.
    pub fn cancel_worker(&mut self, id: &str) {
        if self.workers.demo.is_some() {
            self.workers.notice = Some((id.into(), Severity::Info, "Not cancelled", "The demo ends nothing.".into()));
            return;
        }
        let (dir, job) = (self.workers.dir.clone(), id.to_owned());
        let wake = self.wake_fn();
        if let Ok(j) = Job::start(wake, move |_, _| bridge::cancel(&dir, &job, bridge::process::owner_alive)) {
            self.workers.cancel = Some((id.to_owned(), j));
        }
    }
}

/// Draws the tab: the jobs on the left, the selected one on the right, as Sessions is laid out.
pub fn draw(st: &mut AppState, p: &mut Painter, area: Rect) {
    let pal = *st.pal();
    let rects = st.splitter.rects(area, Axis::Horizontal);
    draw_list(st, p, &pal, rects.0);
    draw_details(st, p, &pal, rects.2);
    let (x, y) = st.background_pointer();
    let hot = st.splitter.hit(area, Axis::Horizontal, x, y);
    mdshared_ui::splitter::draw(p, &pal, rects, Axis::Horizontal, hot);
    st.hits.push((rects.1, Target::Splitter));
}

fn state_color(pal: &Palette, job: &BridgeJob) -> mdshared_ui::gfx::Rgb {
    use bridge::State;
    match &job.state {
        _ if job.pending() && job.owner_gone => pal.warn,
        State::Running => pal.accent,
        State::Failed | State::TimedOut => pal.warn,
        _ => pal.muted,
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn draw_list(st: &mut AppState, p: &mut Painter, pal: &Palette, area: Rect) {
    p.fill(area, pal.pane);
    st.workers.list_area = area;
    let clip = p.clip(area);
    let w = &st.workers;
    let live = w.jobs.iter().filter(|j| j.live() && j.state == bridge::State::Running).count();
    let head = match (w.scanned, live) {
        (false, _) => "Reading the bridge's jobs\u{2026}".to_owned(),
        (true, 0) => "Nothing running".to_owned(),
        (true, 1) => "1 job running".to_owned(),
        (true, n) => format!("{n} jobs running"),
    };
    p.text(Face::Semibold, 14.0, area.x + 18.0, area.y + 24.0, "Jobs", pal.text, 60.0);
    p.text_right(Face::Ui, 12.0, area.right() - 18.0, area.y + 24.0, &head, pal.muted);
    p.fill(Rect::new(area.x, area.y + HEAD_H - 1.0, area.w, p.px()), pal.line);
    let rows = Rect::new(area.x, area.y + HEAD_H, area.w, (area.h - HEAD_H).max(0.0));
    let content = w.jobs.len() as f32 * ROW_H;
    st.workers.list_max = (content - rows.h).max(0.0);
    st.workers.list_scroll = st.workers.list_scroll.clamp(0.0, st.workers.list_max);
    let offset = st.workers.list_scroll;
    let rows_clip = p.clip(rows);
    if let Some(error) = &st.workers.error {
        p.paragraph_wrapped(Face::Ui, 12.0, rows.x + 18.0, rows.y + 14.0, rows.w - 36.0, error, pal.warn, Wrap::Words);
    } else if st.workers.scanned && st.workers.jobs.is_empty() {
        p.text(Face::Ui, 13.0, rows.x + 18.0, rows.y + 24.0, "The bridge has no jobs.", pal.muted, rows.w - 36.0);
    }
    let now = now_ms();
    let first = (offset / ROW_H) as usize;
    let mut hits = Vec::new();
    for (i, job) in st.workers.jobs.iter().enumerate().skip(first) {
        let y = rows.y + i as f32 * ROW_H - offset;
        if y >= rows.bottom() {
            break;
        }
        let r = Rect::new(rows.x + 6.0, y + 1.0, rows.w - 12.0, ROW_H - 2.0);
        let target = Target::Worker(i);
        let selected = st.workers.selected.as_deref() == Some(job.id.as_str());
        if selected {
            p.rbox(r, 5.0, pal.on);
            p.rbox(Rect::new(r.x, r.y + 7.0, 3.0, r.h - 14.0), 1.5, pal.accent);
        } else if st.hover == Some(target) {
            p.rbox(r, 5.0, if st.pressed == Some(target) { pal.press } else { pal.hover });
        }
        let (x, right) = (r.x + 18.0, r.right() - 10.0);
        let state = job.state_word();
        let sw = p.text_right(Face::Semibold, 11.5, right, r.y + 13.0, state, state_color(pal, job));
        p.text(Face::Semibold, 13.5, x, r.y + 13.0, job.name(), pal.text, (right - x - sw - 12.0).max(0.0));
        let who = [job.worker.as_deref(), job.model.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        p.text(Face::Ui, 12.0, x, r.y + 31.0, &who, pal.muted, right - x);
        let time = log::duration(job.elapsed_ms(now) / 1000);
        let tool = job.in_flight.as_deref().or(job.last_tool.as_deref()).unwrap_or("");
        let third = [folder_name(&job.directory), time, tool.to_owned()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("  ·  ");
        p.text(Face::Ui, 11.5, x, r.y + 48.0, &third, pal.faint, right - x);
        hits.push((
            Rect::new(r.x, r.y.max(rows.y), r.w, (r.bottom().min(rows.bottom()) - r.y.max(rows.y)).max(0.0)),
            target,
        ));
    }
    p.restore(rows_clip);
    if let Some((track, thumb)) = mdshared_ui::scroll::scrollbar(rows, content, rows.h, offset) {
        mdshared_ui::scroll::draw_scrollbar(p, pal, track, thumb, false);
    }
    p.restore(clip);
    st.hits.extend(hits);
}

fn draw_details(st: &mut AppState, p: &mut Painter, pal: &Palette, area: Rect) {
    p.fill(area, pal.page);
    st.workers.details_area = area;
    let Some(job) = st.workers.selected_job().cloned() else {
        let text = if st.workers.scanned { "Select a job on the left." } else { "" };
        p.text_center(Face::Ui, 13.0, Rect::new(area.x, area.y + 40.0, area.w, 24.0), text, pal.muted);
        if let Some(o) = &mut st.workers.output {
            o.area = Rect::default();
        }
        return;
    };
    let clip = p.clip(area);
    let first_hit = st.hits.len();
    let x = area.x + PAD;
    let w = (area.w - 2.0 * PAD).max(40.0);
    let offset = st.workers.details_scroll;
    let mut y = area.y + 22.0 - offset;
    y += p.paragraph_wrapped(Face::Semibold, 18.0, x, y, w, job.name(), pal.text, Wrap::Words) + 4.0;
    p.text(Face::Mono, 11.5, x, y + 8.0, &job.id, pal.faint, w);
    y += 30.0;
    let now = now_ms();
    let mut state = job.state_word().to_owned();
    if let Some(waiting) = job.waiting.as_deref().filter(|_| job.state == bridge::State::Queued) {
        state = format!("{state} ({waiting})");
    }
    let tool = match (&job.in_flight, &job.last_tool) {
        (Some(now), _) if job.live() => format!("{now} (running)"),
        (_, Some(last)) => last.clone(),
        _ => "none".into(),
    };
    let facts = [
        ("Worker", job.worker.clone().unwrap_or_else(|| "unknown".into())),
        ("Model", job.model.clone().unwrap_or_else(|| "unknown".into())),
        ("Folder", job.directory.display().to_string()),
        ("State", state),
        (if job.live() { "Running for" } else { "Ran for" }, log::duration(job.elapsed_ms(now) / 1000)),
        ("Last tool", tool),
        ("Process", job.child_pid.map_or_else(|| "none yet".into(), |p| p.to_string())),
    ];
    for (label, value) in facts {
        p.text(Face::Ui, 12.0, x, y + 9.0, label, pal.muted, LABEL_W - 8.0);
        let color = if label == "State" { state_color(pal, &job) } else { pal.text };
        p.text(Face::Ui, 13.0, x + LABEL_W, y + 9.0, &value, color, w - LABEL_W);
        y += 22.0;
    }
    y += 12.0;
    if let Some((_, severity, title, text)) = st.workers.notice.clone().filter(|(id, ..)| *id == job.id) {
        let bar = InfoBar::new(severity, title, text).closable(false);
        let h = bar.height(p, w);
        draw_infobar(p, pal, Rect::new(x, y, w, h), &bar, None);
        y += h + 12.0;
    }
    if job.pending() {
        let busy = st.workers.cancel.is_some();
        let label = if busy { "Ending\u{2026}" } else { "Cancel job\u{2026}" };
        let r = Rect::new(x, y, (p.measure(Face::Semibold, 13.0, label) + 32.0).round(), 32.0);
        let t = Target::WorkerCancel;
        if busy {
            mdshared_ui::overlay::draw_dialog_button_disabled(p, pal, r, label, false);
        } else {
            draw_dialog_button(p, pal, r, label, false, st.hover == Some(t), st.pressed == Some(t));
            st.hits.push((r, t));
        }
        y += 32.0 + 22.0;
    }
    p.fill(Rect::new(x, y - 8.0, w, p.px()), pal.line);
    p.text(Face::Semibold, 13.0, x, y + 10.0, "Output", pal.text, w);
    y += 26.0;
    // The rest of the pane, but never so short that the output has no room: in a short window
    // the details scroll to show it whole.
    let rest = area.bottom() - (y + offset) - 12.0;
    let region = Rect::new(x, y, w, rest.max((area.h - 60.0).min(240.0)).max(0.0));
    if let Some(output) = st.workers.output.as_mut().filter(|o| o.id == job.id) {
        draw_output(p, pal, region, area, output);
    }
    let content = region.bottom() + 12.0 + offset - area.y;
    st.workers.details_max = (content - area.h).max(0.0);
    for (r, _) in &mut st.hits[first_hit..] {
        let bottom = r.bottom().min(area.bottom());
        r.y = r.y.max(area.y);
        r.h = (bottom - r.y).max(0.0);
    }
    if let Some((track, thumb)) = mdshared_ui::scroll::scrollbar(area, content, area.h, offset) {
        mdshared_ui::scroll::draw_scrollbar(p, pal, track, thumb, false);
    }
    p.restore(clip);
}

const ENTRY_TEXT: f32 = 13.0;
const LINE_H: f32 = 20.0;

/// The height an entry takes at `width`.
fn entry_height(p: &mut Painter, e: &log::Entry, width: f32) -> f32 {
    use log::Kind;
    match e.kind {
        Kind::Tool | Kind::Note => LINE_H,
        Kind::Text => p.paragraph_height_wrapped(Face::Ui, ENTRY_TEXT, width, &e.text, Wrap::Anywhere) + 10.0,
        Kind::Thought => p.paragraph_height_wrapped(Face::Ui, 12.0, width, &e.text, Wrap::Anywhere) + 8.0,
        Kind::Error => p.paragraph_height_wrapped(Face::Ui, 12.5, width, &entry_line(e), Wrap::Anywhere) + 8.0,
        Kind::Result => p.paragraph_height_wrapped(Face::Semibold, 12.5, width, &entry_line(e), Wrap::Anywhere) + 8.0,
    }
}

fn entry_line(e: &log::Entry) -> String {
    if e.text.is_empty() { e.title.clone() } else { format!("{}: {}", e.title, e.text) }
}

fn draw_output(p: &mut Painter, pal: &Palette, area: Rect, view: Rect, o: &mut Output) {
    // The pointer is the output's where it shows.
    let (top, bottom) = (area.y.max(view.y), area.bottom().min(view.bottom()));
    o.area = Rect::new(area.x, top, area.w, (bottom - top).max(0.0));
    let width = (area.w - 12.0).max(40.0);
    if o.width != width {
        o.width = width;
        o.heights.clear();
    }
    while o.heights.len() < o.entries.len() {
        let h = entry_height(p, &o.entries[o.heights.len()], width);
        o.heights.push_back(h);
    }
    let lead = if o.cut { 24.0 } else { 0.0 };
    let content = lead + o.heights.iter().sum::<f32>();
    o.max = (content - area.h).max(0.0);
    o.offset = if o.follow { o.max } else { o.offset.clamp(0.0, o.max) };
    let clip = p.clip(area);
    if o.entries.is_empty() {
        let text = match &o.error {
            Some(e) => e.as_str(),
            None if o.reading.is_some() || o.tail.as_ref().is_some_and(|t| t.revision == 0) => "Reading\u{2026}",
            None => "No output yet.",
        };
        p.text(Face::Ui, 13.0, area.x, area.y + 10.0, text, pal.muted, area.w);
    }
    let mut y = area.y - o.offset;
    if o.cut {
        p.text(Face::Ui, 11.5, area.x, y + 10.0, "Earlier output is not shown.", pal.faint, width);
        y += lead;
    }
    use log::Kind;
    for (e, &h) in o.entries.iter().zip(&o.heights) {
        if y + h < area.y {
            y += h;
            continue;
        }
        if y > area.bottom() {
            break;
        }
        match e.kind {
            Kind::Tool => {
                let tw = p.text(Face::Semibold, 12.0, area.x + 14.0, y + 10.0, &e.title, pal.muted, width * 0.4);
                p.icon(mdshared_ui::glyph::RIGHT, 9.0, area.x + 5.0, y + 10.0, pal.faint);
                p.text(
                    Face::Mono,
                    11.5,
                    area.x + 22.0 + tw,
                    y + 10.0,
                    &e.text,
                    pal.faint,
                    (width - 22.0 - tw).max(0.0),
                );
            }
            Kind::Note => {
                let line = entry_line(e);
                p.text(Face::Ui, 12.0, area.x, y + 10.0, &line, pal.faint, width);
            }
            Kind::Text => {
                p.paragraph_wrapped(Face::Ui, ENTRY_TEXT, area.x, y + 4.0, width, &e.text, pal.text, Wrap::Anywhere);
            }
            Kind::Thought => {
                p.paragraph_wrapped(Face::Ui, 12.0, area.x, y + 3.0, width, &e.text, pal.muted, Wrap::Anywhere);
            }
            Kind::Error => {
                let c = mdshared_ui::tokens::legible(pal.warn, pal.page, 4.5);
                p.paragraph_wrapped(Face::Ui, 12.5, area.x, y + 3.0, width, &entry_line(e), c, Wrap::Anywhere);
            }
            Kind::Result => {
                p.paragraph_wrapped(
                    Face::Semibold,
                    12.5,
                    area.x,
                    y + 3.0,
                    width,
                    &entry_line(e),
                    pal.text,
                    Wrap::Anywhere,
                );
            }
        }
        y += h;
    }
    p.restore(clip);
    if let Some((track, thumb)) = mdshared_ui::scroll::scrollbar(area, content, area.h, o.offset) {
        mdshared_ui::scroll::draw_scrollbar(p, pal, track, thumb, false);
    }
}

#[cfg(test)]
mod tests;
