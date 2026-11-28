//! The Sessions details' conversation: the shared conversation view (`Ui` over `Model`, as Chat
//! and the Session view draw it), read-only, in a region that scrolls on its own. It reads the
//! transcript through the same incremental reader: the newest records first, earlier ones only
//! on "Load earlier". Nothing here sends, rewinds or branches.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mdshared_ui::cursor::Cursor;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::theme::Palette;

use super::model::{Model, Source};
use super::ui::{Release, Ui};

/// How often the shown transcript is checked for new records.
pub const POLL: Duration = Duration::from_secs(1);

pub struct Reader {
    /// The session it reads.
    pub id: String,
    model: Model,
    ui: Ui,
    area: Rect,
    /// The keyboard goes here (Ctrl+C, Ctrl+A, Escape) after a click in the region.
    pub focused: bool,
    /// The last press was in the region: its release is the region's too.
    pressed: bool,
    _demo: Option<DemoFile>,
}

impl Reader {
    /// Reads `file`, the session `id`'s transcript, in `cwd`.
    pub fn new(id: &str, file: PathBuf, cwd: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        Self::with(id, Source::File(file), cwd, None, wake)
    }

    /// A made-up conversation for the demo, written to a temporary file that goes with it.
    pub fn demo(id: &str, title: &str, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let demo = DemoFile::write(title)?;
        let (path, cwd) = (demo.path.clone(), demo.dir.clone());
        Ok(Self::with(id, Source::Demo(path), cwd, Some(demo), wake))
    }

    fn with(id: &str, source: Source, cwd: PathBuf, demo: Option<DemoFile>, wake: impl Fn() + Send + 'static) -> Self {
        let model = Model::new(source, cwd, wake);
        model.poll();
        let mut ui = Ui::default();
        ui.set_embedded();
        Self { id: id.to_owned(), model, ui, area: Rect::default(), focused: false, pressed: false, _demo: demo }
    }

    /// Asks the reader for what was appended since.
    pub fn poll(&self) {
        self.model.poll();
    }

    /// Takes what the reader delivered: true when the region needs drawing again.
    pub fn collect(&mut self) -> bool {
        self.model.collect()
    }

    #[cfg(test)]
    pub fn loaded(&self) -> bool {
        self.model.loaded
    }

    /// Draws the conversation in `area`, of which `view` shows: the details pane scrolls the
    /// region with it, and the pointer is the region's only where it shows.
    #[allow(clippy::too_many_arguments)] // Frame inputs from the host, as `Ui::draw` takes them.
    pub fn draw(
        &mut self,
        p: &mut Painter,
        pal: &Palette,
        (area, view): (Rect, Rect),
        diff: super::look::Diff,
        mouse: (f32, f32),
        now: Instant,
        motion: bool,
    ) {
        let (top, bottom) = (area.y.max(view.y), area.bottom().min(view.bottom()));
        self.area = Rect::new(area.x, top, area.w, (bottom - top).max(0.0));
        self.ui.set_diff_colours(diff);
        self.ui.draw(p, pal, area, &mut self.model, mouse, now, motion);
    }

    /// The enlarged picture, over the whole window, when one is open.
    pub fn draw_overlay(&mut self, p: &mut Painter, pal: &Palette, window: Rect, mouse: (f32, f32), radius: f32) {
        if self.ui.overlay_open() {
            self.ui.draw_overlay(p, pal, window, &self.model, mouse, radius);
        }
    }

    pub fn overlay_open(&self) -> bool {
        self.ui.overlay_open()
    }

    /// A press in the region (or anywhere while a picture is enlarged): true when taken.
    pub fn press(&mut self, x: f32, y: f32, clicks: u32, mods: Mods) -> bool {
        if !self.overlay_open() && !self.area.contains(x, y) {
            self.blur();
            return false;
        }
        self.focused = true;
        self.pressed = true;
        self.ui.press(x, y, clicks, mods, &mut self.model);
        true
    }

    /// The end of a press. Only the view's own actions (expand, load earlier, jump to latest,
    /// pictures, links): Stop, Rewind and Branch belong to a running tab and are never offered.
    pub fn release(&mut self, x: f32, y: f32, mods: Mods) -> bool {
        if !std::mem::take(&mut self.pressed) && !self.overlay_open() {
            return false;
        }
        !matches!(self.ui.release(x, y, mods, &mut self.model), Release::Miss)
    }

    /// The pointer moved: true when something under it changed.
    pub fn moved(&mut self, x: f32, y: f32) -> bool {
        self.ui.moved(x, y)
    }

    /// The region takes the pointer here (a drag in it, its scrollbar).
    pub fn owns_pointer(&self, x: f32, y: f32) -> bool {
        self.ui.owns_pointer(x, y) || self.ui.scroll_dragging()
    }

    pub fn pointer_cursor(&self, x: f32, y: f32) -> Option<Cursor> {
        (self.overlay_open() || self.area.contains(x, y)).then(|| self.ui.pointer_cursor(x, y)).flatten()
    }

    pub fn pointer_left(&mut self) {
        self.ui.pointer_left();
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.ui.blur();
    }

    /// The wheel over the region scrolls the conversation. False when it did not move (its
    /// end was reached, or the pointer is not over it): the details around it scroll instead.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if self.overlay_open() {
            return true;
        }
        if !self.area.contains(x, y) {
            return false;
        }
        self.ui.wheel(x, y, dy)
    }

    pub fn key(&mut self, name: &KeyName, mods: Mods) -> bool {
        (self.focused || self.overlay_open()) && self.ui.key(name, mods)
    }

    pub fn scroll_deadline(&self) -> Option<Instant> {
        self.ui.scroll_deadline()
    }
}

