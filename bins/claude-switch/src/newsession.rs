//! The folder, account and app picker for a new session.

use std::path::{Path, PathBuf};
use std::time::Instant;

use claude_switch_core::App;
use claude_switch_core::sessions::{Group, folder_key};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::{DIALOG_FOOTER, DIALOG_PAD, dialog_card};
use mdshared_ui::textfield::{FieldColors, FieldHit, TextField};

use super::{CANCEL, Dialog, Draw, FIELD, OK};
use crate::folder_defaults::{CLI, DESKTOP, draw_app};
use crate::state::{AppState, Target};

const RECENT_LIMIT: usize = 4;
const RECENT: u32 = 10;
const PREVIOUS: u32 = 20;
const NEXT: u32 = 21;
const BROWSE: u32 = 22;
const BROWSE_W: f32 = 84.0;

pub struct NewSession {
    field: TextField,
    hit: Option<FieldHit>,
    recent: Vec<PathBuf>,
    accounts: Vec<(String, String)>,
    account: usize,
    app: App,
    folder_key: Option<String>,
    focus: u8, // folder, account, app
    error: Option<&'static str>,
    /// "New OpenCode tab": the folder only; OpenCode has no Claude account or app.
    opencode: bool,
}

impl AppState {
    pub fn ask_new_session(&mut self) {
        self.dialog = Some(Dialog::NewSession(Box::new(NewSession::new(self))));
    }

    /// Opens the system's folder picker at `start` when it is a folder; the choice comes back as
    /// [`crate::state::Msg::FolderPicked`].
    fn pick_new_session_folder(&self, start: &Path) {
        if self.source.is_demo() {
            return;
        }
        let proxy = self.proxy.clone();
        let start = start.is_dir().then_some(start);
        mdshared_ui::platform::pick_folder_async(start, move |folder| {
            if let Some(folder) = folder
                && let Some(proxy) = proxy
                && let Ok(proxy) = proxy.lock()
            {
                proxy.send(crate::state::Msg::FolderPicked(folder));
            }
        });
    }

    /// Puts a picked folder into the New session dialog, if it is still open.
    pub fn new_session_folder_picked(&mut self, folder: &Path) {
        if let Some(Dialog::NewSession(mut picker)) = self.dialog.take_if(|d| matches!(d, Dialog::NewSession(_))) {
            picker.picked(self, folder);
            self.dialog = Some(Dialog::NewSession(picker));
        }
    }
}

fn recent_folders(groups: &[Group], exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    // The session groups already deduplicate Windows paths and are ordered newest first. A folder
    // deleted since (an old worktree) is left out.
    groups
        .iter()
        .filter(|g| !g.cwd.as_os_str().is_empty() && exists(&g.cwd))
        .take(RECENT_LIMIT)
        .map(|g| g.cwd.clone())
        .collect()
}

fn folder_from_text(text: &str, is_dir: impl FnOnce(&Path) -> bool) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(text.trim());
    if path.as_os_str().is_empty() {
        Err("Enter a folder.")
    } else if !is_dir(&path) {
        Err("That folder does not exist or is not accessible.")
    } else {
        Ok(path)
    }
}

impl NewSession {
    fn new(st: &AppState) -> Self {
        let accounts: Vec<_> = st.config.accounts.iter().map(|a| (a.slug.clone(), a.name.clone())).collect();
        let mut field = TextField::new();
        let selected = st.selected_session().map(|s| &s.cwd).filter(|p| !p.as_os_str().is_empty());
        field.set_text(&selected.cloned().unwrap_or_else(|| st.source.home()).to_string_lossy());
        field.select_all();
        let mut picker = Self {
            field,
            hit: None,
            recent: recent_folders(&st.all_groups, |path| st.source.is_demo() || path.is_dir()),
            accounts,
            account: 0,
            app: App::Cli,
            folder_key: None,
            focus: 0,
            error: None,
            opencode: false,
        };
        picker.sync_defaults(st);
        picker
    }

    pub(crate) fn for_opencode(st: &AppState) -> Self {
        Self { opencode: true, ..Self::new(st) }
    }

    fn sync_defaults(&mut self, st: &AppState) {
        let cwd = Path::new(self.field.text().trim());
        let key = folder_key(cwd);
        if self.folder_key.as_ref() != Some(&key) {
            let (slug, app) = st.folder_open(cwd, App::Cli);
            self.account = self.accounts.iter().position(|(a, _)| a == slug).unwrap_or(0);
            self.app = app;
            self.folder_key = Some(key);
        }
    }

