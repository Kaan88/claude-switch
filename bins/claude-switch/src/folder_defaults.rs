//! Folder launch defaults: editor and details block.

use std::path::Path;

use claude_switch_core::{App, folder_defaults, sessions::folder_key};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::KeyName;
use mdshared_ui::overlay::{DIALOG_PAD, dialog_card, draw_dialog_button};

use crate::dialogs::{CANCEL, Dialog, Draw, OK};
use crate::state::{AppState, Target};

const PREVIOUS: u32 = 10;
const NEXT: u32 = 11;
pub const CLI: u32 = 30;
pub const DESKTOP: u32 = 31;
const CLEAR: u32 = 14;

pub struct Editor {
    key: String,
    folder: String,
    accounts: Vec<(String, String)>,
    account: usize,
    app: App,
    saved: bool,
}

impl AppState {
    pub fn folder_open(&self, cwd: &Path, fallback: App) -> (&str, App) {
        folder_defaults::resolve(&self.marks, &self.config, cwd, fallback)
    }

    pub fn remember_folder_open(&mut self, cwd: &Path, account: &str, app: App) {
        self.set_folder_open(&folder_key(cwd), Some((account, app)));
    }

    fn set_folder_open(&mut self, key: &str, value: Option<(&str, App)>) {
        if key.is_empty() || self.marks.folder_default(key) == value {
            return;
        }
        let before = self.marks.clone();
        if self.marks.set_folder_default(key, value) && !self.save_marks() {
            self.marks = before;
        }
    }

    pub fn edit_folder_open(&mut self) {
        let Some(s) = self.selected_session().filter(|s| !s.cwd.as_os_str().is_empty()) else { return };
        let key = folder_key(&s.cwd);
        let (slug, app) = self.folder_open(&s.cwd, s.last_app);
        let accounts: Vec<_> = self.config.accounts.iter().map(|a| (a.slug.clone(), a.name.clone())).collect();
        let account = accounts.iter().position(|(a, _)| a == slug).unwrap_or(0);
        self.dialog = Some(Dialog::FolderOpen(Box::new(Editor {
            saved: self.marks.folder_default(&key).is_some(),
            key,
            folder: s.cwd.display().to_string(),
            accounts,
            account,
            app,
        })));
    }
}

pub fn app_label(app: App) -> &'static str {
    if app == App::Cli { "the CLI" } else { "Desktop" }
}

pub fn draw_details(st: &mut AppState, p: &mut Painter, x: f32, y: f32, w: f32, cwd: &Path, fallback: App) -> f32 {
    if cwd.as_os_str().is_empty() {
        return y;
    }
    let pal = *st.pal();
    let (slug, app) = st.folder_open(cwd, fallback);
    let text = format!("Opens as {} in {}", st.account_name(slug), app_label(app));
    p.fill(Rect::new(x, y - 6.0, w, p.px()), pal.line);
    p.text(Face::Ui, 12.0, x, y + 16.0, &text, pal.muted, (w - 92.0).max(0.0));
    let r = Rect::new(x + w - 82.0, y + 2.0, 82.0, 28.0);
    let t = Target::FolderOpen;
    draw_dialog_button(p, &pal, r, "Change", false, st.hover == Some(t), st.pressed == Some(t));
    st.hits.push((r, t));
    y + 47.0
}

pub fn draw_app(d: &mut Draw<'_>, p: &mut Painter, r: Rect, app: App) {
    let half = (r.w - 8.0) / 2.0;
    d.button(p, Rect::new(r.x, r.y, half, r.h), "CLI", app == App::Cli, CLI);
    d.button(p, Rect::new(r.x + half + 8.0, r.y, half, r.h), "Desktop", app == App::Desktop, DESKTOP);
}

