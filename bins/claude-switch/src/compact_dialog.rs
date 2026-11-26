//! Handover dialog state, consent, and worker lifecycle. Drawing lives in `draw`.
pub(crate) mod agents;
mod choices;
mod draw;
pub(crate) mod editor;
mod save_reply;
use crate::compact_runner as runner;
use crate::dialogs::{CANCEL, Dialog, OK};
use crate::source::Source;
use crate::state::{AppState, Target};
use claude_switch_core::{Session, compact};
pub use draw::draw_history_row;
use editor::Editor;
use mdshared_app::job::Job;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::{gfx::Rect, platform};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

pub const COMMAND: u32 = 913;
pub const LABEL: &str = "Compact and open…";
const MODEL: u32 = 10;
const ACCOUNT: u32 = 11;
const REFRESH: u32 = 12;
pub(crate) const EDITOR: u32 = 13;
const BACK: u32 = 14;
const COPY: u32 = 15;
const SAVE: u32 = 16;
const PREV: u32 = 17;
const NEXT: u32 = 18;
const AGENT: u32 = 19;
const OPTION: u32 = 100;
const PAGE: usize = 7;

#[derive(Default)]
pub struct Cache {
    models: Option<Vec<String>>,
    retiring: Vec<Work>,
    pub startup: Option<crate::compact_open::NewHandover>,
    pub startup_ack: Option<String>,
    pub standalone_window: bool,
    pub demo: Option<bool>,
    pub demo_long: bool,
    pub history: Option<(String, Vec<PathBuf>)>,
}

enum Update {
    Loaded(Option<std::sync::Arc<compact::Handover>>, Result<Vec<String>, String>),
    Preview(String),
    Agents(String, Vec<agents::Agent>),
}

type Work = Job<Result<Update, String>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Model,
    Account,
    Agent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Setup,
    Consent,
    Preview,
    History,
}

pub struct Compact {
    session: Session,
    source_tab: Option<u64>,
    name: String,
    program: Option<PathBuf>,
    data: Option<std::sync::Arc<compact::Handover>>,
    models: Vec<String>,
    model: usize,
    accounts: Vec<(String, String)>,
    account: usize,
    agents: Vec<agents::Agent>,
    agents_loaded: bool,
    agent: usize,
    agent_focus: usize,
    focus: Editor,
    preview: Editor,
    saved: Option<PathBuf>,
    history: Vec<PathBuf>,
    phase: Phase,
    choice: Option<Choice>,
    scroll: usize,
    edit_area: Rect,
    cell_width: f32,
    job: Option<Work>,
    saving: Option<mpsc::Receiver<Result<Option<()>, String>>>,
    message: Option<(String, bool)>,
}

impl Drop for Compact {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}

impl AppState {
    pub fn start_compact(&mut self) {
        if let Some(request) = self.compact.startup.take() {
            let result = self.open_handover_tab(request);
            if let Some(key) = self.compact.startup_ack.take() {
                crate::compact_cli::acknowledge(&key, result.is_ok());
            }
            if let Err(e) = result {
                self.error("Could not open handover", e);
            }
        }
        if let Some(preview) = self.compact.demo.take() {
            self.apply_scan(crate::demo::scan(&self.config));
            self.select_first();
            self.ask_compact(false);
            if preview && let Some(Dialog::Compact(dialog)) = &mut self.dialog {
                dialog.phase = Phase::Preview;
                let mut text = demo_text(&dialog.name);
                if self.compact.demo_long {
                    text.push_str(&"\nSynthetic handover evidence.".repeat(600));
                }
                dialog.preview.set(text);
            }
        }
    }

    pub fn forget_handover_consents(&mut self) {
        if let Source::Real(roots) = &self.source {
            match claude_switch_core::config::update_compact(roots, |p| {
                p.consents.clear();
                Ok(())
            }) {
                Ok(prefs) => self.config.compact = prefs,
                Err(e) => {
                    self.error("Could not forget consents", e.to_string());
                    return;
                }
            }
        } else {
            self.config.compact.consents.clear();
        }
        self.toast("Handover provider consents cleared.".into(), Instant::now());
    }

