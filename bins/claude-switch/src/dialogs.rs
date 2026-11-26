//! The modal dialogs: a confirmation, the accounts, a name to type, the settings and About. Each
//! is drawn with the toolkit's dialog pieces (`overlay`) or is a `settings::Form`; their
//! controls are recorded as [`Target::Dialog`] hits with the numbers below.

use std::time::Instant;

use claude_switch_core::config::{TERMINAL_FONT_MAX, TERMINAL_FONT_MIN};
use claude_switch_core::share::{LinkState, ShareState};
use claude_switch_core::{App, WorkersUse};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::colour_picker::parse_hex;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::host::Cx;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::{
    DIALOG_FOOTER, DIALOG_PAD, dialog_button_w, dialog_card, draw_backdrop_presence, draw_dialog_button,
    draw_dialog_card_with,
};
use mdshared_ui::settings::{Form, Outcome, Row, Style, Value};
use mdshared_ui::textfield::{FieldColors, TextField};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Msg, Target};
use crate::termtheme::{self, ACCENT_SOURCES, PRESETS, WINDOW_THEMES, preset_index};

#[path = "newsession.rs"]
mod newsession;

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

const SET_WORKERS: u32 = 1;
const SET_EMBEDDED: u32 = 2;
const SET_ARCHIVED: u32 = 3;
const SET_WORKERS_SHOWN: u32 = 4;
const SET_THEME: u32 = 5;
const SET_TERM_COLOURS: u32 = 6;
const SET_TERM_FONT: u32 = 7;
const SET_BACKDROP: u32 = 8;
const SET_ACCENT: u32 = 9;
const SET_ACCENT_COLOUR: u32 = 10;
const SET_REDUCE_MOTION: u32 = 11;
const SET_CORNERS: u32 = 12;
const SET_SHADOWS: u32 = 13;
const SET_CURSOR_BLINK: u32 = 14;
const SET_TERM_BACKGROUND: u32 = 15;
const SET_TERM_FOREGROUND: u32 = 16;
const SET_TERM_ACCENT: u32 = 17;

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
    /// Close the window, with terminal tabs that run.
    Quit,
}

impl Pending {
    /// Whether the confirming button is the primary one, which Enter chooses: yes for opening
    /// something, no for what ends or removes something (there the safe choice, Cancel, is).
    pub fn ok_is_primary(&self) -> bool {
        matches!(self, Pending::Open { .. } | Pending::Copy { .. })
    }
}

#[derive(Clone, Debug)]
pub enum NameFor {
    Add,
    Rename(String),
}

pub enum Dialog {
    Confirm { title: String, text: String, ok: String, then: Pending },
    Accounts { scroll: f32, shares: Vec<(String, Vec<ShareState>)> },
    Name { title: String, field: TextField, stops: Vec<f32>, then: NameFor, error: Option<String> },
    NewSession(Box<newsession::NewSession>),
    Settings(Box<Form>),
    About,
}

pub fn accounts(st: &AppState) -> Dialog {
    let shares =
        st.config.accounts.iter().filter(|a| !a.primary).map(|a| (a.slug.clone(), st.source.share_status(a))).collect();
    Dialog::Accounts { scroll: 0.0, shares }
}

