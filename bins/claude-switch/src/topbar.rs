//! The title row, which is also the tool bar: the window's name, the account drop-down, the
//! search field, New session, Refresh, the menu, and the caption buttons. [`Bar::layout`] is the
//! one place the rectangles come from, for drawing and for the pointer alike.

use mdshared_ui::chrome::{self, TITLE_H, WIN_BTN_W, WinButton};
use mdshared_ui::dropdown::{DROPDOWN_H, Look, draw_dropdown};
use mdshared_ui::focus::draw_ring;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::glyph;
use mdshared_ui::infobar::{Hit, draw_infobar};
use mdshared_ui::textfield::FieldColors;
use mdshared_ui::window;

use crate::state::{AppState, Focus, Msg, Target};
use crate::tabs;

pub struct Bar {
    pub row: Rect,
    pub dot: Rect,
    pub account: Rect,
    pub search: Rect,
    pub clear: Rect,
    pub new_session: Rect,
    pub refresh: Rect,
    pub menu: Rect,
}

const NEW_W: f32 = 118.0;

impl Bar {
    pub fn layout(w: f32) -> Bar {
        let y = ((TITLE_H - DROPDOWN_H) / 2.0).round();
        let row = Rect::new(0.0, 0.0, w, TITLE_H);
        // A narrow window leaves out the name and gives the account less room.
        let left = if Bar::shows_name(w) { 134.0 } else { 14.0 };
        let dot = Rect::new(left, (TITLE_H - 10.0) / 2.0, 10.0, 10.0);
        let account = Rect::new(left + 18.0, y, if w >= 900.0 { 200.0 } else { 170.0 }, DROPDOWN_H);
        let right_free = w - 3.0 * WIN_BTN_W - 16.0;
        let tools_w = NEW_W + 8.0 + 32.0 + 4.0 + 32.0;
        let search_x = account.right() + 12.0;
        let search_w = (right_free - tools_w - 12.0 - search_x).clamp(140.0, 420.0);
        let search = Rect::new(search_x, y, search_w, DROPDOWN_H);
        let clear = Rect::new(search.right() - 26.0, y + 2.0, 24.0, DROPDOWN_H - 4.0);
        let new_session = Rect::new(search.right() + 12.0, y, NEW_W, DROPDOWN_H);
        let refresh = Rect::new(new_session.right() + 8.0, y, 32.0, DROPDOWN_H);
        let menu = Rect::new(refresh.right() + 4.0, y, 32.0, DROPDOWN_H);
        Bar { row, dot, account, search, clear, new_session, refresh, menu }
    }

    pub fn shows_name(w: f32) -> bool {
        w >= 1000.0
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
    if Bar::shows_name(w) {
        p.text(Face::Semibold, 13.0, 16.0, TITLE_H / 2.0, "Claude Switch", pal.text, 112.0);
    }

    // The account: its colour, then the drop-down.
    let current = st.current().clone();
    p.rbox(bar.dot, 5.0, current.color);
    let look = Look {
        hovered: st.hover == Some(Target::Account),
        focused: st.key_used && st.focus == Focus::Account,
        disabled: false,
    };
    draw_dropdown(p, &pal, bar.account, &st.accounts_dd, look);
    st.hits.push((bar.account, Target::Account));

    // The search field.
    let active = st.focus == Focus::Search;
    p.frame(bar.search, 5.0, pal.field, if active { pal.accent } else { pal.line });
    p.icon(glyph::SEARCH, 14.0, bar.search.x + 15.0, bar.search.cy(), pal.muted);
    let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
    let inner = bar.search_inner();
    st.search.draw(p, inner, Face::Ui, 13.0, &colors, "Search sessions", active && focused, caret_on);
    st.search_stops = p.caret_stops(Face::Ui, 13.0, &st.search.display());
    st.hits.push((bar.search, Target::Search));
    if !st.search.is_empty() {
        icon_button(st, p, bar.clear, glyph::CLOSE, Target::SearchClear);
    }
    if st.key_used && active {
        draw_ring(p, &pal, bar.search, 5.0);
    }

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
    let pal = *st.pal();
    let terminal = st.terminal_shown().then(|| st.terminal_scheme());
    let mut top = TITLE_H;
    if !st.tabs.is_empty() {
        let look = tabs::Look {
            hover: st.hover,
            pressed: st.pressed,
            terminal: terminal.map(|s| (s.background, s.foreground, st.term_colours.1)),
            session_view: st.tabs.shown().and_then(|t| t.pane.as_ref()).map(|_| st.session_visible(w)),
        };
        let strip = tabs::strip_rect(top, w);
        if tabs::draw(p, &pal, cx.anim(), &st.tabs, strip, &look, &mut st.hits) {
            cx.animating(strip);
        }
        top += tabs::STRIP_H;
    }
    if let Some(bar) = &st.info {
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
    top
}

/// The tooltip text of a control of the bar, if it has one.
pub fn tip_of(t: Target) -> Option<&'static str> {
    Some(match t {
        Target::Refresh => "Read the sessions again (F5)",
        Target::MenuButton => "Accounts, settings and more",
        Target::NewSession => "Choose a folder and account for a new CLI session (Ctrl+N)",
        Target::SearchClear => "Clear the search",
        Target::Account => "The account sessions are opened as",
        Target::SessionToggle => "Toggle session view (Ctrl+Shift+E)",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_fits_between_the_left_edge_and_the_caption_buttons_from_the_least_width_up() {
        let mut w = 760.0;
        while w <= 3000.0 {
            let b = Bar::layout(w);
            let min = caption(w, WinButton::Min);
            assert!(b.menu.right() <= min.x, "width {w}");
            assert!(b.dot.x >= 14.0 && b.account.x > b.dot.right());
            assert!(b.search.x > b.account.right() && b.search.w >= 140.0, "width {w}");
            assert!(b.new_session.x > b.search.right());
            w += 20.0;
        }
    }
}
