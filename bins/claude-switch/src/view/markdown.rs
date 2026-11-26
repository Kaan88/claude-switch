//! A deliberately small read-only Markdown subset, shaped and drawn by the toolkit.

use std::cell::RefCell;
use std::ops::Range;

use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::richtext::parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontFamily, FontStyle, FontWeight, Layout,
    LayoutContext, OverflowWrap, Selection, StyleProperty,
};
use mdshared_ui::richtext::{self, Brush};
use mdshared_ui::theme::Palette;

use super::links;

const MONO: &str = mdshared_ui::termview::MONO_FAMILY;
const UI: &str = "Segoe UI, Noto Sans, sans-serif";

#[derive(Clone, Copy)]
enum Style {
    Bold,
    Italic,
    Heading,
    Code,
}

#[derive(Default)]
struct Document {
    text: String,
    spans: Vec<(Range<usize>, Style)>,
    links: Vec<(Range<usize>, String)>,
}

impl Document {
    fn styled(&mut self, text: &str, style: Style) {
        let start = self.text.len();
        self.text.push_str(text);
        self.spans.push((start..self.text.len(), style));
    }

    fn inline(&mut self, mut input: &str) {
        while !input.is_empty() {
            if let Some(inside) = input.strip_prefix('[')
                && let Some(label_end) = inside.find(['[', ']'])
                && label_end > 0
                && let Some(to) = inside[label_end..].strip_prefix("](")
                && let Some(dest_end) = to.find([')', '['])
                && let Some(rest) = to[dest_end..].strip_prefix(')')
                && links::target(&to[..dest_end])
            {
                let start = self.text.len();
                self.text.push_str(&inside[..label_end]);
                self.links.push((start..self.text.len(), to[..dest_end].to_owned()));
                input = rest;
                continue;
            }
            let delimited = if input.starts_with("**") {
                Some(("**", Style::Bold))
            } else if input.starts_with('*') {
                Some(("*", Style::Italic))
            } else if input.starts_with('`') {
                Some(("`", Style::Code))
            } else {
                None
            };
            if let Some((delimiter, style)) = delimited
                && let Some((inner, rest)) = input[delimiter.len()..].split_once(delimiter)
                && !inner.is_empty()
            {
                self.styled(inner, style);
                input = rest;
            } else {
                let c = input.chars().next().expect("nonempty inline text");
                self.text.push(c);
                input = &input[c.len_utf8()..];
            }
        }
    }

    fn parse(raw: &str, rich: bool) -> Self {
        let mut doc = Self::default();
        if !rich {
            doc.text = raw.to_owned();
        } else {
            let mut code = false;
            for (i, line) in raw.split('\n').enumerate() {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.trim_start().starts_with("```") {
                    code = !code;
                    continue;
                }
                if i != 0 && !doc.text.is_empty() {
                    doc.text.push('\n');
                }
                let body = line.trim_start_matches('#');
                if code {
                    doc.styled(line, Style::Code);
                } else if (1..=6).contains(&(line.len() - body.len())) && body.starts_with(' ') {
                    let start = doc.text.len();
                    doc.inline(body.trim_start());
                    doc.spans.push((start..doc.text.len(), Style::Heading));
                } else if let Some(body) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                    doc.text.push_str("•  ");
                    doc.inline(body);
                } else if let Some(body) = line.strip_prefix("> ") {
                    doc.text.push_str("│  ");
                    doc.inline(body);
                } else {
                    doc.inline(line);
                }
            }
        }
        for (range, style) in &doc.spans {
            let text = &doc.text[range.clone()];
            if matches!(style, Style::Code) && links::target(text) && !text.starts_with(' ') && !text.ends_with(' ') {
                doc.links.push((range.clone(), text.to_owned()));
            }
        }
        doc.links.sort_unstable_by_key(|(r, _)| r.start);
        let explicit = doc.links.len();
        for (range, target) in links::detect(&doc.text) {
            let i = doc.links[..explicit].partition_point(|(r, _)| r.end <= range.start);
            if doc.links[..explicit].get(i).is_none_or(|(r, _)| r.start >= range.end) {
                doc.links.push((range, target));
            }
        }
        doc
    }
}

thread_local! {
    static SHAPER: RefCell<(FontContext, LayoutContext<Brush>)> =
        RefCell::new((mdshared_text::shared_fonts(), LayoutContext::new()));
}