/// The demo's transcript for the selected session, in a folder of its own under the temporary
/// folder that goes when the reader does. Never one of the session's listed (made-up) paths.
struct DemoFile {
    dir: PathBuf,
    path: PathBuf,
}

impl DemoFile {
    fn write(title: &str) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let dir = loop {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("claude-switch-details-{}-{serial}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let demo = Self { path: dir.join("details.jsonl"), dir };
        std::fs::write(&demo.path, demo_transcript(title, &demo.dir))?;
        Ok(demo)
    }
}

impl Drop for DemoFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A short finished conversation about `title`: prompts, answers and a few tool calls.
pub(crate) fn demo_transcript(title: &str, cwd: &Path) -> String {
    use serde_json::json;
    let cwd = cwd.to_string_lossy();
    let stamp = |sec: u32| format!("2026-11-28T09:{:02}:{:02}Z", 10 + sec / 60, sec % 60);
    let line = |sec: u32, role: &str, content: serde_json::Value| {
        json!({"timestamp": stamp(sec), "type": role, "cwd": cwd, "sessionId": "demo-details",
            "message": {"role": role, "content": content}})
        .to_string()
    };
    let tool =
        |id: &str, name: &str, input: serde_json::Value| json!({"type":"tool_use","id":id,"name":name,"input":input});
    let result = |id: &str, text: &str| json!([{"type":"tool_result","tool_use_id":id,"content":text}]);
    let lines = [
        line(0, "user", json!(format!("{title}. Start with the part that fails."))),
        line(4, "assistant", json!([tool("a", "Grep", json!({"pattern": "reset_state", "path": "src"}))])),
        line(5, "user", result("a", "src/scan.rs:88:    reset_state(&mut list);")),
        line(7, "assistant", json!([tool("b", "Read", json!({"file_path": "src/scan.rs"}))])),
        line(8, "user", result("b", "fn scan() { … }")),
        line(
            20,
            "assistant",
            json!(
                "The state is reset **before** the scan ends, so the second call sees an empty list. \
                 I moved the reset after the scan:\n\n```rust\nlet found = scan_all(&mut list);\nreset_state(&mut list);\n```\n\nThe failing test passes now."
            ),
        ),
        line(40, "user", json!("Good. Does anything else call it that way?")),
        line(
            52,
            "assistant",
            json!(
                "Two other places do. Both read the list after the scan has finished, so they are not affected. I left them as they are."
            ),
        ),
        line(70, "user", json!("Run the whole suite once more, then stop.")),
        line(75, "assistant", json!([tool("c", "Bash", json!({"command": "cargo test"}))])),
        line(96, "user", result("c", "test result: ok. 214 passed; 0 failed")),
        line(98, "assistant", json!("All tests pass. Nothing is committed.")),
    ];
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_text::bitmap::Bitmap;
    use mdshared_ui::gfx::Gfx;

    fn read(reader: &mut Reader) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !reader.loaded() {
            reader.poll();
            reader.collect();
            assert!(Instant::now() < deadline, "the transcript was read");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_details_conversation_reads_a_transcript_and_scrolls_on_its_own() {
        let scratch = crate::testing::Scratch::created("cs-details-reader");
        let file = scratch.join("s.jsonl");
        std::fs::write(&file, demo_transcript("Fix the scan", &scratch)).unwrap();
        let mut reader = Reader::new("s", file.clone(), scratch.to_path_buf(), || {});
        read(&mut reader);
        let pal = mdshared_ui::tokens::Tokens::new(false).palette;
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(420, 260);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let area = Rect::new(0.0, 0.0, 420.0, 260.0);
        let diff = super::super::look::Diff::from_ansi(&[pal.accent; 16]);
        let now = Instant::now();
        reader.draw(&mut p, &pal, (area, area), diff, (-1.0, -1.0), now, false);
        assert!(reader.model.items.len() >= 6, "prompts, answers and tools are items");
        // It opens on the newest message and the wheel scrolls it back, inside its own area only.
        let latest = reader.ui.offset();
        assert!(latest > 0.0, "a short region opens at the end");
        assert!(reader.wheel(100.0, 100.0, 120.0));
        reader.draw(&mut p, &pal, (area, area), diff, (-1.0, -1.0), now, false);
        assert!(reader.ui.offset() < latest);
        assert!(!reader.wheel(100.0, 400.0, 120.0), "the wheel outside the region is not its");
        assert!(reader.wheel(100.0, 100.0, 100_000.0));
        reader.draw(&mut p, &pal, (area, area), diff, (-1.0, -1.0), now, false);
        assert_eq!(reader.ui.offset(), 0.0);
        assert!(!reader.wheel(100.0, 100.0, 120.0), "at its top the wheel goes to the details");
        // A record appended later shows on the next poll.
        let before = reader.model.items.len();
        let more = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"One more thing.\"}}\n";
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str(more);
        std::fs::write(&file, text).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while reader.model.items.len() == before {
            reader.poll();
            reader.collect();
            assert!(Instant::now() < deadline, "the appended prompt was read");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_demo_conversation_lives_in_a_temporary_folder_of_its_own() {
        let reader = Reader::demo("x", "Fix the scan", || {}).unwrap();
        let dir = reader._demo.as_ref().unwrap().dir.clone();
        assert!(dir.starts_with(std::env::temp_dir()) && dir.join("details.jsonl").is_file());
        drop(reader);
        assert!(!dir.exists(), "the demo transcript goes with its reader");
    }
}
