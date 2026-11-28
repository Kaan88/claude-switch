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
    // Claude Code 2.1.332 draws an empty input as the mark and a no-break space.
    cell.is_spacer() || cell.ch == ' ' || cell.ch == '\u{a0}' || faint(cell)
}

/// The mark before Claude Code's input: `❯`, or `>` where Claude Code takes the terminal for one
/// without Unicode (on Windows: no `TERM`, `WT_SESSION` or the like, as in a tab of a program
/// started from the Start menu; Claude Code 2.1.332). A `>` alone proves little, so readers ask
/// for the input box's rule right above its row as well ([`marks_input`]).
pub(in crate::view) fn prompt_mark(c: char) -> bool {
    matches!(c, '❯' | '>')
}

/// Row `row` starts Claude Code's input: its first character is the input mark, and for the
/// plain `>` the row right above is the input box's rule. A `❯` before a numbered text is a
/// menu's highlighted choice (`❯ 1. Yes`) unless the input box's rule is right above it: a
/// message typed or sent as `1. fix a` is the input all the same.
pub(in crate::view) fn marks_input(screen: &Screen, row: usize) -> bool {
    let text = screen.row_text(row);
    let ruled = || row.checked_sub(1).and_then(|above| screen.line(above)).is_some_and(|line| rule(line.cells()));
    match text.trim_start().chars().next() {
        Some('❯') => !numbered_choice(text.trim_start().trim_start_matches('❯')) || ruled(),
        Some('>') => ruled(),
        _ => false,
    }
}

/// `1. Yes`: a menu's numbered choice after its highlight mark.
fn numbered_choice(text: &str) -> bool {
    let text = text.trim_start();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && text[digits..].starts_with(". ")
}

/// A digest of what the CLI draws from the top through its input box's bottom rule (the whole
/// screen when no input box is found). What it draws under that rule (the hints, the list of
/// background agents with their running timers and token counts) is left out: redrawn every
/// second, it would keep the screen from ever going quiet while the CLI is at rest (2.1.332).
pub(in crate::view) fn activity_digest(screen: &Screen) -> u64 {
    use std::hash::{Hash, Hasher};
    // The lowest input mark, right under the box's top rule: an earlier prompt in the history
    // has no rule over it, and a menu (which has no input box) must count whole.
    let input = (0..screen.rows())
        .rev()
        .find(|&row| marks_input(screen, row))
        .filter(|&row| row.checked_sub(1).and_then(|above| screen.line(above)).is_some_and(|line| rule(line.cells())));
    let bottom = input.and_then(|row| {
        (row + 1..screen.rows()).find(|&below| screen.line(below).is_some_and(|line| rule(line.cells())))
    });
    let end = bottom.map_or(screen.rows(), |row| row + 1);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for row in 0..end {
        screen.row_text(row).hash(&mut hasher);
    }
    hasher.finish()
}

fn footer(line: &str) -> bool {
    crate::view::chat::question::screens::menu_footer(line)
}

pub(in crate::view) fn menu(screen: &Screen) -> bool {
    let cursor = screen.cursor().row;
    // Local menu footers are below or adjacent to the input; don't classify old transcript output.
    // With a menu open the CLI may park the cursor below it or in its corner, so a footer as the
    // last row with anything on it counts too.
    // A status line under the idle input box is not a menu, whatever it says.
    let last = (0..screen.rows()).rposition(|row| !screen.row_text(row).trim().is_empty());
    let menu_footer = |row: usize| {
        footer(&screen.row_text(row)) && !crate::view::chat::question::screens::input_box_above(screen, row)
    };
    (cursor.saturating_sub(1)..(cursor + 9).min(screen.rows())).any(menu_footer)
        || last.is_some_and(menu_footer)
        || crate::view::chat::question::screens::live_footer(screen).is_some()
}

/// A row that is only a horizontal rule, as the CLI draws above and below its input, or the
/// rule above it with the session's name ([`named_rule`]).
fn rule(cells: &[Cell]) -> bool {
    let text: String = cells.iter().filter(|c| !c.is_spacer()).map(|c| c.ch).collect();
    let mut chars = text.chars().filter(|&c| c != ' ').peekable();
    chars.peek().is_some() && chars.all(solid) || named_rule(&text)
}

fn solid(c: char) -> bool {
    matches!(c, '─' | '━' | '═')
}

