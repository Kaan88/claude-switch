//! The same terminal surface draws the full tab and the Chat drawer.

use super::{AppState, CURSOR, Cx, Msg, Rect, Target};
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
        let pal = *self.pal();
        let scheme = self.terminal_scheme();
        let (Some(metrics), Some(tab)) = (self.metrics, self.tabs.shown()) else {
            p.fill(area, scheme.background);
            return;
        };
        let inner = self.term_area;
        let t = &tab.term;
        let prefs = &self.config.appearance;
        let view = t.view(self.terminal_focused()).cursor_preference(default_shape(prefs), prefs.cursor_blink);
        let blink = self.blink.filter(|_| cursor_can_blink(&t.screen, view, prefs.cursor_blink));
        let show_cursor = termview::cursor_blinks_on(cx.anim(), CURSOR, blink, &t.screen, view);
        if blink.is_some()
            && cx.anim().animates()
            && let Some(row) = view.cursor_row(&t.screen)
        {
            cx.animating(metrics.row_rect(inner, row));
        }
        let view = view.cursor_shown(show_cursor);
        // Window-level overlays include shadows, rounded edges and modal scrims,
        // which use the opaque painter too. Back them with the solid terminal frame.
        let clear = self.nothing_over()
            && !self.picture_overlay()
            && self.tip.shown().is_none()
            && self.toasts.is_empty()
            && self.undo_hide.is_none();
        let percent = if clear { prefs.terminal_opacity } else { 100 };
        let surface = Surface { area, inner, metrics, scheme, pal };
        surface.draw(p, &mut self.cell_font, t, view, percent);
        self.hits.push((area, Target::Terminal));
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
}
