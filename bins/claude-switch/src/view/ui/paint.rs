use mdshared_ui::gfx::mix;
use mdshared_ui::tokens::legible;

use super::rows::{CHIP_H, TOOL_H, Thumb, chip_rect, chips_height, thumb_rect, thumbs_height};
use super::*;

impl Ui {
    pub(super) fn conversation_footer(&mut self, p: &mut Painter, pal: &Palette, r: Rect, visible: bool, main: bool) {
        let stacked = r.h == 49.0;
        let h = if self.chat { 23.0 } else { 26.0 };
        let y = if stacked { r.bottom() - h } else { r.y + (r.h - h) / 2.0 };
        let latest = Rect::new(r.right() - 110.0, y, 110.0, h);
        if let Some(work) = self.working.as_ref().filter(|_| main) {
            let edge = if visible { latest.x - 5.0 } else { r.right() };
            let stop = Rect::new(edge - 48.0, y, 48.0, h);
            let (text_y, text_w) = if stacked { (r.y + 11.5, r.w) } else { (stop.cy(), stop.x - r.x - 6.0) };
            crate::view::working::draw_line(p, 12.0, r.x, text_y, &work.label, pal.text, text_w);
            if let Some(key) = work.stop {
                self.button(p, pal, stop, "Stop", Action::Stop(key), false);
            } else {
                p.text_center(Face::Ui, 12.0, stop, "Stop", pal.muted);
            }
        }
        if visible {
            if self.follow.active() {
                p.text_center(Face::Ui, 12.0, latest, "Following latest", pal.muted);
            } else {
                self.button(p, pal, latest, "Jump to latest", Action::Latest, false);
            }
        }
    }

