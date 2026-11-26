//! Session picker, rename, and the opt-in title suggestion flow.

use std::collections::VecDeque;
use std::time::Instant;

use claude_switch_core::naming::{self, Excerpts};
use mdshared_app::job::Job;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::{DIALOG_PAD, dialog_card};
use mdshared_ui::textfield::{FieldColors, TextField};

use crate::actions::{OPEN_AS, OPEN_LAST};
use crate::dialogs::{CANCEL, Dialog, Draw, FIELD, OK, Pending};
use crate::list::model::{self, SessionFields};
use crate::session_marks::folder_label;
use crate::state::{AppState, Target};

const RESULT: u32 = 10;
const SKIP: u32 = 5;
const TOPIC_FIELD: u32 = 6;
const VISIBLE: usize = 5;

pub type NameResults = Result<(VecDeque<Suggestion>, usize), String>;

pub struct Suggestion {
    id: String,
    original: String,
    title: String,
    topic: Option<String>,
}

struct Hit {
    id: String,
    title: String,
    hint: String,
}

pub struct Picker {
    hits: Vec<Hit>,
    selected: usize,
    first: usize,
}

impl Picker {
    fn refill(&mut self, st: &AppState, query: &str) {
        let labels: Vec<_> = st.sessions.iter().map(folder_label).collect();
        self.hits = model::rank(
            query.trim(),
            st.sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| st.searchable(s, !query.trim().is_empty()))
                .map(|(i, s)| (i, SessionFields { name: st.marks.name(&s.id), session: s, folder: &labels[i] })),
        )
        .into_iter()
        .map(|i| {
            let s = &st.sessions[i];
            let hint = if s.last_prompt.is_empty() {
                labels[i].clone()
            } else {
                format!("{}  ·  {}", labels[i], s.last_prompt)
            };
            let title =
                if st.is_archived(s) { format!("{} · Archived", st.title_of(s)) } else { st.title_of(s).to_string() };
            Hit { id: s.id.clone(), title, hint }
        })
        .collect();
        self.selected = 0;
        self.first = 0;
    }

    fn step(&mut self, forward: bool) {
        let cap = self.hits.len().saturating_sub(1);
        self.selected = if forward { (self.selected + 1).min(cap) } else { self.selected.saturating_sub(1) };
        self.first = self.first.clamp(self.selected.saturating_sub(VISIBLE - 1), self.selected);
    }
}

pub enum Mode {
    Switch(Picker),
    Rename { id: String, original: String },
    Topic { id: String },
    Working,
    Review { current: Suggestion, rest: VecDeque<Suggestion>, n: usize, total: usize, missing: usize },
}

pub struct Panel {
    field: TextField,
    stops: Vec<f32>,
    topic: TextField,
    topic_stops: Vec<f32>,
    topic_focus: bool,
    invalid_topic: bool,
    mode: Mode,
}

impl Panel {
    fn dialog(mode: Mode, text: &str) -> Dialog {
        let mut field = TextField::new();
        field.set_text(text);
        field.select_all();
        let mut topic = TextField::new();
        if let Mode::Review { current, .. } = &mode
            && let Some(label) = &current.topic
        {
            topic.set_text(label);
            topic.select_all();
        }
        Dialog::Session(Box::new(Self {
            field,
            stops: vec![],
            topic,
            topic_stops: vec![],
            topic_focus: false,
            invalid_topic: false,
            mode,
        }))
    }

