//! The session list: group headings and session rows, laid out by
//! `sections::Sections` so only the rows in view are drawn. Also the relative times.

pub mod grouping;
pub mod model;

use claude_switch_core::grouping::Mode;
use std::time::{SystemTime, UNIX_EPOCH};

use mdshared_ui::anim::{Animator, document};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::glyph;
use mdshared_ui::pane::draw_row_fill;
use mdshared_ui::rows::Nav;
use mdshared_ui::scroll::draw_scrollbar;
use mdshared_ui::sections::Entry;
use mdshared_ui::tokens::legible;

use crate::state::{AppState, Focus, Msg, Target};
use model::Filter;

const CONTROLS_H: f32 = 101.0;

fn control_rects(r: Rect) -> (Rect, Rect, Rect) {
    (
        Rect::new(r.x + 12.0, r.y + 8.0, r.w - 24.0, 28.0),
        Rect::new(r.x + 12.0, r.y + 40.0, 148.0, 28.0),
        Rect::new(r.right() - 111.0, r.y + 40.0, 99.0, 28.0),
    )
}

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

pub fn draw(st: &mut AppState, cx: &mut mdshared_ui::host::Cx<'_, Msg>, p: &mut Painter, area: Rect) {
    let now = cx.now();
    for r in draw_with(st, cx.anim(), p, area, now) {
        cx.animating(r);
    }
    // The strip's elapsed time ticks even with reduced motion, when nothing animates.
    if st.compact.jobs.list.iter().any(|j| j.state == crate::handover_jobs::State::Writing) {
        cx.wake_at(now + std::time::Duration::from_secs(1));
    }
}

/// [`draw`] without a window: returns what to keep redrawing.
pub fn draw_with(
    st: &mut AppState,
    anim: &mut Animator,
    p: &mut Painter,
    area: Rect,
    now: std::time::Instant,
) -> Vec<Rect> {
    let mut moving = Vec::new();
    let pal = *st.pal();
    p.fill(area, pal.pane);
    controls(st, p, area);
    // Background handovers, between the filter chips and the list; no room when there are none.
    let strip = Rect::new(area.x, area.y + CONTROLS_H, area.w, st.compact.jobs.height());
    moving.extend(crate::handover_jobs::draw_strip(st, p, strip, now, Some(&mut *anim)));
    let top = CONTROLS_H + strip.h;
    let area = Rect::new(area.x, area.y + top, area.w, (area.h - top).max(0.0));
    st.list_area = area;
    st.list_scroll_track = None;
    st.sections.view = area.h;
    st.sections.scroll = st.sections.clamp_scroll(st.sections.scroll);
    st.hits.push((area, Target::ListBlank));
    if st.groups.is_empty() && st.conversation_search.on {
        return moving;
    }
    if st.groups.is_empty() {
        let text = if !st.scanned {
            "Reading the sessions\u{2026}"
        } else if st.search.is_empty() && st.filter == Filter::All {
            "No sessions to show."
        } else {
            "No sessions match this view."
        };
        p.text_center(Face::Ui, 13.0, Rect::new(area.x, area.y + 40.0, area.w, 24.0), text, pal.muted);
        return moving;
    }
    let old = p.clip(area);
    let now = now_ms();
    let offset = claude_switch_core::clock::local_offset_minutes();
    let cursor = st.selected.as_ref().and_then(|id| st.entry_of(id));
    let first_hit = st.hits.len();
    for i in st.sections.visible() {
        let r = st.sections.rect(area, i);
        match st.sections.entry(i) {
            Some(Entry::Heading(g)) => heading(st, p, r, g, i, (now, offset)),
            Some(Entry::Row { .. }) if row(st, anim, p, r, i, cursor == Some(i), (now, offset)) => moving.push(r),
            _ => {}
        }
    }
    if let Some((i, r)) = st.sections.sticky(area)
        && let Some(Entry::Heading(g)) = st.sections.entry(i)
    {
        heading(st, p, r, g, i, (now, offset));
    }
    for (r, _) in &mut st.hits[first_hit..] {
        let bottom = r.bottom().min(area.bottom());
        r.y = r.y.max(area.y);
        r.h = (bottom - r.y).max(0.0);
    }
    if let Some((track, thumb)) = st.sections.thumb(area) {
        draw_scrollbar(p, &pal, track, thumb, false);
        st.list_scroll_track = Some(track);
    }
    p.restore(old);
    moving
}

