//! Drawing and presentation for sanitized usage data.

use std::time::Instant;

use claude_switch_core::usage::{Attempt, Error, Limit, Reading, now_ms};
use mdshared_app::fmt::Civil;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::overlay::{draw_dialog_button, draw_dialog_button_disabled};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Target};
use crate::topbar::Bar;

const ROW: f32 = 25.0;
const HAIR_W: f32 = 22.0;

pub fn hairlines(reading: Option<&Reading>) -> Vec<&Limit> {
    let Some(reading) = reading else { return Vec::new() };
    let limits = &reading.limits;
    let key = |name| limits.iter().find(|l| l.key == name);
    let most = limits.iter().filter(|l| l.model()).reduce(|a, b| if a.used >= b.used { a } else { b });
    [key("five_hour"), key("seven_day"), most].into_iter().flatten().collect()
}

pub fn hairline_rect(field: Rect) -> Rect {
    Rect::new(field.right() - 53.0, field.y, 30.0, field.h)
}

fn meter(p: &mut Painter, pal: &Palette, r: Rect, used: f64) {
    let round = r.h / 2.0;
    p.rbox(r, round, pal.line);
    if used > 0.0 {
        let fill = Rect::new(r.x, r.y, r.w * (used / 100.0) as f32, r.h);
        p.rbox(fill, round, if used >= 80.0 { pal.warn } else { pal.muted });
    }
}

pub fn draw_hairlines(p: &mut Painter, pal: &Palette, slot: Rect, limits: &[&Limit]) {
    let y = slot.cy() - (limits.len() as f32 * 5.0 - 2.0) / 2.0;
    for (i, l) in limits.iter().enumerate() {
        meter(p, pal, Rect::new(slot.cx() - HAIR_W / 2.0, y + i as f32 * 5.0, HAIR_W, 3.0), l.used);
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
        53.0 + self.limits().len() as f32 * (ROW + 4.0)
    }

    pub fn draw(&self, p: &mut Painter, pal: &Palette, r: Rect, now: u64) -> Rect {
        let plan = self.plan.unwrap_or("Plan unavailable");
        let noun = if self.running == 1 { "session" } else { "sessions" };
        p.text(
            Face::Semibold,
            12.0,
            r.x,
            r.y + 10.0,
            &format!("{plan} · {} {noun} running", self.running),
            pal.text,
            r.w - 100.0,
        );
        p.text(
            Face::Ui,
            12.0,
            r.x,
            r.y + 30.0,
            &state(self.reading, self.attempt, self.signed_in, now),
            pal.muted,
            r.w - 100.0,
        );
        for (i, limit) in self.limits().iter().enumerate() {
            let y = r.y + 51.0 + i as f32 * (ROW + 4.0);
            let label = format!("{} · {:.0}% used · {}", limit.label(), limit.used, reset(limit, now));
            p.text(Face::Ui, 12.0, r.x, y, &label, pal.text, r.w);
            meter(p, pal, Rect::new(r.x, y + 9.0, r.w, 4.0), limit.used);
        }
        Rect::new(r.right() - 86.0, r.y + 6.0, 86.0, 28.0)
    }
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
    let content = limits.len() as f32 * ROW;
    let view_h = content.min((window.h - field.bottom() - 108.0).max(ROW));
    let card = Rect::new(field.x, field.bottom(), 355.0, 90.0 + view_h);
    let x = card.x + 14.0;
    let w = card.w - 28.0;
    p.frame(card, st.appearance.radius, pal.popup, pal.popup_line);
    p.text(Face::Semibold, 13.0, x, card.y + 20.0, plan, pal.text, w);
    let view = Rect::new(x, card.y + 34.0, w, view_h);
    let scroll = st.usage.popup.scroll.clamp(0.0, (content - view_h).max(0.0));
    let old = p.clip(view);
    for (i, limit) in limits.iter().enumerate() {
        let y = view.y + i as f32 * ROW + ROW / 2.0 - scroll;
        let description = format!("{:.0}% left · {}", 100.0 - limit.used, reset(limit, clock));
        p.text(Face::Ui, 12.0, x, y, &limit.label(), pal.text, 132.0);
        p.text(Face::Ui, 12.0, x + 138.0, y, &description, pal.muted, w - 138.0);
    }
    p.restore(old);
    let state = state(reading, st.usage.attempt(slug), status.map(|s| s.cli_logged_in), clock);
    st.usage.popup.scroll = scroll;
    let bottom = card.bottom();
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
        .map(|(key, used)| Limit { key: key.into(), used, reset: None })
        .collect();
        let reading = Reading { read_at: 0, limits };
        assert_eq!(
            hairlines(Some(&reading)).iter().map(|l| l.key.as_str()).collect::<Vec<_>>(),
            ["five_hour", "seven_day", "seven_day_sonnet"]
        );
        assert!(hairlines(None).is_empty());
    }

    #[test]
    fn reset_threshold_and_age_and_expired_state() {
        let local = |s| Civil::from_unix(s, 120);
        assert_eq!(reset_at(Some(7800), 0, local), "resets in 2 h 10 m");
        assert_eq!(reset_at(Some(86_400), 0, local), "resets Fri 02:00");
        assert_eq!(reset_at(None, 0, local), "reset time unavailable");
        let mut attempt = Attempt::default();
        attempt.finish(Instant::now(), Err(Error::Expired));
        let reading = Reading { read_at: 0, limits: vec![] };
        assert_eq!(
            state(Some(&reading), Some(&attempt), Some(true), 180_000),
            "Read 3 min ago · Open a session to refresh"
        );
    }
}
