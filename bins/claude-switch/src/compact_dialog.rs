//! Handover dialog state, consent, and worker lifecycle. Drawing lives in `draw`.
pub(crate) mod agents;
mod choices;
mod draw;
pub(crate) mod editor;
mod save_reply;
use crate::compact_open::{self, names};
use crate::compact_runner as runner;
use crate::dialogs::{CANCEL, Dialog, OK};
use crate::source::Source;
use crate::state::{AppState, Target};
use claude_switch_core::{Session, compact};
pub use draw::draw_history_row;
use editor::Editor;
use mdshared_app::job::Job;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::textfield::{FieldHit, TextField};
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
pub(crate) const COPY: u32 = 15;
pub(crate) const SAVE: u32 = 16;
const PREV: u32 = 17;
const NEXT: u32 = 18;
const AGENT: u32 = 19;
pub(crate) const NAME_FIELD: u32 = 20;
const OPENCODE: u32 = 21;
pub(crate) const DELETE_ORIGINAL: u32 = 22;
/// The review's “More ▾”: Copy and Save as… when the footer has no room for them.
const MORE: u32 = 23;
const OPTION: u32 = 100;
const PAGE: usize = 7;

#[derive(Default)]
pub struct Cache {
    models: Option<Vec<String>>,
    pub(crate) retiring: Vec<Work>,
    /// Handovers being written, or written, behind the window (`handover_jobs`).
    pub jobs: crate::handover_jobs::Jobs,
    pub startup: Option<crate::compact_open::NewHandover>,
    pub startup_ack: Option<String>,
    /// The handover channel this window claimed at startup, until it serves it.
    pub listener: Option<crate::handover_pipe::Listener>,
    /// Why the channel could not be claimed, shown once the window runs.
    pub listen_failed: Option<String>,
    pub demo: Option<bool>,
    pub demo_long: bool,
    pub history: Option<(String, Vec<PathBuf>)>,
    /// Settings › Handovers' agent choice: (name, label), read when that page opens.
    pub settings_agents: Vec<(String, String)>,
    pub auto: crate::auto_handover::Watcher,
}

pub(crate) enum Update {
    Loaded(Option<std::sync::Arc<compact::Handover>>, Result<Vec<String>, String>),
    Preview(String),
    Agents(String, Vec<agents::Agent>),
}

pub(crate) type Work = Job<Result<Update, String>>;

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
    /// A tab runs this session as the chosen account: the handover continues there.
    pub(super) in_place: bool,
    name: String,
    suggested_name: String,
    new_name: TextField,
    name_focused: bool,
    name_hit: Option<FieldHit>,
    program: Option<PathBuf>,
    data: Option<std::sync::Arc<compact::Handover>>,
    models: Vec<String>,
    model: usize,
    accounts: Vec<(String, String)>,
    account: usize,
    default_agent: Option<String>,
    agents: Vec<agents::Agent>,
    agents_loaded: bool,
    agent: usize,
    agent_focus: usize,
    focus: Editor,
    /// The focus put in the box when the transcript was read: the session's own `/compact`
    /// focus (kind `None`) or the Settings text for the kind picked for this session.
    picked: Option<(Option<compact::FocusKind>, String)>,
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
    /// "Delete the original session afterwards": off each time the dialog opens.
    pub(crate) delete_original: bool,
    /// Where the review's “More ▾” was drawn, when Copy and Save as… were folded into it.
    more_at: Option<Rect>,
    /// `job` writes the handover: the dialog closes and it goes on in the background.
    generating: bool,
    /// The handover was opened (in the CLI or OpenCode): closing does not keep it.
    acted: bool,
    /// A review that opened by itself ignores keys, and clicks that act, until then.
    settle_until: Option<Instant>,
    /// A review that opened by itself has the dialog focused, not its editor or name field:
    /// typing edits nothing until a click in one of them, or Tab.
    unfocused: bool,
}

impl Drop for Compact {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}

impl AppState {
    /// Reads every account's agents for the Settings choice (local files only, never Claude).
    pub fn load_settings_agents(&mut self) {
        let per_account: Vec<_> = self
            .config
            .accounts
            .iter()
            .map(|account| {
                let found = match &self.source {
                    Source::Real(roots) => agents::discover_account(roots, account, || false),
                    Source::Demo => choices::demo_agents(&account.slug),
                };
                (account.name.clone(), found)
            })
            .collect();
        self.compact.settings_agents = agents::settings_choices(&per_account);
    }