    pub fn draw(&mut self, d: &mut Draw<'_>, p: &mut Painter, window: Rect) {
        let (title, height) = match &self.mode {
            Mode::Switch(_) => ("Quick switcher".to_string(), 382.0),
            Mode::Rename { .. } => ("Rename session".into(), 228.0),
            Mode::Topic { .. } => ("Set topic".into(), 210.0),
            Mode::Working => ("Suggesting names…".into(), 154.0),
            Mode::Review { current, n, total, .. } => {
                (format!("Review suggestion  {n} / {total}"), if current.topic.is_some() { 298.0 } else { 242.0 })
            }
        };
        let card = dialog_card(528.0, height, window.w, window.h);
        let footer = d.card(p, card);
        d.title(p, card, &title);
        let x = card.x + DIALOG_PAD;
        let w = card.w - 2.0 * DIALOG_PAD;
        let pal = d.pal;
        let text = |y, label: &str, p: &mut Painter| {
            p.text(Face::Ui, 12.5, x, card.y + y, label, pal.muted, w);
        };
        if !matches!(self.mode, Mode::Working) {
            let r = Rect::new(x, card.y + 54.0, w, 34.0);
            p.frame(r, 6.0, pal.field, if self.topic_focus { pal.line } else { pal.accent });
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            let hint = if matches!(self.mode, Mode::Switch(_)) {
                "Search name, prompt or folder"
            } else if matches!(self.mode, Mode::Topic { .. }) {
                "No topic"
            } else {
                "Use transcript title"
            };
            self.field.draw(p, r.inset(8.0), Face::Ui, 13.0, &colors, hint, !self.topic_focus, true);
            self.stops = p.caret_stops(Face::Ui, 13.0, &self.field.display());
            d.hits.push((r, Target::Dialog(FIELD)));
        }
        match &self.mode {
            Mode::Switch(s) => {
                if s.hits.is_empty() {
                    text(112.0, "No matching sessions.", p);
                }
                for (i, hit) in s.hits.iter().enumerate().skip(s.first).take(VISIBLE) {
                    let r = Rect::new(x, card.y + 100.0 + (i - s.first) as f32 * 40.0, w, 39.0);
                    let target = Target::Dialog(RESULT + i as u32);
                    if s.selected == i || d.hover == Some(target) {
                        p.rbox(r, 5.0, if s.selected == i { pal.on } else { pal.hover });
                    }
                    p.text(Face::Semibold, 13.0, r.x + 9.0, r.y + 11.0, &hit.title, pal.text, r.w - 18.0);
                    p.text(Face::Ui, 11.5, r.x + 9.0, r.y + 28.0, &hit.hint, pal.muted, r.w - 18.0);
                    d.hits.push((r, target));
                }
                p.text(
                    Face::Ui,
                    11.5,
                    x,
                    footer.cy(),
                    "↑↓ Move   ·   Enter Open   ·   Ctrl+Enter Open as…   ·   Esc Close",
                    pal.muted,
                    w,
                );
            }
            Mode::Rename { original, .. } => {
                text(109.0, &format!("Transcript title: {original}"), p);
                text(131.0, "Leave blank to use the transcript title. The transcript is not edited.", p);
                d.footer(p, footer, &[("Save name", true, OK), ("Cancel", false, CANCEL)]);
            }
            Mode::Topic { .. } => {
                text(
                    112.0,
                    if self.invalid_topic {
                        "Enter a plain topic of up to 3 words (60 characters)."
                    } else {
                        "Up to 3 words. Leave blank to remove the topic."
                    },
                    p,
                );
                d.footer(p, footer, &[("Save topic", true, OK), ("Cancel", false, CANCEL)]);
            }
            Mode::Working => {
                text(69.0, "Waiting for Claude. No session will be renamed automatically.", p);
                d.footer(p, footer, &[("Cancel", true, CANCEL)]);
            }
            Mode::Review { current, missing, .. } => {
                text(109.0, &format!("Current title: {}", current.original), p);
                if current.topic.is_some() {
                    text(
                        132.0,
                        if self.invalid_topic {
                            "Topic must be plain text, up to 3 words (60 characters)."
                        } else {
                            "Topic · up to 3 words (blank removes it) · Tab to edit"
                        },
                        p,
                    );
                    let r = Rect::new(x, card.y + 146.0, w, 34.0);
                    p.frame(r, 6.0, pal.field, if self.topic_focus { pal.accent } else { pal.line });
                    let colors = FieldColors {
                        text: pal.text,
                        placeholder: pal.muted,
                        selection: pal.selection,
                        caret: pal.text,
                    };
                    self.topic.draw(p, r.inset(8.0), Face::Ui, 13.0, &colors, "No topic", self.topic_focus, true);
                    self.topic_stops = p.caret_stops(Face::Ui, 13.0, &self.topic.display());
                    d.hits.push((r, Target::Dialog(TOPIC_FIELD)));
                } else {
                    text(131.0, "Edit the suggestion or accept it as shown.", p);
                }
                if *missing > 0 {
                    let y = if current.topic.is_some() { 197.0 } else { 151.0 };
                    text(y, &format!("No usable suggestion for {missing} requested session(s)."), p);
                }
                d.footer(p, footer, &[("Accept", true, OK), ("Skip", false, SKIP), ("Done", false, CANCEL)]);
            }
        }
    }

