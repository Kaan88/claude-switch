//! Find in a terminal: the bar over the grid, the matches in the screen and its scrollback, the
//! current one, and the keys and buttons that move between them.
//!
//! A terminal keeps its own [`Find`] while the bar is open. The matches are found again whenever
//! the query, the case setting or the screen changed ([`Term::changes`]), so new output and a
//! resize (which rewraps the lines) are searched as they come. A match is found across a wrap:
//! the rows a long line wrapped onto are searched as one line, as copying sees them. Beside "Aa"
//! (match case) the bar has whole word and regular expression toggles; a pattern that does not
//! compile shows as invalid and finds nothing.

use mdshared_term::{Point, Screen};
use mdshared_ui::gfx::{Face, Painter, Rect, mix};
use mdshared_ui::glyph;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::termview::{Metrics, View};
use mdshared_ui::textfield::{FieldColors, FieldHit, TextField};
use mdshared_ui::theme::Palette;
use regex_automata::meta::Regex;
use regex_automata::util::syntax;

use crate::state::{AppState, Target};
use crate::term::Term;

/// A control of the find bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FindControl {
    /// The card itself: a press gives the box the keyboard.
    Field,
    /// The text box, where the mouse selects (`fieldmouse`).
    Text,
    Case,
    Word,
    Regex,
    /// To the match above (older output), as Enter does.
    Previous,
    /// To the match below, as Shift+Enter does.
    Next,
    Close,
}

/// One match: its first cell and the last cell of its last character, as absolute lines of the
/// screen's line space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub start: Point,
    pub end: Point,
}

/// The find bar of a terminal, while it is open.
#[derive(Default)]
pub struct Find {
    pub field: TextField,
    /// The bar has the keyboard (a press on the grid gives it back to the program).
    pub focused: bool,
    pub options: Options,
    /// The box's text rectangle as last drawn, for the mouse.
    pub hit: Option<FieldHit>,
    matches: Vec<Hit>,
    current: Option<usize>,
    /// What `matches` were found for: the query and options, and the terminal's change count.
    found: Option<(String, Options, u64)>,
    /// The query compiled for those: `Err` holds why a regular expression is invalid.
    matcher: Option<Result<Matcher, String>>,
    /// The screen's size and its top line when they were found: lines above that top were
    /// scrollback then, which output never rewrites, so only what lies below is searched again.
    scanned: Option<((usize, usize), u64)>,
}

/// The bar's toggles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// A query ready to search with.
#[derive(Clone, Debug)]
pub enum Matcher {
    /// Characters, folded to lower case unless the case must match.
    Text {
        needle: Vec<char>,
        match_case: bool,
        whole_word: bool,
    },
    Regex(Regex),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn fold(c: char, match_case: bool) -> char {
    if match_case { c } else { c.to_lowercase().next().unwrap_or(c) }
}

impl Matcher {
    /// `None` for an empty query; `Err` with the reason for a regular expression that does not
    /// compile.
    pub fn new(query: &str, o: Options) -> Result<Option<Matcher>, String> {
        if query.is_empty() {
            return Ok(None);
        }
        if !o.regex {
            let needle = query.chars().map(|c| fold(c, o.match_case)).collect();
            return Ok(Some(Matcher::Text { needle, match_case: o.match_case, whole_word: o.whole_word }));
        }
        let pattern = if o.whole_word { format!(r"\b(?:{query})\b") } else { query.to_string() };
        Regex::builder()
            .syntax(syntax::Config::new().case_insensitive(!o.match_case))
            .build(&pattern)
            .map(|re| Some(Matcher::Regex(re)))
            .map_err(|e| e.to_string())
    }

