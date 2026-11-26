//! The session list: folders as sticky headings, sessions as two-line rows, laid out by
//! `sections::Sections` so only the rows in view are drawn. Also the relative times.

use std::time::{SystemTime, UNIX_EPOCH};

use claude_switch_core::App;
use mdshared_ui::anim::{Animator, document};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::glyph;
use mdshared_ui::pane::draw_row_fill;
use mdshared_ui::rows::Nav;
use mdshared_ui::scroll::draw_scrollbar;
use mdshared_ui::sections::Entry;
use mdshared_ui::tokens::legible;

use crate::state::{AppState, Focus, Msg, Target};

/// Sizes from this one up show beside the row.
const LARGE: u64 = 1024 * 1024;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// "just now", "5 min ago", "3 h ago", "2 days ago", then the date, in local time.
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    ago_at(now_ms, then_ms, claude_switch_core::clock::local_offset_minutes())
}

/// [`ago`] with the date for a clock `offset` minutes east of UTC.
pub fn ago_at(now_ms: u64, then_ms: u64, offset: i32) -> String {
    if then_ms == 0 {
        return "unknown".into();
    }
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..172_800 => "yesterday".into(),
        172_800..1_209_600 => format!("{} days ago", secs / 86_400),
        _ => mdshared_app::fmt::date((then_ms / 1000) as i64, offset),
    }
}

pub fn app_name(app: App) -> &'static str {
    match app {
        App::Cli => "CLI",
        App::Desktop => "Desktop",
    }
}

pub fn draw(st: &mut AppState, cx: &mut mdshared_ui::host::Cx<'_, Msg>, p: &mut Painter, area: Rect) {
    let pal = *st.pal();
    p.fill(area, pal.pane);
    st.list_area = area;
    st.sections.view = area.h;
    st.sections.scroll = st.sections.clamp_scroll(st.sections.scroll);
    st.hits.push((area, Target::ListBlank));
    if st.groups.is_empty() {
        let text = if !st.scanned {
            "Reading the sessions\u{2026}"
        } else if st.search.is_empty() {
            "No sessions were found on this computer."
        } else {
            "No session matches the search."
        };
        p.text_center(Face::Ui, 13.0, Rect::new(area.x, area.y + 40.0, area.w, 24.0), text, pal.muted);
        return;
    }
    let old = p.clip(area);
    let now = now_ms();
    let cursor = st.selected.clone().and_then(|id| st.entry_of(&id));
    for i in st.sections.visible() {
        let r = st.sections.rect(area, i);
        match st.sections.entry(i) {
            Some(Entry::Heading(g)) => heading(st, p, r, g, i),
            Some(Entry::Row { .. }) if row(st, cx.anim(), p, r, i, cursor == Some(i), now) => cx.animating(r),
            _ => {}
        }
    }
    if let Some((i, r)) = st.sections.sticky(area)
        && let Some(Entry::Heading(g)) = st.sections.entry(i)
    {
        heading(st, p, r, g, i);
    }
    st.list_scroll_track = None;
    if let Some((track, thumb)) = st.sections.thumb(area) {
        draw_scrollbar(p, &pal, track, thumb, false);
        st.list_scroll_track = Some(track);
    }
    p.restore(old);
}