    pub fn key(&mut self, st: &AppState, k: &KeyName, mods: Mods, text: Option<&str>) -> Option<u32> {
        match k {
            KeyName::Escape => return Some(CANCEL),
            KeyName::Enter => return Some(if matches!(self.mode, Mode::Working) { CANCEL } else { OK }),
            KeyName::Tab if matches!(&self.mode, Mode::Review { current, .. } if current.topic.is_some()) => {
                self.topic_focus = !self.topic_focus;
                return None;
            }
            KeyName::Up | KeyName::Down if let Mode::Switch(s) = &mut self.mode => {
                s.step(*k == KeyName::Down);
                return None;
            }
            _ => {}
        }
        let field = if self.topic_focus { &mut self.topic } else { &mut self.field };
        if field.handle_key(k, mods, text, true) == Some(true) {
            self.invalid_topic = false;
            self.refill(st);
        }
        None
    }

    pub fn text_input(&mut self, st: &AppState, text: &str) {
        let field = if self.topic_focus { &mut self.topic } else { &mut self.field };
        if field.insert(text) {
            self.invalid_topic = false;
            self.refill(st);
        }
    }

    fn refill(&mut self, st: &AppState) {
        if let Mode::Switch(s) = &mut self.mode {
            s.refill(st, self.field.text());
        }
    }

    pub fn wheel(&mut self, dy: f32) {
        if let Mode::Switch(s) = &mut self.mode {
            let steps = ((dy.abs() / 40.0).round() as usize).clamp(1, 5);
            for _ in 0..steps {
                s.step(dy < 0.0);
            }
        }
    }

    pub fn click(mut self: Box<Self>, st: &mut AppState, now: Instant, id: u32, x: f32, other: bool) {
        if id == FIELD || id == TOPIC_FIELD {
            let offset = st.hits.iter().find(|(_, t)| *t == Target::Dialog(id)).map_or(0.0, |(r, _)| r.x + 8.0);
            self.topic_focus = id == TOPIC_FIELD;
            if self.topic_focus {
                self.topic.press(x - offset, &self.topic_stops, false);
            } else {
                self.field.press(x - offset, &self.stops, false);
            }
            st.dialog = Some(Dialog::Session(self));
            return;
        }
        if id == CANCEL {
            if matches!(self.mode, Mode::Working)
                && let Some(job) = &st.naming_job
            {
                job.cancel();
            }
            return;
        }
        match &mut self.mode {
            Mode::Switch(picker) => {
                let i = if id == OK { picker.selected } else { id.saturating_sub(RESULT) as usize };
                let Some(hit) = picker.hits.get(i) else {
                    st.dialog = Some(Dialog::Session(self));
                    return;
                };
                if st
                    .by_id
                    .get(&hit.id)
                    .is_some_and(|&i| st.searchable(&st.sessions[i], !self.field.text().trim().is_empty()))
                {
                    let anchor = st.hits.iter().find(|(_, t)| *t == Target::Dialog(RESULT + i as u32)).map(|(r, _)| *r);
                    st.select(&hit.id);
                    st.show_sessions();
                    st.reveal_selected();
                    st.command(now, if other { OPEN_AS } else { OPEN_LAST }, anchor);
                }
            }
            Mode::Rename { id: session, .. } if id == OK => {
                st.rename_session(session, self.field.text());
            }
            Mode::Topic { id: session } if id == OK => {
                if valid_edit(self.field.text()) {
                    st.save_suggestion(session, None, Some(self.field.text()));
                } else {
                    self.invalid_topic = true;
                    st.dialog = Some(Dialog::Session(self));
                }
            }
            Mode::Review { current, rest, n, total, missing } if id == OK || id == SKIP => {
                if id == OK {
                    let topic = current.topic.as_ref().map(|_| self.topic.text());
                    if topic.is_some_and(|s| !valid_edit(s)) {
                        self.invalid_topic = true;
                        st.dialog = Some(Dialog::Session(self));
                        return;
                    }
                    let title = self.field.text();
                    st.save_suggestion(&current.id, (!title.trim().is_empty()).then_some(title), topic);
                }
                st.dialog = review(std::mem::take(rest), *n + 1, *total, *missing);
            }
            _ => st.dialog = Some(Dialog::Session(self)),
        }
    }
}

fn valid_edit(s: &str) -> bool {
    s.trim().is_empty() || claude_switch_core::grouping::valid_topic(s).is_some()
}

fn review(mut rest: VecDeque<Suggestion>, n: usize, total: usize, missing: usize) -> Option<Dialog> {
    let current = rest.pop_front()?;
    let title = current.title.clone();
    Some(Panel::dialog(Mode::Review { current, rest, n, total, missing }, &title))
}

