//! The picture viewer's way through the conversation: the picture before and after the one
//! shown, by the side buttons or Left and Right, and where it stands ("2 of 5").
use mdshared_ui::glyph;

use super::*;

/// A picture of the conversation: its item and its place there.
type Spot = (ItemId, usize);

pub(super) struct Overlay {
    pub id: ItemId,
    pub index: usize,
    pub image: Option<Arc<Image>>,
    pub bounds: Rect,
    /// Every picture of the conversation shown, in its order; renewed as the view draws.
    sequence: Vec<Spot>,
}

/// The side buttons' size, and their distance from the picture frame's edges.
const STEP: f32 = 36.0;
const STEP_INSET: f32 = 10.0;
/// The room between the counter and the buttons either side of it.
const COUNTER_GAP: f32 = 14.0;

impl Overlay {
    pub fn open(m: &mut Model, id: ItemId, index: usize) -> Self {
        let image = m.picture(id, index, true).or_else(|| m.picture(id, index, false));
        Self { id, index, image, bounds: Rect::default(), sequence: m.pictures().collect() }
    }

    fn at(&self) -> Option<usize> {
        self.sequence.iter().position(|&spot| spot == (self.id, self.index))
    }

    fn neighbour(&self, forward: bool) -> Option<Spot> {
        let at = self.at()?;
        let to = if forward { at.checked_add(1)? } else { at.checked_sub(1)? };
        self.sequence.get(to).copied()
    }

    /// "2 of 5", when there is more than one picture and this one is among them.
    fn counter(&self) -> Option<String> {
        let at = self.at()?;
        (self.sequence.len() > 1).then(|| format!("{} of {}", at + 1, self.sequence.len()))
    }

    /// Shows the picture after (or before) this one. Its preview loads only once it is drawn.
    pub fn step(&mut self, forward: bool) -> bool {
        let Some((id, index)) = self.neighbour(forward) else { return false };
        (self.id, self.index, self.image) = (id, index, None);
        true
    }

    /// The conversation's pictures as they are now, and the shown one's preview once the worker
    /// has it: the large one, the thumbnail until then, as when the viewer opens.
    pub fn refresh(&mut self, m: &mut Model) {
        self.sequence.clear();
        self.sequence.extend(m.pictures());
        if let Some(image) = m.picture(self.id, self.index, true) {
            self.image = Some(image);
        } else if self.image.is_none() {
            self.image = m.picture(self.id, self.index, false);
        }
    }
}