    pub(super) fn draw(&mut self, d: &mut Draw<'_>, p: &mut Painter, w: f32, h: f32) {
        let pal = d.pal;
        let recent_height = if self.recent.is_empty() { 0.0 } else { 28.0 + self.recent.len() as f32 * 32.0 };
        let rows = if self.opencode { 132.0 } else { 216.0 };
        let card = dialog_card(500.0, rows + recent_height + DIALOG_FOOTER, w, h);
        let footer = d.card(p, card);
        d.title(p, card, if self.opencode { "New OpenCode tab" } else { "New session" });
        let x = card.x + DIALOG_PAD;
        let width = card.w - 2.0 * DIALOG_PAD;

        let fr = Rect::new(x, card.y + 64.0, width - BROWSE_W - 8.0, 32.0);
        p.text(Face::Ui, 12.0, x, fr.y - 9.0, "Folder", pal.muted, width);
        d.button(p, Rect::new(fr.right() + 8.0, fr.y, BROWSE_W, 32.0), "Browse…", false, BROWSE);
        p.frame(fr, 5.0, pal.field, if self.focus == 0 { pal.accent } else { pal.line });
        let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
        self.field.draw(p, fr.inset(8.0), Face::Ui, 13.0, &colors, "Folder path", self.focus == 0, true);
        self.hit = crate::fieldmouse::hit(p, fr.inset(8.0), 13.0);
        d.hits.push((fr, Target::Dialog(FIELD)));
        if let Some(error) = self.error {
            p.text(Face::Ui, 12.0, x, fr.bottom() + 10.0, error, pal.danger, width);
        }

        let mut y = card.y + 118.0;
        if !self.recent.is_empty() {
            p.text(Face::Ui, 12.0, x, y + 6.0, "Recent folders", pal.muted, width);
            y += 18.0;
            for (i, folder) in self.recent.iter().enumerate() {
                d.button(p, Rect::new(x, y, width, 28.0), &folder.to_string_lossy(), false, RECENT + i as u32);
                y += 32.0;
            }
            y += 10.0;
        }
        if self.opencode {
            let note = "Your own OpenCode setup, in a terminal tab. Not a Claude session.";
            p.text(Face::Ui, 12.0, x, y + 8.0, note, pal.muted, width);
            d.footer(p, footer, &[("Start", true, OK), ("Cancel", false, CANCEL)]);
            return;
        }

        p.text(Face::Ui, 12.0, x, y + 16.0, "Account", pal.muted, 67.0);
        let r = Rect::new(x + 70.0, y, width - 70.0, 32.0);
        p.frame(r, 5.0, pal.field, if self.focus == 1 { pal.accent } else { pal.line });
        if let Some((_, name)) = self.accounts.get(self.account) {
            let inset = if self.accounts.len() > 1 { 44.0 } else { 8.0 };
            p.text(Face::Ui, 13.0, r.x + inset, r.cy(), name, pal.text, r.w - 2.0 * inset);
        }
        if self.accounts.len() > 1 {
            d.button(p, Rect::new(r.x + 2.0, y + 2.0, 34.0, 28.0), "‹", false, PREVIOUS);
            d.button(p, Rect::new(r.right() - 36.0, y + 2.0, 34.0, 28.0), "›", false, NEXT);
        }
        p.text(Face::Ui, 12.0, x, y + 58.0, "App", pal.muted, 67.0);
        let r = Rect::new(x + 70.0, y + 42.0, width - 70.0, 32.0);
        draw_app(d, p, r, self.app);
        if self.focus == 2 {
            mdshared_ui::focus::draw_ring(p, &pal, r, 5.0);
        }
        d.footer(p, footer, &[("Start", true, OK), ("Cancel", false, CANCEL)]);
    }

    fn step_account(&mut self, forward: bool) {
        let n = self.accounts.len();
        if n > 1 {
            self.account = (self.account + if forward { 1 } else { n - 1 }) % n;
        }
    }