impl AppState {
    pub fn quick_switcher(&mut self) {
        let mut picker = Picker { hits: vec![], selected: 0, first: 0 };
        picker.refill(self, "");
        self.dialog = Some(Panel::dialog(Mode::Switch(picker), ""));
    }

    pub fn ask_rename_session(&mut self) {
        if let Some(s) = self.selected_session() {
            self.dialog =
                Some(Panel::dialog(Mode::Rename { id: s.id.clone(), original: s.title.clone() }, self.title_of(s)));
        }
    }

    pub fn ask_set_topic(&mut self) {
        if let Some(s) = self.selected_session() {
            self.dialog = Some(Panel::dialog(Mode::Topic { id: s.id.clone() }, self.marks.topic(&s.id).unwrap_or("")));
        }
    }

    fn save_suggestion(&mut self, id: &str, name: Option<&str>, topic: Option<&str>) {
        let Some(&i) = self.by_id.get(id) else { return };
        let file = &self.sessions[i].file;
        let previous = self.marks.clone();
        let name_changed = name.is_some_and(|name| self.marks.rename(id, file, name));
        let topic_changed = topic.is_some_and(|topic| self.marks.set_topic(id, file, topic));
        if name_changed || topic_changed {
            if !self.save_marks() {
                self.marks = previous;
            }
            self.rebuild_view();
        }
    }

    pub fn ask_naming(&mut self, batch: bool) {
        if self.config_damaged || batch && !self.batch_marks_current() {
            self.warn(
                "Batch naming unavailable",
                "Session marks could not be verified. Refresh or reopen Claude Switch.".into(),
            );
            return;
        }
        if self.naming_job.is_some() {
            self.warn("Suggestions are already running", "Wait for the previous request to finish or cancel.".into());
            return;
        }
        let ids: Vec<_> = if batch {
            let cutoff = crate::list::now_ms().saturating_sub(7 * model::DAY);
            let mut eligible: Vec<_> = self
                .sessions
                .iter()
                .filter(|s| {
                    s.last_active_ms >= cutoff
                        && self.listed(s)
                        && !self.hidden_session(s)
                        && self.marks.name(&s.id).is_none()
                })
                .collect();
            eligible.sort_unstable_by_key(|s| (std::cmp::Reverse(s.last_active_ms), &s.id));
            eligible.into_iter().take(naming::MAX_BATCH).map(|s| s.id.clone()).collect()
        } else {
            self.selected.clone().into_iter().collect()
        };
        let count = ids.len();
        if count == 0 {
            self.warn(
                "No sessions to name",
                "Batch naming uses visible, unnamed sessions active in the last seven days.".into(),
            );
            return;
        }
        let sessions = if count == 1 { "session" } else { "sessions" };
        let a = naming::worker_account(&self.config);
        let mut text = if self.source.is_demo() {
            format!(
                "Demo only: show synthetic suggestions for {count} {sessions}. Nothing is sent.\n\nIn real use this sends the first and up to three recent prompts (up to 500 characters each) to Claude and uses a little of the worker account’s usage."
            )
        } else {
            format!(
                "Send the first and up to three recent prompts from {count} {sessions} to Claude (up to 500 characters each). This uses a little of the {} account’s usage. Review each suggestion before saving.",
                a.name
            )
        };
        if self.config.suggest_topics {
            text.push_str("\n\nAlso suggest a topic of up to three words. This includes existing topic labels so they can be reused.");
        }
        if batch && !self.source.is_demo() {
            text.push_str("\n\nSessions: ");
            let names = ids
                .iter()
                .filter_map(|id| self.by_id.get(id))
                .map(|&i| self.title_of(&self.sessions[i]).chars().take(64).collect::<String>())
                .collect::<Vec<_>>();
            text.push_str(&names.join("; "));
        }
        self.confirm_scroll = 0.0;
        self.dialog = Some(Dialog::Confirm {
            title: format!("Suggest {} for {count} {sessions}?", if count == 1 { "a name" } else { "names" }),
            text,
            ok: "Suggest names".into(),
            then: Pending::Suggest { ids, batch },
        });
    }

    fn batch_marks_current(&self) -> bool {
        if self.marks_read_only || self.config_damaged {
            return false;
        }
        match &self.source {
            crate::source::Source::Demo => true,
            crate::source::Source::Real(roots) => self.marks.unchanged(roots),
        }
    }