    pub(super) fn paint_rows(&mut self, p: &mut Painter, pal: &Palette, m: &mut Model, phase: f32, motion: bool) {
        let body = self.body;
        let clip = p.clip(body);
        let start = self.rows.partition_point(|r| r.top + r.h < self.offset);
        let mouse = self.mouse;
        let ends = self.selection.as_ref().and_then(|s| s.ends(&self.rows));
        for (index, row) in self.rows.iter().enumerate().skip(start) {
            let y = body.y + row.top - self.offset;
            if y >= body.bottom() {
                break;
            }
            let geo = row.geo;
            let frame = Rect::new(body.x + geo.x, y, geo.w, row.h);
            let header = Rect::new(frame.x, frame.y, frame.w, TOOL_H);
            let tone = match row.tone {
                Tone::Accent => pal.accent,
                Tone::Normal => pal.text,
                Tone::Muted => pal.muted,
                Tone::Warning => pal.warn,
            };
            let tx = body.x + geo.tx;
            let mut py = y + geo.ty;
            let mut frame_hit = |r, action| {
                let Some(hit) = intersection(r, body) else { return false };
                self.hits.push((hit, action));
                hit.contains(mouse.0, mouse.1)
            };
            match row.style {
                Style::Prompt => p.rbox(frame, 11.0, mix(pal.page, pal.accent, if pal.dark { 0.17 } else { 0.11 })),
                Style::Answer => {
                    let glyph = Rect::new(body.x, y, geo.x - 3.0, if self.chat { 22.0 } else { 19.0 });
                    p.text_center(Face::Semibold, if self.chat { 18.0 } else { 16.0 }, glyph, "✳︎", pal.accent);
                }
                Style::Tool | Style::Group => {
                    let action = match &row.id {
                        Id::Tool(id) => Action::Tool(*id),
                        Id::Group(id) => Action::Group(*id),
                        _ => unreachable!("tool rows have tool identities"),
                    };
                    let color = if row.busy && motion {
                        mix(tone, pal.muted, 0.21 * (1.0 + (phase * 4.0).sin()))
                    } else {
                        tone
                    };
                    let hot = frame_hit(header, action);
                    // Tool rows recede behind the answers: no box, only a soft fill when opened,
                    // hovered or failed, and an accent outline while a jump points at the row.
                    let failed = matches!(row.tone, Tone::Warning);
                    let jumped = self.tool_jump.is_some_and(|(id, _)| row.id == Id::Tool(id));
                    let panel = if pal.dark { pal.field } else { pal.pane };
                    let wash = mix(pal.page, pal.warn_bg, 0.75);
                    let fill = match (row.open, failed) {
                        (true, true) => Some(mix(panel, pal.warn_bg, 0.5)),
                        (true, false) => Some(panel),
                        (false, true) => Some(wash),
                        (false, false) => None,
                    };
                    if jumped {
                        p.frame(frame, 5.0, fill.unwrap_or(pal.page), pal.accent);
                    } else if let Some(fill) = fill {
                        p.rbox(frame, 5.0, fill);
                    }
                    let mut ground = fill.unwrap_or(pal.page);
                    if hot {
                        p.rbox(header, 5.0, pal.hover);
                        ground = pal.hover;
                    }
                    let badge = if row.background && header.w >= 120.0 {
                        let badge = Rect::new(header.right() - 76.0, header.y + 3.0, 68.0, header.h - 6.0);
                        let Id::Tool(id) = row.id else { unreachable!() };
                        let hot = frame_hit(badge, Action::Shell(id));
                        p.rbox(badge, 4.0, if hot { pal.hover } else { pal.on });
                        p.text_center(Face::Semibold, 10.5, badge, "background", pal.accent);
                        76.0
                    } else {
                        0.0
                    };
                    draw_tool_line(p, pal, header, row, color, ground, badge);
                    self.busy_visible |= row.busy && header.bottom() > body.y;
                }
                Style::Files => {
                    for (index, file) in row.files.iter().enumerate() {
                        let r = chip_rect(geo, index);
                        let r = Rect::new(body.x + r.x, y + r.y, r.w, r.h);
                        if intersection(r, body).is_none() {
                            continue;
                        }
                        let hot = frame_hit(r, Action::File(file.clone()));
                        p.frame(r, 5.0, if hot { pal.hover } else { pal.bar }, pal.line);
                        let name = file.path().file_name().and_then(|s| s.to_str()).unwrap_or("Markdown");
                        p.text(
                            Face::Semibold,
                            11.5,
                            r.x + 8.0,
                            r.cy(),
                            &format!("▤  {name}"),
                            pal.text,
                            (r.w - 16.0).max(0.0),
                        );
                    }
                    for (i, output) in row.outputs.iter().enumerate() {
                        let r = chip_rect(geo, i + row.files.len());
                        let r = Rect::new(body.x + r.x, y + r.y, r.w, r.h);
                        if intersection(r, body).is_some() {
                            let hot = frame_hit(r, Action::Output(output.clone()));
                            p.frame(r, 5.0, if hot { pal.hover } else { pal.bar }, pal.line);
                            let embed = output.embed();
                            p.icon(embed.icon(), 13.0, r.x + 13.0, r.cy(), pal.muted);
                            let label = output.chip();
                            p.text(Face::Semibold, 11.0, r.x + 25.0, r.cy(), &label, pal.text, (r.w - 32.0).max(0.0));
                        }
                    }
                    let top = y + chips_height(row);
                    for (r, action) in paint_thumbs(p, pal, m, &row.pictures, body.x + geo.x, top, geo.w, body) {
                        frame_hit(r, action);
                    }
                    continue;
                }
                Style::Card => {
                    let hot = row.agent.as_ref().is_some_and(|id| frame_hit(frame, Action::Agent(id.clone())));
                    p.rbox(frame, 7.0, if hot { pal.hover } else { pal.pane });
                    let status = row.status();
                    let sw = if status.is_empty() {
                        0.0
                    } else {
                        (p.measure(Face::Ui, 11.5, &status) + 10.0).min(geo.tw * 0.44)
                    };
                    let title = if row.agent.is_some() { format!("{}  ›", row.title) } else { row.title.clone() };
                    p.text(Face::Semibold, 13.0, tx, y + HEAD / 2.0, &title, tone, geo.tw - sw);
                    p.text(Face::Ui, 11.5, tx + geo.tw - sw, y + HEAD / 2.0, &status, pal.muted, sw);
                    self.busy_visible |= row.busy && y + HEAD > body.y;
                }
            }
            if let Some(text) = &row.body {
                let range = ends.and_then(|ends| select::Selection::range_in(ends, index, text.text()));
                text.draw(p, Rect::new(tx, py, geo.tw, text.height()), body, range, pal);
                for (top, h, code) in text.code_blocks() {
                    let r = Rect::new(tx + geo.tw - 60.0, py + top + 5.0, 54.0, 20.0);
                    if h < 30.0 || geo.tw < 120.0 || intersection(r, body).is_none() {
                        continue;
                    }
                    let copied = self.copied.as_deref() == Some(code);
                    let hot = frame_hit(r, Action::CopyCode(code.to_owned()));
                    p.rbox(r, 4.0, if hot { pal.hover } else { pal.bar });
                    p.text_center(Face::Ui, 11.5, r, if copied { "Copied" } else { "Copy" }, pal.muted);
                }
                py += text.height();
            }
            for (r, action) in paint_thumbs(p, pal, m, &row.pictures, tx, py, geo.tw, body) {
                frame_hit(r, action);
            }
            py += thumbs_height(geo.tw, row.pictures.len());
            if row.style == Style::Tool
                && let Some(id) = &row.agent
            {
                let r = Rect::new(tx, py + 3.0, geo.tw.min(164.0), CHIP_H - 4.0);
                let hot = frame_hit(r, Action::Agent(id.clone()));
                p.frame(r, 5.0, if hot { pal.hover } else { pal.bar }, pal.line);
                p.text_center(Face::Semibold, 11.5, r, "Open agent transcript  ›", pal.text);
            }
        }
        p.restore(clip);
    }
}

