//! Drawing and presentation for sanitized usage data.

use std::time::Instant;

use claude_switch_core::usage::{Attempt, Breakdown, Error, Limit, Reading, Slot, now_ms};
use mdshared_app::fmt::Civil;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::overlay::{draw_dialog_button, draw_dialog_button_disabled};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Target};
use crate::topbar::Bar;

const ROW: f32 = 25.0;
const HAIR_W: f32 = 22.0;
const VALUE_INSET: f32 = 8.0;
const NO_SESSION: &str = "Session · — (not reported by Claude)";
const NO_APP_FIGURE: &str = "— = percentage not reported by Claude";

pub fn hairlines(reading: Option<&Reading>) -> Vec<&Limit> {
    let Some(reading) = reading else { return Vec::new() };
    let limits = &reading.limits;
    let slot = |slot| limits.iter().find(|l| l.slot == slot);
    let most = limits.iter().filter(|l| l.slot == Slot::Model).reduce(|a, b| if a.used >= b.used { a } else { b });
    [slot(Slot::Session), slot(Slot::Week), most].into_iter().flatten().collect()
}

pub fn hairline_rect(field: Rect) -> Rect {
    Rect::new(field.right() - 53.0, field.y, 30.0, field.h)
}

fn meter(p: &mut Painter, pal: &Palette, r: Rect, limit: &Limit) {
    let round = r.h / 2.0;
    let warning = limit.warning();
    p.rbox(r, round, if warning { pal.warn_bg } else { pal.line });
    if limit.used > 0.0 || warning {
        let amount = r.w * (limit.used / 100.0) as f32;
        let fill = Rect::new(r.x, r.y, if warning { amount.max(r.h.min(r.w)) } else { amount }, r.h);
        p.rbox(fill, round, if warning { pal.warn } else { pal.muted });
    }
}

pub fn draw_hairlines(p: &mut Painter, pal: &Palette, slot: Rect, limits: &[&Limit]) {
    let y = slot.cy() - (limits.len() as f32 * 5.0 - 2.0) / 2.0;
    for (i, l) in limits.iter().enumerate() {
        meter(p, pal, Rect::new(slot.cx() - HAIR_W / 2.0, y + i as f32 * 5.0, HAIR_W, 3.0), l);
    }
}

fn age(read_at: u64, now: u64) -> String {
    let minutes = now.saturating_sub(read_at) / 60_000;
    match minutes {
        0 => "Read just now".into(),
        1..60 => format!("Read {minutes} min ago"),
        60..1440 => format!("Read {} h ago", minutes / 60),
        _ => format!("Read {} d ago", minutes / 1440),
    }
}

fn reset_at(reset: Option<i64>, now: i64, local: impl Fn(i64) -> Civil) -> String {
    let Some(at) = reset else { return "reset time unavailable".into() };
    let seconds = at.saturating_sub(now);
    match seconds {
        ..=0 => "reset time passed".into(),
        1..60 => "resets in <1 min".into(),
        60..3600 => format!("resets in {} m", seconds / 60),
        3600..86_400 => format!("resets in {} h {} m", seconds / 3600, seconds / 60 % 60),
        _ => {
            let c = local(at);
            let day = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][usize::from(c.weekday)];
            format!("resets {day} {:02}:{:02}", c.hour, c.minute)
        }
    }
}

fn reset(limit: &Limit, now: u64) -> String {
    reset_at(limit.reset, (now / 1000) as i64, crate::usage_fetch::local_time)
}

fn state(reading: Option<&Reading>, attempt: Option<&Attempt>, signed_in: Option<bool>, now: u64) -> String {
    let info = if attempt.is_some_and(|a| a.busy) {
        "Reading usage…".into()
    } else if let Some(err) = attempt.and_then(Attempt::error) {
        match err {
            Error::NoFile | Error::NotLoggedIn => "Not signed in".into(),
            Error::Expired => "Open a session to refresh".into(),
            err => format!("Could not read usage ({err})"),
        }
    } else if reading.is_some_and(|r| r.limits.is_empty()) {
        "No limits reported".into()
    } else if reading.is_none() && signed_in == Some(false) {
        "Not signed in".into()
    } else if reading.is_none() {
        "No usage reading yet".into()
    } else {
        String::new()
    };
    match (reading, info.is_empty()) {
        (Some(r), false) => format!("{} · {info}", age(r.read_at, now)),
        (Some(r), true) => age(r.read_at, now),
        (None, _) => info,
    }
}

pub struct AccountUsage<'a> {
    pub plan: Option<&'a str>,
    pub reading: Option<&'a Reading>,
    pub attempt: Option<&'a Attempt>,
    pub signed_in: Option<bool>,
    pub running: usize,
    pub refresh: bool,
}