fn controls(st: &mut AppState, p: &mut Painter, r: Rect) {
    let pal = *st.pal();
    p.fill(Rect::new(r.x, r.y, r.w, CONTROLS_H), pal.bar);
    let (seg, grouping, menu) = control_rects(r);
    let target = Target::ArchiveMenu;
    let label = if st.filter == Filter::Archived && !st.archive_picks.is_empty() {
        format!("Archive · {} ▾", st.archive_picks.len())
    } else {
        "Archive ▾".into()
    };
    mdshared_ui::overlay::draw_dialog_button(
        p,
        &pal,
        menu,
        &label,
        false,
        st.hover == Some(target),
        st.pressed == Some(target),
    );
    st.hits.push((menu, target));
    let searching = st.conversation_search.on;
    if searching {
        let count = st.groups.first().map_or(0, |g| g.rows.len());
        crate::conversation_draw::search_status(st, p, Rect::new(r.x, r.y, r.w, r.h), count);
    } else {
        p.frame(seg, 7.0, pal.field, pal.line);
        let mode = st.list_mode();
        let by_meaning = mode == Mode::Topics && st.config.meaning.on();
        if matches!(mode, Mode::Projects | Mode::Recent) || by_meaning {
            let (label, on) = match mode {
                Mode::Projects => {
                    (if st.config.group_by_folder { "Group by: Folder" } else { "Group by: Project" }, false)
                }
                Mode::Topics => ("By meaning", st.config.meaning.group_topics),
                _ => ("Sittings", st.config.show_sittings),
            };
            let target = Target::ListGrouping;
            mdshared_ui::overlay::draw_dialog_button(
                p,
                &pal,
                grouping,
                label,
                on,
                st.hover == Some(target),
                st.pressed == Some(target),
            );
            st.hits.push((grouping, target));
        }
    }
    let modes = if st.grouping.topics { 4 } else { 3 };
    let pill_w = (seg.w - 4.0) / modes as f32;
    for (i, mode) in Mode::ALL.into_iter().take(modes).enumerate().filter(|_| !searching) {
        let target = Target::ListMode(mode);
        let selected = st.list_mode() == mode;
        let pill = Rect::new(seg.x + 2.0 + i as f32 * pill_w, seg.y + 2.0, pill_w, 24.0);
        if selected || st.hover == Some(target) {
            p.rbox(pill, 5.0, if selected { pal.on } else { pal.hover });
        }
        p.text_center(Face::Semibold, 12.0, pill, mode.label(), if selected { pal.text } else { pal.muted });
        st.hits.push((pill, target));
    }
    let visible: Vec<_> = Filter::ALL
        .into_iter()
        .filter(|f| match f {
            Filter::Hidden => st.config.show_hidden,
            Filter::Archived => st.archive_count != 0,
            _ => true,
        })
        .collect();
    let gap = 4.0;
    let widths: Vec<_> = visible.iter().map(|f| p.measure(Face::Semibold, 11.5, f.label()) + 14.0).collect();
    let total = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
    let scale = ((r.w - 24.0) / total).clamp(0.0, 1.0);
    let mut x = r.x + 12.0;
    for (&filter, width) in visible.iter().zip(widths) {
        let rect = Rect::new(x, r.y + 74.0, width * scale, 22.0);
        let target = Target::ListFilter(filter);
        let on = st.filter == filter;
        p.frame(
            rect,
            10.0,
            if on {
                pal.on
            } else if st.hover == Some(target) {
                pal.hover
            } else {
                pal.bar
            },
            if on { pal.accent } else { pal.line },
        );
        p.text_center(Face::Semibold, 11.5, rect, filter.label(), if on { pal.text } else { pal.muted });
        st.hits.push((rect, target));
        x += (width + gap) * scale;
    }
    p.fill(Rect::new(r.x, r.y + CONTROLS_H - p.px(), r.w, p.px()), pal.line);
}

