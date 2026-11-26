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
    let text = line.trim().to_ascii_lowercase();
    text.starts_with("esc to cancel")
        || text.starts_with("escape to cancel")
        || text.starts_with("esc to close")
        || text.starts_with("esc to go back")
        || text.contains("enter to select")
        || text.contains("enter to confirm")
        || ((text.contains('↑') || text.contains('↓'))
            && (text.contains("navigate") || text.contains("select") || text.contains("choose")))
}

pub(super) fn menu(screen: &Screen) -> bool {
    let cursor = screen.cursor().row;
    // Local menu footers are below or adjacent to the input; don't classify old transcript output.
    (cursor.saturating_sub(1)..(cursor + 9).min(screen.rows())).any(|row| footer(&screen.row_text(row)))
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
    if line.wrapped() || cursor.row > 0 && screen.line(cursor.row - 1).is_some_and(|line| line.wrapped()) {
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

pub(super) fn clean_prompt(screen: &Screen) -> bool {
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
