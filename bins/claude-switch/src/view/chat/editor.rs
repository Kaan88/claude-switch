//! Composer-only wrapping and pointer geometry. Display hyphens never enter the draft.
use std::collections::HashMap;
use std::ops::{Deref, DerefMut, Range};

use mdshared_ui::gfx::{Face, Painter};
use mdshared_ui::keys::{KeyName, Mods};

use crate::compact_dialog::editor::Editor;

pub(super) const LINE: f32 = 17.0;
pub(super) const TEXT_SIZE: f32 = 13.0;

#[derive(Default)]
pub(super) struct ComposerEditor {
    inner: Editor,
    stops: Vec<(Range<usize>, Vec<f32>)>,
    metrics: Option<(f32, f32, usize)>,
    row_hint: Option<usize>,
}

impl Deref for ComposerEditor {
    type Target = Editor;

    fn deref(&self) -> &Editor {
        &self.inner
    }
}

impl DerefMut for ComposerEditor {
    fn deref_mut(&mut self) -> &mut Editor {
        &mut self.inner
    }
}

impl ComposerEditor {
    pub fn flow(&mut self, p: &mut Painter, width: f32, height: f32) {
        let visible = ((height - 4.0) / LINE).max(1.0) as usize;
        let hyphen = p.measure(Face::Mono, TEXT_SIZE, "-");
        let width = width.max(2.0 * hyphen);
        let changed_width = self.metrics.is_none_or(|(w, hy, _)| (w, hy) != (width, hyphen));
        if self.inner.rows.is_empty() || changed_width {
            let mut advances = HashMap::new();
            self.inner.rows = wrap(&self.inner.text, width, hyphen, |c| {
                *advances.entry(c).or_insert_with(|| {
                    let mut utf8 = [0; 4];
                    p.measure(Face::Mono, TEXT_SIZE, if c == '\t' { " " } else { c.encode_utf8(&mut utf8) })
                })
            });
            self.row_hint = None;
            self.inner.follow = true;
        }
        self.inner.follow |= visible != self.inner.visible;
        self.inner.visible = visible;
        if self.inner.follow {
            let row = self.caret_row();
            self.inner.scroll = self.inner.scroll.min(row).max((row + 1).saturating_sub(visible));
            self.inner.follow = false;
        }
        self.inner.scroll = self.inner.scroll.min(self.inner.rows.len().saturating_sub(visible));
        let key = (width, hyphen, self.inner.scroll);
        if self.metrics != Some(key)
            || changed_width
            || self.stops.len() != (self.inner.rows.len() - self.inner.scroll).min(visible)
        {
            self.measure_stops(p);
        }
        self.metrics = Some(key);
    }

    fn measure_stops(&mut self, p: &mut Painter) {
        self.stops = self
            .inner
            .rows
            .iter()
            .skip(self.inner.scroll)
            .take(self.inner.visible)
            .map(|row| (row.clone(), p.caret_stops(Face::Mono, TEXT_SIZE, &self.inner.text[row.clone()])))
            .collect();
    }

    pub fn set(&mut self, text: String) {
        *self = Self::default();
        self.inner.set(text);
    }

    /// Inserts typed or pasted text without control characters: Escape and other keys carry one as
    /// their text, which would show as a box. Line breaks become `\n`; tabs stay.
    pub fn insert(&mut self, text: &str) {
        let text = typed(text);
        if text.is_empty() {
            return;
        }
        self.inner.insert(&text);
        self.row_hint = None;
        self.metrics = None;
    }

    pub fn move_to(&mut self, byte: usize, select: bool) {
        self.row_hint = None;
        self.inner.move_to(byte, select);
    }

    /// Returns whether the key edited text, not whether it moved the caret or invalidated layout.
    pub fn key(&mut self, k: &KeyName, mods: Mods, text: Option<&str>) -> bool {
        match k {
            KeyName::Up | KeyName::Down => self.vertical(*k == KeyName::Up, mods.shift),
            KeyName::Home | KeyName::End if !mods.ctrl => {
                let row = self.caret_row();
                if let Some(range) = self.rows.get(row) {
                    let byte = if *k == KeyName::Home { range.start } else { range.end };
                    self.place(byte, row, mods.shift);
                } else {
                    self.move_to(if *k == KeyName::Home { 0 } else { self.text.len() }, mods.shift);
                }
            }
            KeyName::Left | KeyName::Right | KeyName::Home | KeyName::End | KeyName::PageUp | KeyName::PageDown => {
                self.row_hint = None;
                self.inner.key(k, mods, None);
            }
            KeyName::Backspace | KeyName::Delete | KeyName::Enter => {
                let before = self.text.len();
                self.row_hint = None;
                self.inner.key(k, mods, None);
                self.metrics = None;
                return self.text.len() != before;
            }
            _ if !mods.ctrl
                && !mods.alt
                && let Some(text) = text.filter(|s| !typed(s).is_empty()) =>
            {
                self.insert(text);
                return true;
            }
            _ => {}
        }
        false
    }