pub fn draw_undo(st: &mut AppState, p: &mut Painter, window: Rect) {
    let Some(undo) = &st.undo_hide else { return };
    let pal = *st.pal();
    let r = Rect::new(window.cx() - 111.0, window.bottom() - 54.0, 222.0, 38.0);
    p.frame(r, crate::windowlook::Cards::for_frame(st, p).decoration(8.0), pal.bar, pal.line);
    p.text(
        Face::Ui,
        13.0,
        r.x + 13.0,
        r.cy(),
        if undo.folder { "Folder hidden" } else { "Session hidden" },
        pal.text,
        132.0,
    );
    let button = Rect::new(r.right() - 70.0, r.y + 5.0, 64.0, 28.0);
    mdshared_ui::overlay::draw_dialog_button(
        p,
        &pal,
        button,
        "Undo",
        false,
        st.hover == Some(Target::UndoHide),
        st.pressed == Some(Target::UndoHide),
    );
    st.hits.push((button, Target::UndoHide));
}

fn heading(st: &mut AppState, p: &mut Painter, r: Rect, g: usize, i: usize, (now, offset): (u64, i32)) {
    let pal = *st.pal();
    p.fill(r, pal.pane);
    if st.hover == Some(Target::Entry(i)) {
        p.rbox(r.inset(2.0), 5.0, pal.hover);
    }
    let open = if st.sections.is_collapsed(g) { 0.0 } else { 1.0 };
    glyph::draw_disclosure(p, r.x + 16.0, r.cy(), 12.0, open, pal.muted);
    let vg = &st.groups[g];
    let summary = st.project_heading_summary(vg, now, offset);
    let max_name = if summary.is_some() { r.w * 0.27 } else { r.w * 0.55 };
    let name_w = p.text(Face::Semibold, 12.5, r.x + 28.0, r.cy(), &vg.name, pal.text, max_name);
    let count = vg.rows.len().to_string();
    p.text(Face::Ui, 12.0, r.x + 36.0 + name_w, r.cy(), &count, pal.muted, 60.0);
    let path = vg
        .key
        .strip_prefix("folder:")
        .and_then(|_| vg.rows.first())
        .and_then(|&r| st.sessions.get(r))
        .map(|s| s.cwd.display().to_string());
    if let Some(path) = summary.as_deref().or(path.as_deref()) {
        let x = r.x + 43.0 + name_w + p.measure(Face::Ui, 12.0, &count);
        let color = if summary.is_some() { pal.muted } else { pal.faint };
        p.text(Face::Ui, 11.5, x, r.cy(), path, color, (r.right() - 12.0 - x).max(0.0));
    }
    p.fill(Rect::new(r.x + 10.0, r.bottom() - p.px(), r.w - 20.0, p.px()), pal.line);
    st.hits.push((r, Target::Entry(i)));
}