/// The rule over the input of a named session: Claude Code writes the name into it near its
/// right end, `──────── Fix the flaky tile cache test ─` (2.1.332; a session is named by
/// `/rename` or `--name`, as this app names its sessions). A run of rule characters, the name
/// between spaces, and the rule again.
pub(in crate::view) fn named_rule(text: &str) -> bool {
    let text = text.trim();
    let lead = text.chars().take_while(|&c| solid(c)).count();
    let tail = text.chars().rev().take_while(|&c| solid(c)).count();
    let total = text.chars().count();
    if lead < 4 || tail == 0 || lead + tail >= total {
        return false;
    }
    let name: String = text.chars().skip(lead).take(total - lead - tail).collect();
    name.starts_with(' ') && name.ends_with(' ') && !name.trim().is_empty()
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

pub(in crate::view::chat) fn prompt(screen: &Screen) -> Option<String> {
    if screen.modes().alt_screen || crate::attention::screen_permission_prompt(screen) || menu(screen) {
        return None;
    }
    let cursor = screen.cursor();
    let line = screen.line(cursor.row)?;
    // A row wrapping onto the prompt row is earlier text, except the input box's top rule: mid-turn
    // the CLI sometimes redraws that full-width rule so that it is marked as wrapping.
    let above = cursor.row.checked_sub(1).and_then(|row| screen.line(row));
    // The prompt row's own wrap mark is a leftover when the row under it is the input box's rule
    // or empty: the CLI redrew the row where a full-width one had been (seen after a declined
    // permission prompt, Claude Code 2.1.332). Input that really wraps goes on in the next row.
    let wraps_on = line.wrapped()
        && screen.line(cursor.row + 1).is_some_and(|next| {
            let cells = next.cells();
            !rule(cells) && cells.iter().any(|c| c.ch != ' ' && !c.is_spacer())
        });
    if wraps_on || above.is_some_and(|above| above.wrapped() && !rule(above.cells())) {
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
    if !prompt_mark(cells[glyph].ch) || faint(&cells[glyph]) || cursor.col <= glyph || cursor.col >= end {
        return None;
    }
    // The plain `>` is the input's mark only in the input box: right under its rule, with the
    // rule that closes the box somewhere below.
    if cells[glyph].ch == '>'
        && !(above.is_some_and(|above| rule(above.cells()))
            && (cursor.row + 1..screen.rows()).any(|row| screen.line(row).is_some_and(|line| rule(line.cells()))))
    {
        return None;
    }
    let content = &cells[glyph + 1..end];
    if content.first().is_some_and(|c| !blank(c)) {
        return None;
    }
    // A no-break space counts as blank above (the empty input is drawn with one) but is kept in the
    // text, so a draft containing one still reads back exactly; `trim` drops it at the ends.
    let text: String = content.iter().filter(|c| !c.is_spacer()).map(|c| if faint(c) { ' ' } else { c.ch }).collect();
    Some(text.trim().to_owned())
}

pub(in crate::view) fn clean_prompt(screen: &Screen) -> bool {
    prompt(screen).as_deref() == Some("")
}

/// The CLI shows a turn running: its interrupt hint in the rows under the input (Claude Code
/// 2.1.332 puts it in the hint row there, and drops it while the input holds text), or its
/// working line (spinner, word, "…", timer) in the rows right above the input box. Above the
/// input only that line counts, never the words: an answer that mentions "esc to interrupt" is
/// not a running turn.
pub(in crate::view::chat) fn working(screen: &Screen) -> bool {
    let row = screen.cursor().row;
    let hint = |i: usize| {
        let row = screen.row_text(i).to_ascii_lowercase();
        row.contains("esc to interrupt") || row.contains("ctrl+c to interrupt")
    };
    (row + 1..(row + 6).min(screen.rows())).any(hint)
        || (row.saturating_sub(5)..row).any(|i| crate::view::working::working_row(&screen.row_text(i)))
}

pub(super) fn interrupted(screen: &Screen) -> bool {
    let row = screen.cursor().row;
    (row.saturating_sub(5)..row)
        .any(|i| screen.row_text(i).trim_start_matches([' ', '⎿', '✻', '✽', '·']).starts_with("Interrupted ·"))
}

pub(in crate::view::chat) fn idle_slash_prompt(screen: &Screen, quiet: bool) -> bool {
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
    let first = cursor.saturating_sub(INPUT_ROWS);
    let start = (0..rows.len()).rposition(|i| marks_input(screen, first + i))?;
    // A rule between the `❯` row and the cursor is the input box's edge: the cursor is elsewhere.
    if rows[start..].iter().any(|row| !row.is_empty() && row.chars().all(|c| matches!(c, '─' | '━' | '═'))) {
        return None;
    }
    let mut rows = rows[start..].to_vec();
    rows[0] = rows[0].trim_start().strip_prefix(prompt_mark).unwrap_or(&rows[0]).to_owned();
    Some(rows)
}

/// The input shows `expected`, a single line too long for one row: its rows from the mark to the
/// cursor read it, wrapping aside.
pub(in crate::view::chat) fn shows_wrapped(screen: &Screen, expected: &str) -> bool {
    let Some(rows) = input_rows(screen) else { return false };
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    rows.len() > 1 && !squash(expected).is_empty() && squash(&rows.join(" ")) == squash(expected)
}

/// The cursor is in the CLI's input, whatever it holds.
pub(in crate::view::chat) fn shows_some_input(screen: &Screen) -> bool {
    input_rows(screen).is_some()
}

/// `[Pasted text #n +m lines]`, as the whole input.
fn paste_marker(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(PASTE_MARKER) else { return false };
    let digits = |s: &str| !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    // A long single line: `[Pasted text #1]` (2.1.332, `recorder compat`'s `queued-text`).
    if rest.strip_suffix(']').is_some_and(digits) {
        return true;
    }
    let Some((n, rest)) = rest.split_once(" +") else { return false };
    let lines = rest.strip_suffix(" lines]").or_else(|| rest.strip_suffix(" line]"));
    let digits = |s: &str| !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    digits(n) && lines.is_some_and(digits)
}

/// The input shows `expected`, a text of several lines: as typed (wrapping and indents aside) or
/// as the CLI's paste marker.
pub(in crate::view::chat) fn shows_lines(screen: &Screen, expected: &str) -> bool {
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
