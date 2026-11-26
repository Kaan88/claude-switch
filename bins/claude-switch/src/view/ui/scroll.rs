use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::theme::Palette;

pub(super) const GUTTER: f32 = 13.0;

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

    pub fn draw(self, p: &mut Painter, pal: &Palette, offset: f32, mouse: (f32, f32), dragging: bool) {
        let hot = dragging || self.track.contains(mouse.0, mouse.1);
        let t = self.track;
        p.rbox(Rect::new(t.cx() - 2.5, t.y, 5.0, t.h), 2.5, if hot { pal.hover } else { pal.line });
        let r = self.thumb(offset);
        let w = if hot { 8.0 } else { 5.0 };
        p.rbox(Rect::new(r.cx() - w / 2.0, r.y, w, r.h), w / 2.0, if dragging { pal.accent } else { pal.thumb });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