    /// The matches in one logical line, as (first, last) character indices.
    fn find(&self, text: &[char], out: &mut Vec<(usize, usize)>) {
        match self {
            Matcher::Text { needle, match_case, whole_word } => {
                let n = needle.len();
                let mut i = 0;
                while i + n <= text.len() {
                    let here = text[i..i + n].iter().zip(needle).all(|(c, m)| fold(*c, *match_case) == *m);
                    let bounded = !whole_word
                        || (i == 0 || !is_word(text[i - 1])) && text.get(i + n).is_none_or(|c| !is_word(*c));
                    if here && bounded {
                        out.push((i, i + n - 1));
                        i += n;
                    } else {
                        i += 1;
                    }
                }
            }
            Matcher::Regex(re) => {
                let mut line = String::with_capacity(text.len());
                let mut starts = Vec::with_capacity(text.len());
                for c in text {
                    starts.push(line.len());
                    line.push(*c);
                }
                for m in re.find_iter(line.as_str()) {
                    if m.is_empty() {
                        continue;
                    }
                    let first = starts.partition_point(|b| *b < m.start());
                    let last = starts.partition_point(|b| *b < m.end()) - 1;
                    out.push((first, last));
                }
            }
        }
    }
}

impl Find {
    pub fn new() -> Find {
        Find { focused: true, ..Find::default() }
    }

    /// Why the regular expression does not compile, while it does not.
    pub fn invalid(&self) -> Option<&str> {
        self.matcher.as_ref()?.as_ref().err().map(String::as_str)
    }

    pub fn current(&self) -> Option<Hit> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    /// "3 of 12", "No results", or nothing while the box is empty.
    pub fn count(&self) -> String {
        if self.invalid().is_some() {
            return "Invalid pattern".into();
        }
        match (self.field.is_empty(), self.matches.len(), self.current) {
            (true, _, _) => String::new(),
            (false, 0, _) => "No results".into(),
            (false, n, Some(i)) => format!("{} of {n}", i + 1),
            (false, n, None) => format!("{n} found"),
        }
    }

    /// Brings the matches up to date with the query and the screen. When the query or the case
    /// setting changed, the current match becomes the last one that starts above `bottom` (the
    /// line under the view), as a search upwards from what is shown, and is returned to be
    /// scrolled to. New output keeps the current match where it was, without scrolling.
    pub fn update(&mut self, screen: &Screen, changes: u64, bottom: u64) -> Option<Hit> {
        let query = self.field.text();
        let same_query = self.found.as_ref().is_some_and(|(q, o, _)| q == query && *o == self.options);
        if same_query && self.found.as_ref().is_some_and(|f| f.2 == changes) {
            return None;
        }
        if !same_query {
            self.matcher = Matcher::new(query, self.options).transpose();
        }
        let matcher = self.matcher.as_ref().and_then(|m| m.as_ref().ok()).cloned();
        let before = self.current();
        let size = (screen.cols(), screen.rows());
        match self.scanned.filter(|(s, _)| same_query && *s == size) {
            // Output only: keep what was found in the scrollback as it was, search the rest.
            Some((_, top)) => {
                let from = logical_start(screen, top.min(screen.screen_start()));
                let first = screen.first_line();
                self.matches.retain(|h| h.start.line >= first && h.start.line < from);
                self.matches.extend(search_from(screen, from, matcher.as_ref()));
            }
            // A new query, or a resize (which rewraps): everything.
            None => self.matches = search_from(screen, screen.first_line(), matcher.as_ref()),
        }
        self.scanned = Some((size, screen.screen_start()));
        self.found = Some((query.to_string(), self.options, changes));
        if same_query {
            self.current = before.and_then(|b| {
                self.matches.iter().position(|h| h.start == b.start).or_else(|| self.last_before(b.start))
            });
            return None;
        }
        self.current = self.last_before(Point::new(bottom, 0)).or((!self.matches.is_empty()).then_some(0));
        self.current()
    }

    fn last_before(&self, at: Point) -> Option<usize> {
        self.matches.partition_point(|h| h.start < at).checked_sub(1)
    }

    /// Moves to the match above (`up`, older output) or below, round from either end.
    pub fn step(&mut self, up: bool) -> Option<Hit> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(match self.current {
            None if up => n - 1,
            None => 0,
            Some(i) if up => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
        });
        self.current()
    }
}

