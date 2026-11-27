use std::io;

use super::super::documents::Document;
use super::super::preview::Preview;
use super::scroll::{GUTTER, Scrollbar};
use super::*;
use mdshared_ui::splitter::{Axis, Hold, Splitter};

pub(super) struct Markdown {
    /// The named file shown, or nothing for text the transcript holds (an MCP task's result).
    file: Option<NamedFile>,
    title: String,
    /// The file's full path, or where the text came from.
    subtitle: String,
    source: Option<io::Result<Document>>,
    text_lines: (usize, Vec<usize>),
    layout: Option<(f32, f32, Palette, Preview)>,
    error: Option<&'static str>,
    pub bounds: Rect,
    pub scroll: Option<Scrollbar>,
    offset: f32,
    max: f32,
}

impl Markdown {
    pub fn new(file: NamedFile) -> Self {
        let title = file.path().file_name().and_then(|s| s.to_str()).unwrap_or("File preview").to_owned();
        let subtitle = file.path().display().to_string();
        Self::with(Some(file), title, subtitle, None)
    }

    /// Markdown the transcript holds, shown as it is; nothing is read from disk.
    pub fn text(title: String, subtitle: String, markdown: String) -> Self {
        Self::with(None, title, subtitle, Some(Ok(Document::Markdown(markdown))))
    }

    fn with(file: Option<NamedFile>, title: String, subtitle: String, source: Option<io::Result<Document>>) -> Self {
        Self {
            file,
            title,
            subtitle,
            source,
            text_lines: (0, Vec::new()),
            layout: None,
            error: None,
            bounds: Rect::default(),
            scroll: None,
            offset: 0.0,
            max: 0.0,
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    #[cfg(test)]
    pub(super) fn rendered(&self) -> bool {
        self.layout.as_ref().is_some_and(|(_, _, _, layout)| layout.text().contains("North ridge"))
    }

    pub fn set_offset(&mut self, offset: f32) -> bool {
        let offset = offset.clamp(0.0, self.max);
        std::mem::replace(&mut self.offset, offset) != offset
    }

    pub fn editor(&mut self) {
        if let Some(file) = &self.file {
            self.error = (!file.open(false)).then_some("Could not open the named file in its default app.");
        }
    }
}

/// The least the conversation keeps beside the docked panel, and the least the panel is.
const MIN_CONVERSATION: f32 = 320.0;
const MIN_PANEL: f32 = 300.0;
/// The panel's width before the divider is first dragged.
const DEFAULT_PANEL: f32 = 540.0;
/// Between the conversation and the panel; the divider lies in its middle.
const GAP: f32 = 13.0;
/// Below this the panel takes the conversation's place instead of docking beside it.
const DOCK_FROM: f32 = 640.0;

/// The panel's reading width: a column of at most so many logical px, centred in the panel, or
/// the whole panel. mdview's page widths with a medium step between narrow and wide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reading {
    Narrow,
    Medium,
    Wide,
    #[default]
    Full,
}

impl Reading {
    pub const ALL: [Reading; 4] = [Reading::Narrow, Reading::Medium, Reading::Wide, Reading::Full];

    pub fn key(self) -> &'static str {
        match self {
            Reading::Narrow => "narrow",
            Reading::Medium => "medium",
            Reading::Wide => "wide",
            Reading::Full => "full",
        }
    }

    pub fn from_key(key: &str) -> Reading {
        Reading::ALL.into_iter().find(|r| r.key() == key.trim()).unwrap_or_default()
    }

    fn label(self) -> &'static str {
        match self {
            Reading::Narrow => "Width: Narrow",
            Reading::Medium => "Width: Medium",
            Reading::Wide => "Width: Wide",
            Reading::Full => "Width: Full",
        }
    }

    /// The text column's greatest width.
    fn column(self) -> Option<f32> {
        match self {
            Reading::Narrow => Some(560.0),
            Reading::Medium => Some(720.0),
            Reading::Wide => Some(900.0),
            Reading::Full => None,
        }
    }

    fn next(self) -> Reading {
        Reading::ALL[(Reading::ALL.iter().position(|r| *r == self).unwrap_or(0) + 1) % Reading::ALL.len()]
    }

    /// The column for a body `body` wide, centred in it.
    pub(super) fn place(self, body: Rect) -> Rect {
        let w = self.column().map_or(body.w, |c| c.min(body.w)).floor();
        Rect::new((body.x + (body.w - w) / 2.0).round(), body.y, w, body.h)
    }
}

