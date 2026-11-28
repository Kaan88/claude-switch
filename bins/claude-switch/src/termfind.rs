//! Find in a terminal: the bar over the grid, the matches in the screen and its scrollback, the
//! current one, and the keys and buttons that move between them.
//!
//! A terminal keeps its own [`Find`] while the bar is open. The matches are found again whenever
//! the query, the case setting or the screen changed ([`Term::changes`]), so new output and a
//! resize (which rewraps the lines) are searched as they come. A match is found across a wrap:
//! the rows a long line wrapped onto are searched as one line, as copying sees them.

use mdshared_term::{Point, Screen};
use mdshared_ui::gfx::{Face, Painter, Rect, mix};
use mdshared_ui::glyph;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::termview::{Metrics, View};
use mdshared_ui::textfield::{FieldColors, TextField};
use mdshared_ui::theme::Palette;

use crate::state::{AppState, Target};
use crate::term::Term;

/// A control of the find bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FindControl {
    /// The card itself and its text box: a press gives it the keyboard.
    Field,
    Case,
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
    pub match_case: bool,
    matches: Vec<Hit>,
    current: Option<usize>,
    /// What `matches` were found for: the query, the case setting and the terminal's change count.
    found: Option<(String, bool, u64)>,
}

impl Find {
    pub fn new() -> Find {
        Find { focused: true, ..Find::default() }
    }

    pub fn current(&self) -> Option<Hit> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    /// "3 of 12", "No results", or nothing while the box is empty.
    pub fn count(&self) -> String {
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
        let same_query = self.found.as_ref().is_some_and(|(q, c, _)| q == query && *c == self.match_case);
        if same_query && self.found.as_ref().is_some_and(|f| f.2 == changes) {
            return None;
        }
        let before = self.current();
        self.matches = search(screen, query, self.match_case);
        self.found = Some((query.to_string(), self.match_case, changes));
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
pub fn search(screen: &Screen, query: &str, match_case: bool) -> Vec<Hit> {
    let fold = |c: char| if match_case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let needle: Vec<char> = query.chars().map(fold).collect();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let (mut line, end) = (screen.first_line(), screen.end_line());
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
                text.push(fold(cell.ch));
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
        let n = needle.len();
        let mut i = 0;
        while i + n <= text.len() {
            if text[i..i + n] == needle[..] {
                let (last, last_col) = cells[i + n - 1];
                out.push(Hit { start: cells[i].0, end: Point::new(last.line, last_col) });
                i += n;
            } else {
                i += 1;
            }
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
    pub previous: Rect,
    pub next: Rect,
    pub close: Rect,
}

pub const BAR_H: f32 = 34.0;
const BUTTON: f32 = 26.0;
const COUNT_W: f32 = 78.0;

impl Bar {
    /// The bar at the top right of the terminal's area, 360 px wide or what fits.
    pub fn layout(area: Rect) -> Bar {
        let w = 360.0_f32.min((area.w - 16.0).max(0.0));
        let card = Rect::new(area.right() - 8.0 - w, area.y + 8.0, w, BAR_H);
        let y = card.y + (BAR_H - BUTTON) / 2.0;
        let button = |right: f32| Rect::new(right - BUTTON, y, BUTTON, BUTTON);
        let close = button(card.right() - 4.0);
        let next = button(close.x - 2.0);
        let previous = button(next.x);
        let case = Rect::new(previous.x - 6.0 - 30.0, y, 30.0, BUTTON);
        let count = Rect::new(case.x - 4.0 - COUNT_W, card.y, COUNT_W, BAR_H);
        let icon = Rect::new(card.x + 4.0, y, BUTTON, BUTTON);
        let field = Rect::new(icon.right() + 2.0, card.y, (count.x - icon.right() - 6.0).max(0.0), BAR_H);
        Bar { card, icon, field, count, case, previous, next, close }
    }
}

impl AppState {
    /// The find bar of the terminal shown has the keyboard.
    pub fn find_focused(&self) -> bool {
        self.terminal_shown()
            && self.nothing_over()
            && self.tabs.shown().is_some_and(|tab| tab.term.find.as_ref().is_some_and(|f| f.focused))
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
        let find = tab.term.find.get_or_insert_with(Find::new);
        find.focused = true;
        find.field.select_all();
        self.clear_session_text_focus();
    }

    pub fn close_find(&mut self) {
        if let Some(tab) = self.tabs.shown_mut() {
            tab.term.find = None;
        }
    }

    /// Finds again after the query changed and scrolls to the match it chose.
    fn find_changed(&mut self) {
        let Some(tab) = self.tabs.shown_mut() else { return };
        let t = &mut tab.term;
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
        let t = &mut tab.term;
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
            KeyName::Char('c' | 'C') if mods.alt && !mods.ctrl => self.toggle_find_case(),
            KeyName::Char('f' | 'F') if mods.ctrl && mods.shift && !mods.alt => self.open_find(),
            _ => {
                let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut()) else { return true };
                if find.field.handle_key(name, mods, text, true) == Some(true) {
                    self.find_changed();
                }
            }
        }
        true
    }

    /// Text an input method committed while the bar has the keyboard.
    pub fn find_text(&mut self, text: &str) {
        if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut())
            && find.field.insert(text)
        {
            self.find_changed();
        }
    }

