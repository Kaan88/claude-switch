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

pub fn draw(st: &mut AppState, p: &mut Painter, area: Rect) {
    let pal = *st.pal();
    p.fill(area, pal.page);
    let Some(s) = st.selected_session().cloned() else {
        let text = if st.scanned { "Select a session on the left." } else { "" };
        p.text_center(Face::Ui, 13.0, Rect::new(area.x, area.y + 40.0, area.w, 24.0), text, pal.muted);
        return;
    };
    let old = p.clip(area);
    let x = area.x + PAD;
    let w = (area.w - 2.0 * PAD).max(40.0);
    let mut y = area.y + 22.0;
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
        facts.push(("Archived", "yes, in the desktop app".into()));
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

    // The last messages.
    p.fill(Rect::new(x, y - 12.0, w, p.px()), pal.line);
    p.text(Face::Semibold, 13.0, x, y + 8.0, "Last messages", pal.text, w);
    y += 26.0;
    match &st.preview {
        Some((id, lines)) if *id == s.id => {
            if lines.is_empty() {
                p.text(Face::Ui, 13.0, x, y + 8.0, "Nothing to show.", pal.muted, w);
            }
            for line in lines {
                if y > area.bottom() {
                    break;
                }
                let (who, c) = if line.from_user { ("You", pal.accent) } else { ("Claude", pal.muted) };
                p.text(Face::Semibold, 12.0, x, y + 8.0, who, c, w);
                y += 18.0;
                y += p.paragraph_wrapped(Face::Ui, 13.0, x, y, w, &line.text, pal.text, Wrap::Anywhere) + 12.0;
            }
        }
        _ => {
            p.text(Face::Ui, 13.0, x, y + 8.0, "Reading the conversation\u{2026}", pal.muted, w);
        }
    }
    p.restore(old);
}

/// The day and minute of a time in ms since 1970, on a clock `offset` minutes east of UTC.
pub fn local_date_time(ms: u64, offset: i32) -> String {
    mdshared_app::fmt::date_time((ms / 1000) as i64, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_is_shown_on_the_local_clock() {
        let at = 1_764_191_400_000; // 2025-11-26 21:10 UTC
        assert_eq!(local_date_time(at, 0), "2025-11-26 21:10");
        assert_eq!(local_date_time(at, 60), "2025-11-26 22:10");
        assert_eq!(local_date_time(at, 180), "2025-11-27 00:10");
        assert_eq!(local_date_time(at, -300), "2025-11-26 16:10");
    }
}
