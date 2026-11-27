use mdshared_ui::gfx::Rect;

use super::editor::LINE;

/// The message box shows this many lines even when empty, as Claude Desktop's does.
pub const MIN_LINES: usize = 3;
/// It grows with the draft up to this share of the chat column, then scrolls.
pub const MAX_SHARE: f32 = 0.4;

/// The editor's height for `lines` rows: the first row's centre sits 13 px down (see
/// `composer::editor`), and the editor shows `(h - 4) / LINE` rows.
/// The editor's height in a cramped window: about two lines.
const COMPACT_EDITOR: f32 = 38.0;
/// The conversation height the three-line box gives way to.
const MIN_CONVERSATION: f32 = 80.0;

fn editor_height(lines: usize) -> f32 {
    8.0 + LINE * lines as f32
}

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
    #[cfg(test)]
    pub fn new(area: Rect, panel: bool, drawer: bool, height: Option<f32>, chips: bool, notice: bool) -> Self {
        Self::with_shells(area, panel, drawer, height, chips, notice, false, false, 1)
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
        lines: usize,
    ) -> Self {
        let panel_w = if panel && (area.w >= 900.0 || reveal) { (area.w * 0.24).clamp(252.0, 300.0) } else { 0.0 };
        let main_w = (area.w - panel_w).max(0.0);
        let measure = main_w.min(770.0);
        // Leave room for the two groups and the gap between them. At narrower measures
        // they stay intact on two lines rather than clipping a label. The right group has
        // Details where the chat paints it wide (`paint`); the box's frame and padding take 38 px.
        let wide = area.w >= 900.0 || panel_w > 0.0;
        let stacked = measure < 750.0 || measure - 38.0 < super::composer::toolbar_one_line(wide);
        let chips_h = if chips { 46.0 } else { 0.0 };
        let notice_h = if notice { 30.0 } else { 0.0 };
        let toolbar_h = if stacked { 53.0 } else { 26.0 };
        let shell_h = if shell_chip { 24.0 } else { 0.0 };
        // The box's frame (4 above, 8 below) and padding (7 each side) around its parts.
        let chrome = 26.0 + chips_h + toolbar_h + notice_h + shell_h;
        // The drawer is sized against the smallest box, so a growing draft never resizes the terminal.
        let wanted = chrome + COMPACT_EDITOR;
        let drawer_h = if drawer {
            let available = (area.h - wanted - 70.0 - 24.0).max((area.h - wanted - 24.0).min(area.h * 0.37));
            height.unwrap_or(area.h * 0.4).clamp(0.0, available.max(0.0))
        } else {
            0.0
        };
        let upper = (area.h - drawer_h - if drawer { 24.0 } else { 0.0 }).max(0.0);
        let x = area.x + (main_w - measure) / 2.0;
        // Three lines when the conversation keeps room to read; the old two-line box when cramped.
        let roomy = chrome + editor_height(MIN_LINES);
        let least = if upper - roomy >= MIN_CONVERSATION { roomy } else { wanted };
        let grown = chrome + editor_height(lines);
        let cap = (upper * MAX_SHARE + 12.0).max(least);
        let composer_h = upper.min(grown.clamp(least, cap));
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
            conversation: Rect::new(area.x, area.y, main_w, conversation_h),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn box_for(area: Rect, drawer: bool, lines: usize) -> Layout {
        Layout::with_shells(area, false, drawer, None, false, false, false, false, lines)
    }

    #[test]
    fn the_box_starts_at_three_lines_grows_with_the_draft_to_forty_percent_then_scrolls() {
        let area = Rect::new(0.0, 80.0, 1000.0, 900.0);
        let empty = box_for(area, false, 1);
        let rows = |l: &Layout| ((l.editor.h - 4.0) / LINE) as usize;
        assert_eq!(rows(&empty), MIN_LINES);
        assert_eq!(rows(&box_for(area, false, 3)), MIN_LINES);
        let five = box_for(area, false, 5);
        assert_eq!(rows(&five), 5);
        assert_eq!(five.conversation.bottom(), five.composer.y);
        assert_eq!(five.composer.bottom(), area.bottom());
        let long = box_for(area, false, 400);
        assert!(long.box_frame.h <= area.h * MAX_SHARE + 0.5, "{}", long.box_frame.h);
        assert!(long.box_frame.h >= area.h * MAX_SHARE - LINE);
        assert!(rows(&long) > 5 && rows(&long) < 400);
    }

    #[test]
    fn a_growing_draft_never_resizes_the_drawer_and_a_cramped_window_keeps_the_conversation() {
        let area = Rect::new(0.0, 80.0, 760.0, 400.0);
        let short = box_for(area, true, 1);
        let long = box_for(area, true, 40);
        assert_eq!(short.drawer, long.drawer);
        assert!(short.conversation.h >= MIN_CONVERSATION, "{}", short.conversation.h);
        let tiny = Rect::new(0.0, 0.0, 700.0, 140.0);
        let cramped = box_for(tiny, false, 1);
        assert_eq!(cramped.editor.h, COMPACT_EDITOR);
        assert!(cramped.composer.h <= tiny.h);
    }
}
