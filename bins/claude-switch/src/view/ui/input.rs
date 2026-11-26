use std::path::Path;

use mdshared_ui::cursor::Cursor;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::*;

impl Ui {
    fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a.clone())
    }

    fn text_at(&self, x: f32, y: f32) -> Option<(&Row, usize, Option<&str>)> {
        if self.overlay_open() || !self.body.contains(x, y) {
            return None;
        }
        let yy = y - self.body.y + self.offset;
        let i = self.rows.partition_point(|r| r.top + r.h < yy);
        let row = self.rows.get(i)?;
        let layout = row.body.as_ref()?;
        let (start, width) = self.text_span(row);
        let dx = x - start;
        let dy = yy - row.top - if row.compact { 7.0 } else { HEAD };
        if dx < 0.0 || dx >= width || dy < 0.0 || dy > layout.height() {
            return None;
        }
        let (at, link) = layout.point(dx, dy);
        Some((row, at, link))
    }

    fn text_span(&self, row: &Row) -> (f32, f32) {
        if row.compact {
            let head = CompactHead::new(self.body.w);
            (self.body.x + head.text_x, head.text_w)
        } else {
            (self.body.x + INSET, self.body.w - 2.0 * INSET)
        }
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.dragging = false;
        self.start = None;
        self.pressed = None;
    }

    pub fn press(&mut self, x: f32, y: f32, clicks: u32, mods: Mods, _: &mut Model) -> bool {
        self.focused = true;
        self.start = None;
        self.dragging = false;
        self.pressed = self.hit(x, y);
        if self.overlay_open() {
            if self.pressed.is_none() && self.overlay.as_ref().is_some_and(|o| !o.bounds.contains(x, y)) {
                self.overlay = None;
            }
            return false;
        }
        if self.pressed == Some(Action::Attach) {
            self.selection = None;
            return true;
        }
        if let Some((row, at, link)) = self.text_at(x, y) {
            let id = row.id.clone();
            let link = link.map(str::to_owned);
            let text = row.body.as_ref().unwrap().text();
            let (a, b) = if clicks >= 3 {
                (0, text.len())
            } else if clicks == 2 {
                word(text, at)
            } else if mods.shift {
                (self.selection.as_ref().filter(|(r, _, _)| *r == id).map_or(at, |s| s.1), at)
            } else {
                (at, at)
            };
            self.start = link.filter(|_| clicks == 1 && !mods.shift).map(|s| (x, y, s));
            self.selection = Some((id, a, b));
            self.dragging = clicks == 1;
        } else {
            self.selection = None;
        }
        false
    }

    pub fn moved(&mut self, x: f32, y: f32) -> (Option<Cursor>, bool) {
        let hot = self.hit(x, y);
        let mut changed = self.hot != hot;
        self.hot = hot.clone();
        if self.dragging {
            if self.start.as_ref().is_some_and(|(sx, sy, _)| (x - sx).abs() + (y - sy).abs() > 4.0) {
                self.start = None;
            }
            if let Some((id, _, caret)) = &self.selection
                && let Some(row) = self.rows.iter().find(|r| r.id == *id)
                && let Some(text) = &row.body
            {
                let (start, width) = self.text_span(row);
                let dx = x - start;
                let dx = if row.compact { dx.clamp(0.0, width) } else { dx };
                let dy = y - self.body.y + self.offset - row.top - if row.compact { 7.0 } else { HEAD };
                let next = text.point(dx, dy).0;
                if next != *caret {
                    self.selection.as_mut().unwrap().2 = next;
                    changed = true;
                    self.start = None;
                    self.pressed = None;
                }
            }
            return (Some(Cursor::Text), changed);
        }
        let text = self.text_at(x, y);
        let cursor = if hot.is_some() || text.as_ref().is_some_and(|(_, _, link)| link.is_some()) {
            Some(Cursor::Hand)
        } else if text.is_some() {
            Some(Cursor::Text)
        } else if self.area.contains(x, y) || self.overlay_open() {
            Some(Cursor::Default)
        } else {
            None
        };
        (cursor, changed)
    }

    pub fn release(&mut self, x: f32, y: f32, mods: Mods, m: &mut Model, cwd: &Path) -> bool {
        let drag = std::mem::take(&mut self.dragging);
        let pressed = self.pressed.take();
        if let Some((sx, sy, target)) = self.start.take()
            && (sx - x).abs() + (sy - y).abs() < 5.0
            && self.text_at(x, y).and_then(|(_, _, l)| l) == Some(target.as_str())
        {
            super::super::links::open(&target, cwd, mods.ctrl || mods.shift);
            return true;
        }
        let Some(action) = pressed.filter(|a| Some(a) == self.hit(x, y).as_ref()) else {
            return drag || self.overlay_open() || self.area.contains(x, y);
        };
        match action {
            Action::Page(page) => {
                if m.agent.is_some() {
                    m.select_agent(None);
                }
                self.agent_origin = None;
                self.show(page);
            }
            Action::Attach => {}
            Action::Earlier => {
                self.offset = 0.0;
                self.follow.scrolled(0.0, self.max.max(2.0));
                m.earlier();
            }
            Action::Latest => self.follow.latest(),
            Action::Group(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
                self.cache = None;
            }
            Action::Tool(id) => {
                if !self.tool_expanded.remove(&id) {
                    self.tool_expanded.insert(id);
                }
                self.cache = None;
            }
            Action::Agent(id) => {
                self.agent_origin = Some(self.page);
                m.select_agent(Some(&id));
                self.show(Page::Conversation);
            }
            Action::Back => {
                m.select_agent(None);
                let page = self.agent_origin.take().unwrap_or(Page::Agents);
                self.show(page);
            }
            Action::Picture(id, index) => {
                let image = m.picture(id, index, true).or_else(|| m.picture(id, index, false));
                self.overlay = Some(Overlay { id, index, image, bounds: Rect::default() });
            }
            Action::Open => {
                if let Some(o) = &self.overlay {
                    m.open_picture(o.id, o.index);
                }
            }
            Action::Close => self.overlay = None,
        }
        true
    }

    pub(super) fn show(&mut self, page: Page) {
        self.page = page;
        self.selection = None;
        self.rows.clear();
        self.cache = None;
        self.offset = 0.0;
        self.follow.latest();
    }

    pub fn wheel(&mut self, dy: f32) {
        if self.overlay_open() {
            return;
        }
        self.offset = (self.offset - dy).clamp(0.0, self.max);
        if self.page == Page::Conversation {
            self.follow.scrolled(self.offset, self.max);
        }
    }

    pub fn key(&mut self, name: &KeyName, mods: Mods) -> bool {
        if self.overlay_open() {
            if *name == KeyName::Escape {
                self.overlay = None;
            }
            return true;
        }
        if !self.focused {
            return false;
        }
        match name {
            KeyName::Char('c') if mods.ctrl && !mods.alt => {
                let Some((id, _, _)) = &self.selection else { return false };
                let Some(text) = self.rows.iter().find(|r| r.id == *id).and_then(|r| r.body.as_ref()).map(|b| b.text())
                else {
                    return false;
                };
                let Some(range) = selection(&self.selection, id, text).filter(|r| !r.is_empty()) else { return false };
                platform::set_clipboard_text(&text[range]);
            }
            KeyName::Char('a') if mods.ctrl && !mods.alt => {
                let Some((id, a, b)) = &mut self.selection else { return false };
                let Some(text) = self.rows.iter().find(|r| r.id == *id).and_then(|r| r.body.as_ref()) else {
                    return false;
                };
                *a = 0;
                *b = text.text().len();
            }
            KeyName::Escape if self.selection.is_some() => {
                self.selection = None;
                self.focused = false;
            }
            _ => return false,
        }
        true
    }
}

fn word(text: &str, at: usize) -> (usize, usize) {
    let class = |c: char| {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    let mut left = at.min(text.len());
    if left == text.len() {
        left = text.char_indices().next_back().map_or(0, |(i, _)| i);
    }
    let Some(c) = text[left..].chars().next() else { return (0, 0) };
    let k = class(c);
    let right = left + text[left..].chars().take_while(|&x| class(x) == k).map(char::len_utf8).sum::<usize>();
    left -= text[..left].chars().rev().take_while(|&x| class(x) == k).map(char::len_utf8).sum::<usize>();
    (left, right)
}
