//! The scrolling folder list of the New session dialog: the folders, the filtered rows, the
//! keyboard's highlight and the scroll position.

use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::KeyName;
use mdshared_ui::pane::draw_row_fill;
use mdshared_ui::scroll::{draw_scrollbar, offset_for_thumb, scrollbar};

use super::{Draw, ROW};
use crate::list::{ago, now_ms};
use crate::recent_folders::{Folder, matching};
use crate::state::Target;

pub const ROW_H: f32 = 28.0;
/// The rows shown at once, in a window with room for them.
pub const ROWS: usize = 8;

pub struct List {
    all: Vec<Folder>,
    /// Indices into `all`: the rows, after the filter.
    shown: Vec<usize>,
    query: String,
    /// The row the keyboard is on, which Enter picks.
    pub highlight: Option<usize>,
    scroll: f32,
    /// What the last frame laid out.
    area: Rect,
    track: Option<(Rect, Rect)>,
    /// Where the pointer holds the dragged thumb, below its top.
    drag: Option<f32>,
}

impl List {
    pub fn new(all: Vec<Folder>) -> Self {
        let shown = matching(&all, "");
        Self {
            all,
            shown,
            query: String::new(),
            highlight: None,
            scroll: 0.0,
            area: Rect::new(0.0, 0.0, 0.0, ROWS as f32 * ROW_H),
            track: None,
            drag: None,
        }
    }

    pub fn folder(&self, row: usize) -> Option<&Folder> {
        self.shown.get(row).map(|&i| &self.all[i])
    }

    #[cfg(test)]
    pub fn folders(&self) -> impl Iterator<Item = &Folder> {
        self.shown.iter().map(|&i| &self.all[i])
    }

    #[cfg(test)]
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    #[cfg(test)]
    pub fn track(&self) -> Option<(Rect, Rect)> {
        self.track
    }

    /// New folders (a finished disk check or scan); the filter and the highlighted folder stay.
    pub fn set_folders(&mut self, all: Vec<Folder>) {
        let key = self.highlight.and_then(|row| self.folder(row)).map(|f| f.key.clone());
        self.shown = matching(&all, &self.query);
        self.all = all;
        self.highlight = key.and_then(|key| self.shown.iter().position(|&i| self.all[i].key == key));
        self.clamp();
    }

    /// The field's text. Words that are not a path highlight the first match, so Enter picks it;
    /// a typed path is left for Enter to start.
    pub fn filter(&mut self, text: &str) {
        self.query = text.to_string();
        self.shown = matching(&self.all, text);
        let words = !text.trim().is_empty() && !text.contains(['\\', '/', ':']);
        self.highlight = (words && !self.shown.is_empty()).then_some(0);
        self.scroll = 0.0;
    }

    /// The list's height with `room` for it: whole rows, at most [`ROWS`]. It follows the number
    /// of folders, not of matches, so the dialog stays still while filtering.
    pub fn height(&self, room: f32) -> f32 {
        let fits = ((room / ROW_H).floor() as usize).clamp(1, ROWS);
        self.all.len().clamp(1, fits) as f32 * ROW_H
    }

    /// "3 of 57" while the filter hides some, else how many there are when they do not all show.
    pub fn count(&self) -> Option<String> {
        if self.shown.len() < self.all.len() {
            Some(format!("{} of {}", self.shown.len(), self.all.len()))
        } else {
            (self.all.len() > self.page()).then(|| format!("{} folders", self.all.len()))
        }
    }

    /// Whole rows in view.
    pub fn page(&self) -> usize {
        ((self.area.h / ROW_H) as usize).max(1)
    }

    fn max_scroll(&self) -> f32 {
        (self.shown.len() as f32 * ROW_H - self.area.h).max(0.0)
    }