impl AccountUsage<'_> {
    fn limits(&self) -> &[Limit] {
        self.reading.map_or(&[], |r| r.limits.as_slice())
    }

    pub fn height(&self) -> f32 {
        53.0 + self.limit_rows() as f32 * (ROW + 4.0) + breakdown_height(breakdown(self.reading))
    }

    fn limit_rows(&self) -> usize {
        self.limits().len() + usize::from(missing_session(self.reading))
    }

    pub fn draw(&self, p: &mut Painter, pal: &Palette, r: Rect, button: Rect, now: u64) {
        let plan = self.plan.unwrap_or("Plan unavailable");
        let noun = if self.running == 1 { "session" } else { "sessions" };
        let header_w = button.x - r.x - 12.0;
        p.text(
            Face::Semibold,
            12.0,
            r.x,
            r.y + 10.0,
            &format!("{plan} · {} {noun} running", self.running),
            pal.text,
            header_w,
        );
        p.text(
            Face::Ui,
            12.0,
            r.x,
            r.y + 30.0,
            &state(self.reading, self.attempt, self.signed_in, now),
            pal.muted,
            header_w,
        );
        let missing = missing_session(self.reading);
        if missing {
            p.text(Face::Ui, 12.0, r.x, r.y + 51.0, NO_SESSION, pal.muted, r.w);
        }
        for (i, limit) in self.limits().iter().enumerate() {
            let y = r.y + 51.0 + (i + usize::from(missing)) as f32 * (ROW + 4.0);
            let label = format!("{} · {:.0}% used · {}", limit.label, limit.used, reset(limit, now));
            p.text(Face::Ui, 12.0, r.x, y, &label, pal.text, r.w);
            meter(p, pal, Rect::new(r.x, y + 9.0, r.w, 4.0), limit);
        }
        draw_breakdown(p, pal, r.x, r.y + 48.0 + self.limit_rows() as f32 * (ROW + 4.0), r.w, breakdown(self.reading));
    }
}

fn missing_session(reading: Option<&Reading>) -> bool {
    reading.is_some_and(|r| !r.limits.iter().any(|l| l.slot == Slot::Session))
}

fn breakdown(reading: Option<&Reading>) -> &[Breakdown] {
    reading.map_or(&[], |r| r.breakdown.as_slice())
}

fn breakdown_height(rows: &[Breakdown]) -> f32 {
    if rows.is_empty() { 0.0 } else { 25.0 + (rows.len() + usize::from(has_missing_figure(rows))) as f32 * 18.0 }
}

fn has_missing_figure(rows: &[Breakdown]) -> bool {
    rows.iter().any(|row| row.percent.is_none())
}

fn breakdown_value(row: &Breakdown) -> String {
    row.percent.map_or_else(|| "—".into(), |value| format!("{value:.0}%"))
}

fn draw_breakdown(p: &mut Painter, pal: &Palette, x: f32, y: f32, w: f32, rows: &[Breakdown]) {
    if rows.is_empty() {
        return;
    }
    p.text(Face::Semibold, 12.0, x, y + 12.0, "This week by app", pal.muted, w);
    for (i, row) in rows.iter().enumerate() {
        let y = y + 32.0 + i as f32 * 18.0;
        p.text(Face::Ui, 12.0, x, y, &row.label, pal.muted, w - 74.0);
        p.text_right(Face::Ui, 12.0, x + w - VALUE_INSET, y, &breakdown_value(row), pal.muted);
    }
    if has_missing_figure(rows) {
        p.text(Face::Ui, 12.0, x, y + 32.0 + rows.len() as f32 * 18.0, NO_APP_FIGURE, pal.muted, w);
    }
}

fn popup_card(window: Rect, field: Rect, height: f32) -> Rect {
    let w = 355.0_f32.min((window.w - 16.0).max(0.0));
    let x = field.x.min(window.right() - 8.0 - w);
    Rect::new(x, field.bottom(), w, height)
}

