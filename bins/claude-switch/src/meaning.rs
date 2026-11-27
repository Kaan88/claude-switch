//! Search, questions and grouping by meaning: the window's side. The model runs on a worker job,
//! never on the window thread; every run that sends something is confirmed first, names what is
//! sent and to which model, and can be cancelled. See DESIGN.md, "Session organization".

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use claude_switch_core::meaning::{self, Answer, Cache, Engine, IndexReport};
use claude_switch_core::naming::{self, Excerpts};
use mdshared_app::job::Job;
use serde_json::{Value, json};

use crate::dialogs::{Dialog, Pending};
use crate::state::AppState;

pub enum Done {
    Indexed(IndexReport),
    Answered(Answer),
}

type Outcome = Result<Done, String>;
type Models = Result<Vec<String>, String>;

#[derive(Default)]
pub struct Meaning {
    pub cache: Cache,
    loaded: bool,
    job: Option<Job<Outcome>>,
    /// The cache as the job last saved it.
    inbox: Arc<Mutex<Option<Cache>>>,
    models: Option<Job<Models>>,
}

impl Meaning {
    pub fn running(&self) -> bool {
        self.job.as_ref().is_some_and(Job::running)
    }

    pub fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }

    pub fn listing_models(&self) -> bool {
        self.models.as_ref().is_some_and(Job::running)
    }
}

/// Where one request goes.
enum Route {
    /// `--demo`: synthetic answers made here; nothing leaves the computer.
    Demo,
    Claude {
        program: PathBuf,
        account: claude_switch_core::Account,
    },
    OpenCode {
        program: PathBuf,
        model: String,
    },
}

impl Route {
    fn submit(&self, request: &str, cancelled: &dyn Fn() -> bool) -> Result<String, String> {
        match self {
            Self::Demo => Ok(demo_answer(request)),
            Self::Claude { program, account } => {
                naming::ask_claude(program, account, request, cancelled).map_err(|e| e.to_string())
            }
            Self::OpenCode { program, model } => {
                crate::compact_runner::search_request(program, model, request, cancelled)
            }
        }
    }
}