    fn toggle_find_case(&mut self) {
        if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut()) {
            find.match_case = !find.match_case;
        }
        self.find_changed();
    }

    /// A press on a control of the bar.
    pub fn find_press(&mut self, control: FindControl) {
        match control {
            FindControl::Field => {
                if let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut()) {
                    find.focused = true;
                }
                if let Some(pane) = self.tabs.shown_mut().and_then(|tab| tab.pane.as_mut()) {
                    pane.chat.show_terminal();
                }
                self.clear_session_text_focus();
            }
            FindControl::Case => self.toggle_find_case(),
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
        let Some(find) = self.tabs.shown_mut().and_then(|tab| tab.term.find.as_mut()) else { return };
        let bar = Bar::layout(area);
        if bar.card.w < 2.0 * BUTTON {
            return;
        }
        let clip = p.clip(area);
        p.frame(bar.card, 6.0, pal.popup, if focused { pal.accent } else { pal.popup_line });
        p.icon(glyph::SEARCH, 14.0, bar.icon.cx(), bar.icon.cy(), pal.muted);
        let colors = FieldColors { text: pal.text, placeholder: pal.muted, selection: pal.selection, caret: pal.text };
        find.field.draw(p, bar.field, Face::Ui, 13.0, &colors, "Find", focused, caret_on);
        let count = find.count();
        let none = find.matches.is_empty() && !find.field.is_empty();
        p.text(Face::Ui, 12.0, bar.count.x, bar.count.cy(), &count, if none { pal.danger } else { pal.muted }, COUNT_W);
        let button = |p: &mut Painter, r: Rect, control: FindControl, on: bool| {
            let hot = hover == Some(Target::Find(control));
            if on {
                p.rbox(r, 5.0, mix(pal.popup, pal.accent, 0.28));
            } else if hot {
                p.rbox(r, 5.0, pal.hover);
            }
        };
        button(p, bar.case, FindControl::Case, find.match_case);
        p.text_center(Face::Ui, 12.0, bar.case, "Aa", if find.match_case { pal.text } else { pal.muted });
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
        for (r, control) in [
            (bar.case, FindControl::Case),
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
        FindControl::Field => return None,
        FindControl::Case => "Match case (Alt+C)",
        FindControl::Previous => "Previous match, above (Enter)",
        FindControl::Next => "Next match, below (Shift+Enter)",
        FindControl::Close => "Close (Escape)",
    })
}

impl Term {
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
