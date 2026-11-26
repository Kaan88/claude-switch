use claude_switch_core::transcript::ChatInfo;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::theme::Palette;

use super::{Chat, Chip, Control, Drawer, Gate, commands, editor, paint::button, submission};
use crate::state::Target;

pub(super) struct Draw<'a> {
    pub pal: &'a Palette,
    pub focused: bool,
    pub wide: bool,
    pub hover: Option<Target>,
    pub hits: &'a mut Vec<(Rect, Target)>,
    pub pins: &'a [String],
}

impl Chat {
    pub(super) fn notice(&self, info: &ChatInfo) -> Option<&'static str> {
        self.reason
            .filter(|r| *r == super::Reason::Permission)
            .map(super::Reason::label)
            .or(self.error)
            .or_else(|| self.reason.map(super::Reason::label))
            .or_else(|| self.image_wait.as_ref().map(|_| "Waiting for Claude Code to confirm the picture…"))
            .or_else(|| {
                self.queue.as_ref().map(|_| {
                    if self.last_gate == Some(Gate::Running) {
                        "A turn is running. This will be sent when it ends."
                    } else if self.drawer_focus {
                        "Queue paused while the terminal has focus."
                    } else {
                        "Queued. Waiting for a verified, empty CLI prompt."
                    }
                })
            })
            .or_else(|| self.can_stop(info).then_some("A turn is running. You can queue the next message."))
    }

    pub(super) fn attachment_pages(&self) -> usize {
        self.chips.iter().filter(|c| matches!(c, Chip::Path(_))).count().div_ceil(3).max(1)
    }
}

