//! The right pane: what is known about the selected session, the buttons that open it (or the
//! line that says where it runs), and the last few messages, read on a worker.

use claude_switch_core::{App, Refusal};
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::infobar::{InfoBar, Severity, draw_infobar};
use mdshared_ui::overlay::draw_dialog_button;

use crate::list::{ago, now_ms};
use crate::state::{AppState, Target};

const PAD: f32 = 24.0;
const LABEL_W: f32 = 104.0;

#[derive(Default)]
pub struct Scroll {
    pub area: Rect,
    pub offset: f32,
    max: f32,
}

impl Scroll {
    pub fn wheel(&mut self, dy: f32) {
        self.offset = (self.offset - dy).clamp(0.0, self.max);
    }
}

pub fn draw(st: &mut AppState, p: &mut Painter, area: Rect) {
    let pal = *st.pal();
    p.fill(area, pal.page);
    st.details_scroll.area = area;
    let Some(s) = st.selected_session().cloned() else {
        let text = if st.scanned { "Select a session on the left." } else { "" };
        p.text_center(Face::Ui, 13.0, Rect::new(area.x, area.y + 40.0, area.w, 24.0), text, pal.muted);
        return;
    };
    let old = p.clip(area);
    let first_hit = st.hits.len();
    let x = area.x + PAD;
    let w = (area.w - 2.0 * PAD).max(40.0);
    let offset = st.details_scroll.offset;
    let mut y = area.y + 22.0 - offset;
    y += p.paragraph_wrapped(Face::Semibold, 18.0, x, y, w, st.title_of(&s), pal.text, Wrap::Words) + 4.0;
    let folder = if s.cwd.as_os_str().is_empty() { "(no folder)".to_string() } else { s.cwd.display().to_string() };
    y += p.paragraph_wrapped(Face::Mono, 12.0, x, y, w, &folder, pal.muted, Wrap::Anywhere) + 14.0;

    // The facts.
    let last = st.last_account.as_ref().filter(|(id, _)| *id == s.id).map(|(_, a)| a.clone());
    let now = now_ms();
    let when = format!(
        "{}  ({})",
        ago(now, s.last_active_ms),
        local_date_time(s.last_active_ms, claude_switch_core::clock::local_offset_minutes())
    );
    let mut facts: Vec<(&str, String)> = vec![
        ("Branch", s.branch.clone().unwrap_or_else(|| "none".into())),
        ("Last active", when),
        ("Size", mdshared_app::fmt::size(s.bytes)),
        ("Last ran in", if s.last_app == App::Cli { "the CLI".into() } else { "the desktop app".into() }),
    ];
    if s.archived {
        facts.push(("Desktop archive", "yes".into()));
    }
    if let Some(topic) = st.marks.topic(&s.id) {
        facts.push(("Topic", topic.to_owned()));
    }
    if let Some(about) = st.meaning_summary(&s.id) {
        facts.push(("About", about.to_owned()));
    }
    for (label, value) in facts {
        p.text(Face::Ui, 12.0, x, y + 9.0, label, pal.muted, LABEL_W - 8.0);
        p.text(Face::Ui, 13.0, x + LABEL_W, y + 9.0, &value, pal.text, w - LABEL_W);
        y += 22.0;
    }
    p.text(Face::Ui, 12.0, x, y + 9.0, "Last ran as", pal.muted, LABEL_W - 8.0);
    if let Some(slug) = &last {
        p.rbox(Rect::new(x + LABEL_W, y + 4.0, 10.0, 10.0), 5.0, st.account_color(slug));
        p.text(Face::Ui, 13.0, x + LABEL_W + 16.0, y + 9.0, &st.account_name(slug), pal.text, w - LABEL_W - 16.0);
    } else {
        p.text(Face::Ui, 13.0, x + LABEL_W, y + 9.0, "Unknown", pal.muted, w - LABEL_W);
    }
    y += 22.0;
    p.text(Face::Ui, 12.0, x, y + 9.0, "Session", pal.muted, LABEL_W - 8.0);
    p.text(Face::Mono, 11.5, x + LABEL_W, y + 9.0, &s.id, pal.faint, w - LABEL_W);
    y += 36.0;
    y = crate::conversation_draw::size(st, p, x, y, w);
    y = crate::folder_defaults::draw_details(st, p, x, y, w, &s.cwd, s.last_app);

    // Open, or where it runs.
    if let Some(acct) = st.tab_account(&s.id) {
        let text = format!("It runs in a tab of this window as {}. Opening it shows that tab.", st.account_name(&acct));
        let bar = InfoBar::new(Severity::Info, "Open in a tab", text).closable(false);
        let h = bar.height(p, w);
        draw_infobar(p, &pal, Rect::new(x, y, w, h), &bar, None);
        y += h + 22.0;
    } else if let Some(r) = st.running_of(&s.id).cloned() {
        let bar =
            InfoBar::new(Severity::Warning, "Open elsewhere", Refusal::AlreadyRunning(r).to_string()).closable(false);
        let h = bar.height(p, w);
        draw_infobar(p, &pal, Rect::new(x, y, w, h), &bar, None);
        y += h + 10.0;
        // The way out while it runs elsewhere: a copy, which is a session of its own.
        let label = "Open a copy in CLI";
        let r = Rect::new(x, y, (p.measure(Face::Semibold, 13.0, label) + 32.0).round(), 32.0);
        let t = Target::OpenCopy;
        draw_dialog_button(p, &pal, r, label, false, st.hover == Some(t), st.pressed == Some(t));
        st.hits.push((r, t));
        y += 32.0 + 26.0;
    } else {
        let (slug, app) = st.folder_open(&s.cwd, s.last_app);
        let name = st.account_name(slug);
        let app = if app == App::Cli { "CLI" } else { "Desktop" };
        let buttons = [
            (format!("Open in {app} as {name}"), true, Target::OpenDefault),
            ("Open as\u{2026}".to_string(), false, Target::OpenAs),
        ];
        let mut bx = x;
        for (label, primary, t) in buttons {
            let bw = (p.measure(Face::Semibold, 13.0, &label) + 32.0).round().min(w);
            if bx > x && bx + bw > x + w {
                bx = x;
                y += 40.0;
            }
            let r = Rect::new(bx, y, bw, 32.0);
            draw_dialog_button(p, &pal, r, &label, primary, st.hover == Some(t), st.pressed == Some(t));
            st.hits.push((r, t));
            bx += bw + 8.0;
        }
        y += 32.0 + 26.0;
    }

    y = crate::compact_dialog::draw_history_row(st, p, &s.id, x, y, w, area);
    y = related(st, p, area, Rect::new(x, y, w, 0.0));

    // The conversation, drawn as Chat draws it, in a region that scrolls on its own: as tall as
    // the pane less its heading, so scrolling the details to their end shows it whole.
    p.fill(Rect::new(x, y - 12.0, w, p.px()), pal.line);
    p.text(Face::Semibold, 13.0, x, y + 8.0, "Conversation", pal.text, w);
    y += 22.0;
    let region = Rect::new(area.x + PAD - 12.0, y, area.w - 2.0 * (PAD - 12.0) - 8.0, conversation_height(area.h));
    match st.details_reader.as_mut().filter(|r| r.id == s.id) {
        Some(reader) => {
            let diff = crate::view::look::Diff::from_ansi(&st.term_colours.0.ansi);
            let now = std::time::Instant::now();
            let mouse = if st.dialog.is_none() { st.mouse } else { (-1.0, -1.0) };
            reader.draw(p, &pal, (region, area), diff, mouse, now, false);
        }
        None => {
            p.text(Face::Ui, 13.0, x, y + 8.0, "The conversation could not be read.", pal.muted, w);
        }
    }
    y = region.bottom();
    let content = (y + offset + 8.0 - area.y).max(0.0);
    st.details_scroll.max = (content - area.h).max(0.0);
    for (r, _) in &mut st.hits[first_hit..] {
        let bottom = r.bottom().min(area.bottom());
        r.y = r.y.max(area.y);
        r.h = (bottom - r.y).max(0.0);
    }
    if let Some((track, thumb)) = mdshared_ui::scroll::scrollbar(area, content, area.h, offset) {
        mdshared_ui::scroll::draw_scrollbar(p, &pal, track, thumb, false);
    }
    p.restore(old);
}

