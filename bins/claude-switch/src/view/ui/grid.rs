//! What a slash command or a shell-mode command printed, drawn as the terminal draws it: the
//! output is fed into an off-screen screen of the app's own emulator (`mdshared_term`) as wide
//! as the box, and its cells are painted with the terminal's font, cell grid and colours
//! (`termview`), so `/context`'s grid of cells and the columns beside it line up and keep their
//! colours. Made once per output and width (the conversation keeps it with the row's body);
//! drawing it only paints.
//!
//! Only text, line ends, tabs and colour codes (SGR) reach the emulator: every other escape
//! sequence and control character is dropped first, so output that moves the cursor, clears
//! the screen or switches to the alternate screen still reads top to bottom as recorded.
//!
//! A line wider than the box wraps, as a terminal of that width would show it: nothing is
//! hidden, no sideways scrolling inside a list that scrolls up and down, and a copied wrapped
//! line comes out as one line.
use std::ops::Range;

use claude_switch_core::transcript::escape_len;
use mdshared_term::{Attrs, Color, Screen, char_width};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::termview::{self, CellFont, Metrics, Scheme, View};
use mdshared_ui::theme::Palette;

/// The size of the output's text, as the conversation's other mono text.
pub(in crate::view) const FONT_SIZE: f32 = 12.0;
/// The most rows an output takes, whatever its width: the screen's own limit.
const MAX_ROWS: usize = mdshared_term::screen::MAX_SIDE;
/// How far text with no colour of its own goes towards the background.
const QUIET: f32 = 0.35;
/// A tab's widest step, for counting rows before they are written.
const TAB: usize = 8;

/// The terminal's font and colours, which the window gives.
#[derive(Default)]
pub(in crate::view) struct Look {
    pub font: CellFont,
    /// The terminal theme's colours; without one, those that go with the palette.
    pub scheme: Option<Scheme>,
    /// Text the program gave no colour keeps the terminal's own colour (a live screen), instead
    /// of the quieter one of command output in the conversation.
    pub loud: bool,
}

impl Look {
    /// The colours the output is drawn in: the terminal theme's, except that text the program
    /// gave no colour is quiet, part way to the background, as the conversation's other tool
    /// output is muted beside the answers. Colours the program named are drawn as they are.
    pub fn scheme(&self, pal: &Palette) -> Scheme {
        let mut scheme = self.scheme.unwrap_or_else(|| Scheme::from_palette(pal));
        if !self.loud {
            scheme.foreground = mdshared_ui::gfx::mix(scheme.foreground, scheme.background, QUIET);
        }
        scheme
    }
}

/// One row of the grid in the plain text: where each column's character starts.
struct Row {
    /// `cols + 1` byte offsets: before the character of each column (the second cell of a wide
    /// character has its first's), then the row's end; past the row's text, its end.
    bounds: Vec<usize>,
}

pub(in crate::view) struct Grid {
    screen: Screen,
    metrics: Metrics,
    /// The rows that hold the output (the screen may have more, all blank).
    used: usize,
    /// The output as plain text: each row's characters without the blanks at its end, a line
    /// break after each row the output ended (not after one it wrapped).
    text: String,
    rows: Vec<Row>,
}

/// One piece of output: text without control characters, a colour code, or a line break.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Sgr(&'a str),
    Tab,
    Break,
}

/// The output read into pieces; everything else is dropped.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let len = match c {
            '\u{1b}' => {
                let len = escape_len(rest);
                if is_sgr(&rest[..len]) {
                    out.push(Piece::Sgr(&rest[..len]));
                }
                len
            }
            '\n' => {
                out.push(Piece::Break);
                1
            }
            '\t' => {
                out.push(Piece::Tab);
                1
            }
            c if c.is_control() => c.len_utf8(),
            _ => {
                let len = rest.find(|c: char| c.is_control()).unwrap_or(rest.len());
                out.push(Piece::Text(&rest[..len]));
                len
            }
        };
        rest = &rest[len..];
    }
    out
}

/// `ESC [`, numbers separated by `;` or `:`, and `m`: colours and attributes, nothing else.
fn is_sgr(seq: &str) -> bool {
    seq.strip_prefix("\u{1b}[")
        .and_then(|s| s.strip_suffix('m'))
        .is_some_and(|params| params.bytes().all(|b| b.is_ascii_digit() || b == b';' || b == b':'))
}

