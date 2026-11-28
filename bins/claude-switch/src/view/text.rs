//! Toolkit-shaped literal text for prompts, tool lines and list details.

use std::cell::RefCell;
use std::ops::Range;

use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::richtext::parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontFamily, FontWeight, Layout, LayoutContext,
    LineHeight, OverflowWrap, Selection, StyleProperty,
};
use mdshared_ui::richtext::{self, Brush};
use mdshared_ui::theme::Palette;

use super::links;

const UI: &str = "Segoe UI, Noto Sans, sans-serif";
const MONO: &str = "Cascadia Mono, Consolas, monospace";

thread_local! {
    static SHAPER: RefCell<(FontContext, LayoutContext<Brush>)> =
        RefCell::new((mdshared_text::shared_fonts(), LayoutContext::new()));
}

pub struct TextLayout {
    links: Vec<(Range<usize>, String)>,
    layout: Layout<Brush>,
    runs: Vec<richtext::RunBox>,
    scale: f32,
}

impl TextLayout {
    pub fn new(
        raw: &str,
        pal: &Palette,
        scale: f32,
        width: f32,
        size: f32,
        files: Option<&links::Files>,
        mono: bool,
    ) -> Self {
        Self::build(raw, pal, scale, width, size, files, mono, None, false)
    }

    /// Semibold interface text, wrapped: a card's question.
    pub fn heading(raw: &str, pal: &Palette, scale: f32, width: f32, size: f32) -> Self {
        Self::build(raw, pal, scale, width, size, None, false, None, true)
    }

    /// Mono text whose lines are `line` times the font size apart: a diff's rows.
    pub fn spaced(raw: &str, pal: &Palette, scale: f32, width: f32, size: f32, line: f32) -> Self {
        Self::build(raw, pal, scale, width, size, None, true, Some(line), false)
    }

    #[allow(clippy::too_many_arguments)] // one shaping call behind the two constructors
    fn build(
        raw: &str,
        pal: &Palette,
        scale: f32,
        width: f32,
        size: f32,
        files: Option<&links::Files>,
        mono: bool,
        line: Option<f32>,
        semibold: bool,
    ) -> Self {
        let links = files.map_or_else(Vec::new, |files| links::existing(raw, files));
        let brush = |[r, g, b]: [u8; 3]| [r, g, b, 255];
        let mut layout = SHAPER.with(|shaper| {
            let (fonts, layouts) = &mut *shaper.borrow_mut();
            let mut build = layouts.ranged_builder(fonts, raw, scale, true);
            build.push_default(StyleProperty::FontFamily(FontFamily::from(if mono { MONO } else { UI })));
            build.push_default(StyleProperty::FontSize(size));
            build.push_default(StyleProperty::Brush(brush(pal.text)));
            build.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
            if let Some(line) = line {
                build.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(line)));
            }
            if semibold {
                build.push_default(StyleProperty::FontWeight(FontWeight::SEMI_BOLD));
            }
            for (range, _) in &links {
                build.push(StyleProperty::Brush(brush(pal.accent)), range.clone());
                build.push(StyleProperty::Underline(true), range.clone());
            }
            build.build(raw)
        });
        layout.break_all_lines(Some(width.max(1.0) * scale));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let runs = if links.is_empty() { Vec::new() } else { richtext::runs(&layout) };
        Self { links, layout, runs, scale }
    }

    pub fn width(&self) -> f32 {
        self.layout.width() / self.scale
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
    fn only_verified_file_ranges_are_underlined_and_hit_testable() {
        let demo = crate::demo::Transcript::new().unwrap();
        let name = "field-notes.md";
        let files = links::Files::from([(name.into(), links::resolve(name, demo.cwd(), &Default::default()).unwrap())]);
        let raw = "enter/leave field-notes.md";
        for scale in [1.0, 1.5] {
            let pal = &mdshared_ui::theme::DARK;
            let text = TextLayout::new(raw, pal, scale, 250.0, 14.0, Some(&files), false);
            assert!(text.link(25.0, 8.0).is_none());
            assert!((90..220).any(|x| text.link(x as f32, 8.0) == Some(name)));
        }
    }

    #[test]
    fn literal_shell_detail_does_not_infer_links_from_command_arguments() {
        let raw = "echo enter/leave https://example.test";
        let pal = &mdshared_ui::theme::DARK;
        let text = TextLayout::new(raw, pal, 1.0, 400.0, 13.0, None, true);
        assert!(text.links.is_empty());
    }
}