    /// A notice in the open handover dialog (not an error).
    pub fn compact_notice(&mut self, text: String) {
        if let Some(Dialog::Compact(dialog)) = &mut self.dialog {
            dialog.message = Some((text, false));
        }
    }

    /// The OpenCode models last listed in this window, if any.
    pub fn listed_models(&self) -> Option<&[String]> {
        self.compact.models.as_deref()
    }
}

impl Cache {
    /// Cancels a worker nobody waits for; the window's exit waits for it to end.
    pub(crate) fn retire(&mut self, work: Work) {
        work.cancel();
        self.retiring.push(work);
    }

    /// Models listed from Settings › Sessions, shared with the handover dialog.
    pub fn set_listed_models(&mut self, models: Vec<String>) {
        self.models = Some(models);
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
        // One handover a session at a time: a written or failed one opens instead.
        if !history && self.background_handover_of(&session.id, Instant::now()) {
            return;
        }
        if let Source::Real(roots) = &self.source {
            // A different window may have saved the default since this window opened.
            self.config.compact = claude_switch_core::config::load(roots).compact;
        }
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
            let kind = compact::FocusKind::NonCode;
            let text = self.config.compact.focus_text(kind).to_owned();
            dialog.focus.set(text.clone());
            dialog.picked = Some((Some(kind), text));
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
        self.cancel_background_handovers();
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
        let suggested_name = names::suggest(&st.marks, &st.sessions, &name);
        let mut new_name = TextField::new();
        new_name.set_text(&suggested_name);
        let mut this = Self {
            source_tab: None,
            in_place: false,
            session,
            name,
            suggested_name,
            new_name,
            name_focused: false,
            name_hit: None,
            accounts,
            account,
            default_agent: st.config.compact.default_agent.clone(),
            agents: Vec::new(),
            agents_loaded: false,
            agent: 0,
            agent_focus: 0,
            program: None,
            data: None,
            models: Vec::new(),
            model: 0,
            focus: Editor::default(),
            picked: None,
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
            delete_original: false,
            more_at: None,
            generating: false,
            acted: false,
            settle_until: None,
            unfocused: false,
        };
        this.reset_agents();
        this.check_in_place(st);
        this
    }

    pub(super) fn check_in_place(&mut self, st: &AppState) {
        let account = self.accounts.get(self.account).map(|a| a.0.as_str());
        self.in_place = account.is_some() && st.in_place_target(&self.session.id, account).is_some();
    }

    #[cfg(test)]
    pub(crate) fn notice_text(&self) -> Option<&str> {
        self.message.as_ref().map(|(text, _)| text.as_str())
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
                    let picked = data.chosen_focus(&st.config.compact);
                    self.focus.set(picked.1.clone());
                    self.picked = Some(picked);
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
                self.apply_agents(agents);
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
        // The dialog closes and the model writes in the background (`handover_jobs`).
        self.generating = self.job.is_some();
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
        let dropped = self.check_agents_now(st);
        if let Some(notice) = &dropped {
            st.toast(notice.clone(), now);
        }
        let Some((account, _)) = self.accounts.get(self.account) else { return Err("Choose an account.".into()) };
        let name = if self.new_name.text() == self.suggested_name {
            names::suggest(&st.marks, &st.sessions, &self.name)
        } else {
            names::validate(self.new_name.text())?.to_owned()
        };
        let Source::Real(roots) = &st.source else {
            let deleted = if self.delete_original { "; the original would be deleted (nothing was)" } else { "" };
            st.toast(format!("Demo: would open “{name}”{deleted}"), now);
            return Ok(());
        };
        compact_open::validate_prompt(&self.preview.text)?;
        let saved = compact_open::keep(roots, &self.session.id, &self.preview.text, self.saved.as_deref())?;
        let basis = self.data.as_ref().map(|data| data.basis());
        if let (Some(tab), Some(basis)) = (st.in_place_target(&self.session.id, Some(account)), basis) {
            self.saved = Some(saved.clone());
            let names = (self.name.clone(), name);
            let text = self.preview.text.clone();
            st.continue_in_place(tab, &self.session.clone(), names, &text, (saved, basis))?;
            // Deleted once the tab has followed Claude Code to the new session (`in_place_done`).
            if let Some(job) = st.tabs.get_mut(tab).and_then(|t| t.in_place.as_mut()) {
                job.delete_original = self.delete_original;
            }
            return Ok(());
        }
        let request = compact_open::prepare(
            roots,
            &st.config,
            &self.session,
            (self.name.clone(), &name),
            account,
            &self.preview.text,
            &saved,
        )?
        .with_agent(self.agent_name())?;
        self.saved = Some(saved);
        st.open_handover_tab(request)?;
        self.delete_original_now(st, now);
        Ok(())
    }

