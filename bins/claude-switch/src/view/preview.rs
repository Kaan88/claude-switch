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

/// A paragraph that holds only this is the room kept above a code block for its header.
pub const SPACER: &str = "\u{200b}";

/// `text` as it is copied: without the spacers kept for code headers, nor the blank line each
/// left between the paragraphs around it.
pub fn copyable(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(SPACER) {
        return text.into();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.contains(SPACER) && line.trim_matches('\u{200b}').is_empty() {
            let blank = |l: &&str| l.trim().is_empty();
            if out.last().is_some_and(blank) && lines.get(i + 1).is_some_and(blank) {
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    out.join("\n").into()
}

/// A top-level code block of a rendered answer, in logical px from the text's top.
pub struct CodeBlock<'a> {
    pub top: f32,
    pub height: f32,
    pub code: &'a str,
    /// The fence's language; empty when it names none.
    pub lang: &'a str,
    /// The room above the box that was kept for a header; 0 when there is none (a block the
    /// renderer placed without the spacer: an indented one, or one in a file's preview).
    pub room: f32,
}

/// `raw` with room for a header before every fenced code block that starts a line: a paragraph
/// of its own that holds a zero-width space (a no-break space would take no line). The renderer draws a code block as one box with no
/// header and has no setting for one, so the header is drawn in this room, joined to the box.
fn with_header_room(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    // The fence that is open: its character and length.
    let mut open: Option<(char, usize)> = None;
    for line in raw.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let fence = text.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = fence.map_or(0, |c| text.chars().take_while(|x| *x == c).count());
        match (open, fence) {
            (None, Some(c)) if run >= 3 && !(c == '`' && text[run..].contains('`')) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                out.push_str(SPACER);
                out.push_str("\n\n");
                open = Some((c, run));
            }
            (Some((c, len)), Some(d)) if c == d && run >= len && text[run..].trim().is_empty() => open = None,
            _ => {}
        }
        out.push_str(line);
    }
    out
}

pub struct Preview {
    id: u64,
    doc: Document,
    options: Options,
    top: f32,
    end: f32,
    local: Vec<(Range<usize>, String)>,
    underlines: Vec<Range<usize>>,
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

fn markdown_paths(doc: &Document) -> Vec<(Range<usize>, String)> {
    let mut excluded: Vec<_> = doc
        .spans
        .iter()
        .filter(|s| matches!(s.kind, SpanKind::Link(_)))
        .map(|s| s.start as usize..s.end as usize)
        .collect();
    excluded.extend(
        doc.blocks
            .iter()
            .filter(|b| {
                matches!(b.kind, BlockKind::CodeBlock { .. } | BlockKind::Diagram { .. } | BlockKind::Math { .. })
            })
            .map(|b| b.text.usize()),
    );
    excluded.sort_unstable_by_key(|r| r.start);
    // Excluded ranges can nest. Coalescing gives monotonic ends as well as starts.
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in excluded {
        if let Some(last) = merged.last_mut().filter(|last| r.start <= last.end) {
            last.end = last.end.max(r.end);
        } else {
            merged.push(r);
        }
    }
    let outside = |r: &Range<usize>| {
        let at = merged.partition_point(|e| e.end <= r.start);
        merged.get(at).is_none_or(|e| r.end <= e.start)
    };
    let mut paths: Vec<_> = doc
        .spans
        .iter()
        .filter(|s| s.kind == SpanKind::Code)
        .map(|s| s.start as usize..s.end as usize)
        .filter_map(|r| {
            let name = doc.text.get(r.clone())?;
            (links::target(name) && !links::url(name) && links::is_markdown(std::path::Path::new(name)))
                .then(|| (r, name.to_owned()))
        })
        .collect();
    paths.retain(|(r, _)| outside(r));
    paths.sort_unstable_by_key(|(r, _)| r.start);
    let code: Vec<_> = paths.iter().map(|(r, _)| r.clone()).collect();
    paths.extend(links::detect(&doc.text).into_iter().filter(|(r, name)| {
        let at = code.partition_point(|e| e.end <= r.start);
        !links::url(name)
            && links::is_markdown(std::path::Path::new(name))
            && outside(r)
            && code.get(at).is_none_or(|e| r.end <= e.start)
    }));
    paths.sort_unstable_by_key(|(r, _)| r.start);
    paths
}

pub fn markdown_names(raw: &str) -> Vec<String> {
    let doc = mdview_core::parse(raw);
    let mut names: Vec<_> = markdown_paths(&doc).into_iter().map(|(_, name)| name).collect();
    names.extend(doc.links.iter().filter(|l| !l.wiki).filter_map(|l| links::markdown_destination(doc.str(l.dest))));
    names
}

/// The colour of the renderer's code box: the conversation draws a code block's header joined to
/// it, in the same colour.
pub fn code_ground(pal: &Palette) -> mdshared_ui::gfx::Rgb {
    pal.field
}

fn theme(pal: &Palette, size: f32) -> Theme {
    Theme {
        bg: pal.page,
        text: pal.text,
        heading: pal.text,
        muted: pal.muted,
        accent: pal.accent,
        border: pal.line,
        code_bg: code_ground(pal),
        head_bg: pal.bar,
        tint: pal.on,
        kw: pal.accent,
        string: pal.text,
        body_size: size,
        body_line_height: 1.5,
        margin: 0.0,
        reading_width: f32::MAX,
        ..if pal.dark { DARK } else { LIGHT }
    }
}

impl Preview {
    /// `files` is present for transcript prose, absent for file previews (which cannot open
    /// additional destinations). The renderer still supplies layout, selection, syntax and pixels.
    pub fn new(raw: &str, pal: &Palette, scale: f32, width: f32, size: f32, files: Option<&links::Files>) -> Self {
        Self::build(raw, pal, scale, width, size, files)
    }