/// Every match of `query` in the screen and its scrollback, top to bottom, not overlapping.
/// Without `match_case` letters match whatever their case. Rows joined by a wrap are one line.
#[cfg(test)]
pub fn search(screen: &Screen, query: &str, match_case: bool) -> Vec<Hit> {
    let options = Options { match_case, ..Options::default() };
    let matcher = Matcher::new(query, options).ok().flatten();
    search_from(screen, screen.first_line(), matcher.as_ref())
}

/// The first line of the logical line `line` belongs to: back over the rows a wrap continued.
fn logical_start(screen: &Screen, mut line: u64) -> u64 {
    line = line.max(screen.first_line());
    while line > screen.first_line() && screen.line_at(line - 1).is_some_and(|l| l.wrapped()) {
        line -= 1;
    }
    line
}

/// [`search`] from line `from` on, which starts a logical line.
fn search_from(screen: &Screen, from: u64, matcher: Option<&Matcher>) -> Vec<Hit> {
    let mut out = Vec::new();
    let Some(matcher) = matcher else { return out };
    let mut found = Vec::new();
    let (mut line, end) = (from.max(screen.first_line()), screen.end_line());
    // One logical line: its characters and, for each, its first cell and the column of its last.
    let mut text: Vec<char> = Vec::new();
    let mut cells: Vec<(Point, usize)> = Vec::new();
    while line < end {
        text.clear();
        cells.clear();
        while line < end {
            let Some(l) = screen.line_at(line) else {
                line += 1;
                break;
            };
            for (col, cell) in l.cells().iter().enumerate() {
                if cell.is_spacer() {
                    continue;
                }
                text.push(cell.ch);
                cells.push((Point::new(line, col), col + usize::from(cell.width.max(1)) - 1));
            }
            line += 1;
            if !l.wrapped() {
                break;
            }
        }
        while text.last() == Some(&' ') {
            text.pop();
            cells.pop();
        }
        found.clear();
        matcher.find(&text, &mut found);
        for &(first, last) in &found {
            let (end, end_col) = cells[last];
            out.push(Hit { start: cells[first].0, end: Point::new(end.line, end_col) });
        }
    }
    out
}

/// The rectangles a match covers on the rows of `view`, in the grid at `inner`.
fn hit_rects(hit: Hit, screen: &Screen, view: View, metrics: &Metrics, inner: Rect) -> Vec<Rect> {
    let top = view.top_line(screen);
    let rows = screen.rows() as u64;
    let last_col = screen.cols().saturating_sub(1);
    let mut out = Vec::new();
    for line in hit.start.line.max(top)..=hit.end.line.min(top + rows - 1) {
        let row = (line - top) as usize;
        let c0 = if line == hit.start.line { hit.start.col } else { 0 };
        let c1 = if line == hit.end.line { hit.end.col } else { last_col };
        let a = metrics.cell_rect(inner, row, c0);
        let b = metrics.cell_rect(inner, row, c1.max(c0));
        out.push(Rect::new(a.x, a.y, b.right() - a.x, a.h));
    }
    out
}

/// Marks the matches the view shows over the drawn grid, the current one in its own colour, as
/// the Markdown viewer marks its find results.
pub fn draw_marks(p: &mut Painter, pal: &Palette, find: &Find, screen: &Screen, view: View, m: &Metrics, inner: Rect) {
    let top = view.top_line(screen);
    let bottom = top + screen.rows() as u64;
    let (others, current) = if pal.dark { (0.38, 0.62) } else { (0.45, 0.62) };
    let clip = p.clip(inner);
    let first = find.matches.partition_point(|h| h.end.line < top);
    for (i, hit) in find.matches.iter().enumerate().skip(first) {
        if hit.start.line >= bottom {
            break;
        }
        let is_current = find.current == Some(i);
        let (colour, alpha) = if is_current { (pal.find_current, current) } else { (pal.find_mark, others) };
        for r in hit_rects(*hit, screen, view, m, inner) {
            p.fill_a(r, colour, alpha);
        }
    }
    p.restore(clip);
}