/// The details conversation's height in a pane `h` tall: the pane less the room of its heading,
/// and never so short that a message does not fit.
fn conversation_height(h: f32) -> f32 {
    (h - 44.0).max(280.0)
}

impl AppState {
    /// The selected session's conversation, while the Sessions page shows it.
    pub(crate) fn shown_reader(&mut self) -> Option<&mut crate::view::reader::Reader> {
        if self.terminal_shown() || self.tabs.workers_shown() {
            return None;
        }
        let selected = self.selected.as_deref()?;
        self.details_reader.as_mut().filter(|r| r.id == selected)
    }

    /// Reads the shown conversation again (on the window's one-second feed timer).
    pub(crate) fn poll_details(&mut self) -> bool {
        self.shown_reader().map(|r| r.poll()).is_some()
    }

    /// The wheel over the details conversation: true when it scrolled.
    pub(crate) fn details_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        self.shown_reader().is_some_and(|r| r.wheel(x, y, dy))
    }
}

fn related_candidate(selected: Option<&str>, id: &str) -> bool {
    selected.is_none_or(|selected| !selected.eq_ignore_ascii_case(id))
}

fn related(st: &mut AppState, p: &mut Painter, view: Rect, r: Rect) -> f32 {
    let pal = *st.pal();
    let mut y = r.y;
    p.fill(Rect::new(r.x, y - 12.0, r.w, p.px()), pal.line);
    p.text(Face::Semibold, 13.0, r.x, y + 8.0, "Related", pal.text, r.w);
    y += 24.0;
    let start = y;
    for (i, (id, relation)) in st.grouping.related.iter().enumerate() {
        if !related_candidate(st.selected.as_deref(), id) {
            continue;
        }
        let Some(s) = st.by_id.get(id).map(|&j| &st.sessions[j]).filter(|s| st.related_visible(s)) else { continue };
        let hit = Rect::new(r.x, y, r.w, 24.0);
        y += hit.h;
        if hit.bottom() <= view.y || hit.y >= view.bottom() {
            continue;
        }
        let target = Target::Related(i);
        if st.hover == Some(target) {
            p.rbox(hit, 5.0, pal.hover);
        }
        let side =
            if st.is_archived(s) { format!("{} · Archived", relation.label()) } else { relation.label().into() };
        let right = p.text_right(Face::Ui, 11.0, hit.right() - 4.0, hit.cy(), &side, pal.muted);
        p.text(Face::Ui, 12.0, hit.x + 4.0, hit.cy(), st.title_of(s), pal.text, (hit.w - right - 16.0).max(0.0));
        st.hits.push((hit, target));
    }
    if y == start {
        let message = if st.grouping.pending.as_ref().is_some_and(|j| j.running()) {
            "Comparing prompts…"
        } else {
            "No related sessions."
        };
        p.text(Face::Ui, 12.0, r.x, y + 9.0, message, pal.muted, r.w);
        y += 24.0;
    }
    y + 22.0
}