pub struct TextLayout {
    doc: Document,
    layout: Layout<Brush>,
    runs: Vec<richtext::RunBox>,
    scale: f32,
}

impl TextLayout {
    pub fn new(raw: &str, pal: &Palette, scale: f32, width: f32, rich: bool) -> Self {
        let doc = Document::parse(raw, rich);
        let brush = |[r, g, b]: [u8; 3]| [r, g, b, 255];
        let mut layout = SHAPER.with(|shaper| {
            let (fonts, layouts) = &mut *shaper.borrow_mut();
            let mut build = layouts.ranged_builder(fonts, &doc.text, scale, true);
            build.push_default(StyleProperty::FontFamily(FontFamily::from(UI)));
            build.push_default(StyleProperty::FontSize(14.0));
            build.push_default(StyleProperty::Brush(brush(pal.text)));
            build.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
            for (range, style) in &doc.spans {
                let range = range.clone();
                match style {
                    Style::Heading => {
                        build.push(StyleProperty::FontSize(18.0), range.clone());
                        build.push(StyleProperty::FontWeight(FontWeight::BOLD), range);
                    }
                    Style::Bold => build.push(StyleProperty::FontWeight(FontWeight::BOLD), range),
                    Style::Italic => build.push(StyleProperty::FontStyle(FontStyle::Italic), range),
                    Style::Code => build.push(StyleProperty::FontFamily(FontFamily::from(MONO)), range),
                }
            }
            for (range, _) in &doc.links {
                build.push(StyleProperty::Brush(brush(pal.accent)), range.clone());
                build.push(StyleProperty::Underline(true), range.clone());
            }
            build.build(&doc.text)
        });
        layout.break_all_lines(Some(width.max(1.0) * scale));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let runs = if doc.links.is_empty() { Vec::new() } else { richtext::runs(&layout) };
        Self { doc, layout, runs, scale }
    }

    pub fn height(&self) -> f32 {
        self.layout.height() / self.scale
    }

    pub fn text(&self) -> &str {
        &self.doc.text
    }

    pub fn at(&self, x: f32, y: f32) -> usize {
        Cursor::from_point(&self.layout, x * self.scale, y * self.scale).index()
    }

    pub fn link(&self, x: f32, y: f32) -> Option<&str> {
        let (x, y) = (x * self.scale, y * self.scale);
        let run = self.runs.iter().find(|r| !r.range.is_empty() && r.rect.contains(x, y))?;
        let i = self.at(x / self.scale, y / self.scale).clamp(run.range.start, run.range.end - 1);
        self.doc.links.iter().find(|(r, _)| r.contains(&i)).map(|(_, link)| link.as_str())
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
    fn formatted_text_keeps_copyable_label_code_and_link_destination() {
        let doc = Document::parse("## Title\n\n- **One** `src/lib.rs`\n- [Notes](notes.md)\n```rs\nx = 1;\n```", true);
        assert_eq!(doc.text, "Title\n\n•  One src/lib.rs\n•  Notes\nx = 1;");
        assert!(doc.links.iter().any(|(r, d)| &doc.text[r.clone()] == "Notes" && d == "notes.md"));
        assert!(doc.links.iter().any(|(r, d)| &doc.text[r.clone()] == "src/lib.rs" && d == "src/lib.rs"));
        assert!(doc.spans.iter().any(|(r, s)| matches!(s, Style::Code) && &doc.text[r.clone()] == "x = 1;"));
    }

    #[test]
    fn monospace_paths_with_spaces_keep_the_whole_destination() {
        let doc = Document::parse(r"Open `C:\work folder\notes.md`.", true);
        assert_eq!(doc.links.len(), 1);
        let (range, link) = &doc.links[0];
        assert_eq!(link, r"C:\work folder\notes.md");
        assert_eq!(&doc.text[range.clone()], link);
    }

    #[test]
    fn stray_brackets_stay_literal_and_do_not_hide_a_later_link() {
        let raw = "[".repeat(30_000) + "[Notes](notes.md)";
        let doc = Document::parse(&raw, true);
        assert_eq!(doc.links, vec![(30_000..30_005, "notes.md".into())]);
        assert_eq!(doc.text, "[".repeat(30_000) + "Notes");
    }
}