    fn clamp(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// Moves the highlight for an arrow, page, Home or End key and brings it into view. False
    /// with no rows.
    pub fn step(&mut self, key: &KeyName) -> bool {
        let Some(last) = self.shown.len().checked_sub(1) else { return false };
        let page = self.page();
        let to = match (key, self.highlight) {
            (KeyName::Home, _) => 0,
            (KeyName::End, _) => last,
            (_, None) => 0,
            (KeyName::Up, Some(row)) => row.saturating_sub(1),
            (KeyName::Down, Some(row)) => row + 1,
            (KeyName::PageUp, Some(row)) => row.saturating_sub(page),
            (KeyName::PageDown, Some(row)) => row + page,
            (_, Some(row)) => row,
        }
        .min(last);
        self.highlight = Some(to);
        let top = to as f32 * ROW_H;
        self.scroll = self.scroll.min(top).max(top + ROW_H - self.area.h);
        self.clamp();
        true
    }

    /// A wheel movement, positive away from the user (towards the top).
    pub fn wheel(&mut self, dy: f32) {
        self.scroll -= dy;
        self.clamp();
    }

    /// A press on the scrollbar: the thumb is taken, or brought under the pointer first.
    pub fn scroll_press(&mut self, x: f32, y: f32) -> bool {
        let Some((track, thumb)) = self.track.filter(|(track, _)| track.contains(x, y)) else { return false };
        let grab = if y >= thumb.y && y < thumb.bottom() { y - thumb.y } else { thumb.h / 2.0 };
        self.drag = Some(grab);
        self.drag_to(track, thumb, y - grab);
        true
    }

    pub fn scroll_drag(&mut self, y: f32) -> bool {
        let (Some(grab), Some((track, thumb))) = (self.drag, self.track) else { return false };
        self.drag_to(track, thumb, y - grab);
        true
    }

    pub fn scroll_release(&mut self) -> bool {
        self.drag.take().is_some()
    }

    fn drag_to(&mut self, track: Rect, thumb: Rect, top: f32) {
        let content = self.shown.len() as f32 * ROW_H;
        self.scroll = offset_for_thumb(track, thumb.h, top, content, self.area.h);
    }

    /// Draws the rows in view into `area`. `chosen` is the key of the folder in the field.
    pub fn draw(&mut self, d: &mut Draw<'_>, p: &mut Painter, area: Rect, chosen: &str, focused: bool) {
        let pal = d.pal;
        self.area = area;
        self.clamp();
        p.frame(area.inset(-1.0), 5.0, pal.field, if focused { pal.accent } else { pal.line });
        let content = self.shown.len() as f32 * ROW_H;
        self.track = scrollbar(area, content, area.h, self.scroll);
        if self.shown.is_empty() {
            let text = if self.all.is_empty() { "No recent folders." } else { "No folder matches." };
            p.text(Face::Ui, 12.0, area.x + 10.0, area.y + ROW_H / 2.0, text, pal.muted, area.w - 20.0);
            return;
        }
        let width = area.w - if self.track.is_some() { 12.0 } else { 0.0 };
        let now = now_ms();
        let old = p.clip(area);
        let first = (self.scroll / ROW_H) as usize;
        for row in first..self.shown.len() {
            let r = Rect::new(area.x, area.y + row as f32 * ROW_H - self.scroll, width, ROW_H);
            if r.y >= area.bottom() {
                break;
            }
            let folder = &self.all[self.shown[row]];
            let target = Target::Dialog(ROW + row as u32);
            let selected = match self.highlight {
                Some(highlight) => highlight == row,
                None => folder.key == chosen,
            };
            draw_row_fill(p, &pal, r, d.hover == Some(target), d.pressed == Some(target), selected);

            let time = ago(now, folder.last_active_ms);
            let time_w = p.text_right(Face::Ui, 12.0, r.right() - 10.0, r.cy(), &time, pal.muted);
            let left = r.x + 10.0;
            let room = (r.right() - 10.0 - time_w - 16.0 - left).max(0.0);
            let name_w = p.text(Face::Ui, 13.0, left, r.cy(), &folder.name, pal.text, room);
            let parent_x = left + name_w + 10.0;
            let rest = left + room - parent_x;
            if rest > 24.0 {
                p.text(Face::Ui, 12.0, parent_x, r.cy(), &folder.parent, pal.muted, rest);
            }

            let top = r.y.max(area.y);
            let hit = Rect::new(r.x, top, r.w, (r.bottom().min(area.bottom()) - top).max(0.0));
            d.hits.push((hit, target));
        }
        p.restore(old);
        if let Some((track, thumb)) = self.track {
            draw_scrollbar(p, &pal, track, thumb, self.drag.is_some());
        }
    }
}
