//! The title row, which is also the tool bar: the window's name, the account drop-down, the
//! search field, New session, Refresh, the menu, and the caption buttons. [`Bar::layout`] is the
//! one place the rectangles come from, for drawing and for the pointer alike.

use mdshared_ui::chrome::{self, TITLE_H, WIN_BTN_W, WinButton};
use mdshared_ui::dropdown::DROPDOWN_H;
use mdshared_ui::focus::draw_ring;
use mdshared_ui::gfx::{Face, Painter, Rect, mix};
use mdshared_ui::glyph;
use mdshared_ui::infobar::{Hit, draw_infobar};
use mdshared_ui::textfield::FieldColors;
use mdshared_ui::window;

use crate::state::{AppState, Focus, Msg, Target};
use crate::tabs;
use crate::usage_ui;

pub struct Bar {
    pub row: Rect,
    pub dot: Rect,
    pub account: Rect,
    pub search: Rect,
    pub clear: Rect,
    pub search_mode: Rect,
    pub new_session: Rect,
    pub refresh: Rect,
    pub menu: Rect,
}

const NEW_W: f32 = 118.0;
const BRAND_W: f32 = 124.0;
pub const RESET_SETTINGS: u32 = 90;
const LOGO: &[u8] = include_bytes!("../../../assets/logo-256.png");

impl Bar {
    pub fn layout(w: f32) -> Bar {
        let y = ((TITLE_H - DROPDOWN_H) / 2.0).round();
        let row = Rect::new(0.0, 0.0, w, TITLE_H);
        let right_free = w - 3.0 * WIN_BTN_W - 16.0;
        let tools_w = NEW_W + 8.0 + 32.0 + 4.0 + 32.0;
        let dot = Rect::new(BRAND_W, (TITLE_H - 10.0) / 2.0, 10.0, 10.0);
        let account_x = dot.x + 18.0;
        let mode_w = if w < 890.0 { 34.0 } else { 112.0 };
        let mode_space = mode_w + 4.0;
        let account_w = (right_free - tools_w - 24.0 - 102.0 - mode_space - account_x).clamp(100.0, 200.0);
        let account = Rect::new(account_x, y, account_w, DROPDOWN_H);
        let search_x = account.right() + 12.0;
        let search_w = (right_free - tools_w - 12.0 - search_x - mode_space).clamp(102.0, 420.0);
        let search = Rect::new(search_x, y, search_w, DROPDOWN_H);
        let clear = Rect::new(search.right() - 26.0, y + 2.0, 24.0, DROPDOWN_H - 4.0);
        let search_mode = Rect::new(search.right() + 4.0, y, mode_w, DROPDOWN_H);
        let new_session = Rect::new(search_mode.right() + 12.0, y, NEW_W, DROPDOWN_H);
        let refresh = Rect::new(new_session.right() + 8.0, y, 32.0, DROPDOWN_H);
        let menu = Rect::new(refresh.right() + 4.0, y, 32.0, DROPDOWN_H);
        Bar { row, dot, account, search, clear, search_mode, new_session, refresh, menu }
    }

    /// The search field's text area.
    pub fn search_inner(&self) -> Rect {
        Rect::new(self.search.x + 30.0, self.search.y, self.search.w - 58.0, self.search.h)
    }
}

pub fn caption(w: f32, b: WinButton) -> Rect {
    let n = match b {
        WinButton::Close => 0,
        WinButton::Max => 1,
        WinButton::Min => 2,
    };
    chrome::win_button_rect(w, n)
}