/// The docked panel's width and reading width: one for the window, kept in `window.ini`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub split: Splitter,
    pub reading: Reading,
}

impl Default for Panel {
    fn default() -> Self {
        Panel { split: Splitter::new(Hold::B(DEFAULT_PANEL)).min(MIN_CONVERSATION, MIN_PANEL), reading: Reading::Full }
    }
}

impl Panel {
    pub fn from_settings(split: &str, reading: &str) -> Panel {
        let mut panel = Panel::default();
        if !split.is_empty() {
            // A closed pane is not something this panel does.
            panel.split.apply_setting(split.split(';').next().unwrap_or_default());
        }
        panel.reading = Reading::from_key(reading);
        panel
    }

    /// The conversation, the divider's grip (when docked) and the panel in `area`. When there is
    /// not room for two reading columns the panel takes the whole area.
    pub(super) fn split(&self, area: Rect) -> (Rect, Option<Rect>, Rect) {
        if area.w < DOCK_FROM {
            return (Rect::new(area.x, area.y, 0.0, area.h), None, area);
        }
        let (a, _, b) = self.split.rects(area, Axis::Horizontal);
        let half = GAP / 2.0;
        let content = Rect::new(a.x, a.y, (a.w - half).floor(), a.h);
        let dock = Rect::new((b.x + half).ceil(), b.y, (b.right() - (b.x + half).ceil()).max(0.0), b.h);
        let grip = Rect::new(content.right(), area.y, dock.x - content.right(), area.h);
        (content, Some(grip), dock)
    }
}

/// `path` as wide as `max` allows: whole, or its last components after an ellipsis, or at
/// least the file name.
fn elide_start(path: &str, max: f32, measure: &mut impl FnMut(&str) -> f32) -> String {
    if measure(path) <= max {
        return path.to_owned();
    }
    let cuts: Vec<usize> = path.match_indices(['\\', '/']).map(|(i, _)| i).collect();
    for &cut in &cuts {
        let tail = format!("…{}", &path[cut..]);
        if measure(&tail) <= max {
            return tail;
        }
    }
    cuts.last().map_or_else(|| path.to_owned(), |&cut| format!("…{}", &path[cut..]))
}

impl Ui {
    /// The viewer's title and subtitle, when it is open.
    #[cfg(test)]
    pub(in crate::view) fn viewer(&self) -> Option<(&str, &str)> {
        self.markdown.as_ref().map(|m| (m.title.as_str(), m.subtitle.as_str()))
    }

    /// The window's panel settings, given to every pane. A divider in hand here keeps its own.
    pub fn set_panel(&mut self, panel: Panel) {
        if !self.panel.split.dragging() {
            self.panel = panel;
        }
    }

    /// The panel's settings once they changed here (a drag let go, another reading width), for
    /// the window to keep and give to its other panes.
    pub fn take_panel_change(&mut self) -> Option<Panel> {
        std::mem::take(&mut self.panel_changed).then_some(self.panel)
    }

    pub(super) fn draw_divider(&self, p: &mut Painter, pal: &Palette, grip: Rect) {
        let (mx, my) = self.mouse;
        if self.panel.split.dragging() || grip.contains(mx, my) {
            let w = 3.0;
            p.rbox(Rect::new((grip.cx() - w / 2.0).round(), grip.y + 4.0, w, (grip.h - 8.0).max(0.0)), 1.5, pal.accent);
        }
    }