pub fn settings(st: &AppState) -> Dialog {
    let mut options = vec!["Same account as the session".to_string(), "The other account".to_string()];
    options.extend(st.config.accounts.iter().map(|a| format!("Always {}", a.name)));
    let value = match &st.config.workers {
        WorkersUse::Same => 0,
        WorkersUse::Other => 1,
        WorkersUse::Account(slug) => st.config.accounts.iter().position(|a| &a.slug == slug).map_or(0, |k| k + 2),
    };
    let refs: Vec<&str> = options.iter().map(String::as_str).collect();
    let themes = WINDOW_THEMES.map(|(_, name)| name);
    let theme = WINDOW_THEMES.iter().position(|(id, _)| *id == st.config.window_theme).unwrap_or(0);
    let scheme_names: Vec<_> = PRESETS.iter().map(|preset| preset.name).collect();
    let mut theme_row = Row::choice(SET_THEME, "Theme", &themes, theme);
    if st.forced_dark.is_some() {
        theme_row = theme_row.help("The --theme launch flag overrides this setting.");
    }
    let a = &st.config.appearance;
    let auto = termtheme::scheme("auto", st.pal());
    let accent = ACCENT_SOURCES.iter().position(|(id, _)| *id == a.accent_source).unwrap_or(1);
    let accent_names = ACCENT_SOURCES.map(|(_, label)| label);
    let colour = |id, label, saved: &Option<String>, fallback| {
        Row::text(id, label, termtheme::swatch(saved.as_deref(), fallback))
    };
    let rows = vec![
        Row::header("Appearance"),
        theme_row,
        Row::choice(
            SET_BACKDROP,
            Backdrop::TITLE,
            &Backdrop::LABELS,
            Backdrop::parse(&a.backdrop).unwrap_or_default().index(),
        )
        .help("Windows 11, for the title bar. Terminal text stays opaque."),
        Row::choice(SET_ACCENT, "Accent source", &accent_names, accent),
        colour(SET_ACCENT_COLOUR, "Window accent", &a.accent_colour, st.pal().accent)
            .help("Used when Accent source is Custom."),
        Row::toggle(SET_REDUCE_MOTION, "Reduce motion", a.reduce_motion)
            .help("Windows’ reduced-motion preference also applies."),
        Row::number(SET_CORNERS, "Dialog and menu corners", i32::from(a.corner_radius), 0, 16).unit("px"),
        Row::toggle(SET_SHADOWS, "Soft shadows", a.soft_shadows),
        Row::header("Terminal"),
        Row::choice(SET_TERM_COLOURS, "Terminal colours", &scheme_names, preset_index(&st.config.terminal_scheme)),
        Row::number(
            SET_TERM_FONT,
            "Terminal font size",
            i32::from(st.config.terminal_font_size),
            i32::from(TERMINAL_FONT_MIN),
            i32::from(TERMINAL_FONT_MAX),
        )
        .unit("px"),
        Row::toggle(SET_CURSOR_BLINK, "Blink terminal cursor", a.cursor_blink)
            .help("Follows the program and system when enabled."),
        Row::header("Custom colours"),
        colour(SET_TERM_BACKGROUND, "Terminal background", &a.terminal_background, auto.background)
            .help("Select Custom in Terminal colours."),
        colour(SET_TERM_FOREGROUND, "Terminal text", &a.terminal_foreground, auto.foreground)
            .help("Adjusted to at least 7:1 contrast."),
        colour(SET_TERM_ACCENT, "Terminal accent", &a.terminal_accent, st.pal().accent)
            .help("Tints the selection, cursor and active tab mark."),
        Row::header("Workers"),
        Row::choice(SET_WORKERS, "Account the bridge's workers use", &refs, value),
        Row::header("Sessions"),
        Row::toggle(SET_EMBEDDED, "Open CLI sessions in a tab of this window", st.config.embedded_terminal)
            .help("Off: they open in Windows Terminal, or a console window without it."),
        Row::toggle(SET_ARCHIVED, "Show archived sessions", st.config.show_archived),
        Row::toggle(SET_WORKERS_SHOWN, "Show sessions started by tools", st.config.show_workers)
            .help("Sessions that tools and subagents started through the SDK, not you."),
    ];
    let mut form = Form::new("Settings", rows);
    for id in [SET_ACCENT_COLOUR, SET_TERM_BACKGROUND, SET_TERM_FOREGROUND, SET_TERM_ACCENT] {
        form.show_as_colour(id);
    }
    Dialog::Settings(Box::new(form))
}

fn name_dialog(title: &str, text: &str, then: NameFor) -> Dialog {
    let mut field = TextField::new();
    field.set_text(text);
    field.select_all();
    Dialog::Name { title: title.into(), field, stops: Vec::new(), then, error: None }
}

// ---- drawing ---------------------------------------------------------------------------------

struct Draw<'a> {
    hits: &'a mut Vec<(Rect, Target)>,
    hover: Option<Target>,
    pressed: Option<Target>,
    pal: Palette,
    radius: f32,
    shadow: f32,
}

