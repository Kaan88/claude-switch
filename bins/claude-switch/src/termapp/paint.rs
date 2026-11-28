//! The same terminal surface draws the full tab and the Chat drawer.

use super::{AppState, CURSOR, Cx, Msg, Rect, Target};

/// The animator's name for the find box's caret.
const FIND_CARET: &str = "term-find-caret";
use crate::term::Term;
use claude_switch_core::config::{AppearancePrefs, TerminalCursor};
use mdshared_term::{CursorShape, Screen};
use mdshared_ui::gfx::Painter;
use mdshared_ui::scroll::draw_scrollbar;
use mdshared_ui::termview::{self, BackgroundOpacity, CellFont, Metrics, Scheme, View};
use mdshared_ui::theme::Palette;
use std::ops::Range;

fn default_shape(prefs: &AppearancePrefs) -> CursorShape {
    match prefs.cursor_shape {
        TerminalCursor::Block => CursorShape::Block,
        TerminalCursor::Bar => CursorShape::Bar,
        TerminalCursor::Underline => CursorShape::Underline,
    }
}

fn cursor_can_blink(screen: &Screen, view: View, enabled: bool) -> bool {
    enabled
        && view.is_focused()
        && screen.cursor().visible
        && (!screen.cursor_style_is_explicit() || screen.cursor().blinking)
        && view.cursor_row(screen).is_some()
}

fn visible_notice(t: &Term, view: View) -> Range<usize> {
    let top = view.top_line(&t.screen);
    let row = |n: u64| n.saturating_sub(top).min(t.screen.rows() as u64) as usize;
    row(t.ended_lines.start)..row(t.ended_lines.end)
}

impl AppState {
    /// Draws the shown tab, including the scrollback and the app-owned end notice.
    pub fn draw_terminal(&mut self, cx: &mut Cx<'_, Msg>, p: &mut Painter, area: Rect) {
        let focused = self.window_focused && self.find_focused();
        let blink = self.blink;
        let caret = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut()).map(|find| {
            let on = find.field.blink_caret(cx.anim(), FIND_CARET, blink, focused);
            if focused && blink.is_some() && cx.anim().animates() {
                cx.animating(crate::termfind::Bar::layout(area).field);
            }
            on
        });
        self.paint_terminal(p, area, caret.unwrap_or(false), |screen, view, blink, row| {
            let shown = termview::cursor_blinks_on(cx.anim(), CURSOR, blink, screen, view);
            if blink.is_some()
                && cx.anim().animates()
                && let Some(row) = row
            {
                cx.animating(row);
            }
            shown
        });
    }

    /// [`draw_terminal`](Self::draw_terminal) without a window, the cursor shown when it is.
    #[cfg(test)]
    pub fn draw_terminal_still(&mut self, p: &mut Painter, area: Rect) {
        self.paint_terminal(p, area, true, |_, _, _, _| true);
    }

    fn paint_terminal(
        &mut self,
        p: &mut Painter,
        area: Rect,
        caret_on: bool,
        cursor: impl FnOnce(&Screen, View, Option<std::time::Duration>, Option<Rect>) -> bool,
    ) {
        let pal = *self.pal();
        let scheme = self.terminal_scheme();
        if let Some(tab) = self.tabs.shown_mut() {
            let t = &mut tab.term;
            let changes = t.changes;
            if let Some(find) = &mut t.find {
                find.update(&t.screen, changes, t.screen.end_line());
            }
        }
        let (Some(metrics), Some(tab)) = (self.metrics, self.tabs.shown()) else {
            p.fill(area, scheme.background);
            return;
        };
        let inner = self.term_area;
        let t = &tab.term;
        let prefs = &self.config.appearance;
        let view = t.view(self.terminal_focused()).cursor_preference(default_shape(prefs), prefs.cursor_blink);
        let blink = self.blink.filter(|_| cursor_can_blink(&t.screen, view, prefs.cursor_blink));
        let show_cursor =
            cursor(&t.screen, view, blink, view.cursor_row(&t.screen).map(|row| metrics.row_rect(inner, row)));
        let view = view.cursor_shown(show_cursor);
        let surface = Surface { area, inner, metrics, scheme, pal };
        surface.draw(p, &mut self.cell_font, t, view, prefs.terminal_opacity);
        self.hits.push((area, Target::Terminal));
        self.draw_find_bar(p, area, caret_on);
    }
}