fn heading(st: &mut AppState, p: &mut Painter, r: Rect, g: usize, i: usize) {
    let pal = *st.pal();
    p.fill(r, pal.pane);
    if st.hover == Some(Target::Entry(i)) {
        p.rbox(r.inset(2.0), 5.0, pal.hover);
    }
    let open = if st.sections.is_collapsed(g) { 0.0 } else { 1.0 };
    glyph::draw_disclosure(p, r.x + 16.0, r.cy(), 12.0, open, pal.muted);
    let vg = &st.groups[g];
    let name_w = p.text(Face::Semibold, 12.5, r.x + 28.0, r.cy(), &vg.name, pal.text, r.w * 0.55);
    let count = vg.rows.len().to_string();
    p.text(Face::Ui, 12.0, r.x + 36.0 + name_w, r.cy(), &count, pal.muted, 60.0);
    let path = vg.rows.first().and_then(|&r| st.sessions.get(r)).map(|s| s.cwd.display().to_string());
    if let Some(path) = path {
        let x = r.x + 52.0 + name_w + p.measure(Face::Ui, 12.0, &count);
        p.text(Face::Ui, 11.5, x, r.cy(), &path, pal.faint, (r.right() - 12.0 - x).max(0.0));
    }
    p.fill(Rect::new(r.x + 10.0, r.bottom() - p.px(), r.w - 20.0, p.px()), pal.line);
    st.hits.push((r, Target::Entry(i)));
}

fn row(st: &mut AppState, anim: &mut Animator, p: &mut Painter, r: Rect, i: usize, selected: bool, now: u64) -> bool {
    let pal = *st.pal();
    let Some(si) = st.session_at(i) else { return false };
    let s = &st.sessions[si];
    let rr = Rect::new(r.x + 6.0, r.y + 1.0, r.w - 12.0, r.h - 2.0);
    let hovered = st.hover == Some(Target::Entry(i));
    let pressed = st.pressed == Some(Target::Entry(i));
    let moving = if st.high_contrast {
        draw_row_fill(p, &pal, rr, hovered, pressed, selected);
        false
    } else {
        let hover = anim.toggle(("session-hover", &s.id), hovered && !selected, document::HOVER);
        let select = anim.toggle(("session-selected", &s.id), selected, document::HOVER);
        p.rbox_a(rr, 5.0, if pressed { pal.press } else { pal.hover }, hover);
        p.rbox_a(rr, 5.0, pal.on, select);
        p.rbox_a(Rect::new(rr.x, rr.y + 7.0, 3.0, rr.h - 14.0), 1.5, pal.accent, select);
        hover != f32::from(u8::from(hovered && !selected)) || select != f32::from(u8::from(selected))
    };
    if selected && st.key_used && st.focus == Focus::List {
        mdshared_ui::focus::draw_ring(p, &pal, rr, 5.0);
    }
    // A session in a tab of this window runs here, whatever the files say yet.
    let running = match st.tab_account(&s.id) {
        Some(acct) => Some((acct, None)),
        None => st.running_of(&s.id).map(|r| (r.account.clone(), Some(r.app))),
    };
    let x = rr.x + 24.0;
    let right = rr.right() - 12.0;
    let line1 = rr.y + 16.0;
    let line2 = rr.y + 34.0;
    if let Some((acct, _)) = &running {
        let c = st.account_color(acct);
        p.rbox(Rect::new(rr.x + 9.0, line1 - 4.0, 8.0, 8.0), 4.0, c);
    }
    let when = ago(now, s.last_active_ms);
    let ww = p.text_right(Face::Ui, 12.0, right, line1, &when, pal.muted);
    let title_c = if s.archived { pal.muted } else { pal.text };
    p.text(Face::Ui, 13.0, x, line1, &s.title, title_c, right - ww - 12.0 - x);

    // The second line: where it last ran, the branch, the size, and whether it runs now.
    let badge = app_name(s.last_app);
    let bw = p.measure(Face::Ui, 11.0, badge) + 12.0;
    let b = Rect::new(x, line2 - 8.0, bw, 16.0);
    p.frame(b, 4.0, if selected { pal.pane } else { pal.field }, pal.line);
    p.text_center(Face::Ui, 11.0, b, badge, pal.muted);
    let cx = b.right() + 8.0;
    let mut tail = Vec::new();
    if let Some(br) = &s.branch {
        tail.push(br.clone());
    }
    if s.bytes >= LARGE {
        tail.push(mdshared_app::fmt::size(s.bytes));
    }
    if s.archived {
        tail.push("archived".into());
    }
    let mut run_w = 0.0;
    if let Some((acct, app)) = &running {
        let c = legible(st.account_color(acct), if selected { pal.on } else { pal.pane }, 4.5);
        let place = app.map_or("this app", app_name);
        let text = format!("running in {place} \u{b7} {}", st.account_name(acct));
        run_w = p.text_right(Face::Semibold, 11.5, right, line2, &text, c) + 12.0;
    }
    if !tail.is_empty() {
        p.text(Face::Ui, 12.0, cx, line2, &tail.join("  \u{b7}  "), pal.muted, (right - run_w - cx).max(0.0));
    }
    st.hits.push((r, Target::Entry(i)));
    moving
}