impl Ui {
    /// The side buttons, where there is a picture that way, quiet until pointed at; and the
    /// counter, left of `close`, only where it clears `open` (a narrow window has no room for it).
    pub(super) fn draw_steps(&mut self, p: &mut Painter, pal: &Palette, frame: Rect, open: Rect, close: Rect) {
        let Some(overlay) = &self.overlay else { return };
        let counter = overlay.counter();
        let steps = [(false, overlay.neighbour(false)), (true, overlay.neighbour(true))];
        if let Some(counter) = counter {
            let w = p.measure(Face::Ui, 12.0, &counter) + 2.0;
            let x = close.x - COUNTER_GAP - w;
            if x >= open.right() + COUNTER_GAP {
                p.text(Face::Ui, 12.0, x, close.cy(), &counter, pal.muted, w);
            }
        }
        if frame.w < STEP * 3.0 || frame.h < STEP {
            return;
        }
        for (forward, to) in steps {
            if to.is_none() {
                continue;
            }
            let x = if forward { frame.right() - STEP_INSET - STEP } else { frame.x + STEP_INSET };
            let r = Rect::new(x, frame.cy() - STEP * 0.5, STEP, STEP);
            let action = if forward { Action::Next } else { Action::Previous };
            let hot = r.contains(self.mouse.0, self.mouse.1);
            if hot {
                let fill = if self.pressed.as_ref() == Some(&action) { pal.press } else { pal.hover };
                p.frame(r, STEP * 0.5, fill, pal.popup_line);
            } else {
                p.rbox_a(r, STEP * 0.5, pal.popup, 0.72);
            }
            let c = if hot { pal.text } else { pal.muted };
            p.icon_rotated(glyph::RIGHT, 16.0, r.cx(), r.cy(), if forward { 0.0 } else { 180.0 }, c);
            self.hits.push((r, action));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write as _;
    use std::time::Duration;

    use mdshared_text::bitmap::Bitmap;
    use mdshared_ui::gfx::Gfx;
    use mdshared_ui::keys::{KeyName, Mods};
    use serde_json::{Value, json};

    use super::*;

    fn png(rgb: [u8; 3]) -> Value {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 48, 32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgb.repeat(48 * 32)).unwrap();
        }
        const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut data = String::new();
        for trio in bytes.chunks(3) {
            let bits = u32::from_be_bytes([0, trio[0], *trio.get(1).unwrap_or(&0), *trio.get(2).unwrap_or(&0)]);
            for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
                data.push(if i > trio.len() { '=' } else { ABC[((bits >> shift) & 63) as usize] as char });
            }
        }
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":data}})
    }

    fn damaged() -> Value {
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"!!!!"}})
    }

    /// A conversation with four pictures: two pasted (the second damaged), a tool's, and one
    /// more pasted after it.
    struct Conversation {
        dir: std::path::PathBuf,
        model: Model,
        events: std::sync::mpsc::Receiver<()>,
    }

    impl Conversation {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cs-picture-nav-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("session.jsonl");
            let mut file = File::create(&path).unwrap();
            let mut record = |role: &str, content: Value| {
                let line = json!({"timestamp":"2026-11-28T10:00:00Z", "type":role,
                    "message":{"role":role, "content":content}});
                writeln!(file, "{line}").unwrap();
            };
            record("user", json!([{"type":"text","text":"two pictures"}, png([200, 40, 40]), damaged()]));
            record("assistant", json!([{"type":"tool_use","id":"shot","name":"Read","input":{"file_path":"a.png"}}]));
            record("user", json!([{"type":"tool_result","tool_use_id":"shot","content":[png([40, 200, 40])]}]));
            record("assistant", json!("Here it is."));
            record("user", json!([{"type":"text","text":"and one more"}, png([40, 40, 200])]));
            drop(file);
            let (wake, events) = std::sync::mpsc::channel();
            let mut model = Model::new(crate::view::model::Source::File(path), dir.clone(), move || {
                let _ = wake.send(());
            });
            model.poll();
            events.recv_timeout(Duration::from_secs(3)).expect("transcript worker wake");
            assert!(model.collect());
            Self { dir, model, events }
        }

        /// Draws the view and the viewer until the worker has nothing more to say.
        fn settle(&mut self, ui: &mut Ui, p: &mut Painter, pal: &Palette, mouse: (f32, f32)) {
            let view = Rect::new(0.0, 0.0, 900.0, 640.0);
            for _ in 0..12 {
                ui.draw(p, pal, view, &mut self.model, mouse, Instant::now(), false);
                ui.draw_overlay(p, pal, view, &self.model, mouse, 10.0);
                if self.events.recv_timeout(Duration::from_millis(300)).is_err() {
                    break;
                }
                self.model.collect();
            }
            ui.draw(p, pal, view, &mut self.model, mouse, Instant::now(), false);
            ui.draw_overlay(p, pal, view, &self.model, mouse, 10.0);
        }
    }

    impl Drop for Conversation {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn shown(ui: &Ui) -> Spot {
        let o = ui.overlay.as_ref().unwrap();
        (o.id, o.index)
    }

    fn hit(ui: &Ui, action: &Action) -> Option<Rect> {
        ui.hits.iter().rev().find_map(|(r, a)| (a == action).then_some(*r))
    }

    fn click(ui: &mut Ui, m: &mut Model, r: Rect) {
        ui.press(r.cx(), r.cy(), 1, Mods::default(), m);
        assert_eq!(ui.release(r.cx(), r.cy(), Mods::default(), m), Release::Handled);
    }

    #[test]
    fn the_sequence_is_every_picture_in_conversation_order() {
        let c = Conversation::new("order");
        let pictures: Vec<_> = c.model.pictures().collect();
        assert_eq!(pictures.len(), 4);
        assert_eq!(pictures.iter().map(|p| p.1).collect::<Vec<_>>(), [0, 1, 0, 0], "both pasted, the tool's, the last");
        assert!(pictures.windows(2).all(|w| w[0] < w[1]), "in the order of the transcript");
        assert_eq!(pictures[0].0, pictures[1].0, "the two pasted pictures are one message's");
    }

    #[test]
    fn arrows_and_side_buttons_step_through_and_stop_at_the_ends() {
        let mut c = Conversation::new("steps");
        let all: Vec<_> = c.model.pictures().collect();
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(900, 640));
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let pal = &mdshared_ui::theme::DARK;
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.overlay = Some(Overlay::open(&mut c.model, all[0].0, all[0].1));
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        // The first: nothing before it, so no Previous button and Left does nothing.
        assert!(hit(&ui, &Action::Previous).is_none());
        assert!(hit(&ui, &Action::Next).is_some());
        assert_eq!(ui.overlay.as_ref().unwrap().counter().as_deref(), Some("1 of 4"));
        assert!(ui.key(&KeyName::Left, Mods::default()));
        assert_eq!(shown(&ui), all[0]);
        // Right goes forward, Left back; a modified arrow is held but does not step.
        assert!(ui.key(&KeyName::Right, Mods::default()));
        assert_eq!(shown(&ui), all[1]);
        assert!(ui.key(&KeyName::Right, Mods { ctrl: true, ..Mods::default() }));
        assert_eq!(shown(&ui), all[1]);
        assert!(ui.key(&KeyName::Left, Mods::default()));
        assert_eq!(shown(&ui), all[0]);
        // The side buttons do the same, and are hit before the picture (which closes it).
        for want in &all[1..] {
            c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
            let next = hit(&ui, &Action::Next).unwrap();
            click(&mut ui, &mut c.model, next);
            assert_eq!(shown(&ui), *want);
        }
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        assert!(hit(&ui, &Action::Next).is_none(), "the last has nothing after it");
        assert_eq!(ui.overlay.as_ref().unwrap().counter().as_deref(), Some("4 of 4"));
        assert!(ui.key(&KeyName::Right, Mods::default()));
        assert_eq!(shown(&ui), all[3]);
        let previous = hit(&ui, &Action::Previous).unwrap();
        click(&mut ui, &mut c.model, previous);
        assert_eq!(shown(&ui), all[2]);
        // Esc closes as before; with the viewer closed the arrows are not the viewer's.
        assert!(ui.key(&KeyName::Escape, Mods::default()));
        assert!(!ui.overlay_open());
    }

    #[test]
    fn a_stepped_to_picture_loads_lazily_and_a_failed_one_says_why() {
        let mut c = Conversation::new("lazy");
        let all: Vec<_> = c.model.pictures().collect();
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(900, 640));
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let pal = &mdshared_ui::theme::DARK;
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.overlay = Some(Overlay::open(&mut c.model, all[0].0, all[0].1));
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        assert!(ui.overlay.as_ref().unwrap().image.is_some(), "the first picture loads");
        for spot in &all[1..] {
            assert!(!c.model.picture_asked(spot.0, spot.1, true), "{spot:?} is not loaded before it is shown");
        }
        // The damaged picture: its preview fails, the viewer says why, and the arrows still work.
        assert!(ui.key(&KeyName::Right, Mods::default()));
        assert!(ui.overlay.as_ref().unwrap().image.is_none());
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        assert!(ui.overlay.as_ref().unwrap().image.is_none());
        assert_eq!(c.model.picture_failed(all[1].0, all[1].1, true), Some("Could not be decoded"));
        assert!(!c.model.picture_asked(all[2].0, all[2].1, true), "the next one waits until it is shown");
        assert!(hit(&ui, &Action::Previous).is_some() && hit(&ui, &Action::Next).is_some());
        assert!(ui.key(&KeyName::Right, Mods::default()));
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        assert_eq!(shown(&ui), all[2]);
        assert!(ui.overlay.as_ref().unwrap().image.is_some(), "the tool's picture loads once shown");
        assert!(!c.model.picture_asked(all[3].0, all[3].1, true));
    }

    #[test]
    fn clicking_outside_still_closes_and_one_picture_has_no_buttons_or_counter() {
        let mut c = Conversation::new("outside");
        let all: Vec<_> = c.model.pictures().collect();
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(900, 640));
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let pal = &mdshared_ui::theme::LIGHT;
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.overlay = Some(Overlay::open(&mut c.model, all[2].0, all[2].1));
        c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
        ui.press(2.0, 2.0, 1, Mods::default(), &mut c.model);
        assert!(!ui.overlay_open(), "a click outside the viewer closes it");
        let mut alone = Overlay::open(&mut c.model, all[2].0, all[2].1);
        alone.sequence = vec![all[2]];
        assert!(alone.counter().is_none() && alone.neighbour(false).is_none() && alone.neighbour(true).is_none());
        alone.sequence.clear();
        assert!(alone.counter().is_none() && !alone.step(true), "a picture no longer listed stays put");
    }

    /// The viewer's bottom row (Open in system viewer, the counter, Close) drawn in a window
    /// `width` wide, with every picture listed or only the shown one (no counter): BGRA rows.
    fn bottom_row(c: &mut Conversation, spot: Spot, width: f32, alone: bool) -> Vec<u8> {
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(width as u32, 640));
        let pal = &mdshared_ui::theme::DARK;
        let window = Rect::new(0.0, 0.0, width, 640.0);
        let mut ui = Ui::default();
        ui.set_chat(true);
        let mut overlay = Overlay::open(&mut c.model, spot.0, spot.1);
        if alone {
            overlay.sequence = vec![spot];
        }
        ui.overlay = Some(overlay);
        {
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            p.fill(window, pal.page);
            ui.draw_overlay(&mut p, pal, window, &c.model, (-1.0, -1.0), 10.0);
        }
        let open = hit(&ui, &Action::Open).unwrap();
        let close = hit(&ui, &Action::Close).unwrap();
        assert!(open.right() < close.x, "Open and Close themselves fit at {width}");
        let stride = bmp.width as usize * 4;
        bmp.pixels[open.y as usize * stride..(open.bottom().ceil() as usize + 1) * stride].to_vec()
    }

    #[test]
    fn the_counter_never_overlaps_open_in_system_viewer() {
        let mut c = Conversation::new("narrow");
        let all: Vec<_> = c.model.pictures().collect();
        // Wide: the counter shows between the buttons.
        assert_ne!(bottom_row(&mut c, all[1], 900.0, false), bottom_row(&mut c, all[1], 900.0, true));
        // Narrow, where "2 of 4" would have reached into Open in system viewer: no counter, so
        // the row is drawn exactly as for a single picture.
        for width in [340.0, 310.0] {
            assert_eq!(bottom_row(&mut c, all[1], width, false), bottom_row(&mut c, all[1], width, true), "at {width}");
        }
    }

    /// `CS_SHOT=<dir>`: the viewer on the conversation's second picture, dark and light, with
    /// the pointer on Next.
    #[test]
    fn viewer_shots() {
        let Ok(dir) = std::env::var("CS_SHOT") else { return };
        for (name, pal) in [("dark", &mdshared_ui::theme::DARK), ("light", &mdshared_ui::theme::LIGHT)] {
            let mut c = Conversation::new(name);
            let all: Vec<_> = c.model.pictures().collect();
            let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(900, 640));
            {
                let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
                p.fill(Rect::new(0.0, 0.0, 900.0, 640.0), pal.page);
                let mut ui = Ui::default();
                ui.set_chat(true);
                ui.overlay = Some(Overlay::open(&mut c.model, all[2].0, all[2].1));
                c.settle(&mut ui, &mut p, pal, (-1.0, -1.0));
                let next = hit(&ui, &Action::Next).unwrap();
                c.settle(&mut ui, &mut p, pal, (next.cx(), next.cy()));
            }
            bmp.write_png(&std::path::Path::new(&dir).join(format!("viewer-{name}.png"))).unwrap();
        }
    }
}