    /// As [`Preview::new`], for an answer in the conversation: with room above each fenced code
    /// block for the header the conversation draws there (the language and Copy).
    pub fn with_code_headers(
        raw: &str,
        pal: &Palette,
        scale: f32,
        width: f32,
        size: f32,
        files: Option<&links::Files>,
    ) -> Self {
        Self::build(&with_header_room(raw), pal, scale, width, size, files)
    }

    fn build(raw: &str, pal: &Palette, scale: f32, width: f32, size: f32, files: Option<&links::Files>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut doc = safe_document(raw);
        let mut local = Vec::new();
        if let Some(files) = files {
            local = markdown_paths(&doc);
            local.retain(|(_, name)| files.contains_key(name));
        }
        let underlines = local.iter().map(|(r, _)| r.clone()).collect();
        let targets: Vec<_> = doc
            .links
            .iter()
            .map(|l| {
                if l.wiki {
                    return None;
                }
                let dest = doc.str(l.dest);
                let files = files?;
                if links::url(dest) {
                    Some(None)
                } else {
                    links::markdown_destination(dest).filter(|name| files.contains_key(name)).map(Some)
                }
            })
            .collect();
        for span in &mut doc.spans {
            if let SpanKind::Link(index) = span.kind {
                match targets.get(index as usize).and_then(Option::as_ref) {
                    Some(Some(name)) => local.push((span.start as usize..span.end as usize, name.clone())),
                    Some(None) => continue,
                    // Quote carries no link or styling in the renderer; keep the span structure
                    // intact so any nested emphasis or HTML spans retain their parents.
                    None => span.kind = SpanKind::Quote,
                }
            }
        }
        local.sort_unstable_by_key(|(range, _)| range.start);
        let options = Options { width, scale, theme: theme(pal, size), max_height: None, base_dir: None };
        let (top, end) = MEASURE.with_borrow_mut(|r| {
            let page = r.layout_exact(&doc, &options);
            // Crop the standalone viewer's outer page padding; this surface supplies its own.
            let top = page.blocks().first().map_or(0.0, |s| s.y);
            let end = (page.total_height() - (24.0 * scale).round()).ceil().max(top);
            (top, end)
        });
        Self { id: NEXT.fetch_add(1, Ordering::Relaxed), doc, options, top, end, local, underlines }
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
        if !hit.on_text {
            return None;
        }
        if let Some(link) = hit.link.and_then(|i| self.doc.links.get(i as usize)) {
            let dest = self.doc.str(link.dest);
            if links::url(dest) {
                return Some(dest);
            }
        }
        let offset = hit.offset as usize;
        let at = self.local.partition_point(|(r, _)| r.start <= offset);
        let (range, name) = self.local.get(at.checked_sub(1)?)?;
        range.contains(&offset).then_some(name)
    }

