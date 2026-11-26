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
    text: String,
    links: Vec<(Range<usize>, String)>,
    layout: Layout<Brush>,
    runs: Vec<richtext::RunBox>,
    scale: f32,
}

impl TextLayout {
    pub fn new(raw: &str, pal: &Palette, scale: f32, width: f32) -> Self {
        let links = links::detect(raw);
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
        Self { text: raw.to_owned(), links, layout, runs, scale }
    }

    pub fn height(&self) -> f32 {
        self.layout.height() / self.scale
    }

    pub fn text(&self) -> &str {
        &self.text
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
