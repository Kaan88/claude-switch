use std::time::{Duration, Instant};

use mdshared_ui::gfx::{Painter, Rect, mix};
use mdshared_ui::theme::Palette;

pub(super) const GUTTER: f32 = 13.0;
const HOLD: Duration = Duration::from_millis(900);
const FADE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(super) struct Visibility {
    offset: Option<f32>,
    active: Option<Instant>,
    hot: bool,
}

impl Visibility {
    pub fn observe(&mut self, offset: f32, hot: bool, now: Instant, motion: bool) -> f32 {
        if self.offset.replace(offset).is_some_and(|old| old != offset) || hot || self.hot {
            self.active = Some(now);
        }
        self.hot = hot;
        let Some(active) = self.active else { return 0.0 };
        let elapsed = now.saturating_duration_since(active);
        if elapsed >= HOLD + FADE {
            self.active = None;
            0.0
        } else if !motion || elapsed <= HOLD {
            1.0
        } else {
            1.0 - (elapsed - HOLD).as_secs_f32() / FADE.as_secs_f32()
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.hot { None } else { self.active.map(|at| at + HOLD + FADE) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Surface {
    Conversation,
    Markdown,
}

#[derive(Clone, Copy)]
pub(super) struct Drag {
    pub surface: Surface,
    pub fraction: f32,
}

#[derive(Clone, Copy)]
pub(super) struct Scrollbar {
    pub track: Rect,
    max: f32,
}

pub(super) enum Press {
    Page(f32),
    Thumb(f32),
}

impl Scrollbar {
    pub fn new(track: Rect, max: f32) -> Option<Self> {
        (track.h > 0.0 && track.w > 0.0 && max > 0.0).then_some(Self { track, max })
    }

    fn thumb(self, offset: f32) -> Rect {
        let r = self.track;
        let max = self.max;
        let h = (r.h * (r.h / (r.h + max))).clamp(18.0_f32.min(r.h / 2.0), r.h);
        let y = r.y + (r.h - h) * offset.clamp(0.0, max) / max;
        Rect::new(r.x, y, r.w, h)
    }

    pub fn press(self, y: f32, offset: f32) -> Press {
        let thumb = self.thumb(offset);
        if y < thumb.y {
            Press::Page((offset - self.track.h).max(0.0))
        } else if y > thumb.bottom() {
            Press::Page((offset + self.track.h).min(self.max))
        } else {
            Press::Thumb(((y - thumb.y) / thumb.h).clamp(0.0, 1.0))
        }
    }

    pub fn drag(self, y: f32, fraction: f32) -> f32 {
        let h = self.thumb(0.0).h;
        let travel = self.track.h - h;
        if travel <= 0.0 {
            return 0.0;
        }
        ((y - self.track.y - fraction * h) / travel).clamp(0.0, 1.0) * self.max
    }

    pub fn draw(self, p: &mut Painter, pal: &Palette, offset: f32, mouse: (f32, f32), dragging: bool, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let hot = dragging || self.track.contains(mouse.0, mouse.1);
        let t = self.track;
        p.rbox(
            Rect::new(t.cx() - 2.5, t.y, 5.0, t.h),
            2.5,
            mix(pal.page, if hot { pal.hover } else { pal.line }, alpha),
        );
        let r = self.thumb(offset);
        let w = if hot { 8.0 } else { 5.0 };
        p.rbox(
            Rect::new(r.cx() - w / 2.0, r.y, w, r.h),
            w / 2.0,
            mix(pal.page, if dragging { pal.accent } else { pal.thumb }, alpha),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_holds_then_fades_and_reduced_motion_snaps() {
        let now = Instant::now();
        let mut v = Visibility::default();
        assert_eq!(v.observe(0.0, false, now, true), 0.0);
        assert_eq!(v.observe(10.0, false, now, true), 1.0);
        assert_eq!(v.observe(10.0, false, now + HOLD, true), 1.0);
        assert!((v.observe(10.0, false, now + HOLD + FADE / 2, true) - 0.5).abs() < 0.01);
        assert_eq!(v.observe(10.0, false, now + HOLD + FADE, true), 0.0);
        assert_eq!(v.observe(10.0, true, now + HOLD + FADE, false), 1.0);
        let start = now + HOLD + FADE;
        assert_eq!(v.observe(10.0, false, start, false), 1.0);
        assert_eq!(v.observe(10.0, false, start + HOLD + FADE / 2, false), 1.0);
        assert_eq!(v.observe(10.0, false, start + HOLD + FADE, false), 0.0);
    }

    #[test]
    fn track_clicks_page_without_starting_a_thumb_drag() {
        let bar = Scrollbar::new(Rect::new(0.0, 40.0, GUTTER, 100.0), 500.0).unwrap();
        assert!(matches!(bar.press(41.0, 250.0), Press::Page(150.0)));
        assert!(matches!(bar.press(139.0, 250.0), Press::Page(350.0)));
        assert!(matches!(bar.press(41.0, 40.0), Press::Page(0.0)));
        assert!(matches!(bar.press(139.0, 440.0), Press::Page(500.0)));
    }

    #[test]
    fn dragging_pins_the_grab_point_and_maps_both_ends_even_for_tiny_viewports() {
        for height in [10.0, 100.0, 2000.0] {
            let bar = Scrollbar::new(Rect::new(0.0, 40.0, GUTTER, height), 1500.0).unwrap();
            let r = bar.thumb(230.0);
            let grab = r.y + r.h * 0.75;
            let Press::Thumb(fraction) = bar.press(grab, 230.0) else { panic!("must hit thumb") };
            assert!((bar.drag(grab, fraction) - 230.0).abs() < 0.1);
            assert_eq!(bar.drag(20.0, fraction), 0.0);
            assert_eq!(bar.drag(40.0 + height + 20.0, fraction), 1500.0);
        }
        assert!(Scrollbar::new(Rect::new(0.0, 0.0, GUTTER, 100.0), 0.0).is_none());
    }
}
