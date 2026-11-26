use std::io;

use super::super::preview::Preview;
use super::scroll::{GUTTER, Scrollbar, Surface};
use super::*;

pub(super) struct Markdown {
    file: NamedFile,
    source: Option<io::Result<String>>,
    layout: Option<(f32, f32, Palette, Preview)>,
    error: Option<&'static str>,
    pub bounds: Rect,
    pub scroll: Option<Scrollbar>,
    offset: f32,
    max: f32,
}

impl Markdown {
    pub fn new(file: NamedFile) -> Self {
        Self {
            file,
            source: None,
            layout: None,
            error: None,
            bounds: Rect::default(),
            scroll: None,
            offset: 0.0,
            max: 0.0,
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    #[cfg(test)]
    pub(super) fn rendered(&self) -> bool {
        self.layout.as_ref().is_some_and(|(_, _, _, layout)| layout.text().contains("North ridge"))
    }

    pub fn set_offset(&mut self, offset: f32) -> bool {
        let offset = offset.clamp(0.0, self.max);
        std::mem::replace(&mut self.offset, offset) != offset
    }

    pub fn editor(&mut self) {
        self.error = (!self.file.open(false)).then_some("Could not open the named file in its default app.");
    }
}

/// Dock next to the transcript without changing its terminal splitter. When there is
/// insufficient room for two reading columns, show the preview in the existing body.
pub(super) fn split(area: Rect) -> (Rect, Rect) {
    if area.w < 640.0 {
        return (Rect::new(area.x, area.y, 0.0, area.h), area);
    }
    let side = (area.w * 0.43).clamp(300.0, 500.0);
    let gap = 13.0;
    (Rect::new(area.x, area.y, area.w - side - gap, area.h), Rect::new(area.right() - side, area.y, side, area.h))
}

impl Ui {
    pub(super) fn draw_markdown(&mut self, p: &mut Painter, pal: &Palette, area: Rect, m: &mut Model) {
        let Some(mut pane) = self.markdown.take() else { return };
        if let Some(result) = m.take_markdown() {
            pane.source = Some(result);
            pane.layout = None;
        }
        pane.bounds = area;
        pane.scroll = None;
        let clip = p.clip(area);
        p.frame(area, 7.0, pal.page, pal.line);
        let title = pane.file.path().file_name().and_then(|s| s.to_str()).unwrap_or("Markdown preview");
        let close = Rect::new(area.right() - 32.0, area.y + 6.0, 25.0, 25.0);
        let editor = Rect::new(close.x - 114.0, close.y, 108.0, close.h);
        if let Some(r) = intersection(close, area) {
            self.button(p, pal, r, "×", Action::CloseMarkdown, false);
        }
        if let Some(r) = intersection(editor, area) {
            self.button(p, pal, r, "Open in editor", Action::Editor, false);
        }
        p.text(Face::Semibold, 13.0, area.x + 11.0, close.cy(), title, pal.text, (editor.x - area.x - 18.0).max(0.0));
        let top = area.y + 37.0;
        p.fill(Rect::new(area.x + 1.0, top, (area.w - 2.0).max(0.0), p.px()), pal.line);
        let top = if let Some(error) = pane.error {
            p.text(Face::Ui, 12.0, area.x + 10.0, top + 12.0, error, pal.warn, (area.w - 20.0).max(0.0));
            top + 26.0
        } else {
            top + 6.0
        };
        let body =
            Rect::new(area.x + 10.0, top, (area.w - 20.0 - GUTTER).max(1.0), (area.bottom() - top - 6.0).max(0.0));
        if body.h > 0.0 {
            match &pane.source {
                Some(Ok(raw)) if !raw.trim().is_empty() => {
                    if pane.layout.as_ref().is_none_or(|(width, scale, colors, _)| {
                        *width != body.w || *scale != p.scale || *colors != *pal
                    }) {
                        let render = Preview::new(raw, pal, p.scale, body.w, 14.0, None);
                        pane.layout = Some((body.w, p.scale, *pal, render));
                    }
                    let view = &pane.layout.as_ref().expect("viewer just laid out").3;
                    pane.max = (view.height() - body.h).max(0.0);
                    pane.offset = pane.offset.clamp(0.0, pane.max);
                    let c = p.clip(body);
                    view.draw(p, Rect::new(body.x, body.y - pane.offset, body.w, view.height()), body, None, pal);
                    p.restore(c);
                    pane.scroll = Scrollbar::new(Rect::new(body.right() + 1.0, body.y, GUTTER, body.h), pane.max);
                    if let Some(bar) = pane.scroll {
                        bar.draw(
                            p,
                            pal,
                            pane.offset,
                            self.mouse,
                            self.scroll_drag.is_some_and(|d| d.surface == Surface::Markdown),
                        );
                    }
                }
                source => {
                    pane.max = 0.0;
                    let (message, color) = match source {
                        None => ("Reading the named file…".into(), pal.muted),
                        Some(Ok(_)) => ("This Markdown file is empty.".into(), pal.muted),
                        Some(Err(error)) => (error.to_string(), pal.warn),
                    };
                    p.paragraph_wrapped(
                        mdshared_ui::gfx::Face::Ui,
                        13.0,
                        body.x,
                        body.y + 2.0,
                        body.w,
                        &message,
                        color,
                        mdshared_ui::gfx::Wrap::Words,
                    );
                }
            }
        }
        p.restore(clip);
        self.markdown = Some(pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_docks_on_wide_chat_and_uses_the_existing_body_at_session_width() {
        let narrow = Rect::new(100.0, 40.0, 330.0, 250.0);
        let (left, right) = split(narrow);
        assert_eq!(left.w, 0.0);
        assert_eq!((right.x, right.w), (narrow.x, narrow.w));
        let wide = Rect::new(100.0, 40.0, 1000.0, 250.0);
        let (left, right) = split(wide);
        assert!(left.w >= 300.0 && right.w >= 300.0);
        assert!(left.right() < right.x);
        assert_eq!(right.right(), wide.right());
    }
}