    /// After a handover that started a new session: the original, if the box was ticked.
    fn delete_original_now(&self, st: &mut AppState, now: Instant) {
        if self.delete_original {
            let result = st.delete_session(&self.session.id);
            st.report_delete(now, &self.name, result, Some("The handover session started."));
        }
    }

    /// The handover as the first prompt of a new OpenCode tab in the session's folder, kept as
    /// for a Claude handover (the long-text rule names that file).
    fn open_in_opencode(&mut self, st: &mut AppState) -> Result<(), String> {
        compact_open::validate_prompt(&self.preview.text)?;
        let saved = match &st.source {
            Source::Real(roots) => {
                let saved = compact_open::keep(roots, &self.session.id, &self.preview.text, self.saved.as_deref())?;
                self.saved = Some(saved.clone());
                saved
            }
            Source::Demo => PathBuf::from("demo-handover.md"),
        };
        st.continue_in_opencode(&self.session, &self.preview.text, &saved)?;
        self.delete_original_now(st, Instant::now());
        Ok(())
    }

    /// The new session name takes the mouse only where a click would focus it.
    pub(crate) fn name_editable(&self) -> bool {
        self.phase == Phase::Preview && self.choice.is_none()
    }

    pub(crate) fn name_parts(&mut self) -> (&mut TextField, Option<FieldHit>) {
        (&mut self.new_name, self.name_hit)
    }

    pub(crate) fn focus_name(&mut self) {
        if self.name_editable() {
            self.name_focused = true;
            self.unfocused = false;
        }
    }