    fn divider_at(&self, x: f32, y: f32) -> bool {
        !self.overlay_open()
            && self.panel_area.is_some_and(|area| self.panel.split(area).1.is_some_and(|grip| grip.contains(x, y)))
    }

    /// A press on the divider takes hold of it.
    pub(super) fn divider_press(&mut self, x: f32, y: f32) -> bool {
        let Some(area) = self.panel_area.filter(|_| self.divider_at(x, y)) else { return false };
        // The splitter's own grip is narrower than the gap: hold the cut where it is.
        let cut = self.panel.split.rects(area, Axis::Horizontal).1;
        self.panel.split.press(area, Axis::Horizontal, x.clamp(cut.x, cut.right() - 0.01), y)
    }

    /// The divider in hand follows the pointer; `None` when it is not in hand.
    pub(super) fn divider_move(&mut self, x: f32, y: f32) -> Option<bool> {
        let area = self.panel_area.filter(|_| self.panel.split.dragging())?;
        Some(self.panel.split.drag(area, Axis::Horizontal, x, y))
    }

    pub(super) fn divider_release(&mut self) -> bool {
        let released = self.panel.split.release();
        self.panel_changed |= released;
        released
    }

    pub(super) fn divider_hot(&self, x: f32, y: f32) -> bool {
        self.panel.split.dragging() || self.divider_at(x, y)
    }

    pub(super) fn next_reading(&mut self) {
        self.panel.reading = self.panel.reading.next();
        self.panel_changed = true;
        if let Some(pane) = &mut self.markdown {
            pane.layout = None;
        }
    }