pub fn draw(st: &mut AppState, p: &mut Painter, w: f32, maximized: bool, focused: bool, caret_on: bool) -> Rect {
    let pal = *st.pal();
    let bar = Bar::layout(w);
    p.fill(bar.row, pal.title);
    p.fill(Rect::new(0.0, TITLE_H - p.px(), w, p.px()), pal.line);
    st.hits.push((bar.row, Target::Title));
    static ICON: std::sync::OnceLock<Option<mdshared_text::image::Image>> = std::sync::OnceLock::new();
    if let Some(im) =
        ICON.get_or_init(|| mdshared_text::image::decode(LOGO, mdshared_text::image::Limits::DEFAULT).ok())
    {
        p.image(&im.rgba, im.width, im.height, Rect::new(10.0, (TITLE_H - 18.0) / 2.0, 18.0, 18.0), 0.0, 1.0);
    }
    p.text(Face::Semibold, 13.0, 34.0, TITLE_H / 2.0, "Claude Switch", pal.text, BRAND_W - 39.0);

    // The account: its colour, then the drop-down.
    let current = st.current().clone();
    p.rbox(bar.dot, 5.0, current.color);
    let r = bar.account;
    let hot = matches!(st.hover, Some(Target::Account | Target::UsageLines)) || st.accounts_dd.is_open();
    if st.key_used && st.focus == Focus::Account {
        draw_ring(p, &pal, r, 6.0);
    }
    let fill = if hot { mix(pal.field, pal.text, 0.05) } else { pal.field };
    let edge = if hot { mix(pal.line, pal.text, 0.18) } else { pal.line };
    p.frame(r, 6.0, fill, edge);
    let reading = if st.config.show_usage { st.usage.reading(&current.slug) } else { None };
    let limits = usage_ui::hairlines(reading);
    let slot = usage_ui::hairline_rect(r);
    let name_w = if limits.is_empty() { r.w - 36.0 } else { slot.x - r.x - 15.0 };
    p.text(Face::Ui, 13.0, r.x + 11.0, r.cy(), &current.name, pal.text, name_w);
    p.icon(glyph::DOWN, 10.0, r.right() - 15.0, r.cy(), pal.muted);
    st.hits.push((r, Target::Account));
    if !limits.is_empty() {
        usage_ui::draw_hairlines(p, &pal, slot, &limits);
        st.hits.push((slot, Target::UsageLines));
    }

    // The search field.
    let active = st.focus == Focus::Search;
    p.frame(bar.search, 5.0, pal.field, if active { pal.accent } else { pal.line });
    p.icon(glyph::SEARCH, 14.0, bar.search.x + 15.0, bar.search.cy(), pal.muted);
    let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
    let inner = bar.search_inner();
    let placeholder = if bar.search.w < 165.0 { "Search" } else { "Search sessions" };
    st.search.draw(p, inner, Face::Ui, 13.0, &colors, placeholder, active && focused, caret_on);
    st.search_hit = crate::fieldmouse::hit(p, inner, 13.0);
    st.hits.push((bar.search, Target::Search));
    if !st.search.is_empty() {
        icon_button(st, p, bar.clear, glyph::CLOSE, Target::SearchClear);
    }
    if st.key_used && active {
        draw_ring(p, &pal, bar.search, 5.0);
    }
    crate::conversation_draw::toggle(st, p, bar.search_mode);

    // New session, Refresh, the menu.
    let fill = chrome::button_fill(
        &pal,
        st.hover == Some(Target::NewSession),
        st.pressed == Some(Target::NewSession),
        false,
        false,
    );
    p.frame(bar.new_session, 5.0, fill.unwrap_or(pal.field), pal.line);
    p.icon(glyph::PLUS, 14.0, bar.new_session.x + 16.0, bar.new_session.cy(), pal.text);
    p.text(Face::Ui, 13.0, bar.new_session.x + 30.0, bar.new_session.cy(), "New session", pal.text, NEW_W - 34.0);
    st.hits.push((bar.new_session, Target::NewSession));
    let scanning = st.scan_job.is_some();
    icon_button(st, p, bar.refresh, glyph::RELOAD, Target::Refresh);
    if scanning {
        p.rbox(Rect::new(bar.refresh.right() - 7.0, bar.refresh.y + 3.0, 4.0, 4.0), 2.0, pal.accent);
    }
    let open = st.menu.as_ref().is_some_and(|(k, _)| *k == crate::state::MenuKind::Main);
    let fill = chrome::button_fill(&pal, st.hover == Some(Target::MenuButton), false, false, open);
    if let Some(f) = fill {
        p.rbox(bar.menu, 5.0, f);
    }
    p.icon(glyph::MORE, 16.0, bar.menu.cx(), bar.menu.cy(), pal.text);
    st.hits.push((bar.menu, Target::MenuButton));

    // The caption buttons, with snap layouts over Maximize.
    for b in [WinButton::Min, WinButton::Max, WinButton::Close] {
        let t = Some(Target::Caption(b));
        let r = caption(w, b);
        chrome::draw_win_button(p, &pal, b, r, maximized, st.hover == t, st.pressed == t);
        st.hits.push((r, Target::Caption(b)));
    }
    let mut max = caption(w, WinButton::Max);
    if !maximized {
        max.y += window::EDGE;
        max.h -= window::EDGE;
    }
    max
}