    pub fn load_handover_history(&mut self, id: &str) {
        self.compact.history = match &self.source {
            Source::Real(roots) => compact::list(roots, id).ok().map(|files| (id.into(), files)),
            Source::Demo => None,
        };
    }

    pub fn ask_compact(&mut self, history: bool) {
        let Some(session) = self.selected_session().cloned() else { return };
        self.ask_compact_session(session, history);
    }

    pub fn ask_compact_session(&mut self, session: Session, history: bool) {
        self.ask_compact_for(session, None, history);
    }

    pub fn ask_tab_compact(&mut self, tab: u64, account: Option<&str>) {
        let Some(id) = self.recheck_tab_identity(tab) else {
            self.warn("Session not verified", "Choose the exact session from the Sessions list.".into());
            return;
        };
        let Some(session) = self.by_id.get(&id).and_then(|&i| self.sessions.get(i)).cloned() else {
            self.refresh(Instant::now());
            self.warn(
                "Session not indexed yet",
                "Try the handover again after the Sessions list refreshes. If it is still missing, use Refresh.".into(),
            );
            return;
        };
        self.ask_compact_for(session, account, false);
        if let Some(Dialog::Compact(dialog)) = &mut self.dialog {
            dialog.source_tab = Some(tab);
        }
    }

    #[cfg(test)]
    pub(crate) fn compact_source(&self) -> Option<(&str, Option<u64>)> {
        match &self.dialog {
            Some(Dialog::Compact(dialog)) => Some((&dialog.session.id, dialog.source_tab)),
            _ => None,
        }
    }

    pub fn ask_compact_for(&mut self, session: Session, account: Option<&str>, history: bool) {
        let mut dialog = Compact::new(self, session, account);
        if history {
            dialog.phase = Phase::History;
            if let Source::Real(roots) = &self.source {
                match compact::list(roots, &dialog.session.id) {
                    Ok(files) => dialog.history = files,
                    Err(e) => dialog.error(format!("Could not list handovers: {e}")),
                }
            }
        } else if self.source.is_demo() {
            dialog.focus.set("Keep the decision and the remaining verification.".into());
            dialog.models = vec!["demo/summary-fast".into(), "demo/summary-thorough".into()];
            self.compact.models = Some(dialog.models.clone());
        } else {
            dialog.load(self, false);
        }
        self.dialog = Some(Dialog::Compact(Box::new(dialog)));
    }

    pub fn take_compact_jobs(&mut self) -> bool {
        self.compact.retiring.retain(Job::running);
        let Some(Dialog::Compact(mut dialog)) = self.dialog.take_if(|d| matches!(d, Dialog::Compact(_))) else {
            return false;
        };
        let mut changed = false;
        if let Some(job) = &dialog.job
            && !job.running()
        {
            let result = job.take().unwrap_or_else(|| Err("The handover worker stopped unexpectedly.".into()));
            dialog.job = None;
            dialog.apply(self, result);
            changed = true;
        }
        if let Some(rx) = &dialog.saving
            && let Some(result) = save_reply::take(rx)
        {
            dialog.saving = None;
            match result {
                Ok(Some(())) => dialog.message = Some(("Saved a Markdown copy.".into(), false)),
                Ok(None) => {}
                Err(e) => dialog.error(e),
            }
            changed = true;
        }
        self.dialog = Some(Dialog::Compact(dialog));
        changed
    }

    pub fn cancel_compact_work(&mut self) -> bool {
        if let Some(Dialog::Compact(dialog)) = &mut self.dialog
            && let Some(job) = dialog.job.take()
        {
            job.cancel();
            self.compact.retiring.push(job);
        }
        self.compact.retiring.retain(Job::running);
        !self.compact.retiring.is_empty()
    }
}