/// A key for the list. True when it was used.
pub fn key(st: &mut AppState, k: &mdshared_ui::keys::KeyName) -> bool {
    use mdshared_ui::keys::KeyName;
    let nav = match k {
        KeyName::Up => Nav::Up,
        KeyName::Down => Nav::Down,
        KeyName::PageUp => Nav::PageUp,
        KeyName::PageDown => Nav::PageDown,
        KeyName::Home => Nav::Home,
        KeyName::End => Nav::End,
        _ => return false,
    };
    let cursor = st.selected.clone().and_then(|id| st.entry_of(&id));
    if let Some(e) = st.sections.nav(cursor, nav)
        && let Some(si) = st.session_at(e)
    {
        let id = st.sessions[si].id.clone();
        st.select(&id);
        st.sections.scroll = st.sections.reveal(e);
    }
    true
}

/// A click on an entry: a heading folds its folder, a row is selected.
pub fn click(st: &mut AppState, entry: usize) {
    match st.sections.entry(entry) {
        Some(Entry::Heading(g)) => {
            st.sections.toggle(g);
            if let Some(vg) = st.groups.get(g) {
                let key = vg.key.clone();
                if !st.collapsed.remove(&key) {
                    st.collapsed.insert(key);
                }
            }
        }
        Some(Entry::Row { .. }) => {
            if let Some(si) = st.session_at(entry) {
                let id = st.sessions[si].id.clone();
                st.select(&id);
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_a_person_says_them() {
        let now = 10_000_000_000;
        assert_eq!(ago(now, now - 5_000), "just now");
        assert_eq!(ago(now, now - 5 * 60_000), "5 min ago");
        assert_eq!(ago(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(now, now - 30 * 3_600_000), "yesterday");
        assert_eq!(ago(now, now - 4 * 86_400_000), "4 days ago");
        assert_eq!(ago_at(now, now - 40 * 86_400_000, 0), "1970-03-17");
        // Late in the evening in UTC is the next day east of it.
        let late = 1_764_191_400_000; // 2025-11-26 21:10 UTC
        assert_eq!(ago_at(late + 40 * 86_400_000, late, 0), "2025-11-26");
        assert_eq!(ago_at(late + 40 * 86_400_000, late, 180), "2025-11-27");
        assert_eq!(ago(now, 0), "unknown");
        assert_eq!(ago(0, 5), "just now");
    }

    #[test]
    fn the_arrow_keys_move_the_selection_across_folders_and_a_heading_click_folds_its_folder() {
        use mdshared_ui::keys::KeyName;
        let mut s = AppState::new(crate::source::Source::Demo, Some(false), crate::winfile::WindowFile::default());
        s.apply_scan(crate::demo::scan(&s.config));
        let first = s.selected.clone();
        for _ in 0..s.groups[0].rows.len() {
            assert!(key(&mut s, &KeyName::Down));
        }
        assert_eq!(s.selected.as_deref(), Some(s.sessions[s.groups[1].rows[0]].id.as_str()));
        key(&mut s, &KeyName::Home);
        assert_eq!(s.selected, first);
        assert!(!key(&mut s, &KeyName::Enter));
        click(&mut s, 0);
        assert!(s.sections.is_collapsed(0));
        assert!(s.collapsed.contains(&s.groups[0].key));
        click(&mut s, 0);
        assert!(!s.sections.is_collapsed(0));
    }
}