/// Pictures as thumbnails from `(x, y)` in `width`: each keeps its aspect within its slot, framed
/// at its own size once decoded, with its caption under it; a picture that cannot be shown keeps
/// the slot and says why. Returns the click targets.
#[allow(clippy::too_many_arguments)] // drawing call: a place plus the things to draw there
fn paint_thumbs(
    p: &mut Painter,
    pal: &Palette,
    m: &mut Model,
    thumbs: &[Thumb],
    x: f32,
    y: f32,
    width: f32,
    body: Rect,
) -> Vec<(Rect, Action)> {
    let mut hits = Vec::new();
    for (n, thumb) in thumbs.iter().enumerate() {
        let slot = thumb_rect(width, n);
        let slot = Rect::new(x + slot.x, y + slot.y, slot.w, slot.h);
        if intersection(Rect::new(slot.x, slot.y, slot.w, slot.h + 24.0), body).is_none() {
            continue;
        }
        let action = Action::Picture(thumb.id, thumb.index);
        let shown = if let Some(image) = m.picture(thumb.id, thumb.index, false) {
            let fit = thumb_fit(image.width as f32, image.height as f32, slot, p.scale);
            p.rbox(fit, 5.0, pal.field);
            paint_image(p, &image, fit, 5.0, false);
            hits.push((fit, action));
            fit
        } else {
            p.rbox(slot, 5.0, pal.field);
            let failed = m.picture_failed(thumb.id, thumb.index, false);
            p.icon(mdshared_ui::glyph::IMAGE, 20.0, slot.cx(), slot.cy() - 12.0, pal.muted);
            let hint = failed.unwrap_or("Loading preview…");
            p.text_center(Face::Ui, 11.5, Rect::new(slot.x, slot.cy() + 2.0, slot.w, 18.0), hint, pal.muted);
            hits.push((slot, action));
            slot
        };
        // The caption sits under the picture itself, in the slot's room.
        p.text(Face::Ui, 11.0, slot.x + 2.0, shown.bottom() + 11.0, &thumb.caption, pal.muted, slot.w - 4.0);
    }
    hits
}

/// A decoded picture's frame in its slot: aspect kept, never enlarged past its own pixels.
fn thumb_fit(w: f32, h: f32, slot: Rect, scale: f32) -> Rect {
    if w <= 0.0 || h <= 0.0 {
        return slot;
    }
    let k = (slot.w / w).min(slot.h / h).min(1.0 / scale.max(0.01));
    Rect::new(slot.x, slot.y, (w * k).max(1.0), (h * k).max(1.0))
}