    /// False closes the dialog; the caller hands it to `AppState::compact_closed`, which keeps
    /// a handover being written, or a written one not acted on, and cancels anything else.
    pub fn click(&mut self, st: &mut AppState, now: Instant, id: u32, x: f32) -> bool {
        if self.settling(now) && matches!(id, OK | OPENCODE | DELETE_ORIGINAL) {
            return true;
        }
        match id {
            CANCEL => return false,
            OK if matches!(self.phase, Phase::Setup | Phase::Consent) => {
                self.make(st, self.phase == Phase::Consent);
                if self.generating {
                    return false;
                }
            }
            OK if self.phase == Phase::Preview && self.choice.is_none() => match self.open(st, now) {
                Ok(()) => {
                    self.acted = true;
                    return false;
                }
                Err(e) => self.error(e),
            },
            OPENCODE if self.phase == Phase::Preview && self.choice.is_none() => match self.open_in_opencode(st) {
                Ok(()) => {
                    self.acted = true;
                    return false;
                }
                Err(e) => self.error(e),
            },
            BACK if self.choice.take().is_some() => {}
            BACK if matches!(self.phase, Phase::Consent | Phase::Preview) => {
                self.phase = Phase::Setup;
                self.name_focused = false;
                self.unfocused = false;
                self.message = None;
            }
            COPY if self.phase == Phase::Preview => {
                platform::set_clipboard_text(&self.preview.text);
                self.message = Some(("Handover copied.".into(), false));
            }
            SAVE if self.phase == Phase::Preview => self.export(st),
            MORE if self.phase == Phase::Preview => {
                if let Some(at) = self.more_at {
                    use mdshared_ui::menu::{Item, Menu};
                    let items = vec![Item::new(COPY, "Copy"), Item::new(SAVE, "Save as…")];
                    let menu = Menu::new(items, at.x, at.bottom(), false).around(at);
                    st.menu = Some((crate::state::MenuKind::HandoverMore, menu));
                }
            }
            DELETE_ORIGINAL if self.phase == Phase::Preview && self.choice.is_none() => {
                self.delete_original = !self.delete_original;
            }
            REFRESH if self.phase == Phase::Setup => self.load(st, true),
            MODEL if self.phase == Phase::Setup && self.job.is_none() => self.pick(Choice::Model, st),
            ACCOUNT if matches!(self.phase, Phase::Setup | Phase::Preview) && self.job.is_none() => {
                self.pick(Choice::Account, st);
            }
            AGENT if self.phase == Phase::Preview => self.pick(Choice::Agent, st),
            NAME_FIELD if self.phase == Phase::Preview && self.choice.is_none() => {
                self.name_focused = true;
                self.unfocused = false;
            }
            EDITOR => {
                self.name_focused = false;
                self.unfocused = false;
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
        if self.settling(Instant::now()) {
            return None;
        }
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
        } else if self.phase == Phase::Preview && matches!(k, KeyName::Char('d' | 'D')) && mods.alt && !mods.ctrl {
            return Some(DELETE_ORIGINAL);
        } else if self.phase == Phase::Preview && *k == KeyName::Tab && !mods.ctrl && !mods.alt {
            // From the dialog itself, Tab goes to the name field first.
            self.name_focused = !self.name_focused || self.unfocused;
            self.unfocused = false;
            if self.name_focused {
                self.new_name.select_all();
            }
        } else if self.phase == Phase::Preview && self.unfocused {
            // Keys for the dialog itself (Esc, Alt+A, Alt+D) are above; nothing is typed.
        } else if self.phase == Phase::Preview && self.name_focused {
            if *k == KeyName::Enter {
                self.name_focused = false;
            } else {
                self.new_name.handle_key(k, mods, text, true);
            }
        } else if *k == KeyName::Enter && mods.ctrl && self.phase == Phase::Setup {
            return Some(OK);
        } else if self.phase == Phase::Preview || (self.phase == Phase::Setup && self.job.is_none()) {
            self.editor().key(k, mods, text);
        }
        None
    }

    pub fn text_input(&mut self, text: &str) {
        if self.settling(Instant::now()) || self.unfocused {
            return;
        }
        if self.choice.is_none() && (self.phase == Phase::Preview || (self.phase == Phase::Setup && self.job.is_none()))
        {
            if self.name_focused {
                self.new_name.insert(text);
            } else {
                self.editor().insert(text);
            }
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

/// The dialog's side of a handover written in the background (`handover_jobs`).
impl Compact {
    pub(crate) fn title(&self) -> &str {
        &self.name
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session.id
    }

    pub(crate) fn generating(&self) -> bool {
        self.generating
    }

    /// A written handover not acted on: closing the review keeps it in the strip.
    pub(crate) fn parkable(&self) -> bool {
        self.phase == Phase::Preview && !self.acted && !self.preview.text.is_empty()
    }

    /// The worker, if any, for the caller to cancel.
    pub(crate) fn take_work(&mut self) -> Option<Work> {
        self.generating = false;
        self.job.take()
    }

    fn settling(&self, now: Instant) -> bool {
        self.settle_until.is_some_and(|until| now < until)
    }

    /// What the background worker ended with, once it has: the review is ready (`Ok`) or the
    /// reason it is not.
    pub(crate) fn poll_generation(&mut self, st: &mut AppState) -> Option<Result<(), String>> {
        if !self.generating || self.job.as_ref()?.running() {
            return None;
        }
        let job = self.job.take()?;
        self.generating = false;
        let result = job.take().unwrap_or_else(|| Err("The handover worker stopped unexpectedly.".into()));
        self.apply(st, result);
        Some(match (self.phase, &self.message) {
            (Phase::Preview, _) => Ok(()),
            (_, Some((reason, true))) => Err(reason.clone()),
            _ => Err("The handover worker stopped unexpectedly.".into()),
        })
    }

    /// Shown again: rechecks where it continues, starts with the delete box off as every time
    /// the review opens, and when it opened by itself (`settle_from`) lets keys settle first.
    pub(crate) fn reopened(&mut self, st: &AppState, settle_from: Option<Instant>) {
        self.check_in_place(st);
        self.delete_original = false;
        self.choice = None;
        self.settle_until = settle_from.map(|at| at + crate::handover_jobs::SETTLE);
        self.unfocused = settle_from.is_some();
        if self.unfocused {
            self.name_focused = false;
        }
    }

    /// Whether typing goes into the editor or the name field.
    #[cfg(test)]
    pub(crate) fn editor_focused(&self) -> bool {
        !self.unfocused
    }

    /// A failed background handover shown again: the setup step with the reason.
    pub(crate) fn failed(&mut self, reason: String) {
        self.phase = Phase::Setup;
        self.error(reason);
    }

    /// Starts writing again with the same choices. False when it did not start (it needs
    /// consent again, or the reason is in the dialog).
    pub(crate) fn retry(&mut self, st: &mut AppState) -> bool {
        self.phase = Phase::Setup;
        self.message = None;
        self.make(st, false);
        self.generating
    }
}

#[cfg(test)]
impl Compact {
    pub(crate) fn is_review(&self) -> bool {
        self.phase == Phase::Preview
    }

    pub(crate) fn preview_text(&self) -> &str {
        &self.preview.text
    }

    pub(crate) fn account_slug(&self) -> Option<&str> {
        self.accounts.get(self.account).map(|a| a.0.as_str())
    }

    pub(crate) fn settle_now(&mut self) {
        self.settle_until = None;
    }

    pub(crate) fn edit_for_test(&mut self, text: &str) {
        self.preview.move_to(self.preview.text.len(), false);
        self.preview.insert(text);
    }

    /// A worker standing in for the model: it returns what `gate` sends, or ends when cancelled.
    pub(crate) fn start_test_generation(
        &mut self,
        wake: impl Fn() + Send + Sync + 'static,
        gate: mpsc::Receiver<Result<String, String>>,
    ) {
        self.job = Job::start(wake, move |cancel, _| {
            loop {
                match gate.recv_timeout(std::time::Duration::from_millis(5)) {
                    Ok(result) => return result.map(Update::Preview),
                    Err(mpsc::RecvTimeoutError::Timeout) if !cancel.cancelled() => {}
                    Err(_) => return Err("Canceled.".into()),
                }
            }
        })
        .ok();
        self.generating = self.job.is_some();
    }
}

impl Compact {
    /// The focus box's label: which focus it holds.
    pub(super) fn focus_label(&self) -> String {
        match &self.picked {
            None => "Focus (optional)".into(),
            Some((_, text)) if *text != self.focus.text => "Focus: your own".into(),
            Some((None, _)) => "Focus: your own, from the session's /compact".into(),
            Some((Some(kind), _)) => {
                format!("Focus: {}, picked for this session (edit or replace it)", kind.label().to_lowercase())
            }
        }
    }
}

fn demo_text(name: &str) -> String {
    format!(
        "## Goal\n\n> Find why a clean release build takes four minutes.\n\n\
         ## Where it stands\n\n> Compared the clean and warm builds. Most time is in dependency compilation.\n\n\
         ## In flight and promised\n\n> None.\n\n\
         ## Decisions and why\n\n> Use a separate target directory so a clean timing cannot disturb an existing build.\n\n\
         ## Standing instructions from the user\n\n> Keep the toolkit unchanged.\n\n\
         ## Open items\n\n> Repeat the timing after disabling the optional feature.\n\n\
         ## Not verified\n\n> Timing on a second machine.\n\n\
         ## Files\n\n- docs/build-notes.md (2)\n\n\
         ## Source session\n\nName: {name}\nID: demo\nFolder: C:\\demo\\mdplay\nBranch: main\n\
         Transcript: C:\\demo\\mdplay.jsonl\n\nLook up further details in the original transcript.\n\n\
         ## Latest task list\n\nPossibly stale: as the session last wrote it.\n\n- [pending] Repeat the feature comparison\n\n\
         ## Last 15 shell commands\n\n    cargo build --release --timings\n\n\
         ## User prompts (verbatim)\n\n### 2026-11-26T09:00:00Z\n\n```\nCompare a clean build with an incremental build and report the timings.\n```\n\n\
         ## Last 6 turns (assistant text only)\n\n```\nThe warm run does not reproduce the delay.\n```\n"
    )
}

#[cfg(test)]
mod tests;
