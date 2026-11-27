//! Conservative recognition of the live CLI prompt, never of scrollback or an echoed command.
use mdshared_term::{Attrs, Cell, Screen};

fn border(c: char) -> bool {
    matches!(c, '│' | '┃' | '║' | '┆' | '┊')
}

fn faint(cell: &Cell) -> bool {
    // Do not guess from RGB: the application/terminal theme may legitimately make typed text gray.
    !cell.attrs.contains(Attrs::INVERSE) && cell.attrs.contains(Attrs::DIM)
}

fn blank(cell: &Cell) -> bool {
    cell.is_spacer() || cell.ch == ' ' || faint(cell)
}

fn footer(line: &str) -> bool {
    crate::view::chat::question::screens::menu_footer(line)
}

pub(super) fn menu(screen: &Screen) -> bool {
    let cursor = screen.cursor().row;
    // Local menu footers are below or adjacent to the input; don't classify old transcript output.
    (cursor.saturating_sub(1)..(cursor + 9).min(screen.rows())).any(|row| footer(&screen.row_text(row)))
}

/// A row that is only a horizontal rule, as the CLI draws above and below its input.
fn rule(cells: &[Cell]) -> bool {
    let mut chars = cells.iter().map(|c| c.ch).filter(|&c| c != ' ').peekable();
    chars.peek().is_some() && chars.all(|c| matches!(c, '─' | '━' | '═'))
}

fn frame(cells: &[Cell], left: usize, right: usize, top: bool) -> bool {
    let pair = (cells.get(left).map(|c| c.ch), cells.get(right).map(|c| c.ch));
    let edge = match (pair, top) {
        ((Some('╭'), Some('╮')) | (Some('┌'), Some('┐')), true)
        | ((Some('╰'), Some('╯')) | (Some('└'), Some('┘')), false) => '─',
        ((Some('┏'), Some('┓')), true) | ((Some('┗'), Some('┛')), false) => '━',
        ((Some('╔'), Some('╗')), true) | ((Some('╚'), Some('╝')), false) => '═',
        _ => return false,
    };
    cells[left + 1..right].iter().all(|c| c.ch == edge)
}

pub(super) fn prompt(screen: &Screen) -> Option<String> {
    if screen.modes().alt_screen || crate::attention::screen_permission_prompt(screen) || menu(screen) {
        return None;
    }
    let cursor = screen.cursor();
    let line = screen.line(cursor.row)?;
    // A row wrapping onto the prompt row is earlier text, except the input box's top rule: mid-turn
    // the CLI sometimes redraws that full-width rule so that it is marked as wrapping.
    let above = cursor.row.checked_sub(1).and_then(|row| screen.line(row));
    if line.wrapped() || above.is_some_and(|above| above.wrapped() && !rule(above.cells())) {
        return None;
    }
    let cells = line.cells();
    let first = cells.iter().position(|c| c.ch != ' ')?;
    let (begin, end) = if border(cells[first].ch) {
        let right = cells.iter().rposition(|c| c.ch != ' ')?;
        if right <= first + 1
            || !border(cells[right].ch)
            || !screen.line(cursor.row.checked_sub(1)?).is_some_and(|line| frame(line.cells(), first, right, true))
        {
            return None;
        }
        let mut closed = false;
        for row in cursor.row + 1..screen.rows() {
            let next = screen.line(row)?.cells();
            if frame(next, first, right, false) {
                closed = true;
                break;
            }
            if !next.get(first).is_some_and(|c| border(c.ch))
                || !next.get(right).is_some_and(|c| border(c.ch))
                || !next.get(first + 1..right)?.iter().all(blank)
            {
                return None;
            }
        }
        if !closed {
            return None;
        }
        (first + 1, right)
    } else {
        (first, cells.len())
    };
    let glyph = begin + cells[begin..end].iter().position(|c| c.ch != ' ')?;
    if cells[glyph].ch != '❯' || faint(&cells[glyph]) || cursor.col <= glyph || cursor.col >= end {
        return None;
    }
    let content = &cells[glyph + 1..end];
    if content.first().is_some_and(|c| !blank(c)) {
        return None;
    }
    let text: String = content.iter().filter(|c| !c.is_spacer()).map(|c| if faint(c) { ' ' } else { c.ch }).collect();
    Some(text.trim().to_owned())
}

