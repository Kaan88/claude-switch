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
    DIALOG_FOOTER, DIALOG_PAD, dialog_button_w, dialog_card, draw_dialog_button, draw_dialog_card_with,
};
use mdshared_ui::textfield::{FieldColors, FieldHit, TextField};
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
pub(crate) use settings::peek::{PEEK_TIP, PeekInput};
pub(crate) use settings::{
    CcPage, CcRead, OPENCODE, TERMINAL, refresh_sessions_page, settings, settings_event, take_claude_code_jobs,
};

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
const ACT_MASTER: u32 = 6;
const ACT_SHARE: u32 = 7;

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
        /// Delete the original once the copy is written (`delete`).
        delete: bool,
    },
    /// Delete a session (to the Recycle Bin).
    Delete {
        id: String,
    },
    Remove {
        slug: String,
    },
    /// Close a terminal tab whose program runs.
    CloseTab(u64),
    /// A pane of a tab: (tab, pane).
    ClosePane(u64, crate::panes::PaneId),
    /// End a bridge job's worker process tree (the Workers tab).
    CancelWorker(String),
    CloseOtherTabs(u64),
    RestoreTab,
    ChatSlash {
        tab: u64,
        session: Option<String>,
        line: String,
        from_editor: bool,
    },
    /// Close the window, with terminal tabs that run.
    Quit,
    Suggest {
        ids: Vec<String>,
        batch: bool,
    },
    /// Summarize sessions for search by meaning.
    MeaningIndex,
    /// Ask a question over the kept summaries.
    MeaningAsk {
        question: String,
    },
    UnarchiveAll,
    RestartOpenCode(crate::opencode_workers::Restart),
    ManageShell {
        tab: u64,
        generation: u64,
        tool: claude_switch_core::transcript::ItemId,
    },
}