impl Draw<'_> {
    fn card(&self, p: &mut Painter, r: Rect) -> Rect {
        draw_dialog_card_with(p, &self.pal, r, self.radius, Some(self.shadow))
    }

    fn button(&mut self, p: &mut Painter, r: Rect, label: &str, primary: bool, id: u32) {
        let t = Some(Target::Dialog(id));
        draw_dialog_button(p, &self.pal, r, label, primary, self.hover == t, self.pressed == t);
        self.hits.push((r, Target::Dialog(id)));
    }

    /// Footer buttons from the right, the first given rightmost.
    fn footer(&mut self, p: &mut Painter, footer: Rect, buttons: &[(&str, bool, u32)]) {
        let y = footer.y + (DIALOG_FOOTER - 32.0) / 2.0;
        let mut right = footer.right() - DIALOG_PAD;
        for &(label, primary, id) in buttons {
            let w = dialog_button_w(p, label);
            self.button(p, Rect::new(right - w, y, w, 32.0), label, primary, id);
            right -= w + 8.0;
        }
    }

    fn title(&self, p: &mut Painter, card: Rect, text: &str) {
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
        let mut style = Style::CLASSIC;
        style.radius = look.radius;
        style.soft_shadow = Some(shadow);
        style.backdrop = t;
        form.set_style(style);
        form.set_high_contrast(st.high_contrast);
        form.draw_animated(p, st.pal(), window, true, cx.anim(), "settings");
        st.dialog = Some(dialog);
        return;
    }
    let pal = *st.pal();
    draw_backdrop_presence(p, &pal, w, h, t);
    let mut d = Draw { hits: &mut st.hits, hover: st.hover, pressed: st.pressed, pal, radius: look.radius, shadow };
    match &mut dialog {
        Dialog::Confirm { title, text, ok, then } => {
            let tw = 460.0 - 2.0 * DIALOG_PAD;
            let th = p.paragraph_height_wrapped(Face::Ui, 13.0, tw, text, Wrap::Words);
            let card = dialog_card(460.0, 70.0 + th + 20.0 + DIALOG_FOOTER, w, h);
            let footer = d.card(p, card);
            d.title(p, card, title);
            p.paragraph_wrapped(Face::Ui, 13.0, card.x + DIALOG_PAD, card.y + 58.0, tw, text, pal.text, Wrap::Words);
            let primary = then.ok_is_primary();
            d.footer(p, footer, &[(ok.as_str(), primary, OK), ("Cancel", !primary, CANCEL)]);
        }
        Dialog::Name { title, field, stops, error, .. } => {
            let card = dialog_card(420.0, 120.0 + DIALOG_FOOTER, w, h);
            let footer = d.card(p, card);
            d.title(p, card, title);
            let fr = Rect::new(card.x + DIALOG_PAD, card.y + 58.0, card.w - 2.0 * DIALOG_PAD, 32.0);
            p.frame(fr, 5.0, pal.field, pal.accent);
            let inner = fr.inset(8.0);
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            field.draw(p, inner, Face::Ui, 13.0, &colors, "Account name", true, true);
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
            draw_accounts((&st.config, &st.statuses), &mut d, p, w, h, scroll, shares)
        }
        Dialog::NewSession(picker) => picker.draw(&mut d, p, w, h),
        Dialog::Settings(_) => {}
    }
    st.dialog = Some(dialog);
}

type StView<'a> =
    (&'a claude_switch_core::Config, &'a std::collections::HashMap<String, claude_switch_core::AccountStatus>);

const BLOCK: f32 = 96.0;
const SHARE_BLOCK: f32 = 58.0;