/// The places of the bar's parts in the terminal's `area`.
pub struct Bar {
    pub card: Rect,
    pub icon: Rect,
    pub field: Rect,
    pub count: Rect,
    pub case: Rect,
    pub word: Rect,
    pub regex: Rect,
    pub previous: Rect,
    pub next: Rect,
    pub close: Rect,
}

pub const BAR_H: f32 = 34.0;
const BUTTON: f32 = 26.0;
const COUNT_W: f32 = 92.0;
const TOGGLE: f32 = 28.0;

impl Bar {
    /// The bar at the top right of the terminal's area, 430 px wide or what fits.
    pub fn layout(area: Rect) -> Bar {
        let w = 430.0_f32.min((area.w - 16.0).max(0.0));
        let card = Rect::new(area.right() - 8.0 - w, area.y + 8.0, w, BAR_H);
        let y = card.y + (BAR_H - BUTTON) / 2.0;
        let button = |right: f32| Rect::new(right - BUTTON, y, BUTTON, BUTTON);
        let close = button(card.right() - 4.0);
        let next = button(close.x - 2.0);
        let previous = button(next.x);
        let regex = Rect::new(previous.x - 6.0 - TOGGLE, y, TOGGLE, BUTTON);
        let word = Rect::new(regex.x - 2.0 - TOGGLE, y, TOGGLE, BUTTON);
        let case = Rect::new(word.x - 2.0 - TOGGLE, y, TOGGLE, BUTTON);
        let count = Rect::new(case.x - 4.0 - COUNT_W, card.y, COUNT_W, BAR_H);
        let icon = Rect::new(card.x + 4.0, y, BUTTON, BUTTON);
        let field = Rect::new(icon.right() + 2.0, card.y, (count.x - icon.right() - 6.0).max(0.0), BAR_H);
        Bar { card, icon, field, count, case, word, regex, previous, next, close }
    }
}

impl AppState {
    /// The find bar of the terminal shown has the keyboard.
    pub fn find_focused(&self) -> bool {
        self.terminal_shown()
            && self.nothing_over()
            && self.tabs.shown().is_some_and(|tab| tab.focused_term().find.as_ref().is_some_and(|f| f.focused))
            && !self.chat_takes_input()
    }