    fn vertical(&mut self, up: bool, select: bool) {
        let row = self.caret_row();
        if let Some(range) = self.rows.get(row) {
            let next = if up { row.saturating_sub(1) } else { (row + 1).min(self.rows.len() - 1) };
            let col = self.text[range.start..self.caret].chars().count();
            self.place(self.row_byte(next, col), next, select);
            return;
        }
        // Text can change before the next paint knows its visual wraps. Still honor hard breaks.
        let start = self.text[..self.caret].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[self.caret..].find('\n').map_or(self.text.len(), |at| self.caret + at);
        let next = if up {
            let Some(end) = start.checked_sub(1) else { return };
            let start = self.text[..end].rfind('\n').map_or(0, |at| at + 1);
            start..end
        } else {
            if end == self.text.len() {
                return;
            }
            let start = end + 1;
            start..self.text[start..].find('\n').map_or(self.text.len(), |at| start + at)
        };
        let col = self.text[start..self.caret].chars().count();
        let at = self.text[next.clone()].char_indices().nth(col).map_or(next.end, |(at, _)| next.start + at);
        self.move_to(at, select);
    }

    fn target(&self, x: f32, y: f32, word: bool) -> (usize, usize) {
        let line = ((y - 4.0).max(0.0) / LINE) as usize;
        let line = line.min(self.stops.len().saturating_sub(1));
        let index = self.inner.scroll + line;
        let Some((row, stops)) = self.stops.get(line).filter(|(range, _)| self.inner.rows.get(index) == Some(range))
        else {
            return (self.inner.caret, self.inner.row_at(self.inner.caret));
        };
        let col =
            stops.windows(2).take_while(|pair| x >= if word { pair[1] } else { (pair[0] + pair[1]) / 2.0 }).count();
        let span = &self.inner.text[row.clone()];
        let at = span.char_indices().nth(col).map_or_else(
            || if word { span.char_indices().next_back().map_or(row.end, |(i, _)| row.start + i) } else { row.end },
            |(i, _)| row.start + i,
        );
        (at, index)
    }

    pub fn hit(&self, x: f32, y: f32) -> usize {
        self.target(x, y, false).0
    }

    pub fn caret_row(&self) -> usize {
        self.row_hint.unwrap_or_else(|| self.inner.row_at(self.inner.caret))
    }

    pub fn on_first_line(&self) -> bool {
        if self.rows.is_empty() { !self.text[..self.caret].contains('\n') } else { self.caret_row() == 0 }
    }

    pub fn on_last_line(&self) -> bool {
        if self.rows.is_empty() {
            !self.text[self.caret..].contains('\n')
        } else {
            self.caret_row() + 1 == self.rows.len()
        }
    }

    pub fn point(&mut self, x: f32, y: f32, select: bool) -> bool {
        let (at, row) = self.target(x, y, false);
        let changed = self.inner.caret != at || self.caret_row() != row;
        self.place(at, row, select);
        changed
    }

    fn place(&mut self, at: usize, row: usize, select: bool) {
        self.inner.move_to(at, select);
        self.row_hint = (row != self.inner.row_at(at)).then_some(row);
    }

    pub fn word_at(&mut self, x: f32, y: f32) {
        let (at, row) = self.target(x, y, true);
        let range = word(&self.inner.text, at);
        self.inner.move_to(range.start, false);
        if self.inner.rows.get(row).is_some_and(|r| r.end == range.end) {
            self.place(range.end, row, true);
        } else {
            self.row_hint = None;
            self.inner.move_to(range.end, true);
        }
    }

    pub fn select_all(&mut self) {
        self.row_hint = None;
        self.inner.move_to(0, false);
        self.inner.move_to(self.inner.text.len(), true);
    }
}

fn kind(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        1
    } else if c == '\n' {
        2
    } else if c.is_whitespace() {
        3
    } else {
        4
    }
}

fn word(text: &str, mut at: usize) -> Range<usize> {
    if at == text.len() {
        at = text.char_indices().next_back().map_or(0, |(i, _)| i);
    }
    let Some(c) = text[at..].chars().next() else { return at..at };
    let k = kind(c);
    if k >= 4 || k == 2 {
        return at..at + c.len_utf8();
    }
    let start = text[..at].char_indices().rev().take_while(|(_, c)| kind(*c) == k).last().map_or(at, |(i, _)| i);
    let end = text[at..].char_indices().find(|(_, c)| kind(*c) != k).map_or(text.len(), |(i, _)| at + i);
    start..end
}

