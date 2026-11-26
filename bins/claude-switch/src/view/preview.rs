//! The same headless renderer as mdview-preview, embedded without its editor scroll protocol.
//! Only pixels and coordinates cross the two toolkit revisions.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::theme::Palette;
use mdview_core::model::{BlockKind, Document, SpanKind};
use mdview_preview::wire::MAX_SIDE;
use mdview_render::{DARK, DocumentRenderer, Hit, LIGHT, Options, Theme};

use super::links;

pub struct Preview {
    id: u64,
    doc: Document,
    options: Options,
    top: f32,
    end: f32,
}

struct Active {
    id: u64,
    renderer: DocumentRenderer,
    rgba: Vec<u8>,
}

thread_local! {
    static MEASURE: RefCell<DocumentRenderer> = RefCell::new(DocumentRenderer::new());
    static VISIBLE: RefCell<VecDeque<Active>> = const { RefCell::new(VecDeque::new()) };
}

fn safe_document(raw: &str) -> Document {
    let mut doc = mdview_core::parse(raw);
    // `base_dir: None` is not a no-I/O policy: the renderer still accepts absolute paths.
    // Keep alt text, but prevent even an attempted local read or remote image lookup.
    for image in &mut doc.images {
        image.src = Default::default();
        image.dark_src = Default::default();
        image.light_src = Default::default();
    }
    // Neither answers nor embedded HTML may auto-launch the viewer's optional engine.
    for block in &mut doc.blocks {
        block.kind = match block.kind {
            BlockKind::Diagram { lang, .. } => BlockKind::CodeBlock { lang },
            BlockKind::Math { .. } => BlockKind::CodeBlock { lang: Default::default() },
            kind => kind,
        };
    }
    for span in &mut doc.spans {
        if matches!(span.kind, SpanKind::Math { .. }) {
            span.kind = SpanKind::Code;
        }
    }
    doc
}

fn theme(pal: &Palette) -> Theme {
    Theme {
        bg: pal.pane,
        text: pal.text,
        heading: pal.text,
        muted: pal.muted,
        accent: pal.accent,
        border: pal.line,
        code_bg: pal.field,
        head_bg: pal.bar,
        tint: pal.on,
        kw: pal.accent,
        string: pal.text,
        body_size: 14.0,
        body_line_height: 1.4,
        margin: 0.0,
        reading_width: f32::MAX,
        ..if pal.dark { DARK } else { LIGHT }
    }
}