fn icon_button(st: &mut AppState, p: &mut Painter, r: Rect, icon: &str, t: Target) {
    let pal = *st.pal();
    if let Some(f) = chrome::button_fill(&pal, st.hover == Some(t), st.pressed == Some(t), false, false) {
        p.rbox(r, 5.0, f);
    }
    p.icon(icon, 15.0, r.cx(), r.cy(), pal.text);
    st.hits.push((r, t));
}

/// Draw the optional tab strip and info bar; return the top of the content below them.
pub fn draw_below_title(st: &mut AppState, cx: &mut mdshared_ui::host::Cx<'_, Msg>, p: &mut Painter, w: f32) -> f32 {
    let (top, strip) = draw_below_title_with(st, cx.anim(), p, w);
    if let Some(strip) = strip {
        cx.animating(strip);
    }
    top
}

/// [`draw_below_title`] without a window: also the tab strip when it is in motion.
pub fn draw_below_title_with(
    st: &mut AppState,
    anim: &mut mdshared_ui::anim::Animator,
    p: &mut Painter,
    w: f32,
) -> (f32, Option<Rect>) {
    let pal = *st.pal();
    let mut moving = None;
    let terminal = st.terminal_shown().then(|| st.terminal_scheme());
    let mut top = TITLE_H;
    if !st.tabs.is_empty() {
        let look = tabs::Look {
            hover: st.hover,
            pressed: st.pressed,
            terminal: terminal.map(|s| (s.background, s.foreground, st.term_colours.1)),
            session_view: st
                .tabs
                .shown()
                .and_then(|t| t.pane.as_ref())
                .filter(|pane| !pane.chat.enabled)
                .map(|_| st.session_visible(w)),
            chat: st.tabs.shown().and_then(|t| t.pane.as_ref()).map(|pane| pane.chat.enabled),
        };
        let strip = tabs::strip_rect(top, w);
        if tabs::draw(p, &pal, anim, (&st.tabs, &st.marks), strip, &look, &mut st.hits) {
            moving = Some(strip);
        }
        top += tabs::STRIP_H;
    }
    let recovery = st.config_damaged.then(|| {
        mdshared_ui::infobar::InfoBar::new(
            mdshared_ui::infobar::Severity::Warning,
            "Settings could not be read",
            "Usage is off. Reset settings to keep a backup and start over.",
        )
        .closable(false)
        .action(RESET_SETTINGS, "Reset settings")
    });
    let pending = st.desktop_job.is_some().then(|| {
        mdshared_ui::infobar::InfoBar::new(
            mdshared_ui::infobar::Severity::Info,
            "Desktop handoff pending",
            "Waiting for Claude Code to finish the handoff.",
        )
        .closable(false)
    });
    for bar in [recovery.as_ref(), pending.as_ref().or(st.info.as_ref())].into_iter().flatten() {
        let r = Rect::new(8.0, top + 8.0, w - 16.0, bar.height(p, w - 16.0));
        if let Some(s) = terminal {
            p.fill(Rect::new(0.0, top, w, r.bottom() + 8.0 - top), s.background);
        }
        let hover = match st.hover {
            Some(Target::InfoClose) => Some(Hit::Close),
            Some(Target::Info(id)) => Some(Hit::Action(id)),
            _ => None,
        };
        draw_infobar(p, &pal, r, bar, hover);
        let parts = bar.parts(p, r);
        if let Some(c) = parts.close {
            st.hits.push((c, Target::InfoClose));
        }
        for (id, rr) in parts.actions {
            st.hits.push((rr, Target::Info(id)));
        }
        top = r.bottom() + 8.0;
    }
    (top, moving)
}