    /// Opens the find bar of the terminal shown (or gives it the keyboard), its text selected.
    /// In Chat the terminal drawer opens for it.
    pub fn open_find(&mut self) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        if let Some(pane) = &mut tab.pane
            && pane.chat.enabled
        {
            pane.chat.show_terminal();
        }
        let find = tab.focused_term_mut().find.get_or_insert_with(Find::new);
        find.focused = true;
        find.field.select_all();
        self.clear_session_text_focus();
    }

    pub fn close_find(&mut self) {
        if let Some(tab) = self.tabs.shown_mut() {
            tab.focused_term_mut().find = None;
        }
    }

    /// Finds again after the query changed and scrolls to the match it chose.
    fn find_changed(&mut self) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let t = tab.focused_term_mut();
        let bottom = t.view(false).top_line(&t.screen) + t.screen.rows() as u64;
        let changes = t.changes;
        if let Some(hit) = t.find.as_mut().and_then(|f| f.update(&t.screen, changes, bottom)) {
            t.reveal(hit.start.line);
        }
    }

    /// Moves to the match above or below and scrolls to it.
    pub fn find_step(&mut self, up: bool) {
        self.find_changed();
        let Some(tab) = self.tabs.shown_mut() else { return };
        let t = tab.focused_term_mut();
        if let Some(hit) = t.find.as_mut().and_then(|f| f.step(up)) {
            t.reveal(hit.start.line);
        }
    }

    /// A key while the find bar has the keyboard. False for the window's own shortcuts, which go
    /// on to the window.
    pub fn find_key(&mut self, name: &KeyName, mods: Mods, text: Option<&str>) -> bool {
        if crate::termkeys::shortcut(name, mods).is_some() {
            return false;
        }
        let only_shift = mods.shift && !mods.ctrl && !mods.alt;
        match name {
            KeyName::Escape => self.close_find(),
            KeyName::Enter | KeyName::F(3) => self.find_step(!only_shift),
            KeyName::Up => self.find_step(true),
            KeyName::Down => self.find_step(false),
            KeyName::Char('c' | 'C') if mods.alt && !mods.ctrl => self.toggle_find(FindControl::Case),
            KeyName::Char('w' | 'W') if mods.alt && !mods.ctrl => self.toggle_find(FindControl::Word),
            KeyName::Char('r' | 'R') if mods.alt && !mods.ctrl => self.toggle_find(FindControl::Regex),
            KeyName::Char('f' | 'F') if mods.ctrl && mods.shift && !mods.alt => self.open_find(),
            _ => {
                let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.focused_term_mut().find.as_mut()) else {
                    return true;
                };
                if find.field.handle_key(name, mods, text, true) == Some(true) {
                    self.find_changed();
                }
            }
        }
        true
    }

    /// Text an input method committed while the bar has the keyboard.
    pub fn find_text(&mut self, text: &str) {
        if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.focused_term_mut().find.as_mut())
            && find.field.insert(text)
        {
            self.find_changed();
        }
    }

    fn toggle_find(&mut self, control: FindControl) {
        if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.focused_term_mut().find.as_mut()) {
            let o = &mut find.options;
            match control {
                FindControl::Case => o.match_case = !o.match_case,
                FindControl::Word => o.whole_word = !o.whole_word,
                FindControl::Regex => o.regex = !o.regex,
                _ => return,
            }
        }
        self.find_changed();
    }

    /// The box's text changed through its menu (Cut, Paste): find again.
    pub(crate) fn find_edited(&mut self) {
        self.find_changed();
    }

    /// The box of the focused pane's bar and where it was drawn, for the mouse.
    pub(crate) fn find_field(&mut self) -> Option<(&mut TextField, Option<FieldHit>)> {
        let find = self.tabs.shown_mut()?.focused_term_mut().find.as_mut()?;
        Some((&mut find.field, find.hit))
    }

    /// A press on a control of the bar.
    pub fn find_press(&mut self, control: FindControl) {
        match control {
            FindControl::Field | FindControl::Text => {
                if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.focused_term_mut().find.as_mut()) {
                    find.focused = true;
                }
                if let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) {
                    pane.chat.show_terminal();
                }
                self.clear_session_text_focus();
            }
            FindControl::Case | FindControl::Word | FindControl::Regex => self.toggle_find(control),
            FindControl::Previous => self.find_step(true),
            FindControl::Next => self.find_step(false),
            FindControl::Close => self.close_find(),
        }
    }

    /// Draws the bar of the terminal shown over its `area`, when it is open.
    pub fn draw_find_bar(&mut self, p: &mut Painter, area: Rect, caret_on: bool) {
        let pal = *self.pal();
        let hover = self.hover;
        let focused = self.window_focused && self.find_focused();
        let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.focused_term_mut().find.as_mut()) else { return };
        let bar = Bar::layout(area);
        if bar.card.w < 2.0 * BUTTON {
            return;
        }
        let clip = p.clip(area);
        let invalid = find.invalid().is_some();
        let edge = if invalid {
            pal.danger
        } else if focused {
            pal.accent
        } else {
            pal.popup_line
        };
        p.frame(bar.card, 6.0, pal.popup, edge);
        p.icon(glyph::SEARCH, 14.0, bar.icon.cx(), bar.icon.cy(), pal.muted);
        let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
        let placeholder = if find.options.regex { "Find (regular expression)" } else { "Find" };
        find.field.draw(p, bar.field, Face::Ui, 13.0, &colors, placeholder, focused, caret_on);
        find.hit = crate::fieldmouse::hit(p, bar.field, 13.0);
        let count = find.count();
        let none = invalid || find.matches.is_empty() && !find.field.is_empty();
        p.text(Face::Ui, 12.0, bar.count.x, bar.count.cy(), &count, if none { pal.danger } else { pal.muted }, COUNT_W);
        let button = |p: &mut Painter, r: Rect, control: FindControl, on: bool| {
            let hot = hover == Some(Target::Find(control));
            if on {
                p.rbox(r, 5.0, mix(pal.popup, pal.accent, 0.28));
            } else if hot {
                p.rbox(r, 5.0, pal.hover);
            }
        };
        let o = find.options;
        let ink = |on: bool| if on { pal.text } else { pal.muted };
        button(p, bar.case, FindControl::Case, o.match_case);
        p.text_center(Face::Ui, 12.0, bar.case, "Aa", ink(o.match_case));
        // Whole word: "ab" over a bar, as editors draw it.
        button(p, bar.word, FindControl::Word, o.whole_word);
        let word = Rect::new(bar.word.x, bar.word.y - 1.0, bar.word.w, bar.word.h);
        p.text_center(Face::Ui, 12.0, word, "ab", ink(o.whole_word));
        p.fill(Rect::new(bar.word.x + 8.0, bar.word.bottom() - 7.0, bar.word.w - 16.0, 1.0), ink(o.whole_word));
        button(p, bar.regex, FindControl::Regex, o.regex);
        p.text_center(Face::Mono, 12.0, bar.regex, ".*", ink(o.regex));
        for (r, control, icon) in [
            (bar.previous, FindControl::Previous, glyph::UP),
            (bar.next, FindControl::Next, glyph::DOWN),
            (bar.close, FindControl::Close, glyph::CLOSE),
        ] {
            button(p, r, control, false);
            let ink = if control != FindControl::Close && find.matches.is_empty() { pal.faint } else { pal.text };
            p.icon(icon, 14.0, r.cx(), r.cy(), ink);
        }
        p.restore(clip);
        self.hits.push((bar.card, Target::Find(FindControl::Field)));
        self.hits.push((bar.field, Target::Find(FindControl::Text)));
        for (r, control) in [
            (bar.case, FindControl::Case),
            (bar.word, FindControl::Word),
            (bar.regex, FindControl::Regex),
            (bar.previous, FindControl::Previous),
            (bar.next, FindControl::Next),
            (bar.close, FindControl::Close),
        ] {
            self.hits.push((r, Target::Find(control)));
        }
    }
}