/// One quiet line: a chevron, the status, the tool's name and its target, all in the muted
/// colours (the duration fainter still), set close after each other like a note. A failure keeps
/// its status in the warning colour; a running call keeps the pulsing accent.
fn draw_tool_line(p: &mut Painter, pal: &Palette, r: Rect, row: &Row, status: [u8; 3], ground: [u8; 3], badge: f32) {
    let (face, size) = (Face::Ui, 11.5);
    let quiet = legible(pal.muted, ground, 4.5);
    let faint = legible(pal.faint, ground, 3.0);
    let status = if matches!(row.tone, Tone::Muted) { faint } else { legible(status, ground, 4.5) };
    let pad = 6.0_f32.min(r.w * 0.1);
    let cy = r.cy();
    let arrow = if row.open { "⌄" } else { "›" };
    p.text_center(Face::Ui, 12.0, Rect::new(r.x + pad, r.y, 10.0, r.h), arrow, faint);
    let mut x = r.x + pad + 15.0;
    let end = (r.right() - pad - badge).max(x);
    let gap = 7.0;
    if row.style == Style::Group {
        let shown = p.text(face, size, x, cy, &row.title, quiet, ((end - x) * 0.7).max(0.0));
        x += shown + gap;
        p.text(face, size, x, cy, &row.stamp, status, (end - x).max(0.0));
        return;
    }
    let available = end - x;
    let duration_w = if row.duration.is_empty() { 0.0 } else { p.measure(face, 11.0, &row.duration) + gap };
    if !row.stamp.is_empty() {
        x += p.text(face, size, x, cy, &row.stamp, status, available * 0.4) + gap;
    }
    // A notice has no status: its title carries a warning or accent tone.
    let name = if row.stamp.is_empty() && !matches!(row.tone, Tone::Muted) { status } else { quiet };
    let room = (end - x - duration_w).max(0.0);
    let name_max = if row.target.is_empty() { room } else { room * 0.6 };
    x += p.text(face, size, x, cy, &row.title, name, name_max) + gap;
    if !row.target.is_empty() {
        x += p.text(face, size, x, cy, &row.target, quiet, (end - x - duration_w).max(0.0)) + gap;
    }
    p.text(face, 11.0, x.min(end - duration_w), cy, &row.duration, faint, duration_w);
}

#[cfg(test)]
mod quiet_tests;

#[cfg(test)]
mod thumb_tests {
    use super::super::rows::THUMB_H;
    use super::*;

    #[test]
    fn a_thumbnail_keeps_its_aspect_and_its_long_side_within_the_slot() {
        let slot = Rect::new(10.0, 20.0, 240.0, THUMB_H);
        let wide = thumb_fit(1920.0, 1080.0, slot, 1.0);
        assert_eq!((wide.x, wide.y, wide.w), (10.0, 20.0, 240.0));
        assert!((wide.h - 135.0).abs() < 0.01);
        let tall = thumb_fit(1000.0, 2000.0, slot, 1.0);
        assert_eq!((tall.w, tall.h), (90.0, 180.0));
        assert_eq!(thumb_fit(40.0, 20.0, slot, 2.0).w, 20.0, "a small picture is not enlarged");
        assert_eq!(thumb_fit(0.0, 20.0, slot, 1.0), slot);
    }

    #[test]
    fn the_demos_pictures_show_as_captioned_thumbnails_that_open_enlarged() {
        use mdshared_text::Bitmap;
        use mdshared_ui::gfx::Gfx;

        let demo = crate::demo::Transcript::new().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        let wait = || events.recv_timeout(std::time::Duration::from_secs(3)).expect("transcript worker wake");
        wait();
        assert!(model.collect());
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(1000, 2400));
        let pal = &mdshared_ui::theme::DARK;
        let view = Rect::new(0.0, 0.0, 1000.0, 2400.0);
        let mut ui = Ui::default();
        ui.set_chat(true);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let ts = std::time::Instant::now();
        let mut pictures = Vec::new();
        for _ in 0..10 {
            ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
            pictures = ui.hits.iter().filter(|(_, a)| matches!(a, Action::Picture(..))).cloned().collect::<Vec<_>>();
            if !pictures.is_empty() && pictures.iter().all(|(r, _)| r.h < THUMB_H || r.w < 240.0) {
                break;
            }
            let _ = events.recv_timeout(std::time::Duration::from_millis(500));
            model.collect();
        }
        assert!(!pictures.is_empty(), "a picture in the demo is drawn as a thumbnail");
        for (r, _) in &pictures {
            assert!(r.w <= 240.0 && r.h <= THUMB_H, "{r:?}");
        }
        if let Ok(path) = std::env::var("CS_SHOT") {
            bmp.write_png(std::path::Path::new(&path)).unwrap();
        }
    }
}