    /// The top-level code blocks, for the header or the Copy button the conversation draws.
    pub fn code_blocks(&self) -> Vec<CodeBlock<'_>> {
        let blocks: Vec<_> = self.active(|a| {
            a.renderer
                .page_layout()
                .map_or_else(Vec::new, |page| page.blocks().iter().map(|b| (b.block, b.y, b.height)).collect())
        });
        let scale = self.options.scale;
        let spacer = |index: u32| {
            self.doc.blocks.get(index as usize).is_some_and(|b| {
                matches!(b.kind, BlockKind::Paragraph { .. })
                    && self.doc.text.get(b.text.usize()).map(|text| text.trim_end_matches(['\r', '\n'])) == Some(SPACER)
            })
        };
        blocks
            .iter()
            .enumerate()
            .filter_map(|(at, &(index, y, h))| {
                let block = self.doc.blocks.get(index as usize)?;
                let BlockKind::CodeBlock { lang } = block.kind else { return None };
                let code = self.doc.text.get(block.text.usize())?;
                // The spacer before it, when this answer was laid out with one.
                let room = at
                    .checked_sub(1)
                    .and_then(|before| blocks.get(before))
                    .filter(|(before, ..)| spacer(*before))
                    .map_or(0.0, |(_, before_y, _)| (y - before_y) / scale);
                Some(CodeBlock {
                    top: (y - self.top) / scale,
                    height: h / scale,
                    code,
                    lang: self.doc.str(lang).trim(),
                    room,
                })
            })
            .collect()
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
            if !self.underlines.is_empty()
                && let Some(page) = a.renderer.page_layout()
            {
                let blocks = page.blocks();
                let first = blocks.partition_point(|b| b.y + b.height <= from);
                let end = blocks.partition_point(|b| b.y < to);
                if let Some((first, last)) = blocks.get(first..end).and_then(|b| b.first().zip(b.last())) {
                    let start = self.doc.blocks[first.block as usize].text.start as usize;
                    let stop = self.doc.blocks[last.block as usize].text.end as usize;
                    let lo = self.underlines.partition_point(|r| r.end <= start);
                    let hi = self.underlines.partition_point(|r| r.start < stop);
                    for range in self.underlines.get(lo..hi).unwrap_or_default() {
                        for r in
                            a.renderer.selection_rects(&self.doc, &self.options, range.start as u32, range.end as u32)
                        {
                            let y = area.y + (r.y + r.h - self.top) / scale - p.px();
                            if y >= clip.y && y < clip.bottom() {
                                p.fill(Rect::new(area.x + r.x / scale, y, r.w / scale, p.px()), pal.accent);
                            }
                        }
                    }
                }
            }
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
    fn only_existing_markdown_and_web_links_are_actionable_at_renderer_hit_positions() {
        let demo = crate::demo::Transcript::new().unwrap();
        let files = links::Files::from([(
            "field-notes.md".into(),
            links::resolve("field-notes.md", demo.cwd(), &Default::default()).unwrap(),
        )]);
        let p = Preview::new(
            "[web](https://example.test) [local](field-notes.md#Heading) `field-notes.md` field-notes.md \
             [missing](gone.md) [mail](mailto:example@example.test)",
            &LIGHT,
            1.0,
            800.0,
            14.0,
            Some(&files),
        );
        let found: std::collections::BTreeSet<_> = (0..800).filter_map(|x| p.point(x as f32, 8.0).1).collect();
        assert_eq!(found, ["https://example.test", "field-notes.md"].into());
        assert_eq!(p.local.len(), 3);
        assert_eq!(p.doc.spans.iter().filter(|s| matches!(s.kind, SpanKind::Link(_))).count(), 2);
        assert!(p.text().contains("web local field-notes.md field-notes.md missing mail"));
    }