/// The day and minute of a time in ms since 1970, on a clock `offset` minutes east of UTC.
pub fn local_date_time(ms: u64, offset: i32) -> String {
    mdshared_app::fmt::date_time((ms / 1000) as i64, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_details_conversation_lets_go_of_the_keys_when_the_keyboard_moves_away() {
        use crate::state::Focus;
        use mdshared_ui::keys::{KeyName, Mods};
        let mut st = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
        st.apply_scan(crate::demo::scan(&st.config));
        st.select_first();
        let file = st.selected_session().unwrap().file.clone();
        let reader = st.shown_reader().unwrap();
        assert_eq!(reader.file, file, "keyed by the transcript as well as the id");
        reader.focused = true;
        let ctrl = Mods { ctrl: true, ..Default::default() };
        st.focus = Focus::Search;
        assert!(!st.session_key(800.0, &KeyName::Char('a'), ctrl), "Ctrl+A goes to the search field");
        assert!(!st.shown_reader().unwrap().focused, "and the conversation let go");
        st.focus = Focus::List;
        assert!(!st.session_key(800.0, &KeyName::Char('a'), ctrl), "nor does it take them back by itself");
    }

    #[test]
    fn related_rows_never_include_the_selected_session_even_with_different_id_case() {
        assert!(!related_candidate(Some("A-123"), "a-123"));
        assert!(related_candidate(Some("A-123"), "A-124"));
    }

    #[test]
    fn the_time_is_shown_on_the_local_clock() {
        let at = 1_764_191_400_000; // 2025-11-26 21:10 UTC
        assert_eq!(local_date_time(at, 0), "2025-11-26 21:10");
        assert_eq!(local_date_time(at, 60), "2025-11-26 22:10");
        assert_eq!(local_date_time(at, 180), "2025-11-27 00:10");
        assert_eq!(local_date_time(at, -300), "2025-11-26 16:10");
    }
}