pub fn draw_popup(st: &mut AppState, p: &mut Painter, window: Rect, now: Instant) {
    if !st.usage.popup.open
        || !st.config.show_usage
        || st.dialog.is_some()
        || st.menu.is_some()
        || st.accounts_dd.is_open()
        || st.palette.is_some()
        || st.picture_overlay()
    {
        return;
    }
    let pal = *st.pal();
    let field = Bar::layout(window.w).account;
    let slug = &st.config.current;
    let reading = st.usage.reading(slug);
    let limits = reading.map_or(&[][..], |r| r.limits.as_slice());
    let status = st.statuses.get(slug);
    let plan = status.and_then(|s| s.plan.as_deref()).unwrap_or("Plan unavailable");
    let can_refresh = !st.source.is_demo() && st.usage.can_refresh(slug, now);
    let clock = now_ms();
    let switch = st.usage_switch_account(clock).map(|a| format!("Open this session as {}", a.name));
    let extra = if switch.is_some() { 36.0 } else { 0.0 };
    let rows = breakdown(reading);
    let missing = missing_session(reading);
    let limit_height = (limits.len() + usize::from(missing)) as f32 * ROW;
    let content = limit_height + breakdown_height(rows);
    let view_h = content.min((window.h - field.bottom() - 108.0 - extra).max(ROW));
    let card = popup_card(window, field, 90.0 + view_h + extra);
    let x = card.x + 14.0;
    let w = card.w - 28.0;
    p.frame(card, st.appearance.radius, pal.popup, pal.popup_line);
    p.text(Face::Semibold, 13.0, x, card.y + 20.0, plan, pal.text, w);
    let view = Rect::new(x, card.y + 34.0, w, view_h);
    let scroll = st.usage.popup.scroll.clamp(0.0, (content - view_h).max(0.0));
    let old = p.clip(view);
    if missing {
        p.text(Face::Ui, 12.0, x, view.y + ROW / 2.0 - scroll, NO_SESSION, pal.muted, w);
    }
    for (i, limit) in limits.iter().enumerate() {
        let y = view.y + (i + usize::from(missing)) as f32 * ROW + ROW / 2.0 - scroll;
        let description = format!("{:.0}% left · {}", 100.0 - limit.used, reset(limit, clock));
        p.text(Face::Ui, 12.0, x, y, &limit.label, pal.text, 132.0);
        p.text(
            Face::Ui,
            12.0,
            x + 138.0,
            y,
            &description,
            if limit.warning() { pal.warn } else { pal.muted },
            w - 138.0,
        );
    }
    draw_breakdown(p, &pal, x, view.y + limit_height - scroll, w, rows);
    p.restore(old);
    let state = state(reading, st.usage.attempt(slug), status.map(|s| s.cli_logged_in), clock);
    st.usage.popup.scroll = scroll;
    let bottom = card.bottom() - extra;
    p.text(Face::Ui, 12.0, x, bottom - 23.0, &state, pal.muted, w - 93.0);
    let button = Rect::new(card.right() - 98.0, bottom - 37.0, 84.0, 28.0);
    st.hits.push((card, Target::UsagePopup));
    if can_refresh {
        draw_dialog_button(
            p,
            &pal,
            button,
            "Refresh",
            false,
            st.hover == Some(Target::UsageRefresh),
            st.pressed == Some(Target::UsageRefresh),
        );
        st.hits.push((button, Target::UsageRefresh));
    } else {
        draw_dialog_button_disabled(p, &pal, button, "Refresh", false);
    }
    if let Some(label) = switch {
        let button = Rect::new(x, bottom - 2.0, w, 28.0);
        draw_dialog_button(
            p,
            &pal,
            button,
            &label,
            false,
            st.hover == Some(Target::UsageOpenAs),
            st.pressed == Some(Target::UsageOpenAs),
        );
        st.hits.push((button, Target::UsageOpenAs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_used_model_is_third_and_missing_limits_do_not_make_placeholders() {
        let limits = [
            ("five_hour", 42.0),
            ("seven_day", 60.0),
            ("seven_day_opus", 80.0),
            ("seven_day_sonnet", 91.0),
            ("extra", 99.0),
        ]
        .into_iter()
        .map(|(key, used)| Limit::legacy(key, used, None))
        .collect();
        let reading = Reading { read_at: 0, limits, breakdown: vec![] };
        assert_eq!(
            hairlines(Some(&reading)).iter().map(|l| l.label.as_str()).collect::<Vec<_>>(),
            ["Session", "Week", "Week, Sonnet"]
        );
        assert!(hairlines(None).is_empty());
    }

    #[test]
    fn missing_session_is_explained_after_a_read_and_missing_app_values_are_not_zero() {
        let only_week = Reading { read_at: 0, limits: vec![Limit::legacy("seven_day", 69.0, None)], breakdown: vec![] };
        assert!(missing_session(Some(&only_week)));
        let mut with_session = only_week;
        with_session.limits.push(Limit::legacy("five_hour", 0.0, None));
        assert!(!missing_session(Some(&with_session)));
        assert!(!missing_session(None));

        let missing = Breakdown { label: "Cowork".into(), percent: None };
        let zero = Breakdown { label: "Chats".into(), percent: Some(0.0) };
        assert_eq!(breakdown_value(&missing), "—");
        assert_eq!(breakdown_value(&zero), "0%");
        assert_eq!(breakdown_height(&[zero, missing]), 25.0 + 3.0 * 18.0);
    }

    #[test]
    fn usage_popup_keeps_its_right_edge_inside_a_narrow_window() {
        for width in [300.0, 467.0, 760.0] {
            let window = Rect::new(0.0, 0.0, width, 500.0);
            let card = popup_card(window, Bar::layout(width).account, 160.0);
            assert!(card.x >= 8.0 && card.right() <= width - 8.0, "{card:?}");
            assert_eq!(card.w, 355.0_f32.min(width - 16.0));
        }
    }

    #[test]
    fn reset_threshold_and_age_and_expired_state() {
        let local = |s| Civil::from_unix(s, 120);
        assert_eq!(reset_at(Some(7800), 0, local), "resets in 2 h 10 m");
        assert_eq!(reset_at(Some(86_400), 0, local), "resets Fri 02:00");
        assert_eq!(reset_at(None, 0, local), "reset time unavailable");
        let mut attempt = Attempt::default();
        attempt.finish(Instant::now(), Err(Error::Expired));
        let reading = Reading { read_at: 0, limits: vec![], breakdown: vec![] };
        assert_eq!(
            state(Some(&reading), Some(&attempt), Some(true), 180_000),
            "Read 3 min ago · Open a session to refresh"
        );
    }
}