impl Pending {
    /// Whether the confirming button is the primary one, which Enter chooses: yes for opening
    /// something, no for what ends or removes something (there the safe choice, Cancel, is).
    pub fn ok_is_primary(&self) -> bool {
        matches!(self, Pending::Open { .. } | Pending::Copy { delete: false, .. } | Pending::RestoreTab)
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
    Name { title: String, field: TextField, hit: Option<FieldHit>, then: NameFor, error: Option<String> },
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

/// The New session picker for a folder only, starting an OpenCode tab.
pub(crate) fn new_opencode_dialog(st: &AppState) -> Dialog {
    Dialog::NewSession(Box::new(newsession::NewSession::for_opencode(st)))
}

pub(crate) fn name_dialog(title: &str, text: &str, then: NameFor) -> Dialog {
    let mut field = TextField::new();
    field.set_text(text);
    field.select_all();
    Dialog::Name { title: title.into(), field, hit: None, then, error: None }
}

// ---- drawing ---------------------------------------------------------------------------------

#[cfg(test)]
impl<'a> Draw<'a> {
    pub(crate) fn for_test(hits: &'a mut Vec<(Rect, Target)>, pal: Palette) -> Self {
        Draw { hits, hover: None, pressed: None, pal, radius: 9.0, shadow: 0.0 }
    }
}

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

    /// Footer buttons from the right, the first given rightmost. Buttons that would not fit
    /// between the card's padded edges narrow toward their labels' own width (see [`fit`]).
    pub fn footer(&mut self, p: &mut Painter, footer: Rect, buttons: &[(&str, bool, u32)]) {
        self.footer_split(p, footer, buttons, &[], 0);
    }

    /// A footer with a group on the left too: `right` from the right edge as [`Draw::footer`],
    /// `left` from the left edge in the order given. When both groups do not fit side by side,
    /// the left group folds into one “More ▾” button with the id `more`, and true is returned
    /// so the caller can offer the folded actions in a menu. Returns the rectangle of the
    /// “More” button when it was drawn.
    pub fn footer_split(
        &mut self,
        p: &mut Painter,
        footer: Rect,
        right: &[(&str, bool, u32)],
        left: &[(&str, bool, u32)],
        more: u32,
    ) -> Option<Rect> {
        let layout = footer_layout(p, footer, right, left);
        let y = footer.y + (DIALOG_FOOTER - 32.0) / 2.0;
        for (&(label, primary, id), (x, w)) in right.iter().zip(&layout.right) {
            self.button(p, Rect::new(*x, y, *w, 32.0), label, primary, id);
        }
        if let Some((x, w)) = layout.more {
            let r = Rect::new(x, y, w, 32.0);
            self.button(p, r, MORE_LABEL, false, more);
            return Some(r);
        }
        for (&(label, primary, id), (x, w)) in left.iter().zip(&layout.left) {
            self.button(p, Rect::new(*x, y, *w, 32.0), label, primary, id);
        }
        None
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

/// The label of the button a footer's left group folds into.
pub(crate) const MORE_LABEL: &str = "More  ▾";
const BUTTON_GAP: f32 = 8.0;
/// Between a footer's left and right groups, at least.
const GROUP_GAP: f32 = 24.0;

/// Where a footer's buttons go: (x, width) for each, and the folded “More” button if any.
#[derive(Debug, Default)]
pub(crate) struct FooterLayout {
    pub right: Vec<(f32, f32)>,
    pub left: Vec<(f32, f32)>,
    pub more: Option<(f32, f32)>,
}

pub(crate) fn footer_layout(
    p: &mut Painter,
    footer: Rect,
    right: &[(&str, bool, u32)],
    left: &[(&str, bool, u32)],
) -> FooterLayout {
    let sizes = |p: &mut Painter, labels: &mut dyn Iterator<Item = &str>| -> Vec<(f32, f32)> {
        labels
            .map(|label| {
                let natural = dialog_button_w(p, label);
                (natural, (p.measure(Face::Semibold, 13.0, label) + 20.0).ceil().min(natural))
            })
            .collect()
    };
    let (start, end) = (footer.x + DIALOG_PAD, footer.right() - DIALOG_PAD);
    let right_sizes = sizes(p, &mut right.iter().map(|b| b.0));
    let mut left_sizes = sizes(p, &mut left.iter().map(|b| b.0));
    let natural = |sizes: &[(f32, f32)]| {
        sizes.iter().map(|s| s.0).sum::<f32>() + BUTTON_GAP * sizes.len().saturating_sub(1) as f32
    };
    let mut folded = false;
    if !left_sizes.is_empty() && natural(&right_sizes) + GROUP_GAP + natural(&left_sizes) > end - start {
        left_sizes = sizes(p, &mut std::iter::once(MORE_LABEL));
        folded = true;
    }
    let group = if left_sizes.is_empty() { 0.0 } else { GROUP_GAP - BUTTON_GAP };
    let all: Vec<_> = right_sizes.iter().chain(&left_sizes).copied().collect();
    let widths = fit(&all, end - start - group, BUTTON_GAP);
    let (right_w, left_w) = widths.split_at(right_sizes.len());
    let mut layout = FooterLayout::default();
    let mut x = end;
    for &w in right_w {
        layout.right.push((x - w, w));
        x -= w + BUTTON_GAP;
    }
    let mut x = start;
    for &w in left_w {
        if folded {
            layout.more = Some((x, w));
        } else {
            layout.left.push((x, w));
        }
        x += w + BUTTON_GAP;
    }
    layout
}

/// Button widths for a row `room` wide: each its natural width when they all fit, otherwise
/// all narrowed by the same share of what each can give (down to its least width).
pub(crate) fn fit(sizes: &[(f32, f32)], room: f32, gap: f32) -> Vec<f32> {
    let gaps = gap * sizes.len().saturating_sub(1) as f32;
    let natural: f32 = sizes.iter().map(|s| s.0).sum();
    let least: f32 = sizes.iter().map(|s| s.1).sum();
    let give = natural - least;
    let t = if natural + gaps <= room || give <= 0.0 { 1.0 } else { ((room - gaps - least) / give).clamp(0.0, 1.0) };
    sizes.iter().map(|&(n, l)| (l + t * (n - l)).floor()).collect()
}

pub fn draw(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter, window: Rect, t: f32) {
    let now = cx.now();
    if draw_with(st, cx.anim(), now, p, window, t) {
        cx.animating(window);
    }
}

/// [`draw`] without a window, at `now`: true while Settings is in motion.
pub fn draw_with(
    st: &mut AppState,
    anim: &mut mdshared_ui::anim::Animator,
    now: std::time::Instant,
    p: &mut Painter,
    window: Rect,
    t: f32,
) -> bool {
    let (w, h) = (window.w, window.h);
    let Some(mut dialog) = st.dialog.take() else { return false };
    let look = st.appearance;
    let cards = crate::windowlook::Cards::for_frame(st, p);
    let shadow = if st.high_contrast { 0.0 } else { cards.decoration(look.shadow) };
    if let Dialog::Settings(form) = &mut dialog {
        let moving = form.draw_layer(p, st, window, t, anim);
        st.dialog = Some(dialog);
        return moving;
    }
    let pal = *st.pal();
    cards.dim(p, &pal, window, t);
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
                    refresh: !demo && usage.can_refresh(&a.slug, now),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut d = Draw {
        hits: &mut st.hits,
        hover: st.hover,
        pressed: st.pressed,
        pal,
        radius: cards.decoration(look.radius),
        shadow,
    };
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
        Dialog::Name { title, field, hit, error, then } => {
            let card = dialog_card(420.0, 120.0 + DIALOG_FOOTER, w, h);
            let footer = d.card(p, card);
            d.title(p, card, title);
            let fr = Rect::new(card.x + DIALOG_PAD, card.y + 58.0, card.w - 2.0 * DIALOG_PAD, 32.0);
            p.frame(fr, 5.0, pal.field, pal.accent);
            let inner = fr.inset(8.0);
            let colors =
                FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
            let placeholder = match then {
                NameFor::Tab(_) => "Tab name (blank resets it)",
                _ => "Account name",
            };
            field.draw(p, inner, Face::Ui, 13.0, &colors, placeholder, true, true);
            *hit = crate::fieldmouse::hit(p, inner, 13.0);
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
    false
}

type StView<'a> = (
    &'a claude_switch_core::Config,
    &'a std::collections::HashMap<String, claude_switch_core::AccountStatus>,
    &'a [AccountUsage<'a>],
);

const BLOCK: f32 = 96.0;
const SHARE_BLOCK: f32 = 58.0;
/// The row about Claude Code's settings, under each account when there are several.
const SETTINGS_BLOCK: f32 = 40.0;

fn account_usage_layout(body: Rect, y: f32, h: f32) -> (Rect, Rect) {
    let left = body.x + DIALOG_PAD - 1.0 + 18.0;
    let right = body.right() - DIALOG_PAD - 7.0;
    let usage = Rect::new(left, y, (right - left).max(0.0), h);
    let width = 86.0_f32.min(usage.w);
    let refresh = Rect::new(right - width, y + 6.0, width, 28.0);
    (usage, refresh)
}

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
    let settings_row = if config.accounts.len() > 1 { SETTINGS_BLOCK } else { 0.0 };
    let content: f32 =
        config.accounts.iter().map(|a| if a.primary { BLOCK } else { BLOCK + SHARE_BLOCK } + settings_row).sum::<f32>()
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
            let (area, button) = account_usage_layout(body, y - 6.0, u.height());
            u.draw(p, &pal, area, button, now);
            if button.y >= body.y && button.bottom() <= body.bottom() {
                if u.refresh {
                    d.button(p, button, "Refresh", false, base + ACT_REFRESH_USAGE);
                } else {
                    mdshared_ui::overlay::draw_dialog_button_disabled(p, &pal, button, "Refresh", false);
                }
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
        if config.accounts.len() > 1 {
            let share = &config.settings_share;
            let master = settings_master(config);
            let text = if a.slug == master {
                "Claude Code settings: the master. A change in any sharing account goes to the others."
            } else if share.shares(&a.slug) {
                "Claude Code settings: shared with the other accounts."
            } else {
                "Claude Code settings: this account's own."
            };
            let colour = if share.shares(&a.slug) || a.slug == master { pal.text } else { pal.muted };
            let mut bx = x + tw;
            if a.slug != master {
                for (label, act) in
                    [(if share.shares(&a.slug) { "Keep own" } else { "Share" }, ACT_SHARE), ("Make master", ACT_MASTER)]
                {
                    let bw = (p.measure(Face::Ui, 13.0, label) + 24.0).round();
                    bx -= bw;
                    let r = Rect::new(bx, y - 8.0, bw, 28.0);
                    if r.y >= body.y && r.bottom() <= body.bottom() {
                        d.button(p, r, label, false, base + act);
                    }
                    bx -= 8.0;
                }
            }
            p.text(Face::Ui, 12.0, x + 18.0, y + 6.0, text, colour, bx - x - 26.0);
            y += SETTINGS_BLOCK;
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
            (OK, Pending::Copy { id, account, delete }) => st.copy_now_then(now, &id, &account, delete),
            (OK, Pending::Delete { id }) => st.delete_confirmed(now, &id),
            (OK, Pending::CloseTab(tab)) => st.close_tab_now(tab),
            (OK, Pending::ClosePane(tab, pane)) => st.close_pane_now(tab, pane),
            (OK, Pending::CancelWorker(id)) => st.cancel_worker(&id),
            (OK, Pending::CloseOtherTabs(tab)) => st.close_other_tabs(tab),
            (OK | CANCEL, Pending::RestoreTab) => st.confirm_restore_tab(now, id == OK),
            (OK, Pending::ChatSlash { tab, session, line, from_editor }) => {
                st.chat_confirm_slash(now, tab, session, &line, from_editor);
            }
            (OK, Pending::ManageShell { tab, generation, tool }) => st.confirm_shell_manager(tab, generation, tool),
            (OK, Pending::Quit) => st.quit = true,
            (OK, Pending::Suggest { ids, batch }) => st.start_naming(ids, batch),
            (OK, Pending::MeaningIndex) => st.start_meaning_index(),
            (OK, Pending::MeaningAsk { question }) => st.start_meaning_question(question),
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
        Dialog::Name { field, hit, then, title, error } => match id {
            OK => submit_name(st, now, field, hit, then, title),
            CANCEL if matches!(then, NameFor::Tab(_)) => {}
            CANCEL => st.dialog = Some(accounts(st, now)),
            _ => st.dialog = Some(Dialog::Name { title, field, hit, then, error }),
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
            if picker.click(st, now, id) {
                st.dialog = Some(Dialog::NewSession(picker));
            }
        }
        Dialog::FolderOpen(editor) => editor.click(st, id),
        Dialog::Archive(archive) => archive.click(st, id),
        Dialog::ArchiveReport(report) => report.click(st, id),
        Dialog::Session(panel) => panel.click(st, now, id, false),
        Dialog::Compact(mut compact) => {
            if compact.click(st, now, id, x) {
                st.dialog = Some(Dialog::Compact(compact));
            } else {
                st.compact_closed(compact, now);
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
        ACT_MASTER | ACT_SHARE => {
            let share = &mut st.config.settings_share;
            if act == ACT_MASTER {
                share.master = a.slug.clone();
                share.off.retain(|s| *s != a.slug);
            } else if share.shares(&a.slug) {
                share.off.push(a.slug.clone());
            } else {
                share.off.retain(|s| *s != a.slug);
            }
            st.save_config();
            match st.source.settings_sync(&st.config) {
                Ok(report) if report.settings.is_some() || !report.prefs.is_empty() => {
                    st.toast("Claude Code settings brought in line; the replaced files were backed up".into(), now)
                }
                Ok(_) => {}
                Err(e) => st.error("Not every account's settings were brought in line", e.to_string()),
            }
            st.dialog = Some(accounts(st, now));
        }
        ACT_REFRESH_USAGE => {
            st.refresh(now);
            st.request_usage(now, Demand::Refresh, Some(&a.slug));
        }
        _ => {}
    }
}

fn submit_name(st: &mut AppState, now: Instant, field: TextField, hit: Option<FieldHit>, then: NameFor, title: String) {
    let name = field.text().trim().to_string();
    let result = match &then {
        NameFor::Add => st.source.add_account(&mut st.config, &name).map(drop),
        NameFor::Rename(slug) => st.source.rename(&mut st.config, slug, &name),
        NameFor::Tab(id) => {
            st.rename_tab(now, *id, &name);
            return;
        }
    };
    match result {
        Ok(()) => {
            accounts_changed(st, now);
            st.dialog = Some(accounts(st, now));
        }
        Err(e) => st.dialog = Some(Dialog::Name { title, field, hit, then, error: Some(e.to_string()) }),
    }
}

/// The account whose Claude Code settings win a tie: the chosen one, or the first account.
fn settings_master(config: &claude_switch_core::Config) -> &str {
    let chosen = &config.settings_share.master;
    if config.accounts.iter().any(|a| &a.slug == chosen) { chosen } else { "main" }
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
            let o = form.key(&st.config, k, mods, text);
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
        Dialog::NewSession(picker) => {
            if picker.key(st, k, mods, text) {
                st.dialog = Some(dialog);
                return true;
            }
        }
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
                panel.click(st, now, id, mods.ctrl);
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

#[cfg(test)]
#[path = "dialogs/accounts_tests.rs"]
mod accounts_tests;
