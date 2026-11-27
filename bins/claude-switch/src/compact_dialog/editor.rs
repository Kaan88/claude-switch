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
            KeyName::Left if mods.ctrl => word_left(&self.text, self.caret),
            KeyName::Right if mods.ctrl => word_right(&self.text, self.caret),
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
                    self.anchor = match (*k == KeyName::Backspace, mods.ctrl) {
                        (true, true) => word_left(&self.text, self.caret),
                        (true, false) => previous(),
                        (false, true) => word_right(&self.text, self.caret),
                        (false, false) => next(),
                    };
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

/// How Ctrl+arrows see a character: a word character, punctuation, a space or tab, or a line break.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Punctuation,
    Space,
    Break,
}

fn class(c: char) -> Class {
    if c == '\n' {
        Class::Break
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punctuation
    }
}

/// Ctrl+Left, as in Windows text boxes: back over spaces, then to the start of the word or of the
/// run of punctuation before them. At the start of a line it goes to the end of the line above.
pub fn word_left(text: &str, at: usize) -> usize {
    let mut chars = text[..at].char_indices().rev().peekable();
    let start = at;
    let mut at = at;
    while let Some(&(i, c)) = chars.peek()
        && class(c) == Class::Space
    {
        at = i;
        chars.next();
    }
    let Some(&(i, c)) = chars.peek() else { return at };
    let run = class(c);
    if run == Class::Break {
        // Spaces at the start of a line stop at the line's start; from there, the line above.
        return if at != start { at } else { i };
    }
    while let Some(&(i, c)) = chars.peek()
        && class(c) == run
    {
        at = i;
        chars.next();
    }
    at
}

/// Ctrl+Right, as in Windows text boxes: past the word or the run of punctuation at the caret and
/// the spaces after it, to the start of the next one. A line break is a stop of its own: from
/// the end of a line it goes to the first word of the next.
pub fn word_right(text: &str, at: usize) -> usize {
    let mut chars = text[at..].char_indices().map(|(i, c)| (at + i, c)).peekable();
    let Some(&(_, c)) = chars.peek() else { return at };
    let run = class(c);
    match run {
        Class::Break => {
            chars.next();
        }
        Class::Space => {}
        _ => {
            while chars.peek().is_some_and(|&(_, c)| class(c) == run) {
                chars.next();
            }
        }
    }
    while chars.peek().is_some_and(|&(_, c)| class(c) == Class::Space) {
        chars.next();
    }
    chars.peek().map_or(text.len(), |&(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_right_and_ctrl_left_stop_at_word_and_punctuation_starts() {
        let text = "Fix the bug, then (maybe) ship.\n  next line";
        let mut stops = vec![0];
        while let Some(&at) = stops.last().filter(|&&at| at < text.len()) {
            stops.push(word_right(text, at));
        }
        let pieces = stops.windows(2).map(|w| &text[w[0]..w[1]]).collect::<Vec<_>>();
        assert_eq!(
            pieces,
            ["Fix ", "the ", "bug", ", ", "then ", "(", "maybe", ") ", "ship", ".", "\n  ", "next ", "line"]
        );
        let mut back = vec![text.len()];
        while let Some(&at) = back.last().filter(|&&at| at > 0) {
            back.push(word_left(text, at));
        }
        let pieces = back.windows(2).map(|w| &text[w[1]..w[0]]).collect::<Vec<_>>();
        assert_eq!(
            pieces,
            ["line", "next ", "  ", "\n", ".", "ship", ") ", "maybe", "(", "then ", ", ", "bug", "the ", "Fix "],
            "back from a line's first word: its start, then the end of the line above"
        );
        assert_eq!(word_right("café au lait", 0), 6, "letters beyond ASCII are word characters");
        assert_eq!((word_left("", 0), word_right("", 0)), (0, 0));
    }

    #[test]
    fn ctrl_backspace_ctrl_delete_and_shift_selection() {
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        let shift = Mods { shift: true, ..Mods::default() };
        let both = Mods { ctrl: true, shift: true, ..Mods::default() };
        let mut e = Editor::default();
        e.set("one two three".into());
        e.move_to(e.text.len(), false);
        e.key(&KeyName::Backspace, ctrl, None);
        assert_eq!(e.text, "one two ", "Ctrl+Backspace removes the word before the caret");
        e.move_to(0, false);
        e.key(&KeyName::Delete, ctrl, None);
        assert_eq!(e.text, "two ", "Ctrl+Delete removes the word and its space");
        e.key(&KeyName::Right, both, None);
        assert_eq!((e.selection(), e.caret), (0..4, 4), "Ctrl+Shift+Right selects a word");
        e.key(&KeyName::Left, shift, None);
        assert_eq!(e.selection(), 0..3, "Shift+Left shrinks it by a character");
        e.key(&KeyName::Right, ctrl, None);
        assert_eq!((e.selection(), e.caret), (4..4, 4), "Ctrl+Right moves on and clears it");
        e.key(&KeyName::Left, both, None);
        assert_eq!(e.selection(), 0..4, "Ctrl+Shift+Left selects back by a word");
        e.move_to(1, false);
        e.key(&KeyName::End, both, None);
        assert_eq!(e.selection(), 1..4, "Ctrl+Shift+End selects to the end");
        e.key(&KeyName::Home, both, None);
        assert_eq!((e.selection(), e.caret), (0..1, 0), "Ctrl+Shift+Home selects to the start");
        e.key(&KeyName::End, ctrl, None);
        assert_eq!((e.selection(), e.caret), (4..4, 4), "Ctrl+End goes to the end");
        e.key(&KeyName::Backspace, ctrl, None);
        assert_eq!(e.text, "", "Ctrl+Backspace after a trailing space takes the word too");
    }

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