impl Preview {
    pub fn new(raw: &str, pal: &Palette, scale: f32, width: f32) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let doc = safe_document(raw);
        let options = Options { width, scale, theme: theme(pal), max_height: None, base_dir: None };
        let (top, end) = MEASURE.with_borrow_mut(|r| {
            let page = r.layout_exact(&doc, &options);
            // Crop the standalone viewer's outer page padding; row headers supply our own.
            let top = page.blocks().first().map_or(0.0, |s| s.y);
            let end = (page.total_height() - (24.0 * scale).round()).ceil().max(top);
            (top, end)
        });
        Self { id: NEXT.fetch_add(1, Ordering::Relaxed), doc, options, top, end }
    }

    fn active<R>(&self, f: impl FnOnce(&mut Active) -> R) -> R {
        VISIBLE.with_borrow_mut(|cache| {
            let index = cache.iter().position(|a| a.id == self.id);
            let active = index.and_then(|i| cache.remove(i)).unwrap_or_else(|| {
                let mut renderer = DocumentRenderer::new();
                renderer.layout_exact(&self.doc, &self.options);
                Active { id: self.id, renderer, rgba: Vec::new() }
            });
            // Only recently visible answers retain raster and hit caches, not each transcript row.
            cache.push_front(active);
            cache.truncate(12);
            f(cache.front_mut().expect("active preview"))
        })
    }

    pub fn height(&self) -> f32 {
        (self.end - self.top) / self.options.scale
    }

    pub fn text(&self) -> &str {
        &self.doc.text
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let scale = self.options.scale;
        self.active(|a| a.renderer.hit_test(&self.doc, &self.options, x * scale, self.top + y * scale))
    }

    fn link_of(&self, hit: Hit) -> Option<&str> {
        let link = self.doc.links.get(hit.link? as usize)?;
        let dest = self.doc.str(link.dest);
        (!link.wiki && links::url(dest)).then_some(dest)
    }

    pub fn point(&self, x: f32, y: f32) -> (usize, Option<&str>) {
        match self.hit(x, y) {
            Some(hit) => (hit.offset as usize, self.link_of(hit)),
            None if y > self.height() / 2.0 => (self.text().len(), None),
            None => (0, None),
        }
    }

    pub fn draw(&self, p: &mut Painter, area: Rect, clip: Rect, selection: Option<Range<usize>>, pal: &Palette) {
        let scale = self.options.scale;
        let from = (self.top + ((clip.y - area.y).max(0.0) * scale).floor()).min(self.end);
        let to = (self.top + ((clip.bottom() - area.y) * scale).ceil()).min(self.end);
        if to <= from {
            return;
        }
        self.active(|a| {
            let frame = a.renderer.view_frame(&self.doc, &self.options, from, (to - from).min(MAX_SIDE as f32));
            let bitmap = &frame.bitmap;
            if frame.band != Some((0, 0)) || a.rgba.len() != bitmap.pixels.len() {
                a.rgba.clear();
                a.rgba.extend_from_slice(&bitmap.pixels);
                for pixel in a.rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2); // Opaque BGRA framebuffer → Painter's RGBA input.
                }
            }
            let frame_rect = Rect::new(
                area.x,
                area.y + (frame.top - self.top) / scale,
                bitmap.width as f32 / scale,
                bitmap.height as f32 / scale,
            );
            p.image(&a.rgba, bitmap.width, bitmap.height, frame_rect, 0.0, 1.0);
            if let Some(range) = selection.filter(|r| !r.is_empty()) {
                for r in a.renderer.selection_rects(&self.doc, &self.options, range.start as u32, range.end as u32) {
                    p.fill_a(
                        Rect::new(area.x + r.x / scale, area.y + (r.y - self.top) / scale, r.w / scale, r.h / scale),
                        pal.selection,
                        0.36,
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_ui::theme::LIGHT;

    #[test]
    fn only_web_links_are_actionable_at_renderer_hit_positions() {
        let p = Preview::new(
            "[web](https://example.test) [local](notes.md) [mail](mailto:example@example.test)",
            &LIGHT,
            1.0,
            320.0,
        );
        let found: std::collections::BTreeSet<_> = (0..320).filter_map(|x| p.point(x as f32, 8.0).1).collect();
        assert_eq!(found, ["https://example.test"].into());
        assert!(p.text().contains("web local mail"));
    }

    #[test]
    fn untrusted_media_and_engine_jobs_become_inert() {
        let source = "![alt](C:/example/private.png)\n\n```mermaid\nA --> B\n```\n\nInline $x$.";
        let original = mdview_core::parse(source);
        assert!(original.blocks.iter().any(|b| matches!(b.kind, BlockKind::Diagram { .. })));
        assert!(original.spans.iter().any(|s| matches!(s.kind, SpanKind::Math { .. })));
        let doc = safe_document(source);
        assert!(!doc.images.is_empty());
        assert!(doc.images.iter().all(|i| i.src.len == 0 && i.dark_src.len == 0 && i.light_src.len == 0));
        assert!(!doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Diagram { .. } | BlockKind::Math { .. })));
        assert!(!doc.spans.iter().any(|s| matches!(s.kind, SpanKind::Math { .. })));
    }

    #[test]
    fn rust_and_shell_fences_rasterize_with_syntax_colors() {
        for raw in ["```rust\nfn demo() -> &str { \"night\" }\n```", "```sh\nif true; then echo \"night\"; fi\n```"] {
            let p = Preview::new(raw, &LIGHT, 1.0, 320.0);
            assert!(matches!(p.doc.blocks[0].kind, BlockKind::CodeBlock { .. }));
            p.active(|a| {
                let frame = a.renderer.view_frame(&p.doc, &p.options, p.top, p.height());
                assert!(
                    frame.bitmap.pixels.as_chunks::<4>().0.iter().any(|px| {
                        let (lo, hi) = px[..3].iter().fold((255, 0), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                        hi - lo > 35
                    }),
                    "{raw}"
                );
            });
        }
    }
}
