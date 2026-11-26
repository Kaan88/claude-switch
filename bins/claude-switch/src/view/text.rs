//! Toolkit-shaped literal text for prompts, tool lines and list details.

use std::cell::RefCell;
use std::ops::Range;

use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::richtext::parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontFamily, Layout, LayoutContext, OverflowWrap,
    Selection, StyleProperty,
};
use mdshared_ui::richtext::{self, Brush};
use mdshared_ui::theme::Palette;

use super::links;

const UI: &str = "Segoe UI, Noto Sans, sans-serif";

thread_local! {
    static SHAPER: RefCell<(FontContext, LayoutContext<Brush>)> =
        RefCell::new((mdshared_text::shared_fonts(), LayoutContext::new()));
}

pub struct TextLayout {
    #[cfg(test)]
    text: String,
    links: Vec<(Range<usize>, String)>,
    layout: Layout<Brush>,
    runs: Vec<richtext::RunBox>,
    scale: f32,
}

impl TextLayout {
    /// Shape the actual elided string. Clipping a full layout would still leave a cut glyph
    /// rather than an ellipsis, and the selectable hit positions would include hidden text.
    pub fn compact(raw: &str, pal: &Palette, scale: f32, width: f32) -> (Self, usize) {
        let line = |text: &str, links| Self::shaped(text, pal, scale, 1_000_000.0, links);
        let fits = |line: &Self| line.layout.width() / scale <= width;
        let full = line(raw, links::detect(raw));
        if fits(&full) {
            return (full, raw.len());
        }
        let ellipsis = line("…", Vec::new());
        if !fits(&ellipsis) {
            return (line("", Vec::new()), 0);
        }
        let cuts: Vec<_> = raw.char_indices().map(|(i, _)| i).collect();
        let (mut low, mut high) = (1, cuts.len());
        let mut best = (ellipsis, 0);
        while low < high {
            let mid = low + (high - low) / 2;
            let prefix = raw[..cuts[mid]].trim_end();
            // An abbreviated URL must never be parsed as a different destination.
            let links = full.links.iter().filter(|(range, _)| range.end <= prefix.len()).cloned().collect();
            let candidate = line(&format!("{prefix}…"), links);
            if fits(&candidate) {
                best = (candidate, prefix.len());
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        best
    }

    pub fn new(raw: &str, pal: &Palette, scale: f32, width: f32) -> Self {
        Self::shaped(raw, pal, scale, width, links::detect(raw))
    }

    fn shaped(raw: &str, pal: &Palette, scale: f32, width: f32, links: Vec<(Range<usize>, String)>) -> Self {
        let brush = |[r, g, b]: [u8; 3]| [r, g, b, 255];
        let mut layout = SHAPER.with(|shaper| {
            let (fonts, layouts) = &mut *shaper.borrow_mut();
            let mut build = layouts.ranged_builder(fonts, raw, scale, true);
            build.push_default(StyleProperty::FontFamily(FontFamily::from(UI)));
            build.push_default(StyleProperty::FontSize(14.0));
            build.push_default(StyleProperty::Brush(brush(pal.text)));
            build.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
            for (range, _) in &links {
                build.push(StyleProperty::Brush(brush(pal.accent)), range.clone());
                build.push(StyleProperty::Underline(true), range.clone());
            }
            build.build(raw)
        });
        layout.break_all_lines(Some(width.max(1.0) * scale));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let runs = if links.is_empty() { Vec::new() } else { richtext::runs(&layout) };
        Self {
            #[cfg(test)]
            text: raw.to_owned(),
            links,
            layout,
            runs,
            scale,
        }
    }

    pub fn height(&self) -> f32 {
        self.layout.height() / self.scale
    }

    pub fn at(&self, x: f32, y: f32) -> usize {
        Cursor::from_point(&self.layout, x * self.scale, y * self.scale).index()
    }

    pub fn link(&self, x: f32, y: f32) -> Option<&str> {
        let run = self.runs.iter().find(|r| !r.range.is_empty() && r.rect.contains(x * self.scale, y * self.scale))?;
        let i = self.at(x, y).clamp(run.range.start, run.range.end - 1);
        self.links.iter().find(|(r, _)| r.contains(&i)).map(|(_, link)| link.as_str())
    }

    pub fn draw(&self, p: &mut Painter, x: f32, y: f32, selection: Option<Range<usize>>, pal: &Palette) {
        if let Some(r) = selection.filter(|r| !r.is_empty()) {
            let from = Cursor::from_byte_index(&self.layout, r.start, Affinity::Downstream);
            let to = Cursor::from_byte_index(&self.layout, r.end, Affinity::Upstream);
            for (b, _) in Selection::new(from, to).geometry(&self.layout) {
                p.fill_a(
                    Rect::new(
                        x + b.x0 as f32 / self.scale,
                        y + b.y0 as f32 / self.scale,
                        (b.x1 - b.x0) as f32 / self.scale,
                        (b.y1 - b.y0) as f32 / self.scale,
                    ),
                    pal.selection,
                    0.36,
                );
            }
        }
        p.rich_text(&self.layout, x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_layout_actually_shapes_an_ellipsis_and_fits_at_both_scales() {
        let raw = "mcp__update_plan   a long summary with Unicode 計画 and more words";
        for scale in [1.0, 1.5] {
            let pal = &mdshared_ui::theme::DARK;
            let (full, end) = TextLayout::compact(raw, pal, scale, 1_000.0);
            assert_eq!((full.text.as_str(), end), (raw, raw.len()));
            let (narrow, end) = TextLayout::compact(raw, pal, scale, 150.0);
            assert!(narrow.layout.width() / scale <= 150.0);
            assert!((1..raw.len()).contains(&end));
            assert_eq!(narrow.text.strip_suffix('…'), Some(&raw[..end]));
            let (empty, end) = TextLayout::compact(raw, pal, scale, 1.0);
            assert_eq!((empty.text.as_str(), end), ("", 0));
        }
    }

    #[test]
    fn a_partial_url_is_not_made_into_a_different_clickable_target() {
        let raw = "tool https://long-destination.example/sensitive/path";
        let pal = &mdshared_ui::theme::DARK;
        let (full, _) = TextLayout::compact(raw, pal, 1.0, 1000.0);
        assert_eq!(full.links.len(), 1);
        let (narrow, end) = TextLayout::compact(raw, pal, 1.0, 165.0);
        assert!(narrow.text.contains("https://"));
        assert!(end < raw.len() && narrow.links.is_empty());
    }
}