    pub(super) fn draw_markdown(&mut self, p: &mut Painter, pal: &Palette, area: Rect, m: &mut Model) {
        let Some(mut pane) = self.markdown.take() else { return };
        // A file read for an earlier pane never replaces text the transcript holds.
        if let Some(result) = m.take_markdown().filter(|_| pane.file.is_some()) {
            pane.text_lines = (0, Vec::new());
            pane.source = Some(result);
            pane.layout = None;
        }
        pane.bounds = area;
        pane.scroll = None;
        let clip = p.clip(area);
        p.frame(area, 7.0, pal.page, pal.line);
        let close = Rect::new(area.right() - 32.0, area.y + 6.0, 25.0, 25.0);
        let mut end = close.x;
        if let Some(r) = intersection(close, area) {
            self.button(p, pal, r, "×", Action::CloseMarkdown, false);
        }
        if let Some(file) = pane.file.as_ref().filter(|f| f.supports_default_app()) {
            let editor = Rect::new(end - 114.0, close.y, 108.0, close.h);
            if let Some(r) = intersection(editor, area) {
                let label = if file.picture() { "Open in app" } else { "Open in editor" };
                self.button(p, pal, r, label, Action::Editor, false);
            }
            end = editor.x;
        }
        let reads = matches!(&pane.source, Some(Ok(Document::Markdown(_) | Document::Text(_))));
        if reads {
            let label = self.panel.reading.label();
            let w = (p.measure(Face::Semibold, 12.0, label) + 18.0).ceil();
            let reading = Rect::new(end - w - 6.0, close.y, w, close.h);
            if reading.x > area.x + 90.0 {
                self.button(p, pal, reading, label, Action::Reading, false);
                end = reading.x;
            }
        }
        p.text(Face::Semibold, 13.0, area.x + 11.0, close.cy(), &pane.title, pal.text, (end - area.x - 18.0).max(0.0));
        // The second row: where the file is, and the other file a bare name could mean.
        let row = close.bottom() + 11.0;
        let mut path_end = area.right() - 10.0;
        if let Some(other) = pane.file.as_ref().and_then(NamedFile::other) {
            let folder = other.path().parent().and_then(|d| d.file_name()).and_then(|s| s.to_str()).unwrap_or("");
            let label = format!("Also in {folder}");
            let w = (p.measure(Face::Semibold, 12.0, &label) + 18.0).ceil().min(area.w * 0.5);
            let r = Rect::new(path_end - w, row - 10.0, w, 20.0);
            self.button(p, pal, r, &label, Action::File(other), false);
            path_end = r.x - 6.0;
        }
        let room = (path_end - area.x - 11.0).max(0.0);
        let shown = elide_start(&pane.subtitle, room, &mut |t| p.measure(Face::Ui, 11.5, t));
        p.text(Face::Ui, 11.5, area.x + 11.0, row, &shown, pal.muted, room);
        let top = row + 12.0;
        p.fill(Rect::new(area.x + 1.0, top, (area.w - 2.0).max(0.0), p.px()), pal.line);
        let top = if let Some(error) = pane.error {
            p.text(Face::Ui, 12.0, area.x + 10.0, top + 12.0, error, pal.warn, (area.w - 20.0).max(0.0));
            top + 26.0
        } else {
            top + 6.0
        };
        let body =
            Rect::new(area.x + 10.0, top, (area.w - 20.0 - GUTTER).max(1.0), (area.bottom() - top - 6.0).max(0.0));
        if body.h > 0.0 {
            match &pane.source {
                Some(Ok(Document::Markdown(raw))) if !raw.trim().is_empty() => {
                    let body = self.panel.reading.place(body);
                    if pane.layout.as_ref().is_none_or(|(width, scale, colors, _)| {
                        *width != body.w || *scale != p.scale || *colors != *pal
                    }) {
                        let render = Preview::new(raw, pal, p.scale, body.w, 14.0, None);
                        pane.layout = Some((body.w, p.scale, *pal, render));
                    }
                    let view = &pane.layout.as_ref().expect("viewer just laid out").3;
                    pane.max = (view.height() - body.h).max(0.0);
                    pane.offset = pane.offset.clamp(0.0, pane.max);
                    let c = p.clip(body);
                    view.draw(p, Rect::new(body.x, body.y - pane.offset, body.w, view.height()), body, None, pal);
                    p.restore(c);
                }
                Some(Ok(Document::Text(text))) if !text.is_empty() => {
                    let body = self.panel.reading.place(body);
                    let lh = 17.0;
                    let cols = (body.w / p.measure(Face::Mono, 12.0, "M").max(1.0)).floor().max(1.0) as usize;
                    if pane.text_lines.0 != cols {
                        pane.text_lines = (cols, text_lines(text, cols));
                    }
                    let starts = &pane.text_lines.1;
                    pane.max = (starts.len() as f32 * lh - body.h).max(0.0);
                    pane.offset = pane.offset.clamp(0.0, pane.max);
                    let clip = p.clip(body);
                    let from = (pane.offset / lh) as usize;
                    for (i, &start) in starts.iter().enumerate().skip(from).take((body.h / lh) as usize + 2) {
                        let end = starts.get(i + 1).copied().unwrap_or(text.len());
                        let line = text[start..end].trim_end_matches(['\r', '\n']);
                        p.text(
                            Face::Mono,
                            12.0,
                            body.x,
                            body.y + (i as f32 + 0.5) * lh - pane.offset,
                            line,
                            pal.text,
                            body.w,
                        );
                    }
                    p.restore(clip);
                }
                Some(Ok(Document::Picture(image))) => {
                    pane.max = 0.0;
                    pane.offset = 0.0;
                    paint_image(p, image, body, 3.0, false);
                }
                source => {
                    pane.max = 0.0;
                    let (message, color) = match source {
                        None => ("Reading the named file…".into(), pal.muted),
                        Some(Ok(_)) => ("This file is empty.".into(), pal.muted),
                        Some(Err(error)) => (error.to_string(), pal.warn),
                    };
                    p.paragraph_wrapped(
                        mdshared_ui::gfx::Face::Ui,
                        13.0,
                        body.x,
                        body.y + 2.0,
                        body.w,
                        &message,
                        color,
                        mdshared_ui::gfx::Wrap::Words,
                    );
                }
            }
            pane.scroll = Scrollbar::new(Rect::new(body.right() + 1.0, body.y, GUTTER, body.h), pane.max);
        }
        p.restore(clip);
        self.markdown = Some(pane);
    }
}