impl Compact {
    fn new(st: &AppState, session: Session, account: Option<&str>) -> Self {
        let accounts: Vec<_> = st.config.accounts.iter().map(|a| (a.slug.clone(), a.name.clone())).collect();
        let account = account.unwrap_or(&st.current().slug);
        let account = accounts.iter().position(|(slug, _)| slug == account).unwrap_or(0);
        let name = st.title_of(&session).to_owned();
        Self {
            source_tab: None,
            session,
            name,
            accounts,
            account,
            agents: Vec::new(),
            agents_loaded: false,
            agent: 0,
            agent_focus: 0,
            program: None,
            data: None,
            models: Vec::new(),
            model: 0,
            focus: Editor::default(),
            preview: Editor::default(),
            saved: None,
            history: Vec::new(),
            phase: Phase::Setup,
            choice: None,
            scroll: 0,
            edit_area: Rect::default(),
            cell_width: 1.0,
            job: None,
            saving: None,
            message: None,
        }
    }

    fn error(&mut self, text: String) {
        self.message = Some((text, true));
    }

    fn load(&mut self, st: &AppState, refresh: bool) {
        if self.job.is_some() || st.source.is_demo() {
            return;
        }
        let program = match runner::opencode() {
            Ok(program) => program,
            Err(e) => {
                self.program = None;
                self.models.clear();
                self.error(e);
                return;
            }
        };
        self.program = Some(program.clone());
        let read = self.data.is_none();
        let (session, name) = (self.session.clone(), self.name.clone());
        let models = (!refresh).then(|| st.compact.models.clone()).flatten();
        self.message = None;
        self.job = Job::start(st.wake_fn(), move |cancel, _| {
            let data = if read {
                Some(
                    compact::Handover::read_cancel(session, name, || cancel.cancelled())
                        .map(std::sync::Arc::new)
                        .map_err(|e| format!("Could not read the transcript: {e}"))?,
                )
            } else {
                None
            };
            let models = models.map_or_else(|| runner::list(&program, || cancel.cancelled()), Ok);
            Ok(Update::Loaded(data, models))
        })
        .map_err(|_| self.error("Could not start the handover worker.".into()))
        .ok();
    }

    fn apply(&mut self, st: &mut AppState, result: Result<Update, String>) {
        match result {
            Ok(Update::Loaded(data, models)) => {
                if let Some(data) = data {
                    self.focus.set(data.focus().unwrap_or_default().to_owned());
                    self.data = Some(data);
                }
                match models {
                    Ok(models) => {
                        let selected = self.models.get(self.model).unwrap_or(&st.config.compact.model);
                        self.model = models.iter().position(|m| m == selected).unwrap_or(0);
                        st.compact.models = Some(models.clone());
                        self.models = models;
                    }
                    Err(e) => self.error(e),
                }
            }
            Ok(Update::Preview(text)) => {
                self.preview.set(text);
                self.phase = Phase::Preview;
                self.saved = None;
            }
            Ok(Update::Agents(slug, agents)) if self.accounts.get(self.account).is_some_and(|a| a.0 == slug) => {
                self.agents = agents;
                self.agents_loaded = true;
                self.agent_focus = self.agent;
                if self.choice == Some(Choice::Agent) {
                    self.scroll = self.agent.min(self.count().saturating_sub(PAGE));
                }
            }
            Ok(Update::Agents(..)) => {}
            Err(e) => self.error(e),
        }
    }

    fn make(&mut self, st: &mut AppState, approve: bool) {
        if self.job.is_some() {
            return;
        }
        if let Some(tab) = self.source_tab
            && st.recheck_tab_identity(tab).as_deref() != Some(self.session.id.as_str())
        {
            self.error("The terminal's session changed or could not be verified. Nothing was sent.".into());
            return;
        }
        let Some(model) = self.models.get(self.model).cloned() else { return };
        let Some(provider) = compact::provider(&model) else { return };
        self.message = None;
        if st.source.is_demo() {
            if !approve && !st.config.compact.consents.contains(provider) {
                self.phase = Phase::Consent;
            } else {
                st.config.compact.consents.insert(provider.into());
                st.config.compact.model = model;
                self.apply(st, Ok(Update::Preview(demo_text(&self.name))));
            }
            return;
        }
        let (Some(program), Some(data)) = (self.program.clone(), self.data.clone()) else { return };
        let Source::Real(roots) = &st.source else { return };
        let mut ask = false;
        let result = claude_switch_core::config::update_compact(roots, |prefs| {
            if !approve && !prefs.consents.contains(provider) {
                ask = true;
                return Err(claude_switch_core::Refusal::Invalid("Provider consent required.".into()));
            }
            prefs.consents.insert(provider.into());
            prefs.model = model.clone();
            Ok(())
        });
        match result {
            Ok(prefs) => st.config.compact = prefs,
            Err(_) if ask => {
                self.phase = Phase::Consent;
                return;
            }
            Err(e) => {
                self.error(format!("Could not remember handover consent: {e}"));
                return;
            }
        }
        self.phase = Phase::Setup;
        let focus = self.focus.text.clone();
        let roots = roots.clone();
        self.job = Job::start(st.wake_fn(), move |cancel, _| {
            runner::make(&roots, &program, &model, &data, &focus, || cancel.cancelled()).map(Update::Preview)
        })
        .map_err(|_| self.error("Could not start the handover worker.".into()))
        .ok();
    }

