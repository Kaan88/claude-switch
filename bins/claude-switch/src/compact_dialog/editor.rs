//! Plain multiline editing model; the dialog paints only the visible wrapped rows.
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;
use std::ops::Range;

#[derive(Default)]
pub struct Editor {
    pub text: String,
    pub caret: usize,
    anchor: usize,
    pub scroll: usize,
    pub follow: bool,
    pub rows: Vec<Range<usize>>,
    pub visible: usize,
    columns: usize,
}

impl Editor {
    pub fn set(&mut self, text: String) {
        *self = Self { text, ..Self::default() };
    }

    /// The fixed end of the selection; the caret is the moving one.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    pub fn insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\0', "");
        let range = self.selection();
        self.caret = range.start + text.len();
        self.text.replace_range(range, &text);
        self.anchor = self.caret;
        self.follow = true;
        self.rows.clear();
        if self.columns != 0 {
            self.wrap(self.columns);
        }
    }

    pub fn move_to(&mut self, byte: usize, select: bool) {
        self.caret = byte;
        if !select {
            self.anchor = byte;
        }
        self.follow = true;
    }

    pub fn row_at(&self, byte: usize) -> usize {
        self.rows.partition_point(|r| r.start <= byte).saturating_sub(1)
    }

    pub fn row_byte(&self, line: usize, col: usize) -> usize {
        let Some(row) = self.rows.get(line) else { return self.text.len() };
        self.text[row.clone()].char_indices().nth(col).map_or(row.end, |(i, _)| row.start + i)
    }

    pub fn wrap(&mut self, columns: usize) {
        let columns = columns.max(1);
        if self.columns != columns || self.rows.is_empty() {
            self.columns = columns;
            self.reflow(columns);
        }
        if self.follow {
            let row = self.row_at(self.caret);
            self.scroll = self.scroll.min(row).max((row + 1).saturating_sub(self.visible));
            self.follow = false;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(self.visible));
    }

    fn reflow(&mut self, columns: usize) {
        self.rows.clear();
        let (mut start, mut count) = (0, 0);
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                self.rows.push(start..i);
                start = i + 1;
                count = 0;
                continue;
            }
            if count == columns {
                self.rows.push(start..i);
                start = i;
                count = 0;
            }
            count += 1;
        }
        self.rows.push(start..self.text.len());
    }

    pub fn wheel(&mut self, dy: f32) {
        let lines = ((dy / 20.0).abs().ceil() as usize).max(1);
        self.scroll = if dy > 0.0 { self.scroll.saturating_sub(lines) } else { self.scroll.saturating_add(lines) };
        self.follow = false;
    }

    pub fn key(&mut self, k: &KeyName, mods: Mods, text: Option<&str>) {
        let previous = || self.text[..self.caret].char_indices().next_back().map_or(0, |(i, _)| i);
        let next = || self.caret + self.text[self.caret..].chars().next().map_or(0, char::len_utf8);
        let row = self.row_at(self.caret);
        let destination = match k {
            KeyName::Char('a' | 'A') if mods.ctrl => {
                self.anchor = 0;
                self.move_to(self.text.len(), true);
                return;
            }
            KeyName::Char('c' | 'C' | 'x' | 'X') if mods.ctrl => {
                let range = self.selection();
                if !range.is_empty() {
                    platform::set_clipboard_text(&self.text[range]);
                    if matches!(k, KeyName::Char('x' | 'X')) {
                        self.insert("");
                    }
                }
                return;
            }
            KeyName::Char('v' | 'V') if mods.ctrl => {
                if let Some(text) = platform::clipboard_text() {
                    self.insert(&text);
                }
                return;
            }
            KeyName::Left => {
                if !mods.shift && !self.selection().is_empty() {
                    self.selection().start
                } else {
                    previous()
                }
            }
            KeyName::Right => {
                if !mods.shift && !self.selection().is_empty() {
                    self.selection().end
                } else {
                    next()
                }
            }
            KeyName::Home => {
                if mods.ctrl {
                    0
                } else {
                    self.rows.get(row).map_or(0, |r| r.start)
                }
            }
            KeyName::End => {
                if mods.ctrl {
                    self.text.len()
                } else {
                    self.rows.get(row).map_or(self.text.len(), |r| r.end)
                }
            }
            KeyName::Up | KeyName::Down | KeyName::PageUp | KeyName::PageDown => {
                let col = self.rows.get(row).map_or(0, |r| self.text[r.start..self.caret].chars().count());
                let row = match k {
                    KeyName::Up => row.saturating_sub(1),
                    KeyName::Down => row + 1,
                    KeyName::PageUp => row.saturating_sub(self.visible),
                    _ => row + self.visible,
                };
                self.row_byte(row, col)
            }
            KeyName::Backspace | KeyName::Delete => {
                if self.selection().is_empty() {
                    self.anchor = if *k == KeyName::Backspace { previous() } else { next() };
                }
                self.insert("");
                return;
            }
            KeyName::Enter => {
                self.insert("\n");
                return;
            }
            _ => {
                if !mods.ctrl
                    && !mods.alt
                    && let Some(text) = text
                {
                    self.insert(text);
                }
                return;
            }
        };
        self.move_to(destination, mods.shift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_unicode_and_preserves_bytes_while_editing() {
        let mut e = Editor::default();
        e.set("ab🐈d\n\nz".into());
        e.wrap(3);
        assert_eq!(e.rows.iter().map(|r| &e.text[r.clone()]).collect::<Vec<_>>(), ["ab🐈", "d", "", "z"]);
        e.move_to(e.row_byte(0, 3), false);
        e.key(&KeyName::Backspace, Mods::default(), None);
        e.insert("é\r\n");
        assert_eq!(e.text, "abé\nd\n\nz");
    }
}