/// The demo's stand-in model, also the tests' fake: a summary from each first prompt, and an
/// answer naming the first three summaries.
pub fn demo_answer(request: &str) -> String {
    let payload: Value = request.lines().last().and_then(|l| serde_json::from_str(l).ok()).unwrap_or(Value::Null);
    let items = payload.as_array().cloned().unwrap_or_default();
    if request.starts_with("Below are short summaries") {
        let ids: Vec<_> = (0..items.len().min(3)).collect();
        return json!({"answer": "Demo only: these sessions look closest. Nothing was sent.", "ids": ids}).to_string();
    }
    items
        .iter()
        .map(|item| {
            let first = item["first"].as_str().unwrap_or("a demo task");
            let words: Vec<_> = first.split_whitespace().take(12).collect();
            let subject = if first.to_lowercase().contains("swipe") { "Gestures" } else { "Demo work" };
            json!({
                "id": item["id"],
                "summary": format!("Worked on: {}", words.join(" ")),
                "keywords": words.iter().take(4).collect::<Vec<_>>(),
                "subject": subject,
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl AppState {
    fn meaning_file(&self) -> Option<PathBuf> {
        match &self.source {
            crate::source::Source::Real(roots) => Some(Cache::path(roots)),
            crate::source::Source::Demo => None,
        }
    }

    /// Reads the kept summaries once, when search by meaning is on.
    pub fn load_meaning(&mut self) {
        if self.config.meaning.on() && !self.meaning.loaded {
            self.meaning.loaded = true;
            if let Some(path) = self.meaning_file() {
                self.meaning.cache = Cache::load(&path);
            }
        }
    }

    /// The local, model-free match of the list's search box against a kept summary.
    pub fn meaning_match(&self, id: &str, query: &str) -> bool {
        self.config.meaning.on()
            && self.meaning.cache.digests.get(id).is_some_and(|d| meaning::score(query, d).is_some())
    }

    /// The kept summary's subject, for grouping Topics by meaning.
    pub fn meaning_subject(&self, id: &str) -> Option<&str> {
        self.meaning.cache.digests.get(id)?.subject.as_deref()
    }

    pub fn meaning_summary(&self, id: &str) -> Option<&str> {
        if !self.config.meaning.on() {
            return None;
        }
        self.meaning.cache.digests.get(id).filter(|d| !d.is_marker()).map(|d| d.summary.as_str())
    }

    pub fn meaning_engine_label(&self) -> String {
        self.config.meaning.engine.label(&naming::worker_account(&self.config).name)
    }

    fn meaning_ready(&mut self) -> bool {
        if !self.config.meaning.on() {
            self.warn(
                "Search by meaning is off",
                "Turn it on in Settings › Sessions › Search by meaning and pick the model there.".into(),
            );
            return false;
        }
        if self.config_damaged {
            self.warn("Search by meaning unavailable", "The settings could not be read. Nothing was sent.".into());
            return false;
        }
        if self.meaning.running() || self.naming_job.as_ref().is_some_and(Job::running) {
            self.warn("A model request is running", "Wait for it to finish or cancel it.".into());
            return false;
        }
        self.load_meaning();
        true
    }

    /// Visible, not hidden sessions, newest first: the only ones whose prompts may be sent.
    fn meaning_sessions(&self) -> Vec<usize> {
        let mut rows: Vec<_> = (0..self.sessions.len())
            .filter(|&i| {
                let s = &self.sessions[i];
                self.listed(s) && !self.hidden_session(s)
            })
            .collect();
        rows.sort_unstable_by_key(|&i| (std::cmp::Reverse(self.sessions[i].last_active_ms), &self.sessions[i].id));
        rows
    }

    fn meaning_stale(&self) -> Vec<usize> {
        self.meaning_sessions()
            .into_iter()
            .filter(|&i| self.meaning.cache.stale(&self.sessions[i].id, self.sessions[i].last_active_ms))
            .take(meaning::RUN_LIMIT)
            .collect()
    }

    pub fn ask_meaning_index(&mut self) {
        if !self.meaning_ready() {
            return;
        }
        let count = self.meaning_stale().len();
        if count == 0 {
            self.note("Nothing to summarize", "Every visible session already has a current summary.".into());
            return;
        }
        let model = self.meaning_engine_label();
        let requests = count.div_ceil(meaning::BATCH);
        let text = if self.source.is_demo() {
            format!("Demo only: make synthetic summaries for {count} sessions. Nothing is sent.")
        } else {
            format!(
                "Send the first and up to three recent prompts (up to 500 characters each) of up to {count} \
                 visible sessions to {model}, in {requests} request(s) of up to {}. The same excerpts as Suggest \
                 names; hidden sessions are never sent. Existing subject labels go along so groups stay stable.\n\n\
                 Each answer (a one-line summary, keywords and a subject) is kept on this computer in \
                 meaning.json beside the settings. Search and grouping then run locally; a session is sent again \
                 only when its prompts change. At most {} sessions per run.",
                meaning::BATCH,
                meaning::RUN_LIMIT
            )
        };
        self.confirm_scroll = 0.0;
        self.dialog = Some(Dialog::Confirm {
            title: format!("Summarize {count} sessions for search by meaning?"),
            text,
            ok: "Summarize".into(),
            then: Pending::MeaningIndex,
        });
    }

    fn meaning_route(&self) -> Result<Route, String> {
        if self.source.is_demo() {
            return Ok(Route::Demo);
        }
        match &self.config.meaning.engine {
            Engine::Claude => Ok(Route::Claude {
                program: crate::run::native_program(Path::new("claude")).map_err(|e| e.to_string())?,
                account: naming::worker_account(&self.config).clone(),
            }),
            Engine::OpenCode(model) => {
                Ok(Route::OpenCode { program: crate::compact_runner::opencode()?, model: model.clone() })
            }
        }
    }

    fn start_meaning_job(
        &mut self,
        work: impl FnOnce(&Route, &mdshared_app::job::Cancel, &mut Cache, &dyn Fn(&Cache)) -> Outcome + Send + 'static,
    ) {
        if !self.meaning_ready() {
            return;
        }
        let route = match self.meaning_route() {
            Ok(route) => route,
            Err(e) => return self.error("Could not start the request", e),
        };
        let mut cache = self.meaning.cache.clone();
        let file = self.meaning_file();
        let inbox = Arc::new(Mutex::new(None));
        self.meaning.inbox = Arc::clone(&inbox);
        let job = Job::start(self.wake_fn(), move |cancel, _| {
            let saved = |cache: &Cache| {
                if let Some(path) = &file {
                    let _ = cache.save(path);
                }
                *inbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(cache.clone());
            };
            work(&route, cancel, &mut cache, &saved)
        });
        match job {
            Ok(job) => {
                self.meaning.job = Some(job);
                let model = self.meaning_engine_label();
                self.dialog = Some(crate::session_panel::Panel::meaning_working(model));
            }
            Err(e) => self.error("Could not start the request", e.to_string()),
        }
    }

    pub fn start_meaning_index(&mut self) {
        let rows = self.meaning_stale();
        let demo = self.source.is_demo();
        let sessions: Vec<_> = rows
            .into_iter()
            .map(|i| {
                let s = &self.sessions[i];
                (s.id.clone(), s.last_active_ms, s.file.clone(), demo.then(|| (s.title.clone(), s.last_prompt.clone())))
            })
            .collect();
        if sessions.is_empty() {
            return;
        }
        // Subjects go along only from sessions that may be sent now (not hidden since).
        let visible: std::collections::HashSet<String> =
            self.meaning_sessions().into_iter().map(|i| self.sessions[i].id.clone()).collect();
        let engine = self.config.meaning.engine.clone();
        self.start_meaning_job(move |route, cancel, cache, saved| {
            let mut items = Vec::new();
            for (id, active_ms, file, demo) in sessions {
                if cancel.cancelled() {
                    return Err("Cancelled".into());
                }
                let excerpts = match demo {
                    Some((title, last)) => Excerpts { first: Some(title), last: vec![last] },
                    None => {
                        let (first, last) = claude_switch_core::sessions::naming_prompts(&file);
                        Excerpts { first, last }
                    }
                };
                items.push(meaning::Item { id, active_ms, excerpts });
            }
            let cancelled = || cancel.cancelled();
            let visible = |id: &str| visible.contains(id);
            meaning::index(
                cache,
                items,
                visible,
                &engine,
                |request| route.submit(request, &cancelled),
                cancelled,
                saved,
            )
            .map(Done::Indexed)
        });
    }

    /// The summaries a question goes over, newest visible sessions first.
    fn meaning_set(&self) -> Vec<(String, meaning::Digest)> {
        let ids: Vec<_> = self.meaning_sessions().into_iter().map(|i| self.sessions[i].id.as_str()).collect();
        meaning::ask_set(&self.meaning.cache, ids).into_iter().map(|(id, d)| (id.to_owned(), d.clone())).collect()
    }

    pub fn ask_meaning_question(&mut self) {
        if self.meaning_ready() {
            self.dialog = Some(crate::session_panel::Panel::meaning_ask(""));
        }
    }

    /// A kept answer is shown at once; anything else asks before sending.
    pub fn submit_meaning_question(&mut self, question: &str) {
        let question = question.trim().to_owned();
        if question.is_empty() || !self.meaning_ready() {
            self.dialog = Some(crate::session_panel::Panel::meaning_ask(&question));
            return;
        }
        let set = self.meaning_set();
        if set.is_empty() {
            self.dialog = None;
            self.note(
                "No summaries yet",
                "Summarize sessions for search by meaning first (command palette). Questions go over those summaries."
                    .into(),
            );
            return;
        }
        let refs: Vec<_> = set.iter().map(|(id, d)| (id.as_str(), d)).collect();
        let basis = meaning::ask_basis(&refs);
        if let Some(answer) = self.meaning.cache.answer(&question, &self.config.meaning.engine.key(), basis).cloned() {
            self.show_meaning_answer(&answer, true);
            return;
        }
        let model = self.meaning_engine_label();
        let count = set.len();
        let text = if self.source.is_demo() {
            "Demo only: a synthetic answer. Nothing is sent.".to_string()
        } else {
            format!(
                "Send your question and the kept one-line summaries, keywords and subjects of {count} sessions \
                 (made earlier from their first and recent prompts) to {model}. No prompts, folder names or \
                 dates are sent. The answer is kept on this computer, so asking the same again sends nothing.\n\n\
                 Question: {question}"
            )
        };
        self.confirm_scroll = 0.0;
        self.dialog = Some(Dialog::Confirm {
            title: "Ask about past sessions?".into(),
            text,
            ok: "Ask".into(),
            then: Pending::MeaningAsk { question },
        });
    }

    pub fn start_meaning_question(&mut self, question: String) {
        let set = self.meaning_set();
        let engine = self.config.meaning.engine.clone();
        self.start_meaning_job(move |route, cancel, cache, saved| {
            let cancelled = || cancel.cancelled();
            let answer = meaning::ask(cache, &question, &set, &engine, |request| route.submit(request, &cancelled))?;
            saved(cache);
            Ok(Done::Answered(answer))
        });
    }

    fn show_meaning_answer(&mut self, answer: &Answer, kept: bool) {
        let ids: Vec<_> = answer.ids.iter().filter(|id| self.by_id.contains_key(*id)).cloned().collect();
        let text =
            if kept { format!("{}\n\n(Kept answer; nothing was sent.)", answer.text) } else { answer.text.clone() };
        self.dialog = Some(crate::session_panel::Panel::meaning_answer(self, &answer.question, text, ids));
    }

    pub fn take_meaning_job(&mut self) -> bool {
        let mut changed = false;
        if let Some(cache) = self.meaning.inbox.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.meaning.cache = cache;
            changed = true;
        }
        if let Some(job) = self.meaning.models.take_if(|job| !job.running()) {
            match job.take() {
                Some(Ok(models)) => self.compact.set_listed_models(models),
                Some(Err(e)) => self.error("Could not list OpenCode models", e),
                None => {}
            }
            crate::dialogs::refresh_sessions_page(self);
            changed = true;
        }
        if changed {
            self.rebuild_view();
        }
        let Some(job) = self.meaning.job.take_if(|job| !job.running()) else { return changed };
        let working = matches!(&self.dialog, Some(Dialog::Session(p)) if p.meaning_working_now());
        if working {
            self.dialog = None;
        }
        if job.cancelled() || !working {
            return true;
        }
        match job.take() {
            Some(Ok(Done::Indexed(report))) => {
                let mut text = format!("{} of {} sent sessions summarized.", report.summarized, report.sent);
                if report.unchanged > 0 {
                    text.push_str(&format!(" {} unchanged (not sent).", report.unchanged));
                }
                if report.empty > 0 {
                    text.push_str(&format!(" {} without a readable prompt.", report.empty));
                }
                if report.skipped > 0 {
                    let n = report.skipped;
                    text.push_str(&format!(" {n} not answered (not sent again until their prompts change)."));
                }
                self.note("Sessions summarized", text);
            }
            Some(Ok(Done::Answered(answer))) => self.show_meaning_answer(&answer, false),
            Some(Err(e)) => self.error("Search by meaning failed", e),
            None => {}
        }
        true
    }

    /// Settings › Sessions › List models: OpenCode's models on a worker.
    pub fn list_meaning_models(&mut self) {
        if self.meaning.listing_models() {
            return;
        }
        if self.source.is_demo() {
            self.compact.set_listed_models(vec!["demo/summary-fast".into(), "demo/summary-thorough".into()]);
            crate::dialogs::refresh_sessions_page(self);
            return;
        }
        let program = match crate::compact_runner::opencode() {
            Ok(program) => program,
            Err(e) => return self.error("Could not list OpenCode models", e),
        };
        match Job::start(self.wake_fn(), move |cancel, _| crate::compact_runner::list(&program, || cancel.cancelled()))
        {
            Ok(job) => self.meaning.models = Some(job),
            Err(e) => self.error("Could not list OpenCode models", e.to_string()),
        }
    }

    /// Settings › Sessions › Forget summaries: the kept file is removed.
    pub fn forget_meaning(&mut self) {
        if self.meaning.running() {
            self.warn("A model request is running", "Cancel it or wait, then forget the summaries.".into());
            return;
        }
        self.meaning.cache = Cache::default();
        self.meaning.loaded = true;
        if let Some(path) = self.meaning_file()
            && let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            self.error("Summaries were not removed", e.to_string());
        }
        self.rebuild_view();
    }
}

#[cfg(test)]
mod tests;