/// How many rows a line of `cells` cells takes at most in `cols` columns (a wide character that
/// does not fit at a row's end leaves its last cell empty).
fn rows_for(cells: usize, cols: usize) -> usize {
    if cols < 2 { cells.max(1) } else { cells.div_ceil(cols - 1).max(1) }
}

/// The cells that surely fit in `rows` rows of `cols` columns.
fn room(rows: usize, cols: usize) -> usize {
    rows * cols.saturating_sub(1).max(1)
}

/// The cells a character takes; a tab its widest step.
fn cells_of(c: char) -> usize {
    if c == '\t' { TAB } else { usize::from(char_width(c)) }
}

/// The line's pieces up to `room` cells, how many cells they take, and whether that is all of it.
fn cut<'a>(line: &[Piece<'a>], room: usize) -> (Vec<Piece<'a>>, usize, bool) {
    let mut out = Vec::with_capacity(line.len());
    let mut cells = 0;
    for piece in line {
        match piece {
            Piece::Text(t) => {
                let mut end = t.len();
                for (i, c) in t.char_indices() {
                    if cells + cells_of(c) > room {
                        end = i;
                        break;
                    }
                    cells += cells_of(c);
                }
                out.push(Piece::Text(&t[..end]));
                if end < t.len() {
                    return (out, cells, false);
                }
            }
            Piece::Tab if cells + TAB > room => return (out, cells, false),
            Piece::Tab => {
                cells += TAB;
                out.push(Piece::Tab);
            }
            Piece::Sgr(s) => out.push(Piece::Sgr(s)),
            Piece::Break => {}
        }
    }
    (out, cells, true)
}

impl Grid {
    /// `source` (with its colour codes, or plain) in a grid as many cells wide as fit `width`.
    pub fn new(source: &str, metrics: Metrics, width: f32) -> Grid {
        Self::with_cols(source, metrics, Self::cols_for(metrics, width))
    }

    /// `source` in a grid of `cols` columns (at least one, at most the screen's limit).
    pub fn with_cols(source: &str, metrics: Metrics, cols: usize) -> Grid {
        let cols = cols.clamp(1, mdshared_term::screen::MAX_SIDE);
        let pieces = pieces(source);
        // The rows it can take at most, so that nothing scrolls off the top; what does not fit
        // in the most the screen holds is left out at the end.
        let mut lines: Vec<Vec<Piece>> = Vec::new();
        let mut bound = 0;
        for line in pieces.split(|p| *p == Piece::Break) {
            let (line, cells, whole) = cut(line, room(MAX_ROWS - bound, cols));
            bound += rows_for(cells, cols);
            lines.push(line);
            if !whole || bound >= MAX_ROWS {
                break;
            }
        }
        let mut screen = Screen::new(cols, bound.max(1), 0);
        for (n, line) in lines.iter().enumerate() {
            if n > 0 {
                // The pen goes on to the next line, as in a terminal.
                screen.feed(b"\r\n");
            }
            for piece in line {
                match piece {
                    Piece::Text(t) | Piece::Sgr(t) => screen.feed(t.as_bytes()),
                    Piece::Tab => screen.feed(b"\t"),
                    Piece::Break => {}
                }
            }
        }
        let mut used = (screen.cursor().row + 1).min(screen.rows());
        while used > 1 && blank(&screen, used - 1) {
            used -= 1;
        }
        let (text, rows) = plain(&screen, used);
        Grid { screen, metrics, used, text, rows }
    }

    /// The columns of a grid in `width`.
    pub fn cols_for(metrics: Metrics, width: f32) -> usize {
        ((width / metrics.cell_w()).floor() as usize).clamp(1, mdshared_term::screen::MAX_SIDE)
    }