    fn export(&mut self, st: &AppState) {
        if self.saving.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let reply = save_reply::Reply::new(tx, st.wake_fn());
        let text = self.preview.text.clone();
        platform::pick_save_async(&PathBuf::from("handover.md"), move |file| {
            let result = file
                .map(|path| {
                    // Save As cannot overwrite any existing file, including the original transcript.
                    compact::export(&path, &text).map_err(|e| format!("Could not save the Markdown copy: {e}"))
                })
                .transpose();
            reply.complete(result);
        });
        self.saving = Some(rx);
    }

    fn open(&mut self, st: &mut AppState, now: Instant) -> Result<(), String> {
        let Some((account, _)) = self.accounts.get(self.account) else { return Err("Choose an account.".into()) };
        let Source::Real(roots) = &st.source else {
            st.toast("Demo: no session was opened.".into(), now);
            return Ok(());
        };
        crate::compact_open::validate_prompt(&self.preview.text)?;
        let saved = crate::compact_open::keep(roots, &self.session.id, &self.preview.text, self.saved.as_deref())?;
        let request = crate::compact_open::prepare(
            roots,
            &st.config,
            &self.session,
            self.name.clone(),
            account,
            &self.preview.text,
            &saved,
        )?
        .with_agent(self.agent_name())?;
        self.saved = Some(saved);
        st.open_handover_tab(request)
    }

    pub fn click(&mut self, st: &mut AppState, now: Instant, id: u32, x: f32) -> bool {
        match id {
            CANCEL => {
                if let Some(job) = self.job.take() {
                    job.cancel();
                    st.compact.retiring.push(job);
                }
                return false;
            }
            OK if self.phase == Phase::Setup => self.make(st, false),
            OK if self.phase == Phase::Consent => self.make(st, true),
            OK if self.phase == Phase::Preview && self.choice.is_none() => match self.open(st, now) {
                Ok(()) => return false,
                Err(e) => self.error(e),
            },
            BACK if self.choice.take().is_some() => {}
            BACK if matches!(self.phase, Phase::Consent | Phase::Preview) => {
                self.phase = Phase::Setup;
                self.message = None;
            }
            COPY if self.phase == Phase::Preview => {
                platform::set_clipboard_text(&self.preview.text);
                self.message = Some(("Handover copied.".into(), false));
            }
            SAVE if self.phase == Phase::Preview => self.export(st),
            REFRESH if self.phase == Phase::Setup => self.load(st, true),
            MODEL if self.phase == Phase::Setup && self.job.is_none() => self.pick(Choice::Model, st),
            ACCOUNT if matches!(self.phase, Phase::Setup | Phase::Preview) && self.job.is_none() => {
                self.pick(Choice::Account, st);
            }
            AGENT if self.phase == Phase::Preview => self.pick(Choice::Agent, st),
            EDITOR => {
                let px = self.edit_area.x;
                let py = self.edit_area.y;
                let col = ((x - px) / self.cell_width).round().max(0.0) as usize;
                let row = ((st.mouse.1 - py) / draw::LINE).floor().max(0.0) as usize;
                let editor = self.editor();
                let byte = editor.row_byte(editor.scroll + row, col);
                editor.move_to(byte, false);
            }
            NEXT | PREV => {
                self.page(id == NEXT);
                if self.choice == Some(Choice::Agent) {
                    self.agent_focus = self.scroll;
                }
            }
            id if id >= OPTION && self.phase == Phase::History => {
                if let Some(path) = self.history.get((id - OPTION) as usize)
                    && !platform::open_file(path, &["md"])
                {
                    self.error("Could not open that Markdown file.".into());
                }
            }
            id if id >= OPTION => self.choose(st, (id - OPTION) as usize),
            _ => {}
        }
        true
    }