/// The tooltip of a control of the bar.
pub fn tip_of(control: FindControl) -> Option<&'static str> {
    Some(match control {
        FindControl::Field | FindControl::Text => return None,
        FindControl::Case => "Match case (Alt+C)",
        FindControl::Word => "Match whole word (Alt+W)",
        FindControl::Regex => "Use regular expression (Alt+R)",
        FindControl::Previous => "Previous match, above (Enter)",
        FindControl::Next => "Next match, below (Shift+Enter)",
        FindControl::Close => "Close (Escape)",
    })
}

impl Term {
    /// Brings an open find bar's matches up to date with the screen (nothing when unchanged).
    pub fn refresh_find(&mut self) {
        let changes = self.changes;
        if let Some(find) = &mut self.find {
            find.update(&self.screen, changes, self.screen.end_line());
        }
    }

    /// Scrolls the view so that `line` shows, about in the middle, unless it already does.
    pub fn reveal(&mut self, line: u64) {
        let rows = self.screen.rows() as u64;
        let start = self.screen.screen_start();
        let top = start - self.scroll as u64;
        if (top..top + rows).contains(&line) {
            return;
        }
        let new_top = line.saturating_sub(rows / 2).clamp(self.screen.first_line(), start);
        self.scroll = (start - new_top) as usize;
    }
}

#[cfg(test)]
mod tests;