fn row(
    st: &mut AppState,
    anim: &mut Animator,
    p: &mut Painter,
    r: Rect,
    i: usize,
    selected: bool,
    clock: (u64, i32),
) -> bool {
    let pal = *st.pal();
    let Some(si) = st.session_at(i) else { return false };
    let s = &st.sessions[si];
    let rr = Rect::new(r.x + 6.0, r.y + 1.0, r.w - 12.0, r.h - 2.0);
    let hovered = matches!(st.hover, Some(Target::Entry(e) | Target::Pin(e)) if e == i);
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
    let flags = st.row_flags(s);
    let archived = st.is_archived(s);
    let x = rr.x + 24.0;
    let right = rr.right() - 10.0;
    let line1 = rr.y + 13.0;
    let line2 = rr.y + 31.0;
    let snippets = st.conversation_search.hits(&s.id);
    let grouping = !st.conversation_search.on;
    if grouping && let Some(&(first, last)) = st.grouping.brackets.get(&si) {
        let top = if first { r.y + 6.0 } else { r.y };
        let end = if last { r.bottom() - 6.0 } else { r.bottom() };
        let x = r.x + 8.0;
        p.fill(Rect::new(x, top, p.px(), end - top), pal.muted);
        if first {
            p.fill(Rect::new(x, top, 5.0, p.px()), pal.muted);
        }
        if last {
            p.fill(Rect::new(x, end - p.px(), 5.0, p.px()), pal.muted);
        }
    }
    let line3 = if snippets.is_some() { rr.bottom() - 10.0 } else { rr.y + 48.0 };
    if flags.running || flags.unseen {
        let c = st
            .tab_account(&s.id)
            .or_else(|| st.running_of(&s.id).map(|r| r.account.clone()))
            .map_or(pal.accent, |acct| st.account_color(&acct));
        p.rbox(Rect::new(rr.x + 9.0, line1 - 4.0, 8.0, 8.0), 4.0, c);
    }
    let star = Rect::new(right - 23.0, line1 - 10.0, 24.0, 22.0);
    let when = ago_at(clock.0, s.last_active_ms, clock.1);
    let time_right = star.x - 5.0;
    let ww = p.text_right(Face::Ui, 11.5, time_right, line1, &when, pal.muted);
    let title_c = if s.archived || archived { pal.muted } else { pal.text };
    p.text(Face::Semibold, 13.0, x, line1, st.title_of(s), title_c, (time_right - ww - 10.0 - x).max(0.0));
    let prompt = if s.last_prompt.is_empty() { "No prompt in the transcript tail" } else { &s.last_prompt };
    if let Some(hits) = snippets {
        crate::conversation_draw::snippets(st, p, Rect::new(x, line2, right - x, 0.0), hits, selected);
    } else {
        p.text(Face::Ui, 12.0, x, line2, prompt, pal.muted, right - x);
    }
    let mut states = Vec::new();
    if archived {
        states.push("Archived");
    }
    if flags.waiting {
        states.push("Waiting for you");
    } else if flags.running {
        states.push("Running");
    }
    if flags.unseen {
        states.push("Unseen");
    }
    if flags.hidden {
        states.push("Hidden");
    }
    let fg = legible(pal.accent, if selected { pal.on } else { pal.pane }, 4.5);
    let status = states.join("  ·  ");
    let status_w =
        if status.is_empty() { 0.0 } else { p.text_right(Face::Semibold, 11.0, right, line3, &status, fg) + 12.0 };
    let mut x = x;
    st.hits.push((r, Target::Entry(i)));
    if grouping && let Some(&count) = st.grouping.earlier.get(&si) {
        let search = !st.search.text().trim().is_empty();
        let expanded = search || st.thread_key(si).is_some_and(|key| st.grouping.expanded.contains(&key));
        let label = format!("{count} earlier");
        let target = Target::ThreadToggle(si);
        glyph::draw_disclosure(p, x + 5.0, line3, 9.0, f32::from(u8::from(expanded)), pal.muted);
        let label_w = p.text(
            Face::Semibold,
            11.0,
            x + 14.0,
            line3,
            &label,
            if st.hover == Some(target) { pal.accent } else { pal.text },
            95.0,
        );
        if !search {
            st.hits.push((Rect::new(x - 4.0, line3 - 9.0, label_w + 23.0, 20.0), target));
        }
        x += label_w + 25.0;
    }
    let mut location = crate::session_marks::folder_label(s);
    if grouping
        && st.list_mode() == Mode::Projects
        && !st.config.group_by_folder
        && let Some(project) = st.project(si)
    {
        location = project.location.clone().unwrap_or_default();
        if let Some(branch) = s.branch.as_deref().filter(|b| Some(*b) != project.branch.as_deref()) {
            if !location.is_empty() {
                location.push_str(" · ");
            }
            location.push_str(branch);
        }
    }
    let label = st.marks.handover_from(&s.id).map_or(location.clone(), |from| {
        if location.is_empty() { format!("From {from}") } else { format!("From {from} · {location}") }
    });
    let label = if st.marks.handed_over_to(&s.id).is_some() {
        if label.is_empty() { "Handed over".to_owned() } else { format!("Handed over · {label}") }
    } else {
        label
    };
    p.text(Face::Ui, 11.0, x, line3, &label, pal.muted, (right - status_w - x).max(0.0));
    if archived && st.filter == Filter::Archived {
        let target = Target::ArchivePick(i);
        let checked = st.archive_picks.contains(&s.id);
        p.frame(
            star.inset(3.0),
            4.0,
            if checked || st.hover == Some(target) { pal.on } else { pal.pane },
            if checked { fg } else { pal.muted },
        );
        if checked {
            p.text_center(Face::Semibold, 12.0, star, "✓", fg);
        }
        st.hits.push((star, target));
    } else if !archived && (hovered || flags.pinned) {
        if st.hover == Some(Target::Pin(i)) {
            p.rbox(star, 5.0, pal.hover);
        }
        p.text_center(
            Face::Ui,
            16.0,
            star,
            if flags.pinned { "★" } else { "☆" },
            if flags.pinned { fg } else { pal.muted },
        );
        st.hits.push((star, Target::Pin(i)));
    }
    moving
}

