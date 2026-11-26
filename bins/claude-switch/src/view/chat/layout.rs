use mdshared_ui::gfx::Rect;

#[derive(Clone, Copy, Default, Debug)]
pub struct Layout {
    pub conversation: Rect,
    pub composer: Rect,
    pub box_frame: Rect,
    pub editor: Rect,
    pub attachments: Rect,
    pub notice: Rect,
    pub toolbar: Rect,
    pub suggestions: Rect,
    pub stacked: bool,
    pub side: Rect,
    pub drawer: Rect,
    pub grip: Rect,
    pub shell_chip: Rect,
}

impl Layout {
    pub fn new(area: Rect, panel: bool, drawer: bool, height: Option<f32>, chips: bool, notice: bool) -> Self {
        Self::with_shells(area, panel, drawer, height, chips, notice, false, false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_shells(
        area: Rect,
        panel: bool,
        drawer: bool,
        height: Option<f32>,
        chips: bool,
        notice: bool,
        shell_chip: bool,
        reveal: bool,
    ) -> Self {
        let panel_w = if panel && (area.w >= 900.0 || reveal) { (area.w * 0.24).clamp(252.0, 300.0) } else { 0.0 };
        let main_w = (area.w - panel_w).max(0.0);
        let measure = main_w.min(770.0);
        // Leave room for the two groups and the gap between them. At narrower measures
        // they stay intact on two lines rather than clipping a label.
        let stacked = measure < 750.0;
        let chips_h = if chips { 46.0 } else { 0.0 };
        let notice_h = if notice { 30.0 } else { 0.0 };
        let toolbar_h = if stacked { 53.0 } else { 26.0 };
        let shell_h = if shell_chip { 24.0 } else { 0.0 };
        let wanted = 64.0 + chips_h + toolbar_h + notice_h + shell_h;
        let drawer_h = if drawer {
            let available = (area.h - wanted - 70.0 - 24.0).max((area.h - wanted - 24.0).min(area.h * 0.37));
            height.unwrap_or(area.h * 0.4).clamp(0.0, available.max(0.0))
        } else {
            0.0
        };
        let upper = (area.h - drawer_h - if drawer { 24.0 } else { 0.0 }).max(0.0);
        let x = area.x + (main_w - measure) / 2.0;
        let composer_h = upper.min(wanted);
        let conversation_h = upper - composer_h;
        let composer = Rect::new(x, area.y + conversation_h, measure, composer_h);
        let box_frame = Rect::new(x + 12.0, composer.y + 4.0, (measure - 24.0).max(0.0), (composer_h - 12.0).max(0.0));
        let inner = box_frame.inset(7.0);
        let attachments = Rect::new(inner.x, inner.y, inner.w, chips_h);
        let toolbar = Rect::new(inner.x, inner.bottom() - toolbar_h, inner.w, toolbar_h);
        let notice = Rect::new(inner.x, toolbar.y - notice_h, inner.w, notice_h);
        let shell_chip = Rect::new(inner.x, notice.y - shell_h, inner.w, shell_h);
        let editor = Rect::new(inner.x, attachments.bottom(), inner.w, (shell_chip.y - attachments.bottom()).max(0.0));
        let popup_h = (conversation_h - 4.0).clamp(0.0, 224.0);
        Self {
            conversation: Rect::new(x, area.y, measure, conversation_h),
            composer,
            box_frame,
            editor,
            toolbar,
            attachments,
            notice,
            suggestions: Rect::new(box_frame.x, box_frame.y - popup_h - 4.0, box_frame.w, popup_h),
            stacked,
            side: Rect::new(area.right() - panel_w, area.y, panel_w, upper),
            drawer: Rect::new(area.x, area.bottom() - drawer_h, area.w, drawer_h),
            grip: Rect::new(area.x, area.y + upper, area.w, if drawer { area.h - upper - drawer_h } else { 0.0 }),
            shell_chip,
        }
    }
}