pub(in crate::view) fn clean_prompt(screen: &Screen) -> bool {
    prompt(screen).as_deref() == Some("")
}

pub(super) fn working(screen: &Screen) -> bool {
    let row = screen.cursor().row;
    (row.saturating_sub(5)..(row + 6).min(screen.rows())).any(|i| {
        let row = screen.row_text(i).to_ascii_lowercase();
        row.contains("esc to interrupt") || row.contains("ctrl+c to interrupt")
    })
}

pub(super) fn interrupted(screen: &Screen) -> bool {
    let row = screen.cursor().row;
    (row.saturating_sub(5)..row)
        .any(|i| screen.row_text(i).trim_start_matches([' ', '⎿', '✻', '✽', '·']).starts_with("Interrupted ·"))
}

pub(super) fn idle_slash_prompt(screen: &Screen, quiet: bool) -> bool {
    quiet && clean_prompt(screen) && !working(screen)
}

pub(super) fn is_image_marker(text: &str) -> bool {
    text.strip_prefix("[Image #").and_then(|s| s.strip_suffix(']')).is_some_and(|n| {
        !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<u32>().is_ok_and(|n| n > 0)
    })
}

/// How many rows a multi-line input may take on the screen.
const INPUT_ROWS: usize = 40;
/// How Claude Code shows a long paste on its input line (to confirm in a live session).
const PASTE_MARKER: &str = "[Pasted text #";

/// The live input from its `❯` row down to the cursor row, without box edges.
fn input_rows(screen: &Screen) -> Option<Vec<String>> {
    if screen.modes().alt_screen || crate::attention::screen_permission_prompt(screen) || menu(screen) {
        return None;
    }
    let cursor = screen.cursor().row;
    let rows: Vec<String> = (cursor.saturating_sub(INPUT_ROWS)..=cursor)
        .map(|row| {
            let text = screen.row_text(row);
            text.trim().trim_start_matches(border).trim_end_matches(border).trim_end().to_owned()
        })
        .collect();
    let start = rows.iter().rposition(|row| row.trim_start().starts_with('❯'))?;
    // A rule between the `❯` row and the cursor is the input box's edge: the cursor is elsewhere.
    if rows[start..].iter().any(|row| !row.is_empty() && row.chars().all(|c| matches!(c, '─' | '━' | '═'))) {
        return None;
    }
    let mut rows = rows[start..].to_vec();
    rows[0] = rows[0].trim_start().trim_start_matches('❯').to_owned();
    Some(rows)
}

/// `[Pasted text #n +m lines]`, as the whole input.
fn paste_marker(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(PASTE_MARKER) else { return false };
    let Some((n, rest)) = rest.split_once(" +") else { return false };
    let lines = rest.strip_suffix(" lines]").or_else(|| rest.strip_suffix(" line]"));
    let digits = |s: &str| !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    digits(n) && lines.is_some_and(digits)
}

/// The input shows `expected`, a text of several lines: as typed (wrapping and indents aside) or
/// as the CLI's paste marker.
pub(super) fn shows_lines(screen: &Screen, expected: &str) -> bool {
    let Some(rows) = input_rows(screen) else { return false };
    let shown = rows.join("\n");
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    // A picture's marker may come first: `[Image #1] [Pasted text #1 +4 lines]`.
    let shown_text = shown.trim();
    let after_picture = shown_text
        .find(']')
        .map(|end| shown_text.split_at(end + 1))
        .filter(|(head, _)| is_image_marker(head))
        .map(|(_, rest)| rest.trim());
    paste_marker(shown_text)
        || after_picture.is_some_and(paste_marker)
        || !squash(expected).is_empty() && squash(&shown) == squash(expected)
}
