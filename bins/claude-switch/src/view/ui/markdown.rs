use std::io;

use super::super::documents::Document;
use super::super::preview::Preview;
use super::scroll::{GUTTER, Scrollbar, Surface};
use super::*;

pub(super) struct Markdown {
    file: NamedFile,
    source: Option<io::Result<Document>>,
    text_lines: (usize, Vec<usize>),
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
            text_lines: (0, Vec::new()),
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
            pane.text_lines = (0, Vec::new());
            pane.source = Some(result);
            pane.layout = None;
        }
        pane.bounds = area;
        pane.scroll = None;
        let clip = p.clip(area);
        p.frame(area, 7.0, pal.page, pal.line);
        let title = pane.file.path().file_name().and_then(|s| s.to_str()).unwrap_or("File preview");
        let close = Rect::new(area.right() - 32.0, area.y + 6.0, 25.0, 25.0);
        let editor = Rect::new(close.x - 114.0, close.y, 108.0, close.h);
        let can_open = pane.file.supports_default_app();
        if let Some(r) = intersection(close, area) {
            self.button(p, pal, r, "×", Action::CloseMarkdown, false);
        }
        if let Some(r) = intersection(editor, area).filter(|_| can_open) {
            let label = if pane.file.picture() { "Open in app" } else { "Open in editor" };
            self.button(p, pal, r, label, Action::Editor, false);
        }
        let title_end = if can_open { editor.x } else { close.x };
        p.text(Face::Semibold, 13.0, area.x + 11.0, close.cy(), title, pal.text, (title_end - area.x - 18.0).max(0.0));
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
                Some(Ok(Document::Markdown(raw))) if !raw.trim().is_empty() => {
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
                }
                Some(Ok(Document::Text(text))) if !text.is_empty() => {
                    let lh = 17.0;
                    let cols = (body.w / p.measure(Face::Mono, 12.0, "M").max(1.0)).floor().max(1.0) as usize;
                    if pane.text_lines.0 != cols {
                        pane.text_lines = (cols, text_lines(text, cols));
                    }
                    let starts = &pane.text_lines.1;
                    pane.max = (starts.len() as f32 * lh - body.h).max(0.0);
                    pane.offset = pane.offset.clamp(0.0, pane.max);
                    let clip = p.clip(body);
                    let from = (pane.offset / lh) as usize;
                    for (i, &start) in starts.iter().enumerate().skip(from).take((body.h / lh) as usize + 2) {
                        let end = starts.get(i + 1).copied().unwrap_or(text.len());
                        let line = text[start..end].trim_end_matches(['\r', '\n']);
                        p.text(
                            Face::Mono,
                            12.0,
                            body.x,
                            body.y + (i as f32 + 0.5) * lh - pane.offset,
                            line,
                            pal.text,
                            body.w,
                        );
                    }
                    p.restore(clip);
                }
                Some(Ok(Document::Picture(image))) => {
                    pane.max = 0.0;
                    pane.offset = 0.0;
                    paint_image(p, image, body, 3.0, false);
                }
                source => {
                    pane.max = 0.0;
                    let (message, color) = match source {
                        None => ("Reading the named file…".into(), pal.muted),
                        Some(Ok(_)) => ("This file is empty.".into(), pal.muted),
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
        p.restore(clip);
        self.markdown = Some(pane);
    }
}

fn text_lines(text: &str, columns: usize) -> Vec<usize> {
    let mut lines = vec![0];
    let mut width = 0;
    for (at, c) in text.char_indices() {
        if c == '\n' {
            lines.push(at + 1);
            width = 0;
        } else if c != '\r' {
            // Approximate fallback glyphs conservatively; only visible lines are shaped.
            let cells = if c == '\t' {
                4
            } else if c.is_ascii() {
                1
            } else {
                2
            };
            if width > 0 && width + cells > columns {
                lines.push(at);
                width = 0;
            }
            width += cells;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_text_wraps_without_dropping_long_lines_or_splitting_utf8() {
        let text = "12345éX\n<img src=x>\n";
        let starts = text_lines(text, 5);
        let actual = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| &text[start..starts.get(i + 1).copied().unwrap_or(text.len())])
            .collect::<Vec<_>>();
        assert_eq!(actual, ["12345", "éX\n", "<img ", "src=x", ">\n", ""]);
    }

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
