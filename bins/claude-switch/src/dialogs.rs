//! The modal dialogs: a confirmation, the accounts, a name to type, the settings and About. Each
//! is drawn with the toolkit's dialog pieces (`overlay`) or contains a `settings::Form`; their
//! controls are recorded as [`Target::Dialog`] hits with the numbers below.

use std::time::Instant;

use claude_switch_core::App;
use claude_switch_core::share::{LinkState, ShareState};
use claude_switch_core::usage::Demand;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::host::Cx;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::{
    DIALOG_FOOTER, DIALOG_PAD, dialog_button_w, dialog_card, draw_backdrop_presence, draw_dialog_button,
    draw_dialog_card_with,
};
use mdshared_ui::textfield::{FieldColors, TextField};
use mdshared_ui::theme::Palette;

use crate::session_panel::Panel;
use crate::state::{AppState, Msg, Target};
use crate::usage_ui::AccountUsage;

#[path = "newsession.rs"]
mod newsession;
mod opencode_rows;
mod settings;
mod terminal_appearance;
pub(crate) use opencode_rows::update_opencode_rows;
pub(crate) use settings::{OPENCODE, TERMINAL, settings, settings_event};

pub const OK: u32 = 1;
pub const CANCEL: u32 = 2;
pub const FIELD: u32 = 4;
pub const ADD: u32 = 5;
/// An account's buttons: `ACCOUNT + index * 10 + one of the `ACT_` numbers.
const ACCOUNT: u32 = 100;
const ACT_LOGIN: u32 = 0;
const ACT_DESKTOP: u32 = 1;
const ACT_RENAME: u32 = 2;
const ACT_REMOVE: u32 = 3;
const ACT_LINK: u32 = 4;
const ACT_REFRESH_USAGE: u32 = 5;

/// What a confirmation leads to.
#[derive(Clone, Debug)]
pub enum Pending {
    Open {
        id: String,
        account: String,
        app: App,
    },
    /// Open a copy (a fork) of a session as an account.
    Copy {
        id: String,
        account: String,
    },
    Remove {
        slug: String,
    },
    /// Close a terminal tab whose program runs.
    CloseTab(u64),
    CloseOtherTabs(u64),
    RestoreTab,
    /// Close the window, with terminal tabs that run.
    Quit,
    Suggest {
        ids: Vec<String>,
        batch: bool,
    },
    UnarchiveAll,
    RestartOpenCode(crate::opencode_workers::Restart),
}

impl Pending {
    /// Whether the confirming button is the primary one, which Enter chooses: yes for opening
    /// something, no for what ends or removes something (there the safe choice, Cancel, is).
    pub fn ok_is_primary(&self) -> bool {
        matches!(self, Pending::Open { .. } | Pending::Copy { .. } | Pending::RestoreTab)
    }
}

#[derive(Clone, Debug)]
pub enum NameFor {
    Add,
    Rename(String),
    Tab(u64),
}

pub enum Dialog {
    Confirm { title: String, text: String, ok: String, then: Pending },
    Accounts { scroll: f32, shares: Vec<(String, Vec<ShareState>)> },
    Name { title: String, field: TextField, stops: Vec<f32>, then: NameFor, error: Option<String> },
    NewSession(Box<newsession::NewSession>),
    FolderOpen(Box<crate::folder_defaults::Editor>),
    Archive(Box<crate::archive::ArchiveDialog>),
    ArchiveReport(crate::archive_report::Report),
    Compact(Box<crate::compact_dialog::Compact>),
    Session(Box<Panel>),
    Settings(Box<settings::Settings>),
    About,
}

pub fn accounts(st: &mut AppState, now: Instant) -> Dialog {
    st.request_usage(now, Demand::Automatic, None);
    let shares =
        st.config.accounts.iter().filter(|a| !a.primary).map(|a| (a.slug.clone(), st.source.share_status(a))).collect();
    Dialog::Accounts { scroll: 0.0, shares }
}

pub(crate) fn name_dialog(title: &str, text: &str, then: NameFor) -> Dialog {
    let mut field = TextField::new();
    field.set_text(text);
    field.select_all();
    Dialog::Name { title: title.into(), field, stops: Vec::new(), then, error: None }
}

// ---- drawing ---------------------------------------------------------------------------------

