use mdshared_ui::gfx::mix;
use mdshared_ui::tokens::legible;

use super::rows::{CHIP_H, TOOL_H, chip_rect};
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
            p.text(Face::Ui, 12.0, r.x, text_y, &work.label, pal.text, text_w.max(0.0));
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
        for row in &self.rows[start..] {
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
                    let ground = if pal.dark { pal.field } else { pal.pane };
                    let jumped = self.tool_jump.is_some_and(|(id, _)| row.id == Id::Tool(id));
                    p.frame(
                        frame,
                        5.0,
                        if hot && !row.open { pal.hover } else { ground },
                        if jumped { pal.accent } else { pal.line },
                    );
                    if row.open {
                        if hot {
                            p.rbox(header.inset(p.px()), 4.0, pal.hover);
                        }
                        if row.style == Style::Tool {
                            p.fill(
                                Rect::new(frame.x + 3.0, header.bottom(), (frame.w - 6.0).max(0.0), p.px()),
                                pal.line,
                            );
                        }
                    }
                    let badge = if row.background && header.w >= 120.0 {
                        let badge = Rect::new(header.right() - 92.0, header.y + 3.0, 68.0, header.h - 6.0);
                        let Id::Tool(id) = row.id else { unreachable!() };
                        let hot = frame_hit(badge, Action::Shell(id));
                        p.rbox(badge, 4.0, if hot { pal.hover } else { pal.on });
                        p.text_center(Face::Semibold, 10.5, badge, "background", pal.accent);
                        72.0
                    } else {
                        0.0
                    };
                    draw_tool_line(p, pal, header, row, color, if hot { pal.hover } else { ground }, badge);
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
                            let label = format!("{} · {}", output.kind(), output.title);
                            p.text(Face::Semibold, 11.0, r.x + 7.0, r.cy(), &label, pal.text, (r.w - 14.0).max(0.0));
                        }
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
                let range = selection(&self.selection, &row.id, text.text());
                text.draw(p, Rect::new(tx, py, geo.tw, text.height()), body, range, pal);
                py += text.height();
            }
            for &(id, i) in &row.pictures {
                let w = geo.tw.clamp(1.0, 240.0);
                let frame = Rect::new(tx, py + 4.0, w, PIC - 8.0);
                py += PIC;
                if intersection(frame, body).is_none() {
                    continue;
                }
                frame_hit(frame, Action::Picture(id, i));
                p.rbox(frame, 5.0, pal.field);
                if let Some(image) = m.picture(id, i, false) {
                    paint_image(p, &image, frame, 4.0, false);
                } else {
                    let hint =
                        if m.picture_failed(id, i, false) { "Preview unavailable" } else { "Loading preview…" };
                    p.text_center(Face::Ui, 12.0, frame, hint, pal.muted);
                }
            }
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

fn draw_tool_line(p: &mut Painter, pal: &Palette, r: Rect, row: &Row, status: [u8; 3], ground: [u8; 3], badge: f32) {
    let face = Face::Mono;
    let size = 11.5;
    let pad = 8.0_f32.min(r.w * 0.1);
    let cy = r.cy();
    let end = (r.right() - pad - 13.0).max(r.x + pad);
    let mut x = r.x + pad;
    let arrow = if row.open { "⌄" } else { "›" };
    p.text_center(Face::Semibold, 14.0, Rect::new(end, r.y, (r.right() - pad - end).max(0.0), r.h), arrow, pal.muted);
    if row.style == Style::Group {
        let width = end - x;
        let sw = (p.measure(face, size, &row.stamp) + 7.0).min(width * 0.56);
        p.text(face, size, x, cy, &row.title, pal.text, (width - sw).max(0.0));
        p.text(face, size, end - sw, cy, &row.stamp, legible(status, ground, 4.5), sw);
        return;
    }
    let end = (end - badge).max(x);
    let available = end - x;
    let status_w = (p.measure(face, size, &row.stamp) + 7.0).min(available * 0.41);
    let duration_w =
        if row.duration.is_empty() { 0.0 } else { (p.measure(face, size, &row.duration) + 7.0).min(available * 0.27) };
    let rest = (available - status_w - duration_w).max(0.0);
    let name_w = if row.target.is_empty() { rest } else { (p.measure(face, size, &row.title) + 7.0).min(rest * 0.61) };
    p.text(face, size, x, cy, &row.stamp, legible(status, ground, 4.5), status_w);
    x += status_w;
    p.text(face, size, x, cy, &row.title, legible(pal.accent, ground, 4.5), name_w);
    x += name_w;
    p.text(face, size, x, cy, &row.target, pal.text, (rest - name_w).max(0.0));
    p.text(face, size, end - duration_w, cy, &row.duration, pal.muted, duration_w);
}