fn draw_accounts(
    (config, statuses): StView<'_>,
    d: &mut Draw<'_>,
    p: &mut Painter,
    w: f32,
    h: f32,
    scroll: &mut f32,
    shares: &[(String, Vec<ShareState>)],
) {
    let pal = d.pal;
    let content: f32 =
        config.accounts.iter().map(|a| if a.primary { BLOCK } else { BLOCK + SHARE_BLOCK }).sum::<f32>() + 8.0;
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
                parts.extend(s.plan.clone());
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
            (OK, Pending::Quit) => st.quit = true,
            (OK, Pending::Remove { slug }) => {
                match st.source.remove(&mut st.config, &slug) {
                    Ok(()) => accounts_changed(st, now),
                    Err(e) => st.error("Not removed", e.to_string()),
                }
                st.dialog = Some(accounts(st));
            }
            (CANCEL, Pending::Remove { .. }) => st.dialog = Some(accounts(st)),
            (CANCEL, _) => {}
            (_, then) => st.dialog = Some(Dialog::Confirm { title, text, ok, then }),
        },
        Dialog::Name { mut field, stops, then, title, error } => match id {
            OK => submit_name(st, now, field, stops, then, title),
            CANCEL => st.dialog = Some(accounts(st)),
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
            st.dialog = Some(accounts(st));
        }
        _ => {}
    }
}

fn submit_name(st: &mut AppState, now: Instant, field: TextField, stops: Vec<f32>, then: NameFor, title: String) {
    let name = field.text().trim().to_string();
    let result = match &then {
        NameFor::Add => st.source.add_account(&mut st.config, &name).map(drop),
        NameFor::Rename(slug) => st.source.rename(&mut st.config, slug, &name),
    };
    match result {
        Ok(()) => {
            accounts_changed(st, now);
            st.dialog = Some(accounts(st));
        }
        Err(e) => st.dialog = Some(Dialog::Name { title, field, stops, then, error: Some(e.to_string()) }),
    }
}

/// After an account was added, renamed or removed: save, tell the bridge, read again.
fn accounts_changed(st: &mut AppState, now: Instant) {
    st.save_config();
    if let Err(e) = st.source.write_workers(&st.config) {
        st.error("The workers' list was not updated", e.to_string());
    }
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
            settings_outcome(st, o);
            return true;
        }
        Dialog::Name { field, .. } => {
            if field.handle_key(k, mods, text, true).is_some() {
                st.dialog = Some(dialog);
                return true;
            }
        }
        Dialog::NewSession(picker) => picker.key(k, mods, text),
        _ => {}
    }
    st.dialog = Some(dialog);
    match k {
        KeyName::Escape => click(st, now, CANCEL, 0.0),
        // Enter chooses the button drawn as primary.
        KeyName::Enter => match &st.dialog {
            Some(Dialog::Accounts { .. }) | Some(Dialog::About) => click(st, now, CANCEL, 0.0),
            Some(Dialog::Confirm { then, .. }) if !then.ok_is_primary() => click(st, now, CANCEL, 0.0),
            _ => click(st, now, OK, 0.0),
        },
        _ => {}
    }
    true
}

pub fn text_input(st: &mut AppState, text: &str) {
    match &mut st.dialog {
        Some(Dialog::Name { field, .. }) => {
            field.insert(text);
        }
        Some(Dialog::NewSession(picker)) => picker.text_input(text),
        _ => {}
    }
}

pub fn wheel(st: &mut AppState, x: f32, y: f32, dy: f32) {
    match &mut st.dialog {
        Some(Dialog::Settings(form)) => {
            form.wheel(x, y, -dy);
        }
        Some(Dialog::Accounts { scroll, .. }) => *scroll = (*scroll - dy).max(0.0),
        _ => {}
    }
}

