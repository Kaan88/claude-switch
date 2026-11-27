//! The folder, account and app picker for a new session.

use std::path::{Path, PathBuf};
use std::time::Instant;

use claude_switch_core::App;
use claude_switch_core::sessions::folder_key;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::{DIALOG_FOOTER, DIALOG_PAD, dialog_card};
use mdshared_ui::textfield::{FieldColors, FieldHit, TextField};

use super::{CANCEL, Dialog, Draw, FIELD, OK};
use crate::folder_defaults::{CLI, DESKTOP, draw_app};
use crate::recent_folders::clean;
use crate::state::{AppState, Target};

#[path = "newsession/list.rs"]
mod list;
use list::List;

/// A folder row's hit is `ROW` plus its place in the filtered list.
const ROW: u32 = 1000;
const PREVIOUS: u32 = 20;
const NEXT: u32 = 21;
const BROWSE: u32 = 22;
const BROWSE_W: f32 = 84.0;
/// The dialog's keyboard stops. The folder list is entered with the arrows, not with Tab.
const FOLDER: u8 = 0;
const ACCOUNT: u8 = 1;
const APP: u8 = 2;
const LIST: u8 = 3;

pub struct NewSession {
    field: TextField,
    hit: Option<FieldHit>,
    list: List,
    accounts: Vec<(String, String)>,
    account: usize,
    app: App,
    folder_key: Option<String>,
    focus: u8,
    error: Option<&'static str>,
    /// "New OpenCode tab": the folder only; OpenCode has no Claude account or app.
    opencode: bool,
}