pub(super) fn hyphen(text: &str, row: &Range<usize>) -> bool {
    !row.is_empty()
        && text[..row.end].chars().next_back().is_some_and(|c| !c.is_whitespace())
        && text[row.end..].chars().next().is_some_and(|c| !c.is_whitespace())
}

fn wrap(text: &str, width: f32, hyphen: f32, mut advance: impl FnMut(char) -> f32) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '\n' {
            rows.push(start..at);
            start = at + 1;
            used = 0.0;
        } else if c.is_whitespace() {
            // Single separating spaces may remain as a trailing caret stop; very long runs stay editable.
            if used >= width && at > start && text[..at].chars().next_back().is_some_and(char::is_whitespace) {
                rows.push(start..at);
                start = at;
                used = 0.0;
            }
            used += advance(c);
        } else {
            let mut len = advance(c);
            while let Some((_, next)) = chars.next_if(|(_, c)| !c.is_whitespace()) {
                len += advance(next);
            }
            let end = chars.peek().map_or(text.len(), |(i, _)| *i);
            if start < at && used + len > width {
                rows.push(start..at);
                start = at;
                used = 0.0;
            }
            if len > width {
                for (i, ch) in text[at..end].char_indices() {
                    let i = at + i;
                    let w = advance(ch);
                    if i > start && len > width && used + w + hyphen > width {
                        rows.push(start..i);
                        start = i;
                        len -= used;
                        used = 0.0;
                    }
                    used += w;
                }
            } else {
                used += len;
            }
        }
    }
    rows.push(start..text.len());
    rows
}

/// Text as the editor keeps it: `\r\n` and a lone `\r` as `\n`, tabs kept, other control
/// characters dropped.
fn typed(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) {
        return std::borrow::Cow::Borrowed(text);
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    std::borrow::Cow::Owned(text.chars().filter(|&c| !c.is_control() || matches!(c, '\n' | '\t')).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_and_other_control_characters_never_reach_the_text() {
        let mut editor = ComposerEditor::default();
        editor.insert("another, ");
        assert!(!editor.key(&KeyName::Escape, Mods::default(), Some("\u{1b}")));
        editor.insert("\u{1b}\u{7f}\u{0}");
        editor.insert("a\r\nb\rc\td");
        assert_eq!(editor.text, "another, a\nb\nc\td");
    }

    #[test]
    fn whole_words_move_to_the_next_row_and_only_long_words_split() {
        let text = "use force\n\nééééééé longer";
        let rows = wrap(text, 6.0, 1.0, |_| 1.0);
        let rendered = rows.iter().map(|r| format!("{}{}", &text[r.clone()], if hyphen(text, r) { "-" } else { "" }));
        assert_eq!(rendered.collect::<Vec<_>>(), ["use ", "force", "", "ééééé-", "éé ", "longer"]);
        assert_eq!(word(text, 6), 4..9);
        assert_eq!(&text[word(text, text.len())], "longer");
        let wide = "go 猫猫";
        assert_eq!(wrap(wide, 6.0, 1.0, |c| if c == '猫' { 2.0 } else { 1.0 }), [0..3, 3..9]);
    }

    #[test]
    fn clicks_and_keyboard_extension_keep_utf8_selection() {
        let mut e = ComposerEditor::default();
        e.set("é fox".into());
        e.rows.push(0..6);
        e.stops = vec![(0..6, vec![0., 10., 20., 30., 40., 50.])];
        let at = e.hit(17., 6.);
        assert_eq!(at, 3);
        e.word_at(46., 6.);
        assert_eq!(&e.text[e.selection()], "fox");
        e.move_to(at, false);
        let end = e.hit(35., 6.);
        e.move_to(end, true);
        assert_eq!(&e.text[e.selection()], "fo");
        e.select_all();
        assert_eq!(&e.text[e.selection()], "é fox");
    }

    #[test]
    fn caret_keeps_the_clicked_side_of_a_display_wrap() {
        let mut e = ComposerEditor::default();
        e.set("a b".into());
        e.rows = vec![0..2, 2..3];
        e.stops = vec![(0..2, vec![0., 10., 20.]), (2..3, vec![0., 10.])];
        e.point(21., 7., false);
        assert_eq!((e.caret, e.caret_row()), (2, 0));
        e.point(0., 24., false);
        assert_eq!((e.caret, e.caret_row()), (2, 1));
    }
}
