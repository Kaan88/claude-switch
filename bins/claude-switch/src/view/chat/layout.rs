use mdshared_ui::gfx::Rect;

use super::editor::LINE;
use super::welcome::{GREETING, STARTS};
use crate::view::look::INSET;

/// The widest the composer's text and a card's get, whatever the reading width.
const MAX_MEASURE: f32 = 900.0;
/// The box's padding beside its parts: with the editor's own 8 px the draft sits `INSET` inside.
const BOX_PAD: f32 = 8.0;

/// The message box shows this many lines even when empty, as Claude Desktop's does.
pub const MIN_LINES: usize = 3;
/// It grows with the draft up to this share of the chat column, then scrolls.
pub const MAX_SHARE: f32 = 0.4;

/// The editor's height for `lines` rows: the first row's centre sits 13 px down (see
/// `composer::editor`), and the editor shows `(h - 4) / LINE` rows.
/// The editor's height in a cramped window: about two lines.
const COMPACT_EDITOR: f32 = 46.0;
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
    /// A new session: the room over the box for the greeting, and the row under it for the
    /// starts. Both empty otherwise.
    pub greeting: Rect,
    pub starts: Rect,
    /// The window has room for the Details panel: a click that opens it can show it.
    pub panel_room: bool,
}

impl Layout {
    #[cfg(test)]
    pub fn new(area: Rect, panel: bool, drawer: bool, height: Option<f32>, chips: bool, notice: bool) -> Self {
        let left = super::composer::left_group(&super::composer::DEFAULT_CHIPS);
        Self::with_shells(area, panel, drawer, height, chips, notice, false, false, 1, left, None, false)
    }

    /// The Details panel's width in `area`: nothing when it is hidden or the window too narrow.
    pub fn panel_width(area: Rect, panel: bool, reveal: bool) -> f32 {
        if panel && Self::panel_room(area, reveal) { (area.w * 0.24).clamp(252.0, 300.0) } else { 0.0 }
    }

    /// Whether the Details panel can show in `area`: a wide window, or a section that reveals it.
    pub fn panel_room(area: Rect, reveal: bool) -> bool {
        area.w >= 900.0 || reveal
    }