/// A key for the list. True when it was used.
pub fn key(st: &mut AppState, k: &mdshared_ui::keys::KeyName) -> bool {
    use mdshared_ui::keys::KeyName;
    if matches!(k, KeyName::Left | KeyName::Right)
        && st.list_mode() == Mode::Threads
        && !st.conversation_search.on
        && st.search.text().trim().is_empty()
        && let Some(i) = st.selected.as_ref().and_then(|id| st.by_id.get(id)).copied()
        && st.grouping.earlier.contains_key(&i)
        && let Some(key) = st.thread_key(i)
    {
        if st.grouping.expanded.contains(&key) != (*k == KeyName::Right) {
            st.toggle_thread(i);
        }
        return true;
    }
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

    #[test]
    fn controls_and_virtual_rows_fit_at_760_by_480_in_every_mode() {
        use mdshared_ui::splitter::Axis;
        let mut s = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
        s.apply_scan(crate::demo::scan(&s.config));
        // Title bar, tab strip, and demo restore offer are all visible at this size.
        let top = mdshared_ui::chrome::TITLE_H + crate::tabs::STRIP_H + 38.0;
        let pane = s.splitter.rects(Rect::new(0.0, top, 760.0, 480.0 - top), Axis::Horizontal).0;
        let (mode, option, archive) = control_rects(pane);
        assert!(mode.bottom() < option.y && option.right() < archive.x);
        assert!(archive.right() <= pane.right() && (mode.w - 4.0) / 4.0 >= 60.0);
        let rows = Rect::new(pane.x, pane.y + CONTROLS_H, pane.w, pane.h - CONTROLS_H);
        assert_eq!(rows.bottom(), 480.0);
        for mode in Mode::ALL {
            s.config.sessions_mode = mode;
            s.sections.view = rows.h;
            s.rebuild_view();
            let visible = s.sections.visible();
            assert!((3..=10).contains(&visible.len()), "{mode:?}: {visible:?}");
            assert!(visible.iter().copied().all(|i| {
                let r = s.sections.rect(rows, i);
                r.bottom() > rows.y && r.y < rows.bottom() && r.h > 0.0
            }));
            assert!(s.sections.len() > visible.len() * 20);
        }
    }
}