/// What a settings form's event gave.
pub fn settings_outcome(st: &mut AppState, o: Outcome) {
    match o {
        Outcome::Closed => st.dialog = None,
        Outcome::Changed { id, value } => {
            match (id, value) {
                (SET_WORKERS, Value::Choice(i)) => {
                    st.config.workers = match i {
                        0 => WorkersUse::Same,
                        1 => WorkersUse::Other,
                        k => st
                            .config
                            .accounts
                            .get(k - 2)
                            .map_or(WorkersUse::Same, |a| WorkersUse::Account(a.slug.clone())),
                    };
                    if let Err(e) = st.source.write_workers(&st.config) {
                        st.error("The workers' setting was not written", e.to_string());
                    }
                }
                (SET_EMBEDDED, Value::Bool(b)) => st.config.embedded_terminal = b,
                (SET_ARCHIVED, Value::Bool(b)) => {
                    st.config.show_archived = b;
                    st.rebuild_view();
                }
                (SET_WORKERS_SHOWN, Value::Bool(b)) => {
                    st.config.show_workers = b;
                    st.rebuild_view();
                }
                (SET_THEME, Value::Choice(i)) if i < WINDOW_THEMES.len() => {
                    st.config.window_theme = WINDOW_THEMES[i].0.into();
                }
                (SET_TERM_COLOURS, Value::Choice(i)) if i < PRESETS.len() => {
                    st.config.terminal_scheme = PRESETS[i].id.into();
                }
                (SET_TERM_FONT, Value::Number(n)) => {
                    st.config.terminal_font_size =
                        n.clamp(i32::from(TERMINAL_FONT_MIN), i32::from(TERMINAL_FONT_MAX)) as u8;
                }
                (SET_BACKDROP, Value::Choice(i)) => {
                    st.config.appearance.backdrop = Backdrop::from_index(i).key().into()
                }
                (SET_ACCENT, Value::Choice(i)) if i < ACCENT_SOURCES.len() => {
                    if ACCENT_SOURCES[i].0 == "custom" && st.config.appearance.accent_colour.is_none() {
                        st.config.appearance.accent_colour = Some(termtheme::hex(st.pal().accent));
                    }
                    st.config.appearance.accent_source = ACCENT_SOURCES[i].0.into();
                }
                (SET_REDUCE_MOTION, Value::Bool(b)) => st.config.appearance.reduce_motion = b,
                (SET_SHADOWS, Value::Bool(b)) => st.config.appearance.soft_shadows = b,
                (SET_CURSOR_BLINK, Value::Bool(b)) => st.config.appearance.cursor_blink = b,
                (SET_CORNERS, Value::Number(n)) => st.config.appearance.corner_radius = n.clamp(0, 16) as u8,
                (id, Value::Text(text)) => {
                    let a = &mut st.config.appearance;
                    let field = match id {
                        SET_ACCENT_COLOUR => &mut a.accent_colour,
                        SET_TERM_BACKGROUND => &mut a.terminal_background,
                        SET_TERM_FOREGROUND => &mut a.terminal_foreground,
                        SET_TERM_ACCENT => &mut a.terminal_accent,
                        _ => return,
                    };
                    let Some(colour) = parse_hex(&text) else { return };
                    *field = Some(termtheme::hex(colour));
                }
                _ => return,
            }
            st.retheme();
            st.save_config();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, term::Term, winfile::WindowFile};

    #[test]
    fn appearance_changes_apply_to_hidden_tabs_and_the_flag_still_wins() {
        for forced in [None, Some(true)] {
            let mut st = AppState::new(Source::Demo, forced, WindowFile::default());
            settings_outcome(&mut st, Outcome::Changed { id: SET_THEME, value: Value::Choice(1) });
            assert_eq!(st.config.window_theme, "light");
            assert_eq!(st.pal().dark, forced.unwrap_or(false));
            settings_outcome(
                &mut st,
                Outcome::Changed { id: SET_TERM_COLOURS, value: Value::Choice(preset_index("black")) },
            );
            assert_eq!(st.config.terminal_scheme, "black");

            let ids = ["a", "b"].map(|label| {
                let new =
                    crate::tabs::NewTab { label: label.into(), color: st.pal().muted, session: None, account: None };
                st.tabs.open(new, Term::empty(st.grid))
            });
            st.show_sessions();
            let area = Rect::new(0.0, 80.0, 760.0, 400.0);
            st.layout_terminals(area, area, 1.0);
            let before = st.grid;
            settings_outcome(&mut st, Outcome::Changed { id: SET_TERM_FONT, value: Value::Number(24) });
            st.layout_terminals(area, area, 1.0);
            assert!(st.grid.0 < before.0 && st.grid.1 < before.1);
            for id in ids {
                let screen = &st.tabs.get(id).unwrap().term.screen;
                assert_eq!((screen.cols(), screen.rows()), (usize::from(st.grid.0), usize::from(st.grid.1)));
            }
        }
    }
}