    pub(super) fn click(&mut self, st: &mut AppState, now: Instant, id: u32) -> bool {
        self.sync_defaults(st);
        match id {
            CANCEL => return false,
            OK => {
                match folder_from_text(self.field.text(), |path| {
                    if st.source.is_demo() {
                        path == st.source.home() || st.all_groups.iter().any(|g| g.cwd == path)
                    } else {
                        path.is_dir()
                    }
                }) {
                    Ok(cwd) if self.opencode => {
                        if let Err(e) = st.open_opencode_tab(&cwd, crate::opencode_tab::Start::Fresh) {
                            st.error("Could not open OpenCode", e);
                        }
                        return false;
                    }
                    Ok(cwd) => {
                        if let Some((slug, _)) = self.accounts.get(self.account) {
                            st.new_session_now(now, self.app, slug, &cwd);
                        }
                        return false;
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.focus = 0;
                    }
                }
            }
            FIELD => self.focus = 0,
            BROWSE => {
                self.focus = 0;
                st.pick_new_session_folder(Path::new(self.field.text().trim()));
            }
            PREVIOUS | NEXT => {
                self.focus = 1;
                self.step_account(id == NEXT);
            }
            CLI | DESKTOP => {
                self.focus = 2;
                self.app = if id == CLI { App::Cli } else { App::Desktop };
            }
            id if id >= RECENT => {
                if let Some(folder) = self.recent.get((id - RECENT) as usize) {
                    self.field.set_text(&folder.to_string_lossy());
                    self.error = None;
                    self.focus = 0;
                    self.sync_defaults(st);
                }
            }
            _ => {}
        }
        true
    }

    /// Enter and Escape go through the common dialog routes to Start and Cancel.
    pub(super) fn key(&mut self, st: &AppState, key: &KeyName, mods: Mods, text: Option<&str>) {
        match key {
            KeyName::Tab if self.opencode => self.focus = 0,
            KeyName::Tab => self.focus = (self.focus + if mods.shift { 2 } else { 1 }) % 3,
            KeyName::Left | KeyName::Up if self.focus == 1 => self.step_account(false),
            KeyName::Right | KeyName::Down if self.focus == 1 => self.step_account(true),
            KeyName::Left | KeyName::Up if self.focus == 2 => self.app = App::Cli,
            KeyName::Right | KeyName::Down if self.focus == 2 => self.app = App::Desktop,
            _ if self.focus == 0 && self.field.handle_key(key, mods, text, true) == Some(true) => {
                self.error = None;
                self.sync_defaults(st);
            }
            _ => {}
        }
    }

    pub(crate) fn field_parts(&mut self) -> (&mut TextField, Option<FieldHit>) {
        (&mut self.field, self.hit)
    }

    /// A folder chosen in the system's folder picker.
    pub(crate) fn picked(&mut self, st: &AppState, folder: &Path) {
        self.field.set_text(&folder.to_string_lossy());
        self.field.select_all();
        self.error = None;
        self.focus = 0;
        self.sync_defaults(st);
    }

    pub(crate) fn focus_folder(&mut self) {
        self.focus = 0;
    }

    /// The folder changed other than by typing (the right-click menu).
    pub(crate) fn edited(&mut self, st: &AppState) {
        self.error = None;
        self.sync_defaults(st);
    }

