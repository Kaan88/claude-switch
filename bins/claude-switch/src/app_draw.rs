//! Composes the app frame back to front.

use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::overlay::Bubble;
use mdshared_ui::splitter::{self, Axis};

use crate::state::{AppState, Focus, Msg, Target};
use crate::topbar::{self, Bar};
use crate::{details, list, windowlook};

fn draw_tip(st: &AppState, p: &mut Painter, w: f32, h: f32) {
    if let Some(over) = st.tip.shown() {
        let pal = *st.pal();
        let tip = Bubble::tip(p, &over.text, over.anchor, w, h);
        windowlook::Cards::for_frame(st, p).draw(p, tip.rect, pal.popup, |p| tip.draw(p, &pal));
    }
}

pub fn draw(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter) {
    st.sync_usage_setting();
    st.mask_covered_hover(cx.now());
    let pal = *st.pal();
    let (w, h) = cx.size();
    let window = Rect::new(0.0, 0.0, w, h);
    st.hits.clear();
    p.fill(window, pal.page);

    let search_focus = cx.focused()
        && st.focus == Focus::Search
        && st.dialog.is_none()
        && st.palette.is_none()
        && !st.terminal_shown();
    let caret_on = st.search.blink_caret(cx.anim(), "search", st.blink, search_focus);
    if search_focus && st.blink.is_some() && cx.anim().animates() {
        cx.animating(Bar::layout(w).search);
    }
    let max = topbar::draw(st, p, w, cx.maximized(), search_focus, caret_on);
    cx.frost(p, Bar::layout(w).row);
    cx.set_maximize_button(Some(max));
    let top = topbar::draw_below_title(st, cx, p, w);
    let top = st.draw_tab_restore(p, top, w);
    st.main_top = top;

    let area = Rect::new(0.0, top, w, (h - top).max(0.0));
    let term_area = st.layout_session_view(cx, area);
    if !st.tabs.is_empty() {
        st.layout_terminals(term_area, area, cx.scale());
    }
    if st.terminal_shown() {
        if st.chat_active() {
            st.draw_chat(cx, p, area, &pal);
            if term_area.h > 0.0 {
                st.draw_terminal(cx, p, term_area);
            }
        } else {
            st.draw_terminal(cx, p, term_area);
            st.draw_session_view(cx, p, area, &pal);
        }
    } else {
        let rects = st.splitter.rects(area, Axis::Horizontal);
        list::draw(st, cx, p, rects.0);
        details::draw(st, p, rects.2);
        let (x, y) = st.background_pointer();
        let hot = st.splitter.hit(area, Axis::Horizontal, x, y);
        splitter::draw(p, &pal, rects, Axis::Horizontal, hot);
        st.hits.push((rects.1, Target::Splitter));
    }

    crate::resume_tabs::draw_toasts(st, cx, p, window);
    list::draw_undo(st, p, window);
    // The Peek button's tip belongs above the Settings dialog; every other tip is under overlays.
    let peek_tip = st.tip.shown().is_some_and(|over| over.key == Target::SettingsPeek);
    if !peek_tip {
        draw_tip(st, p, w, h);
    }
    st.draw_session_overlay(p, &pal, window);
    windowlook::draw_overlays(st, cx, p, window);
    if peek_tip && !st.settings_peeking() {
        draw_tip(st, p, w, h);
    }
    st.usage.repaint_at = if st.config.show_usage
        && (st.usage.popup.open || matches!(st.dialog, Some(crate::dialogs::Dialog::Accounts { .. })))
    {
        let now = cx.now();
        let at = *st.usage.repaint_at.get_or_insert(now + std::time::Duration::from_secs(5));
        cx.wake_at(at);
        Some(at)
    } else {
        None
    };
}