    /// The screen the grid was laid out on.
    #[cfg(test)]
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn cols(&self) -> usize {
        self.screen.cols()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn height(&self) -> f32 {
        self.used as f32 * self.metrics.cell_h()
    }

    /// As wide as its widest row.
    pub fn width(&self) -> f32 {
        let widest = (0..self.used)
            .filter_map(|r| self.screen.line(r))
            .map(|line| line.cells().iter().rposition(|c| !is_blank(c)).map_or(0, |i| i + 1))
            .max()
            .unwrap_or(0);
        widest as f32 * self.metrics.cell_w()
    }

    /// The place in the text nearest `(x, y)` (from the grid's top left corner).
    pub fn at(&self, x: f32, y: f32) -> usize {
        let row = ((y / self.metrics.cell_h()).floor().max(0.0) as usize).min(self.used - 1);
        let col = ((x / self.metrics.cell_w()).round().max(0.0) as usize).min(self.cols());
        self.rows[row].bounds[col]
    }

    /// Draws the grid with its top left corner at `(x, y)`, and `selection` (a range of the
    /// text) over it: only its rows that `clip` shows.
    pub fn draw(
        &self,
        p: &mut Painter,
        (x, y): (f32, f32),
        clip: Rect,
        selection: Option<Range<usize>>,
        pal: &Palette,
        look: &mut Look,
    ) {
        let scheme = look.scheme(pal);
        let area = Rect::new(x, y, self.cols() as f32 * self.metrics.cell_w(), self.height());
        let view = View::new().cursor_shown(false);
        let h = self.metrics.cell_h();
        let first = (((clip.y - y) / h).floor().max(0.0) as usize).min(self.used);
        let rows = first..((((clip.bottom() - y) / h).ceil().max(0.0) as usize).min(self.used)).max(first);
        termview::draw_rows(
            p,
            &mut look.font,
            &scheme,
            &self.metrics,
            area,
            &self.screen,
            None,
            view,
            std::slice::from_ref(&rows),
        );
        let Some(sel) = selection.filter(|s| !s.is_empty()) else { return };
        for (r, row) in self.rows.iter().enumerate().take(rows.end).skip(rows.start) {
            for cols in selected_runs(row, sel.clone()) {
                let cell = self.metrics.cell_rect(area, r, cols.start);
                let w = (cols.end - cols.start) as f32 * self.metrics.cell_w();
                // As the toolkit's text selection shows it: its colour at 0.35 over the text.
                p.fill_a(Rect::new(cell.x, cell.y, w, cell.h), pal.selection, 0.35);
            }
        }
    }
}

/// The runs of columns of `row` whose characters `sel` covers.
fn selected_runs(row: &Row, sel: Range<usize>) -> Vec<Range<usize>> {
    let end = *row.bounds.last().unwrap_or(&0);
    let mut runs: Vec<Range<usize>> = Vec::new();
    for (col, &at) in row.bounds.iter().enumerate().take(row.bounds.len() - 1) {
        if at < end && sel.contains(&at) {
            match runs.last_mut() {
                Some(run) if run.end == col => run.end = col + 1,
                _ => runs.push(col..col + 1),
            }
        }
    }
    runs
}

fn is_blank(c: &mdshared_term::Cell) -> bool {
    c.ch == ' ' && c.bg == Color::Default && !c.attrs.contains(Attrs::INVERSE)
}

fn blank(screen: &Screen, row: usize) -> bool {
    screen.line(row).is_none_or(|line| line.cells().iter().all(is_blank))
}

/// The plain text of the first `used` rows, and where each column starts in it.
fn plain(screen: &Screen, used: usize) -> (String, Vec<Row>) {
    let mut text = String::new();
    let mut rows = Vec::with_capacity(used);
    for r in 0..used {
        let Some(line) = screen.line(r) else { break };
        let start = text.len();
        let mut bounds = Vec::with_capacity(screen.cols() + 1);
        for col in 0..screen.cols() {
            let cell = line.cell(col);
            if cell.is_spacer() {
                bounds.push(bounds.last().copied().unwrap_or(text.len()));
            } else {
                bounds.push(text.len());
                text.push(cell.ch);
            }
        }
        // A row the output ended keeps no blanks at its end; a wrapped row is all text.
        if !line.wrapped() {
            text.truncate(start + text[start..].trim_end_matches(' ').len());
        }
        let end = text.len();
        bounds.push(end);
        for b in &mut bounds {
            *b = (*b).min(end);
        }
        rows.push(Row { bounds });
        if r + 1 < used && !line.wrapped() {
            text.push('\n');
        }
    }
    (text, rows)
}

#[cfg(test)]
pub(in crate::view) mod tests;