impl Draw<'_> {
    pub fn composer(mut self, p: &mut Painter, chat: &mut Chat, info: &ChatInfo) {
        let l = chat.layout;
        let clip = p.clip(l.composer);
        p.rbox(l.box_frame, 11.0, if self.focused { self.pal.accent } else { self.pal.line });
        p.rbox(l.box_frame.inset(1.0), 10.0, self.pal.field);
        self.attachments(p, chat);
        self.editor(p, chat);
        self.notice(p, chat, info);
        self.toolbar(p, chat, info);
        p.restore(clip);
        self.suggestions(p, chat);
    }

    fn hit(&mut self, p: &mut Painter, r: Rect, label: &str, control: Control, picked: bool) {
        let target = Target::Chat(control);
        button(p, self.pal, r, label, self.hover == Some(target), picked);
        self.hits.push((r, target));
    }

    fn attachments(&mut self, p: &mut Painter, chat: &Chat) {
        if chat.chips.is_empty() {
            return;
        }
        let r = chat.layout.attachments;
        let mut x = r.x + 4.0;
        if let Some((i, Chip::Clipboard(pic))) =
            chat.chips.iter().enumerate().find(|(_, c)| matches!(c, Chip::Clipboard(_)))
        {
            let card = Rect::new(x, r.y + 1.0, if pic.thumbnail.is_some() { 65.0 } else { 138.0 }, 42.0);
            p.rbox(card, 5.0, self.pal.bar);
            if let Some(im) = &pic.thumbnail {
                let s = (62.0 / im.w as f32).min(40.0 / im.h as f32);
                let (w, h) = (im.w as f32 * s, im.h as f32 * s);
                p.image(&im.rgba, im.w, im.h, Rect::new(card.cx() - w / 2.0, card.cy() - h / 2.0, w, h), 4.0, 1.0);
            } else {
                p.text(Face::Ui, 11.5, card.x + 7.0, card.cy(), "Clipboard picture", self.pal.text, 111.0);
            }
            if !chat.locked() {
                self.hit(p, Rect::new(card.right() - 17.0, card.y, 17.0, 17.0), "×", Control::Remove(i), false);
            }
            x = card.right() + 6.0;
        }
        let pages = chat.attachment_pages();
        let more = if pages > 1 { 62.0 } else { 0.0 };
        let w = ((r.right() - x - more) / 3.0 - 6.0).clamp(45.0, 150.0);
        for (i, chip) in
            chat.chips.iter().enumerate().filter(|(_, c)| matches!(c, Chip::Path(_))).skip(chat.chip_page * 3).take(3)
        {
            let Chip::Path(path) = chip else { continue };
            let chip = Rect::new(x, r.y + 8.0, w, 26.0);
            p.rbox(chip, 6.0, self.pal.bar);
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("File");
            p.text(Face::Ui, 11.5, chip.x + 7.0, chip.cy(), name, self.pal.text, (chip.w - 26.0).max(0.0));
            if !chat.locked() {
                self.hit(p, Rect::new(chip.right() - 19.0, chip.y, 19.0, chip.h), "×", Control::Remove(i), false);
            }
            x += w + 6.0;
        }
        if pages > 1 {
            let label = format!("{}/{} ›", chat.chip_page + 1, pages);
            self.hit(p, Rect::new(r.right() - 62.0, r.y + 8.0, 60.0, 26.0), &label, Control::MoreChips, false);
        }
    }

    fn editor(&mut self, p: &mut Painter, chat: &mut Chat) {
        let e = chat.layout.editor;
        self.hits.push((e, Target::Chat(Control::Editor)));
        let c = p.clip(e);
        let x = e.x + 8.0;
        let width = (e.w - 16.0).max(0.0);
        let ed = &mut chat.editor;
        ed.flow(p, width, e.h);
        let selection = ed.selection();
        let caret_row = ed.caret_row();
        if ed.text.is_empty() {
            p.text(Face::Ui, 13.0, x, e.y + 14.0, "Message Claude Code…", self.pal.muted, width);
        }
        for (i, row) in ed.rows.iter().enumerate().skip(ed.scroll).take(ed.visible) {
            let y = e.y + 13.0 + (i - ed.scroll) as f32 * editor::LINE;
            let span = &ed.text[row.clone()];
            let mut before = |at| p.measure(Face::Mono, editor::TEXT_SIZE, &ed.text[row.start..at]);
            let start = selection.start.max(row.start);
            let end = selection.end.min(row.end);
            let selected = (end > start).then(|| (before(start), before(end)));
            let caret = (self.focused && i == caret_row).then(|| before(ed.caret.min(row.end)));
            if let Some((a, b)) = selected {
                p.fill(Rect::new(x + a, y - 8.0, b - a, 16.0), self.pal.on);
            }
            let painted;
            let span = if editor::hyphen(&ed.text, row) {
                painted = format!("{span}-");
                &painted
            } else {
                span
            };
            if !span.is_empty() {
                p.text(Face::Mono, editor::TEXT_SIZE, x, y, span, self.pal.text, width);
            }
            if let Some(at) = caret {
                p.fill(Rect::new(x + at, y - 7.0, p.px(), 15.0), self.pal.accent);
            }
        }
        p.restore(c);
    }

    fn notice(&mut self, p: &mut Painter, chat: &Chat, info: &ChatInfo) {
        let Some(note) = chat.notice(info) else { return };
        let r = chat.layout.notice.inset(2.0);
        p.rbox(r, 5.0, if chat.error.is_some() { self.pal.warn_bg } else { self.pal.bar });
        let mut x = r.right() - 3.0;
        let mut add = |text: &str, w: f32, control| {
            x -= w;
            self.hit(p, Rect::new(x, r.y + 2.0, w, r.h - 4.0), text, control, false);
            x -= 4.0;
        };
        if chat.locked() {
            add("Cancel", 54.0, Control::CancelSend);
            if chat.queue.is_some() {
                add("Send now", 76.0, Control::SendNow);
            }
        } else if chat.error == Some(submission::CHECK) && chat.last_gate == Some(Gate::Check) && chat.reason.is_none()
        {
            add("Send anyway", 100.0, Control::Override);
        } else if chat.last_gate == Some(Gate::Running) && chat.can_stop(info) {
            add("Stop", 47.0, Control::Stop);
        }
        p.text(Face::Ui, 11.5, r.x + 7.0, r.cy(), note, self.pal.text, (x - r.x - 11.0).max(0.0));
    }

    fn toolbar(&mut self, p: &mut Painter, chat: &Chat, info: &ChatInfo) {
        let l = chat.layout;
        let r = l.toolbar;
        let mut x = r.x + 1.0;
        let model = chat.cli_report.model.as_deref().or(info.model.as_deref()).unwrap_or("—");
        let effort = chat.cli_report.effort.unwrap_or("—");
        let model = format!("Model {} ▾", compact_model(model));
        let effort = format!("Effort {effort} ▾");
        let context = info.context.map_or_else(|| "Context —".into(), |n| format!("Context {}", tokens(n)));
        for (label, w, control) in [
            (model.as_str(), 120.0, Control::Model),
            (effort.as_str(), 80.0, Control::Effort),
            (context.as_str(), 90.0, Control::Context),
            ("Compact", 58.0, Control::Compact),
            ("Clear", 43.0, Control::Clear),
            ("/ …", 35.0, Control::Commands),
        ] {
            self.hit(
                p,
                Rect::new(x, r.y + 1.0, w, 24.0),
                label,
                control,
                control == Control::Commands && chat.commands.open,
            );
            x += w + 4.0;
        }
        let y = r.y + if l.stacked { 28.0 } else { 1.0 };
        let rsend = Rect::new(r.right() - 63.0, y, 61.0, 24.0);
        let target = Target::Chat(Control::Send);
        let armed = !chat.locked() && matches!(chat.last_gate, Some(Gate::Ready | Gate::Running));
        let label = if chat.image_wait.is_some() {
            "Waiting"
        } else if chat.queue.is_some() {
            "Queued"
        } else if chat.last_gate == Some(Gate::Running) {
            "Queue"
        } else {
            "Send"
        };
        if armed {
            p.rbox(rsend, 6.0, self.pal.accent);
            if self.hover == Some(target) {
                p.rbox(rsend.inset(2.0), 4.0, self.pal.on_accent);
            }
            let ink = if self.hover == Some(target) { self.pal.accent } else { self.pal.on_accent };
            p.text_center(Face::Semibold, 12.0, rsend, label, ink);
            self.hits.push((rsend, target));
        } else if chat.locked() {
            button(p, self.pal, rsend, label, false, false);
        } else {
            self.hit(p, rsend, label, Control::Send, false);
        }
        let mut x = rsend.x - 4.0;
        if self.wide {
            x -= 53.0;
            self.hit(p, Rect::new(x, y, 53.0, 24.0), "Details", Control::Panel, !chat.side_hidden);
            x -= 4.0;
        }
        x -= 76.0;
        self.hit(
            p,
            Rect::new(x, y, 76.0, 24.0),
            if chat.drawer == Drawer::Closed { "Terminal ▴" } else { "Terminal ▾" },
            Control::Drawer,
            chat.drawer != Drawer::Closed,
        );
        x -= 63.0;
        let attach = Rect::new(x, y, 59.0, 24.0);
        if !chat.locked() {
            self.hit(p, attach, "Attach…", Control::Attach, false);
        } else {
            button(p, self.pal, attach, "Attach…", false, false);
        }
    }

    fn suggestions(&mut self, p: &mut Painter, chat: &mut Chat) {
        if !chat.commands.open {
            return;
        }
        let r = chat.layout.suggestions;
        const HEADER: f32 = 22.0;
        const ROW: f32 = 25.0;
        let max_rows = ((r.h - HEADER - 4.0) / ROW).max(0.0) as usize;
        if max_rows == 0 {
            return;
        }
        let matches = chat.commands.matches(&chat.editor.text, self.pins);
        let visible = matches.len().max(1).min(max_rows);
        let selected = chat.commands.selected.min(matches.len().saturating_sub(1));
        let first = selected.saturating_sub(visible - 1);
        let h = HEADER + ROW * visible as f32 + 4.0;
        let r = Rect::new(r.x, r.bottom() - h, r.w, h);
        chat.layout.suggestions = r;
        p.rbox(r, 8.0, self.pal.popup_line);
        p.rbox(r.inset(1.0), 7.0, self.pal.popup);
        self.hits.push((r, Target::Chat(Control::CommandBackground)));
        let clip = p.clip(r);
        let heading = if chat.commands.loading() {
            "Commands · reading local names…"
        } else {
            "Commands · ↑↓ choose · Tab fills · Enter runs"
        };
        p.text(Face::Ui, 11.0, r.x + 10.0, r.y + 11.0, heading, self.pal.muted, r.w - 20.0);
        if matches.is_empty() {
            p.text(
                Face::Ui,
                12.0,
                r.x + 10.0,
                r.y + HEADER + ROW / 2.0,
                "No matching suggestions. Enter runs the command you typed.",
                self.pal.muted,
                r.w - 20.0,
            );
        }
        for (n, &index) in matches.iter().enumerate().skip(first).take(visible) {
            let cmd = &chat.commands.catalog[index];
            let row = Rect::new(r.x + 4.0, r.y + HEADER + (n - first) as f32 * ROW, r.w - 8.0, ROW - 1.0);
            let star = Rect::new(row.x, row.y, 27.0, row.h);
            let target = Target::Chat(Control::Suggestion(index));
            if n == selected || self.hover == Some(target) {
                p.rbox(row, 5.0, if n == selected { self.pal.on } else { self.pal.hover });
            }
            let pinned = self.pins.contains(&cmd.name);
            self.hit(p, star, if pinned { "★" } else { "☆" }, Control::PinCommand(index), pinned);
            let mut right = row.right() - 3.0;
            for (i, value) in commands::choices(&cmd.name).iter().enumerate().rev() {
                let w = (p.measure(Face::Semibold, 11.5, value) + 12.0).max(33.0);
                right -= w;
                self.hit(p, Rect::new(right, row.y + 2.0, w, row.h - 4.0), value, Control::Argument(index, i), false);
                right -= 4.0;
            }
            let name = format!("/{}", cmd.name);
            let x = star.right() + 5.0;
            let name_w = p.measure(Face::Semibold, 12.0, &name).min(((right - x) * 0.63).max(0.0));
            p.text(Face::Semibold, 12.0, x, row.cy(), &name, self.pal.text, name_w + 1.0);
            let desc = x + name_w + 11.0;
            p.text(Face::Ui, 11.5, desc, row.cy(), &cmd.description, self.pal.muted, (right - desc - 3.0).max(0.0));
            self.hits.push((Rect::new(star.right(), row.y, (right - star.right()).max(0.0), row.h), target));
        }
        p.restore(clip);
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.into() } else { s.chars().take(max - 1).chain(['…']).collect() }
}