struct Surface {
    area: Rect,
    inner: Rect,
    metrics: Metrics,
    scheme: Scheme,
    pal: Palette,
}

impl Surface {
    fn draw(&self, p: &mut Painter, font: &mut CellFont, t: &Term, view: View, percent: u8) {
        let Self { area, inner, metrics, scheme, pal } = *self;
        let sel = t.selection.as_ref().filter(|s| !s.is_empty());
        let translucent = percent < 100 && p.frost_key().is_some();
        // termview can raise individual cells to opaque. Its premultiplied blitter
        // then takes the LCD path on those cells unless the masks themselves are gray.
        static OPAQUE_AA: std::sync::OnceLock<mdshared_text::Subpixel> = std::sync::OnceLock::new();
        font.set_subpixel(if translucent {
            mdshared_text::Subpixel::None
        } else {
            *OPAQUE_AA.get_or_init(mdshared_ui::platform::subpixel_order)
        });
        let opacity = BackgroundOpacity::new(f32::from(percent) / 100.0).with_padding(true);
        let mut grid = inner;
        if translucent {
            grid.w = (area.right() - inner.x).max(0.0);
            grid.h = (area.bottom() - inner.y).max(0.0);
            // Right/bottom slack belongs to the real grid; draw the fixed top/left
            // margins as blank terminal background, using the same contrast floor.
            let blank = Screen::new(1, 1, 0);
            for pad in [
                Rect::new(area.x, area.y, area.w, (inner.y - area.y).min(area.h)),
                Rect::new(area.x, inner.y, (inner.x - area.x).min(area.w), grid.h),
            ] {
                termview::draw_screen_with_background(
                    p,
                    font,
                    &scheme,
                    &metrics,
                    pad,
                    &blank,
                    None,
                    View::new().cursor_shown(false),
                    opacity,
                );
            }
        } else {
            p.fill(area, scheme.background);
        }
        termview::draw_screen_with_background(p, font, &scheme, &metrics, grid, &t.screen, sel, view, opacity);
        if let Some(find) = &t.find {
            crate::termfind::draw_marks(p, &pal, find, &t.screen, view, &metrics, inner);
        }
        // Selection, explicit backgrounds and cursor ink are already backed by termview.
        // The ended message is ordinary grid text, so repaint its bands as opaque.
        let notice = visible_notice(t, view);
        if translucent && !notice.is_empty() {
            termview::draw_rows_with_background(
                p,
                font,
                &scheme,
                &metrics,
                inner,
                &t.screen,
                sel,
                view,
                &[notice],
                BackgroundOpacity::new(1.0),
            );
        }
        let back = view.scroll_in(&t.screen);
        if back > 0 && inner.h > 0.0 {
            let total = (t.screen.scrollback_len() + t.screen.rows()) as f32;
            let track = Rect::new(area.right() - 8.0, inner.y, 6.0, inner.h);
            let h = (inner.h * t.screen.rows() as f32 / total).clamp(16.0_f32.min(inner.h), inner.h);
            let top = (t.screen.scrollback_len() - back) as f32 / total * inner.h;
            let thumb = Rect::new(track.x, inner.y + top.min(inner.h - h), track.w, h);
            if translucent {
                p.fill(track, scheme.background);
            }
            draw_scrollbar(p, &pal, track, thumb, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_term::{Point, Selection, SelectionKind};
    use mdshared_text::Bitmap;
    use mdshared_ui::{frost::Key, gfx::Gfx, theme::DARK};

    fn surface() -> Surface {
        Surface {
            area: Rect::new(0.0, 0.0, 105.0, 76.0),
            inner: Rect::new(6.0, 6.0, 93.0, 64.0),
            metrics: Metrics::fixed(13.0, 1.0, 8.0, 16.0),
            scheme: Scheme::from_palette(&DARK),
            pal: DARK,
        }
    }

    fn draw(t: &Term, view: View, percent: u8, backdrop: bool) -> Bitmap {
        let mut bmp = Bitmap::new(105, 76);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        p.set_frost_key(backdrop.then(|| Key::new(DARK.page, 217)));
        surface().draw(&mut p, &mut CellFont::new(), t, view, percent);
        bmp
    }

    fn pixel(b: &Bitmap, x: usize, y: usize) -> [u8; 4] {
        let i = (y * b.width as usize + x) * 4;
        b.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn opening_the_more_menu_keeps_the_grid_and_gray_glyphs_translucent() {
        use crate::{source::Source, tabs::NewTab, windowlook::Cards};
        use mdshared_ui::{menu::Entrance, overlay::Bubble};
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        st.config.appearance.terminal_opacity = 80;
        let mut t = Term::empty((50, 12));
        t.feed(b"\x1b[?25l\x1b[38;2;185;185;185mGrey \x1b[2mdim");
        st.tabs.open(NewTab { label: "Preview".into(), color: [0; 3], session: None, account: None }, t);
        let window = Rect::new(0.0, 0.0, 480.0, 240.0);
        st.layout_terminals(window, window, 1.0);
        let mut bmp = Bitmap::new(480, 240);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        p.set_frost_key(Some(Key::new(DARK.page, 217)));
        st.paint_terminal(&mut p, window, false, |_, _, _, _| false);
        let base = p.bmp.pixels.clone();
        let ground = pixel(p.bmp, 0, 0);
        assert!(ground[3] < 255);
        st.main_menu(Rect::new(446.0, 8.0, 24.0, 24.0));
        st.paint_terminal(&mut p, window, false, |_, _, _, _| false);
        assert_eq!(p.bmp.pixels.as_slice(), base.as_slice(), "opening a menu repainted the grid differently");
        let cards = Cards::for_frame(&st, &p);
        let pal = *st.pal();
        let look = cards.menu(st.appearance.menu_look());
        let (_, menu) = st.menu.as_mut().unwrap();
        menu.set_look(look);
        for t in [0.5, 1.0] {
            p.bmp.pixels.copy_from_slice(&base);
            menu.draw_entering_with(&mut p, window, t, Entrance::Drop, |p, m| m.draw_in(p, &pal, window));
            let r = menu.rect();
            let a = pixel(p.bmp, r.cx() as usize, r.cy() as usize)[3];
            if t < 1.0 {
                assert!((ground[3] + 1..255).contains(&a));
            } else {
                assert_eq!(a, 255);
            }
            for y in 0..240 {
                for x in 0..r.x as usize {
                    let i = (y * 480 + x) * 4;
                    assert_eq!(pixel(p.bmp, x, y), base[i..i + 4], "fringe or dimming outside the menu");
                }
            }
        }
        // A tooltip with a toolkit-owned shadow is clipped to its own backing.
        p.bmp.pixels.copy_from_slice(&base);
        let tip = Bubble::tip(&mut p, "Preview", Rect::new(152.0, 80.0, 32.0, 24.0), window.w, window.h);
        cards.draw(&mut p, tip.rect, pal.popup, |p| tip.draw(p, &pal));
        assert_eq!(pixel(p.bmp, tip.rect.x as usize, tip.rect.y as usize)[3], 255);
        let x = tip.rect.x as usize - 1;
        let y = tip.rect.cy() as usize;
        let i = (y * 480 + x) * 4;
        assert_eq!(pixel(p.bmp, x, y), base[i..i + 4]);

        p.bmp.pixels.copy_from_slice(&base);
        cards.dim(&mut p, &pal, window, 1.0);
        let dim = pixel(p.bmp, 0, 0);
        assert!((ground[3] + 1..255).contains(&dim[3]));
        for desktop in [0.0, 255.0] {
            let visible = |px: [u8; 4]| f32::from(px[0]) + desktop * (1.0 - f32::from(px[3]) / 255.0);
            assert!((visible(dim) - visible(ground) * 0.55).abs() < 1.5, "scrim over {desktop}");
        }
    }

    #[test]
    fn gray_terminal_glyphs_composite_over_black_and_white_without_fringe() {
        let mut term = Term::empty((11, 4));
        term.feed(b"\x1b[38;2;185;185;185mS\x1b[2mS");
        let mut s = surface();
        s.scheme.background = [16; 3];
        let mut bmp = Bitmap::new(105, 76);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        p.set_frost_key(Some(Key::new(DARK.page, 217)));
        let mut font = CellFont::new();
        font.set_subpixel(mdshared_text::Subpixel::Rgb);
        s.draw(&mut p, &mut font, &term, View::new().cursor_shown(false), 80);
        for col in 0..2 {
            let ink = s.scheme.cell_colors(&term.screen.cell(0, col)).glyph.unwrap()[0];
            let x0 = 6 + 8 * col;
            let ground = pixel(&bmp, x0, 6);
            let mut partial = false;
            for y in 6..22 {
                for x in x0..x0 + 8 {
                    let [b, g, r, a] = pixel(&bmp, x, y);
                    assert_eq!((r, g), (g, b), "LCD fringe at {x},{y}");
                    for desktop in [0_u16, 255] {
                        let over = |c, a| u16::from(c) + desktop * (255 - u16::from(a)) / 255;
                        let bg = over(ground[0], ground[3]);
                        let fg = u16::from(ink);
                        let visible = over(r, a);
                        assert!((bg.min(fg).saturating_sub(1)..=bg.max(fg) + 1).contains(&visible));
                        partial |= visible > bg.min(fg) + 1 && visible + 1 < bg.max(fg);
                    }
                }
            }
            assert!(partial, "no antialiased edge in column {col}");
        }
    }

    #[test]
    fn translucent_grid_and_four_margins_keep_selection_and_end_opaque() {
        let mut term = Term::empty((11, 4));
        term.feed(b" \x1b[44m \x1b[0mselect\r\n\r\n");
        let mut selection = Selection::start(&term.screen, Point::new(0, 2), SelectionKind::Stream, 1);
        selection.extend(&term.screen, Point::new(0, 7));
        term.selection = Some(selection);
        term.finish(mdshared_pty::Exit { code: Some(0), signal: None });
        let live = draw(&term, View::new(), 80, true);
        let top = draw(&term, View::new().scrolled(3), 80, true);
        let normal = pixel(&top, 8, 9);
        assert!(normal[3] < 255);
        for (x, y) in [(8, 2), (2, 9), (104, 9), (10, 75)] {
            assert_eq!(pixel(&top, x, y), normal, "padding at {x},{y}");
        }
        for (x, y) in [(16, 9), (24, 9), (98, 40)] {
            assert_eq!(pixel(&top, x, y)[3], 255, "overlay at {x},{y}");
        }
        assert_eq!(pixel(&live, 8, 9)[3], 255, "ended rows");
        assert_eq!(draw(&term, View::new(), 80, false).pixels, draw(&term, View::new(), 100, true).pixels);
    }

    #[test]
    fn rendered_cursor_preference_does_not_override_explicit_shape_or_visibility() {
        let mut term = Term::empty((11, 4));
        let frame = |t: &Term, shape| {
            let prefs = AppearancePrefs { cursor_shape: shape, ..Default::default() };
            draw(t, View::new().cursor_preference(default_shape(&prefs), prefs.cursor_blink), 100, true)
        };
        let [block, bar, underline] = TerminalCursor::OPTIONS.map(|(shape, _, _)| frame(&term, shape));
        assert_ne!(block.pixels, bar.pixels);
        assert_ne!(block.pixels, underline.pixels);
        assert_ne!(bar.pixels, underline.pixels);
        term.feed(b"\x1b[6 q");
        assert_eq!(frame(&term, TerminalCursor::Underline).pixels, bar.pixels);
        term.feed(b"\x1b[0 q");
        assert_eq!(frame(&term, TerminalCursor::Underline).pixels, underline.pixels);
        term.feed(b"\x1b[?25l");
        let hidden = frame(&term, TerminalCursor::Block);
        assert_ne!(hidden.pixels, block.pixels);
        assert_eq!(hidden.pixels, frame(&term, TerminalCursor::Bar).pixels);
    }

    #[test]
    fn blink_requires_focus_visibility_and_an_allowing_program_style() {
        let mut screen = Screen::new(12, 2, 0);
        let view = View::new().cursor_preference(CursorShape::Bar, true);
        assert!(cursor_can_blink(&screen, view, true));
        assert!(!cursor_can_blink(&screen, view, false));
        assert!(!cursor_can_blink(&screen, view.focused(false), true));
        screen.feed(b"\x1b[4 q");
        assert!(!cursor_can_blink(&screen, view, true));
        screen.feed(b"\x1b[5 q");
        assert!(cursor_can_blink(&screen, view, true));
        screen.feed(b"\x1b[?25l");
        assert!(!cursor_can_blink(&screen, view, true));
    }

    #[test]
    fn opaque_end_notice_tracks_wrapped_lines_after_scroll_and_resize() {
        let mut term = Term::empty((12, 4));
        term.feed(b"last output");
        term.finish(mdshared_pty::Exit { code: Some(0), signal: None });
        assert_eq!(term.ended_lines.end - term.ended_lines.start, 4);
        assert_eq!(visible_notice(&term, View::new()), 0..4);
        assert_eq!(visible_notice(&term, View::new().scrolled(1)), 1..4);
        term.resize(12, 2);
        assert_eq!(visible_notice(&term, View::new()), 0..2);
        assert_eq!(visible_notice(&term, View::new().scrolled(3)), 1..2);
    }

    /// What happens in the simulated window at a moment, besides the blink's own frames.
    enum Event {
        /// The program's output, fed as `term_woke` feeds it.
        Output(&'static [u8]),
        /// A key, as `key` restarts the blink, and its echo.
        Key(&'static [u8]),
        /// A redraw for something else: a spinner, the pointer, a toast.
        Redraw,
    }

    /// Drives the cursor's blink as the host does, on a simulated clock: a frame at every event
    /// and at every deadline the animator gives, the shown tab's output restarting the blink only
    /// where `term_woke` does. Returns each change of the cursor, (ms, on).
    fn blink_run(events: &[(u64, Event)], end: u64, policy: mdshared_ui::anim::Policy) -> Vec<(u64, bool)> {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut anim = mdshared_ui::anim::Animator::new();
        anim.set_policy(policy);
        let mut term = Term::empty((100, 30));
        term.feed(crate::term::tests::PROMPT);
        let view = term.view(true).cursor_preference(CursorShape::Block, true);
        let half = Some(Duration::from_millis(530));
        let (mut at, mut next, mut deadline) = (0, 0, Some(0));
        let mut changes: Vec<(u64, bool)> = Vec::new();
        while at < end {
            let event = events.get(next).filter(|(t, _)| deadline.is_none_or(|d| *t <= d));
            at = match (event, deadline) {
                (Some((t, _)), _) => *t,
                (None, Some(d)) => d,
                (None, None) => break,
            };
            if let Some((_, e)) = event {
                next += 1;
                match e {
                    Event::Output(bytes) if term.feed_marked(bytes) => anim.forget(CURSOR),
                    Event::Key(echo) => {
                        anim.forget(CURSOR);
                        term.feed(echo);
                    }
                    Event::Output(_) | Event::Redraw => {}
                }
            }
            anim.begin_frame(start + Duration::from_millis(at));
            let blink = half.filter(|_| cursor_can_blink(&term.screen, view, true));
            let on = termview::cursor_blinks_on(&mut anim, CURSOR, blink, &term.screen, view);
            deadline = anim.end_frame().map(|d| d.duration_since(start).as_millis() as u64);
            if changes.last().is_none_or(|&(_, was)| was != on) {
                changes.push((at, on));
            }
        }
        changes
    }

    /// Claude Code's output while it runs a command, recorded through ConPTY (ms after the
    /// command started): the counter every second, the title's spinner in between, one notice
    /// that comes and goes on rows of its own; and a redraw of the window's own every 16 ms
    /// for a while (a tab's spinner).
    fn claude_working() -> Vec<(u64, Event)> {
        let title = "\x1b]0;\u{25d0} Claude Code\x07".as_bytes();
        let notice = b"\x1b[?25l\x1b[13;81H\x1b[K\x1b[38;2;255;193;7m\x1b[14;25HYou've used 99%\x1b[11;3H\x1b[?25h";
        let gone = b"\x1b[?25l\x1b[14;1H\x1b[K\x1b[11;3H\x1b[?25h";
        let mut events: Vec<(u64, Event)> = Vec::new();
        for (t, text) in [(1305, title), (1965, title), (2928, title), (3890, title), (4851, title), (5812, title)] {
            events.push((t, Event::Output(text)));
        }
        for t in [1310, 2313, 3307, 4316, 5313, 6318, 7316, 8312, 9316, 10313] {
            events.push((t, Event::Output(crate::term::tests::COUNTER)));
        }
        events.push((5412, Event::Output(notice)));
        events.push((5439, Event::Output(gone)));
        events.extend((0..120).map(|i| (6000 + i * 16, Event::Redraw)));
        events.sort_by_key(|(t, _)| *t);
        events
    }

    fn halves(changes: &[(u64, bool)]) -> Vec<u64> {
        changes.windows(2).map(|w| w[1].0 - w[0].0).collect()
    }

    #[test]
    fn the_blink_keeps_its_rhythm_through_repaints_around_the_cursor_and_other_redraws() {
        use mdshared_ui::anim::Policy;
        let changes = blink_run(&claude_working(), 12_000, Policy::Always);
        assert_eq!(changes.first(), Some(&(0, true)));
        assert!(changes.len() > 20, "{changes:?}");
        assert!(halves(&changes).iter().all(|&h| h == 530), "uneven: {:?}", halves(&changes));

        // A key restarts it, on, and the rhythm resumes from there; so does output that changes
        // the cursor's cell. (Both stay on the parked cell, where the recorded counter puts the
        // cursor back.)
        let mut events = claude_working();
        events.push((2_100, Event::Key(b"")));
        events.push((7_777, Event::Output(b"i\x08")));
        events.sort_by_key(|(t, _)| *t);
        let changes = blink_run(&events, 12_000, Policy::Always);
        let between =
            |from: u64, to: u64| changes.iter().copied().filter(|&(t, _)| t > from && t < to).collect::<Vec<_>>();
        for (from, to) in [(2_100, 7_777), (7_777, 12_000)] {
            let after = between(from, to);
            assert_eq!(after.first(), Some(&(from + 530, false)), "{changes:?}");
            assert!(halves(&after).iter().all(|&h| h == 530), "{changes:?}");
        }

        // Reduced motion: steady, and no frame asked for it.
        assert_eq!(blink_run(&claude_working(), 12_000, Policy::Never), [(0, true)]);
    }
}