pub(crate) struct Draw<'a> {
    pub hits: &'a mut Vec<(Rect, Target)>,
    pub hover: Option<Target>,
    pressed: Option<Target>,
    pub pal: Palette,
    radius: f32,
    shadow: f32,
}

impl Draw<'_> {
    pub fn card(&self, p: &mut Painter, r: Rect) -> Rect {
        draw_dialog_card_with(p, &self.pal, r, self.radius, Some(self.shadow))
    }

    pub(crate) fn button(&mut self, p: &mut Painter, r: Rect, label: &str, primary: bool, id: u32) {
        let t = Some(Target::Dialog(id));
        draw_dialog_button(p, &self.pal, r, label, primary, self.hover == t, self.pressed == t);
        self.hits.push((r, Target::Dialog(id)));
    }

    /// Footer buttons from the right, the first given rightmost.
    pub fn footer(&mut self, p: &mut Painter, footer: Rect, buttons: &[(&str, bool, u32)]) {
        let y = footer.y + (DIALOG_FOOTER - 32.0) / 2.0;
        let mut right = footer.right() - DIALOG_PAD;
        for &(label, primary, id) in buttons {
            let w = dialog_button_w(p, label);
            self.button(p, Rect::new(right - w, y, w, 32.0), label, primary, id);
            right -= w + 8.0;
        }
    }

    pub fn title(&self, p: &mut Painter, card: Rect, text: &str) {
        p.text(
            Face::Semibold,
            16.0,
            card.x + DIALOG_PAD,
            card.y + 34.0,
            text,
            self.pal.text,
            card.w - 2.0 * DIALOG_PAD,
        );
    }
}