/// The tooltip text of a control of the bar, if it has one.
pub fn tip_of(t: Target) -> Option<&'static str> {
    Some(match t {
        Target::Refresh => "Read the sessions again (F5)",
        Target::MenuButton => "Accounts, settings and more",
        Target::NewSession => "Choose a folder, account and app for a new session (Ctrl+N)",
        Target::ArchiveMenu => "Archive older sessions or manage the archive",
        Target::ArchivePick(_) => "Select this archived session for unarchiving",
        Target::SearchClear => "Clear the search",
        Target::SearchConversation => "In conversations (Ctrl+Shift+F) — toggle full-text search",
        Target::Account => "The account sessions are opened as",
        Target::SessionToggle => "Toggle session view (Ctrl+Shift+E)",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_text::bitmap::Bitmap;
    use mdshared_ui::gfx::Gfx;

    #[test]
    fn the_bar_fits_between_the_left_edge_and_the_caption_buttons_from_the_least_width_up() {
        let mut w = 760.0;
        while w <= 3000.0 {
            let b = Bar::layout(w);
            let min = caption(w, WinButton::Min);
            assert!(b.menu.right() <= min.x, "width {w}");
            assert!(b.dot.x >= 14.0 && b.account.x > b.dot.right());
            assert!(b.search.x > b.account.right() && b.search.w >= 102.0, "width {w}");
            assert!(b.search_mode.x > b.search.right() && b.new_session.x > b.search_mode.right());
            w += 20.0;
        }
    }

    #[test]
    fn the_logo_and_warning_hairline_rasterize_at_one_and_one_and_a_half_scale() {
        for dark in [true, false] {
            let mut demo = AppState::new(crate::source::Source::Demo, Some(dark), Default::default());
            let warning = demo.pal().warn;
            let title = demo.pal().title;
            let bgr = |[r, g, b]: [u8; 3]| [b, g, r, 255];
            let mut gfx = Gfx::new();
            for scale in [1.0, 1.5] {
                let mut bmp = Bitmap::new((760.0 * scale) as u32, (TITLE_H * scale) as u32);
                demo.hits.clear();
                draw(&mut demo, &mut Painter::new(&mut bmp, &mut gfx, scale), 760.0, false, false, true);
                let logo = (10.0 * scale) as usize..(28.0 * scale) as usize;
                let painted_logo_pixels = bmp
                    .pixels
                    .chunks_exact(bmp.width as usize * 4)
                    .skip((11.0 * scale).round() as usize)
                    .take((18.0 * scale) as usize)
                    .flat_map(|row| row[logo.start * 4..logo.end * 4].as_chunks::<4>().0.iter())
                    .filter(|p| **p != bgr(title))
                    .count();
                assert!(painted_logo_pixels > 100);
                assert!(bmp.pixels.as_chunks::<4>().0.contains(&bgr(warning)));
                assert!(demo.hits.iter().any(|(_, target)| *target == Target::UsageLines));
                if scale == 1.5
                    && let Some(folder) = std::env::var_os("CS_USAGE_TEST_SHOTS")
                {
                    let name = if dark { "dark" } else { "light" };
                    bmp.write_png(&std::path::Path::new(&folder).join(format!("{name}-header-150.png"))).unwrap();
                }
            }
            demo.config.show_usage = false;
            demo.hits.clear();
            let mut bmp = Bitmap::new(760, TITLE_H as u32);
            draw(&mut demo, &mut Painter::new(&mut bmp, &mut gfx, 1.0), 760.0, false, false, true);
            assert!(!demo.hits.iter().any(|(_, target)| *target == Target::UsageLines));
        }
    }
}