fn text_lines(text: &str, columns: usize) -> Vec<usize> {
    let mut lines = vec![0];
    let mut width = 0;
    for (at, c) in text.char_indices() {
        if c == '\n' {
            lines.push(at + 1);
            width = 0;
        } else if c != '\r' {
            // Approximate fallback glyphs conservatively; only visible lines are shaped.
            let cells = if c == '\t' {
                4
            } else if c.is_ascii() {
                1
            } else {
                2
            };
            if width > 0 && width + cells > columns {
                lines.push(at);
                width = 0;
            }
            width += cells;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_text_wraps_without_dropping_long_lines_or_splitting_utf8() {
        let text = "12345éX\n<img src=x>\n";
        let starts = text_lines(text, 5);
        let actual = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| &text[start..starts.get(i + 1).copied().unwrap_or(text.len())])
            .collect::<Vec<_>>();
        assert_eq!(actual, ["12345", "éX\n", "<img ", "src=x", ">\n", ""]);
    }

    #[test]
    fn markdown_docks_on_wide_chat_and_uses_the_existing_body_at_session_width() {
        let panel = Panel::default();
        let narrow = Rect::new(100.0, 40.0, 330.0, 250.0);
        let (left, grip, right) = panel.split(narrow);
        assert_eq!(left.w, 0.0);
        assert!(grip.is_none());
        assert_eq!((right.x, right.w), (narrow.x, narrow.w));
        let wide = Rect::new(100.0, 40.0, 1000.0, 250.0);
        let (left, grip, right) = panel.split(wide);
        assert!(left.w >= 300.0 && right.w >= 300.0);
        assert!(left.right() < right.x);
        let grip = grip.unwrap();
        assert!(grip.x <= left.right() && grip.right() >= right.x, "the grip covers the gap");
        assert_eq!(right.right(), wide.right());
    }

    #[test]
    fn the_divider_drags_within_both_least_widths_and_is_remembered() {
        let area = Rect::new(0.0, 0.0, 1200.0, 600.0);
        let mut panel = Panel::default();
        let cut = panel.split.rects(area, Axis::Horizontal).1;
        assert!(panel.split.press(area, Axis::Horizontal, cut.cx(), 300.0));
        panel.split.drag(area, Axis::Horizontal, 400.0, 300.0);
        panel.split.release();
        let (left, _, right) = panel.split(area);
        assert!((right.w - (800.0 - GAP / 2.0)).abs() <= 1.0, "{right:?}");
        assert!(left.w >= MIN_CONVERSATION - GAP);
        // Too far: the conversation keeps its least width.
        panel.split.press(area, Axis::Horizontal, panel.split.rects(area, Axis::Horizontal).1.cx(), 1.0);
        panel.split.drag(area, Axis::Horizontal, 10.0, 1.0);
        panel.split.release();
        assert!(panel.split(area).0.w >= MIN_CONVERSATION - GAP);
        let again = Panel::from_settings(&panel.split.to_setting(), "wide");
        assert_eq!(again.split(area), panel.split(area));
        assert_eq!(again.reading, Reading::Wide);
        assert_eq!(Panel::from_settings("", "unknown"), Panel::default());
        // A narrower window still leaves the conversation its least width.
        let smaller = Rect::new(0.0, 0.0, 1000.0, 600.0);
        assert!(again.split(smaller).0.w >= MIN_CONVERSATION - GAP);
    }

    #[test]
    fn reading_widths_cycle_and_centre_a_column_no_wider_than_the_body() {
        let body = Rect::new(10.0, 0.0, 800.0, 100.0);
        let mut r = Reading::Full;
        let mut seen = Vec::new();
        for _ in 0..4 {
            r = r.next();
            let column = r.place(body);
            assert!(column.w <= body.w && column.x >= body.x && column.right() <= body.right() + 0.5);
            assert!((column.cx() - body.cx()).abs() <= 1.0);
            seen.push(r);
        }
        assert_eq!(seen, [Reading::Narrow, Reading::Medium, Reading::Wide, Reading::Full]);
        assert_eq!(Reading::Narrow.place(body).w, 560.0);
        assert_eq!(Reading::Wide.place(body).w, 800.0);
        assert!(Reading::ALL.iter().all(|r| Reading::from_key(r.key()) == *r));
    }

    #[test]
    fn a_long_path_keeps_its_end() {
        let mut width = |t: &str| t.chars().count() as f32;
        let path = r"C:\Users\someone\Projects\repo\CHANGELOG.md";
        assert_eq!(elide_start(path, 100.0, &mut width), path);
        assert_eq!(elide_start(path, 20.0, &mut width), r"…\repo\CHANGELOG.md");
        assert_eq!(elide_start(path, 5.0, &mut width), r"…\CHANGELOG.md");
        assert_eq!(elide_start("CHANGELOG.md", 5.0, &mut width), "CHANGELOG.md");
    }

    #[test]
    fn the_docked_viewer_shows_its_path_drags_wider_and_changes_its_reading_width() {
        use mdshared_text::Bitmap;
        use mdshared_ui::cursor::Cursor;
        use mdshared_ui::gfx::Gfx;
        use mdshared_ui::keys::Mods;

        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        let wait = || events.recv_timeout(std::time::Duration::from_secs(3)).expect("transcript worker wake");
        wait();
        assert!(model.collect());
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(1200, 720);
        let pal = &mdshared_ui::theme::DARK;
        let ts = std::time::Instant::now();
        let view = Rect::new(0.0, 0.0, 1200.0, 700.0);
        let mut ui = Ui::default();
        ui.set_chat(true);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
        let chip = ui.hits.iter().find_map(|(r, a)| matches!(a, Action::File(_)).then_some(*r)).unwrap();
        ui.press(chip.cx(), chip.cy(), 1, Mods::default(), &mut model);
        ui.release(chip.cx(), chip.cy(), Mods::default(), &mut model);
        for _ in 0..8 {
            wait();
            model.collect();
            ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
            if ui.markdown.as_ref().is_some_and(Markdown::rendered) {
                break;
            }
        }
        let dock = ui.markdown.as_ref().unwrap().bounds;
        let area = ui.panel_area.expect("docked beside the conversation");
        let grip = ui.panel.split(area).1.unwrap();
        assert!(matches!(ui.pointer_cursor(grip.cx(), grip.cy()), Some(Cursor::ResizeEw)));
        assert!(ui.take_panel_change().is_none());

        // Drag the divider 200 px to the left: the viewer is that much wider, and the change
        // is reported once, on release.
        assert!(!ui.press(grip.cx(), grip.cy(), 1, Mods::default(), &mut model));
        assert!(ui.moved(grip.cx() - 200.0, grip.cy()));
        ui.set_panel(Panel::default());
        assert!(ui.take_panel_change().is_none(), "nothing until let go");
        assert_eq!(ui.release(grip.cx() - 200.0, grip.cy(), Mods::default(), &mut model), Release::Handled);
        let panel = ui.take_panel_change().expect("the new width, to keep");
        ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
        let wider = ui.markdown.as_ref().unwrap().bounds;
        assert!((wider.w - dock.w - 200.0).abs() <= 3.5, "{dock:?} {wider:?}");
        assert_eq!(wider.right(), dock.right());

        // The reading width button cycles and is reported too.
        let reading = ui.hits.iter().find_map(|(r, a)| (*a == Action::Reading).then_some(*r)).unwrap();
        ui.press(reading.cx(), reading.cy(), 1, Mods::default(), &mut model);
        ui.release(reading.cx(), reading.cy(), Mods::default(), &mut model);
        let next = ui.take_panel_change().unwrap();
        assert_eq!((next.reading, next.split), (Reading::Narrow, panel.split));
        ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
        assert!(ui.markdown.as_ref().unwrap().rendered());
        if let Ok(path) = std::env::var("CS_SHOT") {
            bmp.write_png(std::path::Path::new(&path)).unwrap();
        }
    }
}