pub fn draw(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter, window: Rect, t: f32) {
    let (w, h) = (window.w, window.h);
    let Some(mut dialog) = st.dialog.take() else { return };
    let look = st.appearance;
    let shadow = if st.high_contrast { 0.0 } else { look.shadow };
    if let Dialog::Settings(form) = &mut dialog {
        form.draw(p, st, window, t, cx.anim());
        st.dialog = Some(dialog);
        return;
    }
    let pal = *st.pal();
    draw_backdrop_presence(p, &pal, w, h, t);
    let usage = if matches!(dialog, Dialog::Accounts { .. }) && st.config.show_usage {
        let counts = st.config.accounts.iter().map(|a| st.running_for_usage(&a.slug)).collect::<Vec<_>>();
        let (usage, statuses, demo) = (&st.usage, &st.statuses, st.source.is_demo());
        st.config
            .accounts
            .iter()
            .zip(counts)
            .map(|(a, running)| {
                let status = statuses.get(&a.slug);
                AccountUsage {
                    plan: status.and_then(|s| s.plan.as_deref()),
                    reading: usage.reading(&a.slug),
                    attempt: usage.attempt(&a.slug),
                    signed_in: status.map(|s| s.cli_logged_in),
                    running,
                    refresh: !demo && usage.can_refresh(&a.slug, cx.now()),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut d = Draw { hits: &mut st.hits, hover: st.hover, pressed: st.pressed, pal, radius: look.radius, shadow };
    match &mut dialog {
        Dialog::Confirm { title, text, ok, then } => {
            let tw = 460.0 - 2.0 * DIALOG_PAD;
            let th = p.paragraph_height_wrapped(Face::Ui, 13.0, tw, text, Wrap::Words);
            let body = th.min((h - 110.0 - DIALOG_FOOTER).max(32.0));
            let card = dialog_card(460.0, 90.0 + body + DIALOG_FOOTER, w, h);
            let footer = d.card(p, card);
            d.title(p, card, title);
            let r = Rect::new(card.x + DIALOG_PAD, card.y + 56.0, tw, body + 4.0);
            st.confirm_scroll = st.confirm_scroll.clamp(0.0, (th - body).max(0.0));
            let clip = p.clip(r);
            p.paragraph_wrapped(Face::Ui, 13.0, r.x, r.y + 2.0 - st.confirm_scroll, tw, text, pal.text, Wrap::Words);
            p.restore(clip);
            if th > body {
                p.text(Face::Ui, 11.0, r.x, r.bottom() + 14.0, "Scroll to review the full list.", pal.muted, tw);
            }
            let primary = then.ok_is_primary();
            d.footer(p, footer, &[(ok.as_str(), primary, OK), ("Cancel", !primary, CANCEL)]);
        }
        Dialog::Name { title, field, stops, error, then } => {
            let card = dialog_card(420.0, 120.0 + DIALOG_FOOTER, w, h);
            let footer = d.card(p, card);
            d.title(p, card, title);
            let fr = Rect::new(card.x + DIALOG_PAD, card.y + 58.0, card.w - 2.0 * DIALOG_PAD, 32.0);
            p.frame(fr, 5.0, pal.field, pal.accent);
            let inner = fr.inset(8.0);
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            let placeholder =
                if matches!(then, NameFor::Tab(_)) { "Tab name (blank resets it)" } else { "Account name" };
            field.draw(p, inner, Face::Ui, 13.0, &colors, placeholder, true, true);
            *stops = p.caret_stops(Face::Ui, 13.0, &field.display());
            d.hits.push((fr, Target::Dialog(FIELD)));
            if let Some(e) = error {
                p.text(Face::Ui, 12.0, fr.x, fr.bottom() + 14.0, e, pal.danger, fr.w);
            }
            d.footer(p, footer, &[("OK", true, OK), ("Cancel", false, CANCEL)]);
        }
        Dialog::About => {
            let card = dialog_card(420.0, 200.0, w, h);
            let footer = d.card(p, card);
            d.title(p, card, "Claude Switch");
            let x = card.x + DIALOG_PAD;
            let tw = card.w - 2.0 * DIALOG_PAD;
            p.text(Face::Ui, 13.0, x, card.y + 62.0, concat!("Version ", env!("CARGO_PKG_VERSION")), pal.muted, tw);
            p.paragraph(
                Face::Ui,
                13.0,
                x,
                card.y + 78.0,
                tw,
                "Lists the Claude Code sessions on this computer and opens them in the CLI or the desktop app, as either account.",
                pal.text,
            );
            d.footer(p, footer, &[("Close", true, CANCEL)]);
        }
        Dialog::Accounts { scroll, shares } => {
            draw_accounts((&st.config, &st.statuses, &usage), &mut d, p, w, h, scroll, shares)
        }
        Dialog::NewSession(picker) => picker.draw(&mut d, p, w, h),
        Dialog::FolderOpen(editor) => editor.draw(&mut d, p, window),
        Dialog::Archive(archive) => archive.draw(&mut d, p, window),
        Dialog::ArchiveReport(report) => report.draw(&mut d, p, window),
        Dialog::Compact(dialog) => dialog.draw(&mut d, p, w, h),
        Dialog::Session(panel) => panel.draw(&mut d, p, window),
        Dialog::Settings(_) => {}
    }
    st.dialog = Some(dialog);
}

type StView<'a> = (
    &'a claude_switch_core::Config,
    &'a std::collections::HashMap<String, claude_switch_core::AccountStatus>,
    &'a [AccountUsage<'a>],
);

const BLOCK: f32 = 96.0;
const SHARE_BLOCK: f32 = 58.0;

fn draw_accounts(
    (config, statuses, usage): StView<'_>,
    d: &mut Draw<'_>,
    p: &mut Painter,
    w: f32,
    h: f32,
    scroll: &mut f32,
    shares: &[(String, Vec<ShareState>)],
) {
    let pal = d.pal;
    let content: f32 = config.accounts.iter().map(|a| if a.primary { BLOCK } else { BLOCK + SHARE_BLOCK }).sum::<f32>()
        + usage.iter().map(AccountUsage::height).sum::<f32>()
        + 8.0;
    let now = claude_switch_core::usage::now_ms();
    let card = dialog_card(640.0, 60.0 + content + DIALOG_FOOTER, w, h);
    let footer = d.card(p, card);
    d.title(p, card, "Accounts");
    let body = Rect::new(card.x + 1.0, card.y + 54.0, card.w - 2.0, footer.y - card.y - 54.0);
    *scroll = scroll.clamp(0.0, (content - body.h).max(0.0));
    let old = p.clip(body);
    let x = card.x + DIALOG_PAD;
    let tw = card.w - 2.0 * DIALOG_PAD;
    let mut y = body.y - *scroll;
    for (k, a) in config.accounts.iter().enumerate() {
        let base = ACCOUNT + k as u32 * 10;
        p.rbox(Rect::new(x, y + 7.0, 10.0, 10.0), 5.0, a.color);
        let nw = p.text(Face::Semibold, 14.0, x + 18.0, y + 12.0, &a.name, pal.text, tw * 0.6);
        if a.primary {
            p.text(Face::Ui, 12.0, x + 26.0 + nw, y + 12.0, "first account", pal.muted, 160.0);
        }
        let status = statuses.get(&a.slug);
        let line = match status {
            Some(s) if s.cli_logged_in => {
                let mut parts = vec!["Signed in to the CLI".to_string()];
                parts.extend(s.display_name.clone());
                if usage.is_empty() {
                    parts.extend(s.plan.clone());
                }
                parts.extend(s.email.clone());
                parts.join(" \u{b7} ")
            }
            Some(_) => "Not signed in to the CLI".to_string(),
            None => "Not read yet".to_string(),
        };
        let desk = match status {
            Some(s) if s.desktop_used => "Desktop app used",
            Some(_) => "Desktop app not used yet",
            None => "",
        };
        p.text(Face::Ui, 12.0, x + 18.0, y + 33.0, &format!("{line}    {desk}"), pal.muted, tw - 18.0);
        let mut bx = x + 18.0;
        let by = y + 50.0;
        let mut buttons =
            vec![("Sign in to CLI", ACT_LOGIN), ("Open Desktop as this account", ACT_DESKTOP), ("Rename", ACT_RENAME)];
        if !a.primary {
            buttons.push(("Remove", ACT_REMOVE));
        }
        for (label, act) in buttons {
            let bw = (p.measure(Face::Ui, 13.0, label) + 24.0).round();
            let r = Rect::new(bx, by, bw, 28.0);
            if r.y >= body.y && r.bottom() <= body.bottom() {
                d.button(p, r, label, false, base + act);
            } else {
                draw_dialog_button(p, &pal, r, label, false, false, false);
            }
            bx += bw + 8.0;
        }
        y += BLOCK;
        if let Some(u) = usage.get(k) {
            let ar = Rect::new(x + 18.0, y - 6.0, tw - 18.0, u.height());
            let button = u.draw(p, &pal, ar, now);
            if !u.refresh {
                mdshared_ui::overlay::draw_dialog_button_disabled(p, &pal, button, "Refresh", false);
            } else if button.y >= body.y && button.bottom() <= body.bottom() {
                d.button(p, button, "Refresh", false, base + ACT_REFRESH_USAGE);
            }
            y += u.height();
        }
        if !a.primary {
            let states = shares.iter().find(|(s, _)| s == &a.slug).map_or(&[][..], |(_, v)| v.as_slice());
            let names =
                |want: LinkState| -> Vec<&str> { states.iter().filter(|s| s.state == want).map(|s| s.name).collect() };
            let (linked, missing, conflict) =
                (names(LinkState::Linked), names(LinkState::Missing), names(LinkState::Conflict));
            let mut text = format!(
                "Shared with the first account: {}",
                if linked.is_empty() { "nothing yet".into() } else { linked.join(", ") }
            );
            if !missing.is_empty() {
                text.push_str(&format!(". Not linked yet: {}", missing.join(", ")));
            }
            let lw = if missing.is_empty() { tw - 18.0 } else { tw - 120.0 };
            p.paragraph_wrapped(Face::Ui, 12.0, x + 18.0, y - 8.0, lw, &text, pal.text, Wrap::Anywhere);
            if !conflict.is_empty() {
                let c = format!("{} is this account's own and is left alone.", conflict.join(", "));
                p.text(Face::Ui, 12.0, x + 18.0, y + 34.0, &c, pal.warn, tw - 18.0);
            }
            if !missing.is_empty() {
                let r = Rect::new(x + tw - 92.0, y - 6.0, 92.0, 28.0);
                if r.y >= body.y && r.bottom() <= body.bottom() {
                    d.button(p, r, "Link now", true, base + ACT_LINK);
                }
            }
            y += SHARE_BLOCK;
        }
        if k + 1 < config.accounts.len() {
            p.fill(Rect::new(x, y - 10.0, tw, p.px()), pal.line);
        }
    }
    p.restore(old);
    d.footer(p, footer, &[("Close", true, CANCEL), ("Add account…", false, ADD)]);
}

// ---- events ----------------------------------------------------------------------------------

/// A press on a dialog control.
pub fn click(st: &mut AppState, now: Instant, id: u32, x: f32) {
    let Some(dialog) = st.dialog.take() else { return };
    match dialog {
        Dialog::Confirm { then, title, text, ok } => match (id, then) {
            (OK, Pending::Open { id, account, app }) => st.open_now(now, &id, &account, app),
            (OK, Pending::Copy { id, account }) => st.copy_now(now, &id, &account),
            (OK, Pending::CloseTab(tab)) => st.close_tab_now(tab),
            (OK, Pending::CloseOtherTabs(tab)) => st.close_other_tabs(tab),
            (OK | CANCEL, Pending::RestoreTab) => st.confirm_restore_tab(now, id == OK),
            (OK, Pending::Quit) => st.quit = true,
            (OK, Pending::Suggest { ids, batch }) => st.start_naming(ids, batch),
            (OK, Pending::UnarchiveAll) => st.unarchive(now, None),
            (OK | CANCEL, Pending::RestartOpenCode(request)) => st.confirm_restart_opencode(request, id == OK),
            (OK, Pending::Remove { slug }) => {
                match st.source.remove(&mut st.config, &slug) {
                    Ok(()) => accounts_changed(st, now),
                    Err(e) => st.error("Not removed", e.to_string()),
                }
                st.dialog = Some(accounts(st, now));
            }
            (CANCEL, Pending::Remove { .. }) => st.dialog = Some(accounts(st, now)),
            (CANCEL, Pending::Quit) => st.restore_prompt_after_cancelled_quit(now),
            (CANCEL, _) => {}
            (_, then) => st.dialog = Some(Dialog::Confirm { title, text, ok, then }),
        },
        Dialog::Name { mut field, stops, then, title, error } => match id {
            OK => submit_name(st, now, field, stops, then, title),
            CANCEL if matches!(then, NameFor::Tab(_)) => {}
            CANCEL => st.dialog = Some(accounts(st, now)),
            FIELD => {
                let inner_x =
                    x - st.hits.iter().find(|(_, t)| *t == Target::Dialog(FIELD)).map_or(0.0, |(r, _)| r.x + 8.0);
                field.press(inner_x, &stops, false);
                st.dialog = Some(Dialog::Name { title, field, stops, then, error });
            }
            _ => st.dialog = Some(Dialog::Name { title, field, stops, then, error }),
        },
        Dialog::About => {
            if id != CANCEL {
                st.dialog = Some(Dialog::About);
            }
        }
        Dialog::Accounts { scroll, shares } => match id {
            CANCEL => {}
            ADD => st.dialog = Some(name_dialog("Add an account", "", NameFor::Add)),
            _ if id >= ACCOUNT => {
                let (k, act) = (((id - ACCOUNT) / 10) as usize, (id - ACCOUNT) % 10);
                st.dialog = Some(Dialog::Accounts { scroll, shares });
                account_action(st, now, k, act);
            }
            _ => st.dialog = Some(Dialog::Accounts { scroll, shares }),
        },
        Dialog::NewSession(mut picker) => {
            if picker.click(st, now, id, x) {
                st.dialog = Some(Dialog::NewSession(picker));
            }
        }
        Dialog::FolderOpen(editor) => editor.click(st, id),
        Dialog::Archive(archive) => archive.click(st, id),
        Dialog::ArchiveReport(report) => report.click(st, id),
        Dialog::Session(panel) => panel.click(st, now, id, x, false),
        Dialog::Compact(mut compact) => {
            if compact.click(st, now, id, x) {
                st.dialog = Some(Dialog::Compact(compact));
            }
        }
        d @ Dialog::Settings(_) => st.dialog = Some(d),
    }
}

fn account_action(st: &mut AppState, now: Instant, k: usize, act: u32) {
    let Some(a) = st.config.accounts.get(k).cloned() else { return };
    match act {
        ACT_LOGIN => match st.source.login(&st.config, &a.slug) {
            Ok(l) => st.start(now, &l, None, Some(&a.slug), format!("Opened the sign-in for {} in a terminal", a.name)),
            Err(e) => st.error("Not started", e.to_string()),
        },
        ACT_DESKTOP => match st.source.open_desktop(&st.config, &a.slug) {
            Ok(l) => st.start(now, &l, None, Some(&a.slug), format!("Opened the desktop app as {}", a.name)),
            Err(e) => st.error("Not started", e.to_string()),
        },
        ACT_RENAME => st.dialog = Some(name_dialog(&format!("Rename {}", a.name), &a.name, NameFor::Rename(a.slug))),
        ACT_REMOVE => {
            st.dialog = Some(Dialog::Confirm {
                title: format!("Remove {}?", a.name),
                text: format!(
                    "The account leaves this list. Its folders and its logins stay on disk, untouched: {}",
                    a.cli_dir.parent().unwrap_or(&a.cli_dir).display()
                ),
                ok: "Remove".into(),
                then: Pending::Remove { slug: a.slug },
            })
        }
        ACT_LINK => {
            if let Err(e) = st.source.share_apply(&a) {
                st.error("Not every link was made", e.to_string());
            }
            st.dialog = Some(accounts(st, now));
        }
        ACT_REFRESH_USAGE => st.request_usage(now, Demand::Refresh, Some(&a.slug)),
        _ => {}
    }
}

fn submit_name(st: &mut AppState, now: Instant, field: TextField, stops: Vec<f32>, then: NameFor, title: String) {
    let name = field.text().trim().to_string();
    let result = match &then {
        NameFor::Add => st.source.add_account(&mut st.config, &name).map(drop),
        NameFor::Rename(slug) => st.source.rename(&mut st.config, slug, &name),
        NameFor::Tab(id) => {
            if let Some(tab) = st.tabs.get_mut(*id) {
                tab.rename = (!name.is_empty()).then_some(name);
            }
            return;
        }
    };
    match result {
        Ok(()) => {
            accounts_changed(st, now);
            st.dialog = Some(accounts(st, now));
        }
        Err(e) => st.dialog = Some(Dialog::Name { title, field, stops, then, error: Some(e.to_string()) }),
    }
}

/// After an account was added, renamed or removed: save, tell the bridge, read again.
fn accounts_changed(st: &mut AppState, now: Instant) {
    st.save_config();
    st.rebuild_accounts();
    st.refresh(now);
}

/// A key while a dialog is open. True when it was used.
pub fn key(st: &mut AppState, now: Instant, k: &KeyName, mods: Mods, text: Option<&str>) -> bool {
    let Some(mut dialog) = st.dialog.take() else { return false };
    match &mut dialog {
        Dialog::Settings(form) => {
            let o = form.key(k, mods, text);
            st.dialog = Some(dialog);
            settings_event(st, o);
            return true;
        }
        Dialog::Name { field, .. } => {
            if field.handle_key(k, mods, text, true).is_some() {
                st.dialog = Some(dialog);
                return true;
            }
        }
        Dialog::NewSession(picker) => picker.key(st, k, mods, text),
        Dialog::FolderOpen(editor) => editor.key(k),
        Dialog::Archive(archive) => archive.key(st, k),
        Dialog::Compact(compact) => {
            let action = compact.key(k, mods, text);
            st.dialog = Some(dialog);
            if let Some(action) = action {
                click(st, now, action, 0.0);
            }
            return true;
        }
        Dialog::Session(panel) => {
            let id = panel.key(st, k, mods, text);
            if let Some(id) = id
                && let Dialog::Session(panel) = dialog
            {
                panel.click(st, now, id, 0.0, mods.ctrl);
            } else {
                st.dialog = Some(dialog);
            }
            return true;
        }
        _ => {}
    }
    st.dialog = Some(dialog);
    match k {
        KeyName::Escape => click(st, now, CANCEL, 0.0),
        // Enter chooses the button drawn as primary.
        KeyName::Enter => match &st.dialog {
            Some(Dialog::Accounts { .. }) | Some(Dialog::About) => click(st, now, CANCEL, 0.0),
            Some(Dialog::Archive(_) | Dialog::ArchiveReport(crate::archive_report::Report::Saved(_))) => {
                click(st, now, CANCEL, 0.0)
            }
            Some(Dialog::Confirm { then, .. }) if !then.ok_is_primary() => click(st, now, CANCEL, 0.0),
            _ => click(st, now, OK, 0.0),
        },
        _ => {}
    }
    true
}

mod input;
pub use input::{text_input, wheel};