    pub(super) fn text_input(&mut self, st: &AppState, text: &str) {
        if self.focus == 0 && self.field.insert(text) {
            self.error = None;
            self.sync_defaults(st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{actions, demo, dialogs, source::Source, winfile::WindowFile};

    fn demo_state() -> AppState {
        let mut st = AppState::new(Source::Demo, Some(false), WindowFile::default());
        st.apply_scan(demo::scan(&st.config));
        st
    }

    fn picker(st: &AppState) -> &NewSession {
        match &st.dialog {
            Some(Dialog::NewSession(picker)) => picker,
            _ => panic!("expected the New session dialog"),
        }
    }

    #[test]
    fn the_new_session_command_asks_with_the_selected_folder_or_home_and_current_account() {
        let mut st = demo_state();
        let now = Instant::now();
        let id = st.sessions[0].id.clone();
        st.select(&id);
        st.switch_account("work");
        st.command(now, actions::NEW_SESSION, None);
        let modal = picker(&st);
        assert_eq!(modal.field.text(), st.sessions[0].cwd.to_string_lossy());
        assert_eq!(modal.accounts[modal.account].0, "work");
        assert!(st.toasts.is_empty(), "asking doesn't start anything");
        dialogs::key(&mut st, now, &KeyName::Escape, Mods::default(), None);
        assert!(st.dialog.is_none() && st.toasts.is_empty());
        st.selected = None;
        st.command(now, actions::NEW_SESSION, None);
        assert_eq!(picker(&st).field.text(), st.source.home().to_string_lossy());
    }

    #[test]
    fn recent_choices_leave_out_folders_that_no_longer_exist() {
        let st = demo_state();
        let all = recent_folders(&st.all_groups, |_| true);
        let gone = all[0].clone();
        let shown = recent_folders(&st.all_groups, |path| path != gone);
        assert!(!shown.contains(&gone));
        assert_eq!(shown[0], all[1]);
        assert_eq!(shown.len(), RECENT_LIMIT.min(all.len() + usize::from(st.all_groups.len() > RECENT_LIMIT) - 1));
    }

    #[test]
    fn a_picked_folder_fills_the_field_and_keeps_the_dialog_open() {
        let mut st = demo_state();
        let now = Instant::now();
        st.ask_new_session();
        dialogs::click(&mut st, now, BROWSE, 0.0);
        assert!(matches!(st.dialog, Some(Dialog::NewSession(_))), "Browse keeps the dialog (demo opens no picker)");
        let folder = st.sessions[1].cwd.clone();
        st.new_session_folder_picked(&folder);
        let modal = picker(&st);
        assert_eq!(modal.field.text(), folder.to_string_lossy());
        assert!(modal.error.is_none());
        st.dialog = None;
        st.new_session_folder_picked(&folder);
        assert!(st.dialog.is_none(), "a late pick after the dialog closed opens nothing");
    }

    #[test]
    fn recent_choices_are_distinct_and_newest_first() {
        let mut st = demo_state();
        st.ask_new_session();
        let recent = &picker(&st).recent;
        assert_eq!(recent.len(), RECENT_LIMIT);
        assert_eq!(recent[0], st.sessions[0].cwd);
        let mut expected = Vec::new();
        for session in &st.sessions {
            if !expected.contains(&session.cwd) {
                expected.push(session.cwd.clone());
            }
        }
        assert_eq!(recent.as_slice(), &expected[..RECENT_LIMIT]);
    }

    #[test]
    fn choosing_a_folder_loads_its_default_and_successful_start_remembers_an_override() {
        let mut st = demo_state();
        let now = Instant::now();
        let cwd = st.all_groups[1].cwd.clone();
        st.remember_folder_open(&cwd, "work", App::Desktop);
        st.ask_new_session();
        dialogs::click(&mut st, now, RECENT + 1, 0.0);
        assert_eq!(picker(&st).accounts[picker(&st).account].0, "work");
        assert_eq!(picker(&st).app, App::Desktop);
        dialogs::click(&mut st, now, CLI, 0.0);
        dialogs::click(&mut st, now, OK, 0.0);
        assert_eq!(st.folder_open(&cwd, App::Desktop), ("work", App::Cli));
        assert_eq!(st.config.current, "main");
    }

    #[test]
    fn enter_refuses_an_invalid_folder_then_starts_with_the_chosen_folder_and_account() {
        let mut st = demo_state();
        let now = Instant::now();
        st.ask_new_session();
        dialogs::text_input(&mut st, r"C:\made-up\missing");
        dialogs::key(&mut st, now, &KeyName::Enter, Mods::default(), None);
        assert!(picker(&st).error.is_some());
        assert!(st.toasts.is_empty());
        let chosen = picker(&st).recent[1].clone();
        dialogs::click(&mut st, now, RECENT + 1, 0.0);
        assert_eq!(picker(&st).field.text(), chosen.to_string_lossy());
        dialogs::key(&mut st, now, &KeyName::Tab, Mods::default(), None);
        dialogs::key(&mut st, now, &KeyName::Right, Mods::default(), None);
        assert_eq!(picker(&st).accounts[picker(&st).account].0, "work");
        dialogs::key(&mut st, now, &KeyName::Enter, Mods::default(), None);
        assert!(st.dialog.is_none() && !st.toasts.is_empty());
        assert_eq!(st.config.current, "main", "the pick is only for this launch");
    }

    #[test]
    fn real_folder_validation_refuses_blank_missing_and_file_paths() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(folder_from_text(&here.to_string_lossy(), Path::is_dir), Ok(here.to_owned()));
        assert!(folder_from_text(" ", Path::is_dir).is_err());
        let file = here.join("Cargo.toml");
        assert!(folder_from_text(&file.to_string_lossy(), Path::is_dir).is_err());
        assert!(folder_from_text(&file.join("missing").to_string_lossy(), Path::is_dir).is_err());
    }
}