    /// The reading column of a conversation `main_w` wide when the conversation pane has not
    /// said where its text goes: the medium reading width, centred.
    fn default_column(x: f32, main_w: f32) -> (f32, f32) {
        let room = (main_w - 2.0 * (12.0 + INSET) - 13.0).max(1.0);
        let w = room.min(720.0).floor();
        ((x + 12.0 + INSET + (room - w) / 2.0).round(), w)
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
        left: f32,
        column: Option<(f32, f32)>,
        welcome: bool,
    ) -> Self {
        let panel_w = Self::panel_width(area, panel, reveal);
        let main_w = (area.w - panel_w).max(0.0);
        // One column: the box is the conversation's reading column plus its padding on each
        // side, so the draft starts where the messages do. At the full reading width it stops
        // at `MAX_MEASURE`.
        let (column_x, column_w) = column.unwrap_or_else(|| Self::default_column(area.x, main_w));
        let text_w = column_w.min(MAX_MEASURE);
        let measure = (text_w + 2.0 * INSET).min(main_w);
        // Leave room for the two groups and the gap between them. At narrower measures
        // they stay intact on two lines rather than clipping a label. The right group has
        // Details where the chat paints it wide (`paint`); the box's padding takes 16 px.
        let wide = area.w >= 900.0 || panel_w > 0.0;
        let stacked = measure - 2.0 * BOX_PAD < super::composer::toolbar_one_line(wide, left);
        let chips_h = if chips { 46.0 } else { 0.0 };
        let notice_h = if notice { 30.0 } else { 0.0 };
        let toolbar_h = if stacked { 64.0 } else { 34.0 };
        let shell_h = if shell_chip { 24.0 } else { 0.0 };
        // The box's margins (4 above, 12 below, where its shadow falls) and padding (6 above,
        // 8 below) around its parts.
        let chrome = 30.0 + chips_h + toolbar_h + notice_h + shell_h;
        // The drawer is sized against the smallest box, so a growing draft never resizes the terminal.
        let wanted = chrome + COMPACT_EDITOR;
        let drawer_h = if drawer {
            let available = (area.h - wanted - 70.0 - 24.0).max((area.h - wanted - 24.0).min(area.h * 0.37));
            height.unwrap_or(area.h * 0.4).clamp(0.0, available.max(0.0))
        } else {
            0.0
        };
        let upper = (area.h - drawer_h - if drawer { 24.0 } else { 0.0 }).max(0.0);
        let x = (column_x + (column_w - text_w) / 2.0 - INSET).max(area.x).min(area.x + (main_w - measure).max(0.0));
        // Three lines when the conversation keeps room to read; the old two-line box when cramped.
        let roomy = chrome + editor_height(MIN_LINES);
        let least = if upper - roomy >= MIN_CONVERSATION { roomy } else { wanted };
        let grown = chrome + editor_height(lines);
        let cap = (upper * MAX_SHARE + 16.0).max(least);
        let composer_h = upper.min(grown.clamp(least, cap));
        // A new session has its box in the middle of the window, under a greeting and over a
        // row of starts, where the window has the room for them.
        let group = GREETING + composer_h + STARTS;
        let welcome = welcome && upper >= group + 24.0;
        let conversation_h = if welcome { ((upper - group) * 0.42).round() + GREETING } else { upper - composer_h };
        let composer = Rect::new(area.x, area.y + conversation_h, main_w, composer_h);
        let box_frame = Rect::new(x, composer.y + 4.0, measure, (composer_h - 16.0).max(0.0));
        let inner = Rect::new(
            box_frame.x + BOX_PAD,
            box_frame.y + 6.0,
            (box_frame.w - 2.0 * BOX_PAD).max(0.0),
            (box_frame.h - 14.0).max(0.0),
        );
        let attachments = Rect::new(inner.x, inner.y, inner.w, chips_h);
        let toolbar = Rect::new(inner.x, inner.bottom() - toolbar_h, inner.w, toolbar_h);
        let notice = Rect::new(inner.x, toolbar.y - notice_h, inner.w, notice_h);
        let shell_chip = Rect::new(inner.x, notice.y - shell_h, inner.w, shell_h);
        let editor = Rect::new(inner.x, attachments.bottom(), inner.w, (shell_chip.y - attachments.bottom()).max(0.0));
        let popup_h = (conversation_h - 4.0).clamp(0.0, 224.0);
        let (greeting, starts) = if welcome {
            (
                Rect::new(box_frame.x, area.y, box_frame.w, conversation_h),
                Rect::new(box_frame.x, box_frame.bottom(), box_frame.w, STARTS),
            )
        } else {
            Default::default()
        };
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
            greeting,
            starts,
            panel_room: Self::panel_room(area, reveal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_for(area: Rect, drawer: bool, lines: usize) -> Layout {
        let left = super::super::composer::left_group(&super::super::composer::DEFAULT_CHIPS);
        Layout::with_shells(area, false, drawer, None, false, false, false, false, lines, left, None, false)
    }

    #[test]
    fn a_new_sessions_box_is_in_the_middle_with_room_for_the_greeting_and_the_starts() {
        let left = super::super::composer::left_group(&super::super::composer::DEFAULT_CHIPS);
        let at =
            |area: Rect| Layout::with_shells(area, false, false, None, false, false, false, false, 1, left, None, true);
        let area = Rect::new(0.0, 80.0, 1100.0, 800.0);
        let l = at(area);
        let usual = box_for(area, false, 1);
        assert_eq!(
            (l.box_frame.x, l.box_frame.w, l.box_frame.h),
            (usual.box_frame.x, usual.box_frame.w, usual.box_frame.h)
        );
        assert!(l.box_frame.bottom() < usual.box_frame.y, "the box is well above the foot of the window");
        assert_eq!(l.conversation.bottom(), l.composer.y);
        assert!(l.greeting.h >= GREETING && l.greeting.bottom() == l.composer.y);
        assert_eq!((l.starts.y, l.starts.h), (l.box_frame.bottom(), STARTS));
        assert!(l.starts.bottom() <= area.bottom());
        // A window too short for all of it keeps the box at its foot, as in any session.
        let short = Rect::new(0.0, 80.0, 1100.0, 300.0);
        let l = at(short);
        assert_eq!(l.composer.bottom(), short.bottom());
        assert_eq!((l.greeting.h, l.starts.h), (0.0, 0.0));
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
