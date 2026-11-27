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
        let byte = self.text[row.clone()].char_indices().nth(col).map_or(row.end, |(i, _)| row.start + i);
        byte.min(self.row_end(line))
    }

    /// The last caret position shown on a row. A soft-wrapped row's end is the next row's
    /// start, where the caret is drawn on the next row, so it stops one character before: on
    /// the space a word wrap broke at, or before the last character of a word cut in two.
    pub fn row_end(&self, line: usize) -> usize {
        let Some(row) = self.rows.get(line) else { return self.text.len() };
        let soft = self.rows.get(line + 1).is_some_and(|next| next.start == row.end);
        if soft {
            self.text[row.clone()].char_indices().next_back().map_or(row.end, |(i, _)| row.start + i)
        } else {
            row.end
        }
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

    /// Wraps at word boundaries, as the Chat composer does: a row breaks after the spaces before
    /// the word that would not fit (they hang past the edge, unseen), and inside a word only when
    /// that word alone is longer than a row. Rows are contiguous byte ranges; a hard line break
    /// is the one byte between two rows.
    fn reflow(&mut self, columns: usize) {
        self.rows.clear();
        // `count` is the characters of the row so far; `space` is where the word after the last
        // whitespace starts, and how many characters precede it.
        let (mut start, mut count, mut space) = (0, 0, None::<(usize, usize)>);
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                self.rows.push(start..i);
                (start, count, space) = (i + 1, 0, None);
                continue;
            }
            if count >= columns && !c.is_whitespace() {
                match space.take() {
                    Some((at, before)) if at > start => {
                        self.rows.push(start..at);
                        start = at;
                        count -= before;
                    }
                    _ => {
                        self.rows.push(start..i);
                        (start, count) = (i, 0);
                    }
                }
            }
            count += 1;
            if c.is_whitespace() {
                space = Some((i + c.len_utf8(), count));
            }
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
                    self.row_end(row)
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
        assert_eq!(e.row_byte(0, 3), 2, "past a cut row's end stays on that row");
        e.move_to(e.row_byte(1, 0), false);
        e.key(&KeyName::Backspace, Mods::default(), None);
        e.insert("é\r\n");
        assert_eq!(e.text, "abé\nd\n\nz");
    }

    fn rows(e: &Editor) -> Vec<&str> {
        e.rows.iter().map(|r| &e.text[r.clone()]).collect()
    }

    #[test]
    fn wraps_at_word_boundaries_and_cuts_only_a_word_longer_than_the_row() {
        let mut e = Editor::default();
        e.set("German audio plus back to back support\nabcdefghijkl xy".into());
        e.wrap(12);
        assert_eq!(
            rows(&e),
            ["German audio ", "plus back to ", "back support", "abcdefghijkl ", "xy"],
            "no word is split; a word exactly one row long stays whole"
        );
        e.wrap(5);
        assert!(rows(&e).contains(&"abcde") && rows(&e).contains(&"fghij"), "{:?}", rows(&e));
        assert!(rows(&e).contains(&"back ") && rows(&e).contains(&"plus "), "{:?}", rows(&e));
        // Spaces hang past the edge rather than starting the next row.
        e.set("ab     cd".into());
        e.wrap(4);
        assert_eq!(rows(&e), ["ab     ", "cd"]);
        // Wide and multi-byte characters keep their bytes whole.
        e.set("é🐈 ŝ🐈é ab".into());
        e.wrap(3);
        assert_eq!(rows(&e), ["é🐈 ", "ŝ🐈é ", "ab"]);
    }

    #[test]
    fn caret_moves_and_selects_across_word_wrapped_rows() {
        let mut e = Editor::default();
        e.set("one two three four".into());
        e.visible = 10;
        e.wrap(9);
        assert_eq!(rows(&e), ["one two ", "three ", "four"]);
        let none = Mods::default();
        let shift = Mods { shift: true, ..Mods::default() };
        // End stays on the row: before the space the row broke at.
        e.key(&KeyName::End, none, None);
        assert_eq!((e.caret, e.row_at(e.caret)), (7, 0));
        // Down keeps the column, capped to what the shorter row shows.
        e.key(&KeyName::Down, none, None);
        assert_eq!((e.caret, e.row_at(e.caret)), (13, 1), "after “three”");
        e.key(&KeyName::Down, none, None);
        assert_eq!((e.caret, e.row_at(e.caret)), (18, 2), "end of the last row");
        e.key(&KeyName::Home, none, None);
        assert_eq!(e.caret, 14);
        e.key(&KeyName::Up, none, None);
        assert_eq!((e.caret, e.row_at(e.caret)), (8, 1), "start of “three”");
        // Right from the row's last position crosses to the next row's start.
        e.move_to(7, false);
        e.key(&KeyName::Right, none, None);
        assert_eq!((e.caret, e.row_at(e.caret)), (8, 1));
        // Shift+Down selects across the soft break.
        e.move_to(4, false);
        e.key(&KeyName::Down, shift, None);
        assert_eq!(&e.text[e.selection()], "two thre");
        // Clicking past a row's end lands on that row.
        assert_eq!(e.row_byte(1, 40), 13);
        assert_eq!(e.row_byte(2, 40), 18);
        // Typing reflows: the caret follows its word to the next row.
        e.move_to(7, false);
        e.insert("sss");
        assert_eq!(rows(&e), ["one ", "twosss ", "three ", "four"]);
        assert_eq!(e.row_at(e.caret), 1);
    }
}