fn tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 100_000 {
        format!("{}.{:01}k", n / 1000, n % 1000 / 100)
    } else {
        format!("{}k", n / 1000)
    }
}

fn compact_model(model: &str) -> String {
    if let Some(model) = model.strip_prefix("claude-") {
        for family in ["sonnet", "opus", "haiku", "fable"] {
            if model.split('-').any(|part| part == family) {
                let mut versions = model.split('-').filter(|s| s.len() == 1 && s.bytes().all(|b| b.is_ascii_digit()));
                let family = format!("{}{}", family[..1].to_ascii_uppercase(), &family[1..]);
                return match (versions.next(), versions.next()) {
                    (Some(major), Some(minor)) => format!("{family} {major}.{minor}"),
                    (Some(major), None) => format!("{family} {major}"),
                    _ => family,
                };
            }
        }
    }
    short(model, 12)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::chat::{Layout, clipboard};
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;

    #[test]
    fn narrow_and_wide_composers_rasterize_inside_box_and_toolbar_hits_do_not_overlap() {
        let mut gfx = Gfx::new();
        let app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
        let pal = app.pal();
        let mut info = ChatInfo::default();
        info.model = Some("claude-sonnet-4-6".into());
        info.context = Some(12_321);
        for width in [760, 900, 999, 1100] {
            let area = Rect::new(0.0, 0.0, width as f32, 480.0);
            let layout = Layout::new(area, true, false, None, true, true);
            let mut chat =
                Chat { layout, last_gate: Some(Gate::Check), error: Some(submission::CHECK), ..Default::default() };
            chat.chips.push(Chip::Clipboard(clipboard::Picture {
                seq: 42,
                thumbnail: Some(clipboard::Thumbnail { rgba: vec![161, 75, 32, 255], w: 1, h: 1 }),
            }));
            chat.chips.push(Chip::Path("fixture.txt".into()));
            chat.editor.set("/model".into());
            chat.commands.open = true;
            let mut hits = Vec::new();
            let mut bitmap = Bitmap::new(width, 480);
            let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
            p.fill(area, pal.page);
            Draw { pal, focused: true, wide: width >= 900, hover: None, hits: &mut hits, pins: &["model".into()] }
                .composer(&mut p, &mut chat, &info);
            let tool = hits.iter().filter(|(r, _)| layout.toolbar.contains(r.cx(), r.cy())).collect::<Vec<_>>();
            assert_eq!(tool.len(), if width >= 900 { 10 } else { 9 });
            for (i, (a, _)) in tool.iter().enumerate() {
                assert!(layout.box_frame.contains(a.x, a.y));
                for (b, _) in tool.iter().skip(i + 1).copied() {
                    assert!(a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y);
                }
            }
            assert!(hits.iter().any(|(r, t)| *t == Target::Chat(Control::Remove(0))
                && layout.attachments.contains(r.cx(), r.cy())));
            assert!(hits.iter().any(|(_, t)| *t == Target::Chat(Control::Override)));
            assert_eq!(hits.iter().filter(|(_, t)| matches!(t, Target::Chat(Control::Argument(_, _)))).count(), 6);
            let pixel =
                ((layout.attachments.y + 21.0) as usize * width as usize + (layout.attachments.x + 36.0) as usize) * 4;
            assert_eq!(&bitmap.pixels[pixel..pixel + 4], &[32, 75, 161, 255]);
            if width == 1100
                && let Some(root) = std::env::var_os("CS_COMPOSER_SHOTS")
            {
                bitmap.write_png(&std::path::Path::new(&root).join("chat-composer-dark.png")).unwrap();
            }
        }
    }
}
