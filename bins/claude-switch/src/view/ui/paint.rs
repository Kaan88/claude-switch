use mdshared_ui::gfx::mix;
use mdshared_ui::tokens::legible;

use super::rows::{CHIP_H, Thumb, chip_rect, chips_height, thumb_rect, thumbs_height};
use super::*;

impl Ui {
    pub(super) fn conversation_footer(&mut self, p: &mut Painter, pal: &Palette, r: Rect, visible: bool, main: bool) {
        let stacked = r.h == 49.0;
        let h = 26.0;
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

    pub(super) fn paint_rows(&mut self, p: &mut Painter, pal: &Palette, m: &mut Model) {
        // Rows are laid out in the whole body, and drawn above the jump button's foot.
        let body = self.shown_body();
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
                // A neutral bubble; answers have no mark and no bubble.
                Style::Prompt => p.rbox(frame, 16.0_f32.min(frame.h / 2.0), crate::view::look::bubble(pal)),
                Style::Answer => {}
                Style::Tool | Style::Group | Style::Edit => {
                    let action = match &row.id {
                        Id::Tool(id) => Action::Tool(*id),
                        Id::Group(id) => Action::Group(*id),
                        _ => unreachable!("tool rows have tool identities"),
                    };
                    let head = Rect::new(frame.x, frame.y, frame.w, row.head);
                    let hot = frame_hit(head, action);
                    let jumped = self.tool_jump.is_some_and(|(id, _)| row.id == Id::Tool(id));
                    if row.style == Style::Edit {
                        let colours = self.diff.unwrap_or_else(|| crate::view::look::Diff::fallback(pal));
                        draw_edit(p, pal, colours, frame, row, hot, jumped);
                    } else {
                        // The CLI's own "ctrl+b" offer, on the running command's line.
                        let offered = match (&row.id, self.offer) {
                            (Id::Tool(id), Some((offer, busy))) if *id == offer => Some(busy),
                            (Id::Group(_), Some((_, busy))) if row.offer => Some(busy),
                            _ => None,
                        };
                        let mut reserve = 0.0;
                        if let Some(busy) = offered.filter(|_| head.w >= 260.0) {
                            let label = if busy { "Moving to background…" } else { "Run in background" };
                            let w = p.measure(Face::Ui, 12.0, label) + 22.0;
                            let pill = Rect::new(head.right() - w, head.cy() - 12.0, w, 24.0);
                            let over = !busy && pill.contains(mouse.0, mouse.1);
                            let edge = crate::view::look::edge(pal);
                            p.frame(pill, 12.0, if over { pal.hover } else { pal.page }, edge);
                            p.text_center(Face::Ui, 12.0, pill, label, if busy { pal.muted } else { pal.text });
                            self.offer_rect = intersection(pill, body);
                            reserve = w + 8.0;
                        }
                        if row.background && head.w >= 120.0 + reserve {
                            let badge = Rect::new(head.right() - reserve - 76.0, head.cy() - 8.0, 68.0, 16.0);
                            let Id::Tool(id) = row.id else { unreachable!() };
                            let hot = frame_hit(badge, Action::Shell(id));
                            p.rbox(badge, 4.0, if hot { pal.hover } else { pal.on });
                            p.text_center(Face::Semibold, 10.5, badge, "background", pal.accent);
                            reserve += 76.0;
                        }
                        let look = ToolLook { chat: self.chat, hot, jumped, reserve };
                        if row.step.is_some() {
                            draw_step(p, pal, frame, row, look);
                        } else {
                            draw_line(p, pal, head, row, look);
                        }
                    }
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
                if row.boxed {
                    // What a call printed: in a box of its own under the call's line.
                    let (px, py_) = super::rows::BOX_PAD;
                    let r = Rect::new(tx - px, py - py_, geo.tw + 2.0 * px, text.height() + 2.0 * py_);
                    p.frame(r, 8.0, pal.field, pal.line);
                }
                // A diff's lines are never wrapped: its row cuts them.
                let cut = (row.style == Style::Edit).then(|| p.clip(Rect::new(tx, py, geo.tw, text.height())));
                text.draw(p, Rect::new(tx, py, geo.tw, text.height()), body, range, pal);
                if let Some(cut) = cut {
                    p.restore(cut);
                }
                for block in text.code_blocks() {
                    let code = block.code;
                    let copied = self.copied.as_deref() == Some(code);
                    let word = if copied { "Copied" } else { "Copy" };
                    let box_top = py + block.top;
                    if block.room >= CODE_HEAD && geo.tw >= 120.0 {
                        // A header joined to the renderer's box: the language left, Copy right.
                        let head = Rect::new(tx, box_top - CODE_HEAD, geo.tw, CODE_HEAD);
                        if intersection(Rect::new(head.x, head.y, head.w, head.h + 24.0), body).is_none() {
                            continue;
                        }
                        let lap = 12.0;
                        // Drawn as the renderer draws its box: a hairline of one device pixel.
                        let frame = Rect::new(head.x, head.y, head.w, head.h + lap);
                        p.rbox(frame, 6.0, pal.line);
                        p.rbox(frame.inset(p.px()), (6.0 - p.px()).max(0.0), pal.field);
                        // The seam: the box's own top edge and corners, and this frame's lower edge.
                        p.fill(Rect::new(head.x + p.px(), box_top - 1.0, head.w - 2.0 * p.px(), lap + 1.0), pal.field);
                        let lang = if block.lang.is_empty() { "text" } else { block.lang };
                        if !block.lang.is_empty() {
                            // The renderer's own label in the box's corner: the header has it now.
                            let w = p.measure(Face::Ui, 12.0, block.lang) + 26.0;
                            p.fill(Rect::new(head.right() - w - 2.0, box_top + 2.0, w, 22.0), pal.field);
                        }
                        let cy = head.cy() + 3.0;
                        p.text(Face::Ui, 11.5, head.x + 16.0, cy, lang, pal.muted, (head.w - 110.0).max(0.0));
                        let w = p.measure(Face::Ui, 11.5, word) + 34.0;
                        let r = Rect::new(head.right() - w - 6.0, cy - 11.0, w, 22.0);
                        let hot = frame_hit(r, Action::CopyCode(code.to_owned()));
                        if hot {
                            p.rbox(r, 5.0, pal.hover);
                        }
                        let ink = if hot { pal.text } else { pal.muted };
                        p.icon(mdshared_ui::glyph::COPY, 13.0, r.x + 12.0, cy, ink);
                        p.text(Face::Ui, 11.5, r.x + 24.0, cy, word, ink, w);
                        continue;
                    }
                    // No room for a header: Copy beside the renderer's own language label, not over it.
                    let label = if block.lang.is_empty() { 0.0 } else { p.measure(Face::Ui, 12.0, block.lang) + 14.0 };
                    let r = Rect::new(tx + geo.tw - 60.0 - label, box_top + 5.0, 54.0, 20.0);
                    if block.height < 30.0 || geo.tw < 120.0 + label || intersection(r, body).is_none() {
                        continue;
                    }
                    let hot = frame_hit(r, Action::CopyCode(code.to_owned()));
                    p.rbox(r, 4.0, if hot { pal.hover } else { pal.bar });
                    p.text_center(Face::Ui, 11.5, r, word, pal.muted);
                }
                py += text.height();
            }
            for (r, action) in paint_thumbs(p, pal, m, &row.pictures, tx, py, geo.tw, body) {
                frame_hit(r, action);
            }
            py += thumbs_height(geo.tw, row.pictures.len());
            if let (Some(actions), Id::Message(id)) = (row.actions, &row.id) {
                // Under the message, on pointing at it (or its buttons), and always under the
                // newest answer.
                let band = Rect::new(body.x, y, body.w, row.h + super::rows::ACTIONS_ROOM);
                if actions.newest || band.contains(mouse.0, mouse.1) {
                    let text = row.body.as_ref().map(|b| b.text());
                    let copied = text.is_some() && self.copied.as_deref() == text;
                    let mut buttons = Vec::new();
                    if text.is_some() {
                        let word = if copied { "Copied" } else { "Copy" };
                        buttons.push((mdshared_ui::glyph::COPY, word, Action::CopyMessage(*id)));
                    }
                    if actions.rewind {
                        buttons.push((mdshared_ui::glyph::UNDO, "Rewind", Action::Rewind(*id)));
                    }
                    if actions.branch {
                        buttons.push((crate::view::look::BRANCH, "Branch", Action::Branch(*id)));
                    }
                    let widths: Vec<f32> =
                        buttons.iter().map(|(_, word, _)| p.measure(Face::Ui, 11.5, word) + 32.0).collect();
                    let total = widths.iter().sum::<f32>() + 2.0 * (widths.len() as f32 - 1.0);
                    // Your message's under its bubble, at the right; an answer's at the left.
                    let mut x = if row.style == Style::Prompt { frame.right() - total } else { frame.x - 6.0 };
                    let top = y + row.h + 3.0;
                    for ((icon, word, action), w) in buttons.into_iter().zip(widths) {
                        let r = Rect::new(x, top, w, super::rows::ACTION_H);
                        x += w + 2.0;
                        let hot = frame_hit(r, action);
                        if hot {
                            p.rbox(r, 5.0, pal.hover);
                        }
                        let ink = if hot { pal.text } else { pal.muted };
                        p.icon(icon, 13.0, r.x + 13.0, r.cy(), ink);
                        p.text(Face::Ui, 11.5, r.x + 24.0, r.cy(), word, ink, w);
                    }
                }
            }
            if row.style == Style::Tool
                && let Some(id) = &row.agent
            {
                let py = py + if row.boxed { super::rows::BOX_PAD.1 } else { 0.0 };
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

/// The header of a code block: the language and Copy.
const CODE_HEAD: f32 = 28.0;

/// How a tool row is drawn this frame.
#[derive(Clone, Copy)]
struct ToolLook {
    chat: bool,
    /// The pointer is over its line.
    hot: bool,
    /// A jump from the Details panel points at it.
    jumped: bool,
    /// Room taken at the line's right end (a badge, "Run in background").
    reserve: f32,
}

/// A run of calls, or one call or notice by itself: one quiet line in words at the column's
/// edge, with what failed in the warning colour and a chevron after it. It is in the text colour
/// while a call runs, and under the pointer.
fn draw_line(p: &mut Painter, pal: &Palette, r: Rect, row: &Row, look: ToolLook) {
    let size = if look.chat { 13.5 } else { 12.0 };
    let ground = if look.jumped { pal.on } else { pal.page };
    if look.jumped {
        p.rbox(Rect::new(r.x - 4.0, r.y + 1.0, r.w + 4.0, r.h - 2.0), 5.0, ground);
    }
    let quiet = legible(pal.muted, ground, 4.5);
    let faint = legible(pal.faint, ground, 3.0);
    let warn = legible(pal.warn, ground, 4.5);
    let ink = if row.running || look.hot { pal.text } else { quiet };
    // A notice has no status: its title carries a warning or accent tone.
    let title = match row.tone {
        Tone::Warning if row.stamp.is_empty() => warn,
        Tone::Accent if row.stamp.is_empty() => legible(pal.accent, ground, 4.5),
        _ => ink,
    };
    let cy = r.cy();
    let end = (r.right() - look.reserve).max(r.x);
    let stamp = if row.stamp.is_empty() { String::new() } else { format!("· {}", row.stamp) };
    let stamp_w = if stamp.is_empty() { 0.0 } else { p.measure(Face::Ui, size - 0.5, &stamp) + 8.0 };
    let duration = if row.open { row.duration.as_str() } else { "" };
    let duration_w = if duration.is_empty() { 0.0 } else { p.measure(Face::Ui, size - 2.0, duration) + 10.0 };
    let room = (end - r.x - stamp_w - 18.0 - duration_w).max(0.0);
    let mut x = r.x;
    let title_max = if row.target.is_empty() { room } else { room * 0.6 };
    x += p.text(Face::Ui, size, x, cy, &row.title, title, title_max);
    if !row.target.is_empty() {
        let gap = p.measure(Face::Ui, size, " ");
        x += gap + p.text(Face::Ui, size, x + gap, cy, &row.target, ink, (r.x + room - x - gap).max(0.0));
    }
    x += 8.0;
    if !stamp.is_empty() {
        x += p.text(Face::Ui, size - 0.5, x, cy, &stamp, warn, stamp_w) + 8.0;
    }
    let chevron = if row.open { mdshared_ui::glyph::DOWN } else { mdshared_ui::glyph::RIGHT };
    p.icon(chevron, 11.0, x + 3.0, cy + 0.5, faint);
    if !duration.is_empty() {
        p.text_right(Face::Ui, size - 2.0, end, cy, duration, faint);
    }
}

/// A step of an open run, on its timeline: an icon for what it was on a thin line, the verb, the
/// target in the mono face, what went wrong in the warning colour, and how long it took at the
/// right.
fn draw_step(p: &mut Painter, pal: &Palette, frame: Rect, row: &Row, look: ToolLook) {
    use crate::view::ui::rows::STEP_INDENT;
    let (size, mono) = if look.chat { (13.0, 12.0) } else { (11.5, 11.0) };
    let head = Rect::new(frame.x, frame.y, frame.w, row.head);
    let cy = head.cy();
    let indent = STEP_INDENT.min(frame.w * 0.1);
    let (first, last) = row.step.unwrap_or((true, true));
    let top = if first { cy } else { frame.y };
    let bottom = if last { cy } else { frame.bottom() };
    if indent >= 20.0 {
        p.fill(Rect::new(frame.x + 7.0, top, p.px(), (bottom - top).max(0.0)), pal.line);
        // The page under the icon, so the line does not cross it.
        p.rbox(Rect::new(frame.x - 1.0, cy - 9.0, 18.0, 18.0), 9.0, pal.page);
        let failed = matches!(row.tone, Tone::Warning) && !row.stamp.is_empty();
        let (icon, tone) = match () {
            _ if failed => (mdshared_ui::glyph::CLOSE, pal.warn),
            _ if row.running => (row.kind.icon(), pal.text),
            _ => (row.kind.icon(), pal.muted),
        };
        p.icon(icon, 13.0, frame.x + 7.5, cy, tone);
    }
    let tx = frame.x + indent;
    let end = (head.right() - look.reserve).max(tx);
    let ground = match (look.jumped, look.hot) {
        (true, _) => pal.on,
        (false, true) => pal.hover,
        _ => pal.page,
    };
    if look.jumped || look.hot {
        p.rbox(Rect::new(tx - 6.0, head.y + 2.0, end - tx + 6.0, head.h - 4.0), 5.0, ground);
    }
    let quiet = legible(pal.muted, ground, 4.5);
    let faint = legible(pal.faint, ground, 3.0);
    let warn = legible(pal.warn, ground, 4.5);
    let soft = legible(crate::view::look::soft(pal), ground, 4.5);
    let duration_w = if row.duration.is_empty() { 0.0 } else { p.measure(Face::Ui, size - 1.5, &row.duration) + 10.0 };
    let stamp_w = if row.stamp.is_empty() { 0.0 } else { p.measure(Face::Ui, size - 1.0, &row.stamp) + 8.0 };
    let room = (end - tx - duration_w - stamp_w - 6.0).max(0.0);
    let verb = match row.tone {
        Tone::Warning if row.stamp.is_empty() => warn,
        Tone::Accent if row.stamp.is_empty() => legible(pal.accent, ground, 4.5),
        _ if row.running => pal.text,
        _ => quiet,
    };
    let mut x = tx;
    x += p.text(Face::Ui, size, x, cy, &row.title, verb, if row.target.is_empty() { room } else { room * 0.6 }) + 6.0;
    if !row.target.is_empty() {
        let (face, at) = if row.mono { (Face::Mono, mono) } else { (Face::Ui, size) };
        x += p.text(face, at, x, cy + 0.5, &row.target, soft, (tx + room - x).max(0.0)) + 8.0;
    }
    if !row.stamp.is_empty() {
        p.text(Face::Ui, size - 1.0, x, cy, &row.stamp, warn, stamp_w);
    }
    if !row.duration.is_empty() {
        p.text_right(Face::Ui, size - 1.5, end - 4.0, cy, &row.duration, faint);
    }
}

/// A file edit: a row with a thin border (a pencil, the path, the lines added in green and
/// removed in red, a chevron) that opens into the diff: tinted lines with their signs, and line
/// numbers when the transcript has them. The lines' text is the row's body, drawn by the caller.
fn draw_edit(
    p: &mut Painter,
    pal: &Palette,
    colours: crate::view::look::Diff,
    frame: Rect,
    row: &Row,
    hot: bool,
    jumped: bool,
) {
    use mdshared_ui::glyph;
    let ground = match (row.open, hot) {
        (true, _) => pal.field,
        (false, true) => pal.hover,
        _ => pal.page,
    };
    p.frame(frame, 9.0, ground, if jumped { pal.accent } else { pal.line });
    let head = Rect::new(frame.x, frame.y, frame.w, row.head);
    let cy = head.cy();
    let ink = colours.on(ground);
    p.icon(glyph::EDIT, 13.0, frame.x + 16.0, cy, legible(pal.muted, ground, 4.5));
    let plus = format!("+{}", row.counts.0);
    let minus = format!("−{}", row.counts.1);
    let counts_w = p.measure(Face::Semibold, 12.0, &plus) + 6.0 + p.measure(Face::Semibold, 12.0, &minus);
    let stamp_w = if row.stamp.is_empty() { 0.0 } else { p.measure(Face::Ui, 12.0, &row.stamp) + 10.0 };
    let mut x = frame.x + 32.0;
    let room = (frame.right() - 30.0 - counts_w - stamp_w - 12.0 - x).max(0.0);
    let path = row.target.replace('\\', "/");
    x += p.text(Face::Mono, 12.5, x, cy + 0.5, &path, pal.text, room) + 12.0;
    if frame.right() - 30.0 - x >= counts_w {
        x += p.text(Face::Semibold, 12.0, x, cy, &plus, ink.add, 80.0) + 6.0;
        x += p.text(Face::Semibold, 12.0, x, cy, &minus, ink.del, 80.0) + 10.0;
        if !row.stamp.is_empty() {
            p.text(Face::Ui, 12.0, x, cy, &row.stamp, legible(pal.muted, ground, 4.5), stamp_w);
        }
    }
    p.icon(if row.open { glyph::DOWN } else { glyph::RIGHT }, 11.0, frame.right() - 16.0, cy + 0.5, pal.faint);
    let Some(text) = row.body.as_ref().filter(|_| row.open && !row.diff.lines.is_empty()) else { return };
    p.fill(Rect::new(frame.x + 1.0, head.bottom(), frame.w - 2.0, p.px()), pal.line);
    let clip = p.clip(Rect::new(frame.x + 1.0, head.bottom() + 1.0, frame.w - 2.0, frame.h - row.head - 2.0));
    let line = text.height() / row.diff.lines.len() as f32;
    let top = frame.y + row.geo.ty;
    let gutter = row.geo.tx - row.geo.x;
    let faint = legible(pal.faint, ground, 3.0);
    for (i, diff) in row.diff.lines.iter().enumerate() {
        let r = Rect::new(frame.x + 1.0, top + i as f32 * line, frame.w - 2.0, line);
        let tone = match diff.sign {
            '+' => Some(ink.add),
            '-' => Some(ink.del),
            _ => None,
        };
        if let Some(tone) = tone {
            p.fill(r, mix(ground, tone, if pal.dark { 0.17 } else { 0.13 }));
        }
        if row.diff.breaks.contains(&i) {
            p.fill(Rect::new(r.x, r.y, r.w, p.px()), pal.line);
        }
        if let Some(number) = diff.number {
            p.text_right(Face::Mono, 11.0, frame.x + gutter - 26.0, r.cy(), &number.to_string(), faint);
        }
        if let Some(tone) = tone {
            let sign = if diff.sign == '+' { "+" } else { "−" };
            p.text(Face::Mono, 12.0, frame.x + gutter - 16.0, r.cy(), sign, tone, 12.0);
        }
    }
    if row.diff.more > 0 {
        let y = top + text.height() + 10.0;
        let more = format!("{} more lines in the file's change", row.diff.more);
        p.text(Face::Ui, 11.5, frame.x + 16.0, y, &more, faint, frame.w - 32.0);
    }
    p.restore(clip);
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