    pub fn start_naming(&mut self, ids: Vec<String>, batch: bool) {
        if self.config_damaged || batch && !self.batch_marks_current() {
            self.warn(
                "Batch naming unavailable",
                "Session marks changed or could not be read. Nothing was sent.".into(),
            );
            return;
        }
        let count = ids.len();
        let cutoff = crate::list::now_ms().saturating_sub(7 * model::DAY);
        let sessions: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let s = self.by_id.get(&id).map(|&i| &self.sessions[i])?;
                if batch
                    && (s.last_active_ms < cutoff
                        || !self.listed(s)
                        || self.hidden_session(s)
                        || self.marks.name(&id).is_some())
                {
                    return None;
                }
                let topic = self.marks.topic(&id).unwrap_or_default().to_owned();
                Some((id, self.title_of(s).to_string(), s.file.clone(), topic))
            })
            .collect();
        if sessions.len() != count || count == 0 {
            self.warn("Session selection changed", "Open the naming dialog again. Nothing was sent.".into());
            return;
        }
        let account = naming::worker_account(&self.config).clone();
        let demo = self.source.is_demo();
        let topics = self.config.suggest_topics.then(|| {
            let mut unique = std::collections::BTreeMap::new();
            for (_, _, _, label) in &sessions {
                if !label.is_empty() {
                    unique.entry(label.to_lowercase()).or_insert_with(|| label.to_owned());
                }
            }
            unique.into_values().collect::<Vec<_>>()
        });
        let marks = match &self.source {
            crate::source::Source::Real(roots) if batch => Some((roots.clone(), self.marks.clone())),
            _ => None,
        };
        match Job::start(self.wake_fn(), move |cancel, _| {
            let mut queue = VecDeque::new();
            if demo {
                for (i, (id, original, _, _)) in sessions.into_iter().enumerate() {
                    let title = if i == 0 { "Refine recent session navigation" } else { "Review the session changes" };
                    let topic = topics.as_ref().map(|_| "Session navigation".into());
                    queue.push_back(Suggestion { id, original, title: title.to_string(), topic });
                }
            } else {
                let verify = || {
                    if marks.as_ref().is_some_and(|(roots, marks)| !marks.unchanged(roots)) {
                        Err("Session marks changed or could not be read. Nothing was sent.")
                    } else {
                        Ok(())
                    }
                };
                verify()?;
                let mut excerpts = Vec::new();
                let mut named = Vec::new();
                for (id, original, path, old_topic) in sessions {
                    if cancel.cancelled() {
                        return Err("Cancelled".into());
                    }
                    let (first, last) = claude_switch_core::sessions::naming_prompts(&path);
                    if first.is_some() || !last.is_empty() {
                        excerpts.push(Excerpts { first, last });
                        named.push((id, original, old_topic));
                    }
                }
                verify()?;
                let program = crate::run::native_program(std::path::Path::new("claude")).map_err(|e| e.to_string())?;
                let mut suggestions =
                    naming::suggest(&program, &account, &excerpts, topics.as_deref(), || cancel.cancelled())
                        .map_err(|e| e.to_string())?;
                for (i, (id, original, old_topic)) in named.into_iter().enumerate() {
                    if let Some(item) = suggestions.remove(&i) {
                        let topic = topics.as_ref().map(|_| item.topic.unwrap_or(old_topic));
                        queue.push_back(Suggestion { id, original, title: item.title, topic });
                    }
                }
            }
            let missing = count.saturating_sub(queue.len());
            Ok((queue, missing))
        }) {
            Ok(job) => {
                self.naming_job = Some(job);
                self.dialog = Some(Panel::dialog(Mode::Working, ""));
            }
            Err(e) => self.error("Could not start suggestions", e.to_string()),
        }
    }

    pub fn take_naming_job(&mut self) -> bool {
        let Some(job) = self.naming_job.take_if(|job| !job.running()) else { return false };
        if job.cancelled() || !matches!(&self.dialog, Some(Dialog::Session(p)) if matches!(p.mode, Mode::Working)) {
            return false;
        }
        self.dialog = None;
        match job.take() {
            Some(Ok((queue, missing))) if !queue.is_empty() => {
                let count = queue.len();
                self.dialog = review(queue, 1, count, missing);
            }
            Some(Err(e)) => self.error("Could not suggest names", e),
            _ => self.warn("No name suggestions", "No readable prompts or no valid titles were returned.".into()),
        }
        true
    }
}

#[cfg(test)]
mod tests;