impl Editor {
    pub fn draw(&self, d: &mut Draw<'_>, p: &mut Painter, window: Rect) {
        let card = dialog_card(440.0, 260.0, window.w, window.h);
        let footer = d.card(p, card);
        d.title(p, card, "Folder default");
        let x = card.x + DIALOG_PAD;
        let w = card.w - 2.0 * DIALOG_PAD;
        let y = card.y;
        p.text(Face::Ui, 12.0, x, y + 61.0, &self.folder, d.pal.muted, w);
        p.text(Face::Ui, 12.0, x, y + 92.0, "Account", d.pal.muted, 74.0);
        let r = Rect::new(x + 77.0, y + 76.0, w - 77.0, 32.0);
        p.frame(r, 5.0, d.pal.field, d.pal.line);
        if let Some((_, name)) = self.accounts.get(self.account) {
            p.text_center(Face::Semibold, 13.0, Rect::new(r.x + 38.0, r.y, r.w - 76.0, r.h), name, d.pal.text);
        }
        if self.accounts.len() > 1 {
            d.button(p, Rect::new(r.x + 2.0, r.y + 2.0, 34.0, 28.0), "‹", false, PREVIOUS);
            d.button(p, Rect::new(r.right() - 36.0, r.y + 2.0, 34.0, 28.0), "›", false, NEXT);
        }
        p.text(Face::Ui, 12.0, x, y + 135.0, "App", d.pal.muted, 74.0);
        draw_app(d, p, Rect::new(x + 77.0, y + 119.0, w - 77.0, 32.0), self.app);
        p.text(Face::Ui, 12.0, x, y + 175.0, "Opening or starting a session updates this default.", d.pal.muted, w);
        let mut buttons = vec![("Save", true, OK), ("Cancel", false, CANCEL)];
        if self.saved {
            buttons.push(("Clear default", false, CLEAR));
        }
        d.footer(p, footer, &buttons);
    }

    pub fn key(&mut self, key: &KeyName) {
        match key {
            KeyName::Left => self.change(PREVIOUS),
            KeyName::Right => self.change(NEXT),
            KeyName::Up | KeyName::Down => self.change(if self.app == App::Cli { DESKTOP } else { CLI }),
            _ => {}
        }
    }

    fn change(&mut self, id: u32) {
        let n = self.accounts.len();
        match id {
            PREVIOUS if n > 1 => self.account = (self.account + n - 1) % n,
            NEXT if n > 1 => self.account = (self.account + 1) % n,
            CLI => self.app = App::Cli,
            DESKTOP => self.app = App::Desktop,
            _ => {}
        }
    }

    pub fn click(mut self: Box<Self>, st: &mut AppState, id: u32) {
        match id {
            OK => {
                if let Some((slug, _)) = self.accounts.get(self.account) {
                    st.set_folder_open(&self.key, Some((slug, self.app)));
                }
            }
            CLEAR => st.set_folder_open(&self.key, None),
            CANCEL => {}
            _ => {
                self.change(id);
                st.dialog = Some(Dialog::FolderOpen(self));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::Pending;
    use crate::source::Source;
    use std::time::Instant;

    #[test]
    fn default_open_targets_remembered_account_and_app_and_refusals_do_not_save() {
        let mut st = AppState::new(Source::Demo, None, Default::default());
        st.apply_scan(crate::demo::scan(&st.config));
        let now = Instant::now();
        let id = st.sessions[7].id.clone();
        let cwd = st.sessions[7].cwd.clone();
        st.remember_folder_open(&cwd, "work", App::Desktop);
        st.select(&id);
        st.command(now, crate::actions::OPEN_LAST, None);
        assert!(matches!(
            st.dialog.take(),
            Some(Dialog::Confirm { then: Pending::Open { id: target, account, app: App::Desktop }, .. })
                if target == id && account == "work"
        ));
        let running = st.sessions[0].clone();
        let before = st.folder_open(&running.cwd, App::Cli);
        let before = (before.0.to_owned(), before.1);
        st.open(now, &running.id, "work".into(), App::Desktop);
        assert_eq!(st.folder_open(&running.cwd, App::Cli), (before.0.as_str(), before.1));
    }
}
