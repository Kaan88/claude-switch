use mdshared_ui::gfx::Rect;

#[derive(Clone, Copy, Default, Debug)]
pub struct Layout {
    pub conversation: Rect,
    pub composer: Rect,
    pub editor: Rect,
    pub side: Rect,
    pub drawer: Rect,
    pub grip: Rect,
}

impl Layout {
    pub fn new(area: Rect, panel: bool, drawer: bool, height: Option<f32>, chips: bool) -> Self {
        let drawer_h = if drawer {
            let reserved = if chips { 243.0 } else { 226.0 };
            height.unwrap_or(area.h * 0.4).clamp(0.0, (area.h - reserved).max(area.h * 0.37).max(0.0))
        } else {
            0.0
        };
        let upper = (area.h - drawer_h - if drawer { 24.0 } else { 0.0 }).max(0.0);
        let panel_w = if panel && area.w >= 900.0 { (area.w * 0.24).clamp(222.0, 300.0) } else { 0.0 };
        let main_w = (area.w - panel_w).max(0.0);
        let measure = main_w.min(770.0);
        let x = area.x + (main_w - measure) / 2.0;
        let composer_h = upper.min(if chips { 139.0 } else { 111.0 });
        let conversation_h = upper - composer_h;
        let composer = Rect::new(x, area.y + conversation_h, measure, composer_h);
        let editor = Rect::new(
            x + 12.0,
            composer.y + if chips { 30.0 } else { 2.0 },
            (measure - 24.0).max(0.0),
            (composer_h - if chips { 82.0 } else { 54.0 }).max(0.0),
        );
        Self {
            conversation: Rect::new(x, area.y, measure, conversation_h),
            composer,
            editor,
            side: Rect::new(area.right() - panel_w, area.y, panel_w, upper),
            drawer: Rect::new(area.x, area.bottom() - drawer_h, area.w, drawer_h),
            grip: Rect::new(area.x, area.y + upper, area.w, if drawer { area.h - upper - drawer_h } else { 0.0 }),
        }
    }
}