impl AppState {
    pub fn ask_new_session(&mut self) {
        self.check_folders();
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

    /// Brings an open New session dialog's folder list up to date.
    pub(crate) fn reload_new_session(&mut self) {
        if let Some(Dialog::NewSession(mut picker)) = self.dialog.take_if(|d| matches!(d, Dialog::NewSession(_))) {
            picker.list.set_folders(self.recent_folders());
            self.dialog = Some(Dialog::NewSession(picker));
        }
    }

    /// The tooltip of a control of the open dialog: a listed folder's full path.
    pub(crate) fn dialog_tip(&self, target: Target) -> Option<String> {
        let (Some(Dialog::NewSession(picker)), Target::Dialog(id)) = (&self.dialog, target) else { return None };
        let folder = picker.list.folder(id.checked_sub(ROW)? as usize)?;
        Some(folder.path.to_string_lossy().into_owned())
    }

    fn folder_list(&mut self) -> Option<&mut List> {
        match &mut self.dialog {
            Some(Dialog::NewSession(picker)) => Some(&mut picker.list),
            _ => None,
        }
    }

    /// A press on the folder list's scrollbar; true when it took it.
    pub(crate) fn folder_scroll_press(&mut self, x: f32, y: f32) -> bool {
        self.folder_list().is_some_and(|list| list.scroll_press(x, y))
    }

    /// The pointer moved; true while it drags the folder list's scrollbar.
    pub(crate) fn folder_scroll_drag(&mut self, y: f32) -> bool {
        self.folder_list().is_some_and(|list| list.scroll_drag(y))
    }

    /// Ends a drag of the folder list's scrollbar; true when there was one.
    pub(crate) fn folder_scroll_release(&mut self) -> bool {
        self.folder_list().is_some_and(|list| list.scroll_release())
    }

    /// Puts a picked folder into the New session dialog, if it is still open.
    pub fn new_session_folder_picked(&mut self, folder: &Path) {
        if let Some(Dialog::NewSession(mut picker)) = self.dialog.take_if(|d| matches!(d, Dialog::NewSession(_))) {
            picker.picked(self, folder);
            self.dialog = Some(Dialog::NewSession(picker));
        }
    }
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
            list: List::new(st.recent_folders()),
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
        // The card without the list's rows: the fields, the list's label and gap, the footer.
        let fixed = if self.opencode { 132.0 } else { 216.0 } + 28.0 + DIALOG_FOOTER;
        let list_height = self.list.height(h - 48.0 - fixed);
        let card = dialog_card(600.0, fixed + list_height, w, h);
        let footer = d.card(p, card);
        d.title(p, card, if self.opencode { "New OpenCode tab" } else { "New session" });
        let x = card.x + DIALOG_PAD;
        let width = card.w - 2.0 * DIALOG_PAD;

        let fr = Rect::new(x, card.y + 64.0, width - BROWSE_W - 8.0, 32.0);
        p.text(Face::Ui, 12.0, x, fr.y - 9.0, "Folder", pal.muted, width);
        d.button(p, Rect::new(fr.right() + 8.0, fr.y, BROWSE_W, 32.0), "Browse…", false, BROWSE);
        let typing = self.focus == FOLDER;
        p.frame(fr, 5.0, pal.field, if typing { pal.accent } else { pal.line });
        let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
        let placeholder = "Folder path, or type to filter the list";
        self.field.draw(p, fr.inset(8.0), Face::Ui, 13.0, &colors, placeholder, typing, true);
        self.hit = crate::fieldmouse::hit(p, fr.inset(8.0), 13.0);
        d.hits.push((fr, Target::Dialog(FIELD)));
        if let Some(error) = self.error {
            p.text(Face::Ui, 12.0, x, fr.bottom() + 10.0, error, pal.danger, width);
        }

        let mut y = card.y + 118.0;
        p.text(Face::Ui, 12.0, x, y + 6.0, "Recent folders", pal.muted, width);
        if let Some(count) = self.list.count() {
            p.text_right(Face::Ui, 12.0, x + width, y + 6.0, &count, pal.muted);
        }
        y += 18.0;
        let chosen = folder_key(&clean(Path::new(self.field.text().trim())));
        self.list.draw(d, p, Rect::new(x, y, width, list_height), &chosen, self.focus == LIST);
        y += list_height + 10.0;
        if self.opencode {
            let note = "Your own OpenCode setup, in a terminal tab. Not a Claude session.";
            p.text(Face::Ui, 12.0, x, y + 8.0, note, pal.muted, width);
            d.footer(p, footer, &[("Start", true, OK), ("Cancel", false, CANCEL)]);
            return;
        }

        p.text(Face::Ui, 12.0, x, y + 16.0, "Account", pal.muted, 67.0);
        let r = Rect::new(x + 70.0, y, width - 70.0, 32.0);
        p.frame(r, 5.0, pal.field, if self.focus == ACCOUNT { pal.accent } else { pal.line });
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
        if self.focus == APP {
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
                        self.focus = FOLDER;
                    }
                }
            }
            FIELD => self.focus = FOLDER,
            BROWSE => {
                self.focus = FOLDER;
                st.pick_new_session_folder(Path::new(self.field.text().trim()));
            }
            PREVIOUS | NEXT => {
                self.focus = ACCOUNT;
                self.step_account(id == NEXT);
            }
            CLI | DESKTOP => {
                self.focus = APP;
                self.app = if id == CLI { App::Cli } else { App::Desktop };
            }
            id if id >= ROW => self.pick(st, (id - ROW) as usize),
            _ => {}
        }
        true
    }

    /// Puts the listed folder at `index` into the field, with its folder default.
    fn pick(&mut self, st: &AppState, index: usize) {
        let Some(path) = self.list.folder(index).map(|f| f.path.to_string_lossy().into_owned()) else { return };
        self.field.set_text(&path);
        self.list.highlight = None;
        self.error = None;
        self.focus = FOLDER;
        self.sync_defaults(st);
    }

    /// The field's text was edited: it filters the list.
    fn typed(&mut self, st: &AppState) {
        self.error = None;
        self.focus = FOLDER;
        self.list.filter(self.field.text());
        self.sync_defaults(st);
    }

    fn tab(&mut self, back: bool) {
        self.focus = match (self.focus, back) {
            _ if self.opencode => FOLDER,
            (LIST, true) => FOLDER,
            (FOLDER | LIST, false) | (APP, true) => ACCOUNT,
            (ACCOUNT, false) | (FOLDER, true) => APP,
            _ => FOLDER,
        };
    }

    /// True when the key was used up. Otherwise Enter and Escape go on through the common dialog
    /// routes to Start and Cancel.
    pub(super) fn key(&mut self, st: &AppState, key: &KeyName, mods: Mods, text: Option<&str>) -> bool {
        let listing = matches!(self.focus, FOLDER | LIST);
        match key {
            KeyName::Escape => return false,
            KeyName::Enter => {
                let Some(index) = self.list.highlight.filter(|_| listing) else { return false };
                self.pick(st, index);
            }
            KeyName::Tab => self.tab(mods.shift),
            KeyName::Left | KeyName::Up if self.focus == ACCOUNT => self.step_account(false),
            KeyName::Right | KeyName::Down if self.focus == ACCOUNT => self.step_account(true),
            KeyName::Left | KeyName::Up if self.focus == APP => self.app = App::Cli,
            KeyName::Right | KeyName::Down if self.focus == APP => self.app = App::Desktop,
            KeyName::Up | KeyName::Down | KeyName::PageUp | KeyName::PageDown if listing => {
                if self.list.step(key) {
                    self.focus = LIST;
                }
            }
            // In the field these two move the caret.
            KeyName::Home | KeyName::End if self.focus == LIST => {
                self.list.step(key);
            }
            _ if listing => match self.field.handle_key(key, mods, text, true) {
                Some(true) => self.typed(st),
                // The caret moved: the keyboard is in the field again.
                Some(false) => self.focus = FOLDER,
                None => return false,
            },
            _ => return false,
        }
        true
    }

    pub(super) fn wheel(&mut self, dy: f32) {
        self.list.wheel(dy);
    }

    pub(crate) fn field_parts(&mut self) -> (&mut TextField, Option<FieldHit>) {
        (&mut self.field, self.hit)
    }

    /// A folder chosen in the system's folder picker.
    pub(crate) fn picked(&mut self, st: &AppState, folder: &Path) {
        self.field.set_text(&folder.to_string_lossy());
        self.field.select_all();
        self.list.highlight = None;
        self.error = None;
        self.focus = FOLDER;
        self.sync_defaults(st);
    }

    pub(crate) fn focus_folder(&mut self) {
        self.focus = FOLDER;
    }

    /// The folder changed other than by typing (the right-click menu).
    pub(crate) fn edited(&mut self, st: &AppState) {
        self.typed(st);
    }

    pub(super) fn text_input(&mut self, st: &AppState, text: &str) {
        if matches!(self.focus, FOLDER | LIST) && self.field.insert(text) {
            self.typed(st);
        }
    }
}

#[cfg(test)]
#[path = "newsession/tests.rs"]
mod tests;