    #[test]
    fn a_previewed_file_cannot_supply_new_clickable_destinations() {
        let p = Preview::new("[other](private.md) [web](https://example.test)", &LIGHT, 1.0, 320.0, 14.0, None);
        assert!((0..320).all(|x| p.point(x as f32, 8.0).1.is_none()));
        assert!(!p.doc.spans.iter().any(|s| matches!(s.kind, SpanKind::Link(_))));
    }

    #[test]
    fn prose_paths_and_explicit_destinations_do_not_infer_links_from_labels_or_fences() {
        let raw = "[fake-label.md](https://example.test) [guide](docs/Field%20Guide.md#heading) \
                   `docs/Second Guide.md` or notes.md.\n\n```sh\ncat do-not-link.md\n```";
        assert_eq!(markdown_names(raw), ["docs/Second Guide.md", "notes.md", "docs/Field Guide.md"]);
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
    fn a_code_header_is_joined_to_the_box_in_the_boxs_own_colour() {
        for pal in [mdshared_ui::theme::DARK, mdshared_ui::theme::LIGHT] {
            assert_eq!(theme(&pal, 14.0).code_bg, code_ground(&pal));
        }
    }

    #[test]
    fn the_room_kept_for_code_headers_is_never_copied() {
        let shown = format!("Run this:\n\n{SPACER}\n\ncargo test\n\nThen this:\n{SPACER}\ncargo fmt");
        assert_eq!(copyable(&shown), "Run this:\n\ncargo test\n\nThen this:\ncargo fmt");
        assert!(!copyable(&format!("{SPACER}\n\nx")).contains(SPACER));
        assert_eq!(copyable("no spacer\n\nhere"), "no spacer\n\nhere");
    }

    #[test]
    fn an_answers_fenced_blocks_get_room_for_a_header_and_say_their_language() {
        let raw =
            "Before:\n```rust\nfn a() {}\n```\nand\n\n~~~\nplain ``` inside\n~~~\n\n- item\n\n      indented code\n";
        let spaced = with_header_room(raw);
        assert_eq!(spaced.matches(SPACER).count(), 2, "one spacer per fenced block, none for a fence inside one");
        assert!(spaced.contains(&format!("Before:\n\n{SPACER}\n\n```rust\n")), "{spaced:?}");
        assert_eq!(with_header_room("no code here"), "no code here");
        // An inline run of backticks does not open a fence.
        assert_eq!(with_header_room("```not a fence``` text"), "```not a fence``` text");
        let p = Preview::with_code_headers(raw, &LIGHT, 1.0, 400.0, 16.0, None);
        let blocks = p.code_blocks();
        let fenced: Vec<_> = blocks.iter().filter(|b| b.room > 0.0).collect();
        assert_eq!(fenced.iter().map(|b| b.lang).collect::<Vec<_>>(), ["rust", ""]);
        assert!(fenced.iter().all(|b| b.room >= 28.0), "{:?}", fenced.iter().map(|b| b.room).collect::<Vec<_>>());
        assert_eq!(fenced[0].code.trim_end(), "fn a() {}");
        assert_eq!(fenced[1].code.trim_end(), "plain ``` inside");
        // A file's preview has no header room: its blocks are as the renderer draws them.
        let plain = Preview::new(raw, &LIGHT, 1.0, 400.0, 16.0, None);
        assert!(plain.code_blocks().iter().all(|b| b.room == 0.0));
        assert!(plain.height() < p.height());
    }

    #[test]
    fn rust_and_shell_fences_rasterize_with_syntax_colors() {
        for raw in ["```rust\nfn demo() -> &str { \"night\" }\n```", "```sh\nif true; then echo \"night\"; fi\n```"] {
            let p = Preview::new(raw, &LIGHT, 1.0, 320.0, 14.0, None);
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