    pub fn key(&mut self, k: &KeyName, mods: Mods, text: Option<&str>) -> Option<u32> {
        if *k == KeyName::Escape {
            if self.choice.take().is_some() {
                return None;
            }
            return Some(if self.phase == Phase::Consent { BACK } else { CANCEL });
        }
        if self.choice == Some(Choice::Agent) {
            return self.agent_key(k);
        }
        if self.choice.is_some() || self.phase == Phase::History {
            match k {
                KeyName::PageUp | KeyName::Up => self.page(false),
                KeyName::PageDown | KeyName::Down => self.page(true),
                KeyName::Enter if self.choice.is_some() => return Some(OPTION + self.scroll as u32),
                _ => {}
            }
        } else if self.phase == Phase::Preview && matches!(k, KeyName::Char('a' | 'A')) && mods.alt && !mods.ctrl {
            return Some(AGENT);
        } else if *k == KeyName::Enter && mods.ctrl && self.phase == Phase::Setup {
            return Some(OK);
        } else if self.phase == Phase::Preview || (self.phase == Phase::Setup && self.job.is_none()) {
            self.editor().key(k, mods, text);
        }
        None
    }

    pub fn text_input(&mut self, text: &str) {
        if self.choice.is_none() && (self.phase == Phase::Preview || (self.phase == Phase::Setup && self.job.is_none()))
        {
            self.editor().insert(text);
        }
    }

    fn editor(&mut self) -> &mut Editor {
        if self.phase == Phase::Preview { &mut self.preview } else { &mut self.focus }
    }

    fn page(&mut self, next: bool) {
        if next {
            self.scroll = (self.scroll + PAGE).min(self.count().saturating_sub(PAGE));
        } else {
            self.scroll = self.scroll.saturating_sub(PAGE);
        }
    }

    pub fn wheel(&mut self, dy: f32) {
        if self.choice.is_some() || self.phase == Phase::History {
            self.scroll = if dy > 0.0 {
                self.scroll.saturating_sub(1)
            } else {
                (self.scroll + 1).min(self.count().saturating_sub(PAGE))
            };
            if self.choice == Some(Choice::Agent) {
                self.agent_focus = self.agent_focus.clamp(self.scroll, self.scroll + PAGE - 1);
            }
        } else {
            self.editor().wheel(dy);
        }
    }
}

fn demo_text(name: &str) -> String {
    format!(
        "## Goal\n\n> Find why a clean release build takes four minutes.\n\n\
         ## Where it stands\n\n> Compared the clean and warm builds. Most time is in dependency compilation.\n\n\
         ## Decisions and why\n\n> Use a separate target directory so a clean timing cannot disturb an existing build.\n\n\
         ## Standing instructions from the user\n\n> Keep the toolkit unchanged.\n\n\
         ## Open items\n\n> Repeat the timing after disabling the optional feature.\n\n\
         ## Not verified\n\n> Timing on a second machine.\n\n\
         ## Files\n\n- docs/build-notes.md (2)\n\n\
         ## Source session\n\nName: {name}\nID: demo\nFolder: C:\\demo\\mdplay\nBranch: main\n\
         Transcript: C:\\demo\\mdplay.jsonl\n\nLook up further details in the original transcript.\n\n\
         ## Latest task list\n\n- [pending] Repeat the feature comparison\n\n\
         ## Last 15 shell commands\n\n    cargo build --release --timings\n\n\
         ## User prompts (verbatim)\n\n### 2026-11-26T09:00:00Z\n\n```\nCompare a clean build with an incremental build and report the timings.\n```\n\n\
         ## Last 6 turns (assistant text only)\n\n```\nThe warm run does not reproduce the delay.\n```\n"
    )
}

#[cfg(test)]
mod tests;
