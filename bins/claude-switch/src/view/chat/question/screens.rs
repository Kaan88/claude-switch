//! Every piece of Claude Code screen text the Chat cards rely on, in one place. None of it is a
//! protocol: each pattern is a conservative reading of the classic renderer, to be confirmed in a
//! live session. A screen that does not match exactly is not answered.
use mdshared_term::Screen;

use crate::view::chat::submission::screen::prompt_mark;

/// Footer hints under a CLI menu, picker or dialog (the last non-empty row), lower case.
const MENU_FOOTERS: [&str; 9] = [
    "esc to cancel",
    "escape to cancel",
    "esc to close",
    "esc to go back",
    "esc to exit",
    "enter to select",
    "enter to confirm",
    "enter to continue",
    // The plan approval's only hint; it wraps onto the plan file's path (Claude Code 2.1.332).
    "ctrl+g to edit",
];
/// The row the CLI shows under a running shell command that can move to the background.
const BACKGROUND_HINT: &str = "(ctrl+b to run in background)";
/// How the call's block reads once the command runs in the background.
const BACKGROUNDED: &str = "Running in the background";
/// How far above the input row the Ctrl+B hint may be (the working line comes between), how many
/// rows of output above it its call's row may be, and how far up a backgrounded call is looked for.
const HINT_ROWS: usize = 8;
const BLOCK_ROWS: usize = 12;
const MOVED_ROWS: usize = 24;
/// The rewind list and its choices for one prompt.
const REWIND_TITLE: &str = "Rewind";
const REWIND_LIST: &str = "Restore the code and/or conversation to the point before…";
const REWIND_LIST_FOOTER: &str = "Enter to continue · Esc to cancel";
const REWIND_CURRENT: &str = "(current)";
const REWIND_ACTIONS: &str = "Restore to the point before you sent this message:";
/// The hints under the input box about the agent list: its entry in the input's hint row, and
/// the list's own hints while it has the keys.
const LIST_MANAGE: &str = "↓ to manage";
const LIST_SELECT: &str = "↑/↓ to select";
/// With the highlight on an agent: "enter to view · x to stop" (or "x to clear" once it ended),
/// with "ctrl+x ctrl+k to stop all agents" after them when there are several.
const LIST_VIEW: &str = "enter to view";
const LIST_STOP: &str = "x to stop";
const LIST_CLEAR: &str = "x to clear";
/// How many rows above its footer a panel's card copies when it cannot tell where the panel
/// starts.
const PANEL_ROWS: usize = 14;

/// A menu footer: one of the hints, or arrows with a word for moving or choosing.
pub(in crate::view::chat) fn menu_footer(line: &str) -> bool {
    let text = line.trim().to_ascii_lowercase();
    MENU_FOOTERS.iter().any(|f| text.contains(f))
        || (text.contains('↑') || text.contains('↓'))
            && (text.contains("navigate") || text.contains("select") || text.contains("choose"))
}
/// The free-text row after an `AskUserQuestion`'s options.
pub(super) const OTHER: &str = "Type something";
/// The row that leaves an `AskUserQuestion` to talk about it.
pub(super) const CHAT_ABOUT: &str = "Chat about this";
/// How a permission prompt asks (the row above its first choice).
pub(super) const PERMISSION_ASKS: [&str; 2] = ["Do you want to ", "Would you like to "];
/// The decline choice that lets the user tell Claude what to do instead.
pub(super) const TELL_CLAUDE: &str = "tell Claude";
/// A footer hint that Tab opens a comment on the highlighted choice, lower case.
pub(super) const AMEND: &str = "tab to amend";
/// How the comment field Tab opens shows on its choice's row: the choice, this mark, then the text
/// typed so far. Without it the row is still a menu row, where digits pick choices.
pub(super) const COMMENT_MARK: &str = ":";
/// Rows only a permission prompt shows, lower case. A menu with one of them among its own rows
/// (below the nearest rule above its footer) is never offered as a generic menu card, even when
/// the prompt itself is not recognised.
const PERMISSION_LOOKS: [&str; 5] =
    ["do you want to ", "would you like to ", "tab to amend", "don't ask again", "tell claude what to do"];
/// How many rows above its footer a prompt's look is searched.
const LOOK_ROWS: usize = 24;
/// The plan approval question shown for `ExitPlanMode`.
pub(super) const PLAN_ASKS: &str = "Would you like to proceed?";
/// The plan approval choice that declines (keeps planning), in older and newer wording
/// ("Tell Claude what to change" in Claude Code 2.1.332).
pub(super) const PLAN_KEEP: [&str; 2] = ["keep planning", "Tell Claude what to change"];
/// The permission mode indicators under the input box, lower case, with the chip's names.
const MODES: [(&str, &str); 6] = [
    ("accept edits on", "Accept edits"),
    ("plan mode on", "Plan"),
    ("auto mode on", "Auto"),
    ("don't ask on", "Don't ask"),
    ("bypass permissions on", "Bypass"),
    ("manual mode on", "Manual"),
];
/// How far below the input row the mode indicator may be.
const MODE_ROWS: usize = 4;
/// How far above a menu's footer its rows may start.
const MENU_ROWS: usize = 60;
/// How many rows above its first choice a question's tab row and text may use.
const FORM_ABOVE: usize = 20;
/// How many rows above a permission question its tool description may use.
const BODY_ROWS: usize = 16;

fn border(c: char) -> bool {
    matches!(c, '│' | '┃' | '║' | '┆' | '┊')
}

pub(super) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A row without its box edges and highlight mark; whether it was highlighted.
pub(super) fn clean(row: &str) -> (bool, String) {
    let row = row.trim().trim_matches(border).trim();
    let highlighted = row.starts_with(['❯', '>']);
    (highlighted, collapse(row.trim_start_matches(['❯', '>'])))
}

/// A box's top or bottom edge, or a rule (dashed ones frame a command or diff inside a prompt):
/// never part of a prompt's text.
fn rule(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| matches!(c, '─' | '━' | '═' | '╌' | '┄' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘'))
}

/// The solid rule the CLI draws over a prompt or menu: above it is the conversation.
fn solid_rule(text: &str) -> bool {
    text.chars().count() >= 20 && text.chars().all(|c| matches!(c, '─' | '━' | '═'))
}

/// How many rows with text under a menu's footer are passed over to find it.
const UNDER_FOOTER: usize = 6;

/// The footer of whatever the CLI draws at the bottom: the last non-empty row, or the row right
/// above it when the footer wrapped (it then ends with its `·` separator). `rows` are the screen's
/// cleaned rows; the last row is found on the screen itself, where an empty input (`❯`) counts.
/// A menu takes the input box's place, so while no input row is under it a footer a few rows up
/// still counts, whatever the CLI or a redraw left under it (a rule, a notice): the
/// conversation's own words are above the input row, never under it.
fn footer_row(screen: &Screen, rows: &[String]) -> Option<usize> {
    let last = (0..screen.rows()).rposition(|row| !screen.row_text(row).trim().is_empty())?;
    let footer = if menu_footer(&rows[last]) {
        last
    } else {
        let prev = last.checked_sub(1)?;
        if menu_footer(&rows[prev]) && rows[prev].ends_with('·') {
            prev
        } else {
            let mut texts = (0..last).rev().filter(|&row| !rows[row].is_empty()).take(UNDER_FOOTER);
            let footer = texts.find(|&row| menu_footer(&rows[row]))?;
            let input = |row| crate::view::chat::submission::screen::marks_input(screen, row);
            if (footer + 1..=last).any(input) {
                return None;
            }
            footer
        }
    };
    (!input_box_above(screen, footer)).then_some(footer)
}

/// The row of the footer of a menu, picker or dialog that has the screen, if one shows.
pub(in crate::view) fn live_footer(screen: &Screen) -> Option<usize> {
    let rows: Vec<String> = (0..screen.rows()).map(|row| clean(&screen.row_text(row)).1).collect();
    footer_row(screen, &rows)
}

/// The CLI's input box (a `❯` row right under a solid rule, closed by another rule) sits just above
/// `row`: `row` is then a status line under the idle input (a custom status line or a tip that
/// happens to read like a menu hint), not a menu's footer.
pub(in crate::view::chat) fn input_box_above(screen: &Screen, row: usize) -> bool {
    let text = |r: usize| screen.row_text(r).trim().to_owned();
    let Some(bottom) = (row.saturating_sub(8)..row).rev().find(|&r| solid_rule(&text(r))) else {
        return false;
    };
    let Some(top) = (bottom.saturating_sub(12)..bottom).rev().find(|&r| solid_rule(&text(r))) else {
        return false;
    };
    let first = text(top + 1);
    first.starts_with(prompt_mark) && numbered(first.trim_start_matches(prompt_mark).trim()).is_none()
}

pub(super) fn numbered(text: &str) -> Option<(usize, &str)> {
    let (n, rest) = text.split_once(". ")?;
    (!n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit())).then_some(())?;
    Some((n.parse().ok()?, rest))
}

/// A menu entry as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub row: usize,
    pub highlighted: bool,
    pub text: String,
}

/// The numbered menu above the screen's last row, which must be a menu footer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Menu {
    pub entries: Vec<Entry>,
    /// Cleaned rows of the screen, for the text around the entries.
    pub lines: Vec<String>,
    pub footer: usize,
}

impl Menu {
    pub fn highlight(&self) -> Option<usize> {
        let mut marked = self.entries.iter().enumerate().filter(|(_, e)| e.highlighted);
        let (first, _) = marked.next()?;
        marked.next().is_none().then_some(first)
    }

    pub fn texts(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.text.clone()).collect()
    }

    /// Non-empty rows above the first entry, nearest last, without box rules, from the prompt's
    /// own top rule down: never the conversation above it.
    pub fn above_box(&self, rows: usize) -> Vec<&str> {
        let first = self.entries.first().map_or(0, |e| e.row);
        let start = first.saturating_sub(rows);
        let lines = self.lines.get(start..first).unwrap_or_default();
        let top = lines.iter().rposition(|text| solid_rule(text)).map_or(0, |at| at + 1);
        lines[top..].iter().map(String::as_str).filter(|text| !text.is_empty() && !rule(text)).collect()
    }

    /// Non-empty rows above the first entry, nearest last, without box rules.
    pub fn above(&self, rows: usize) -> Vec<&str> {
        let first = self.entries.first().map_or(0, |e| e.row);
        self.lines
            .get(first.saturating_sub(rows)..first)
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .filter(|text| !text.is_empty() && !rule(text))
            .collect()
    }

    /// Rows from entry `i` to the footer, joined.
    /// `None` for an entry the menu does not have: never an index out of bounds.
    pub fn rows_from(&self, i: usize) -> Option<String> {
        let rows = self.lines.get(self.entries.get(i)?.row..self.footer)?;
        Some(collapse(&rows.join(" ")))
    }

    /// The text in the comment field open on entry `i`, whose choice reads `choice`: its row and
    /// the rows it wraps onto, up to the next entry. `None` while the row shows no field.
    pub fn comment(&self, i: usize, choice: &str) -> Option<String> {
        let entry = self.entries.get(i)?;
        let end = self.entries.get(i + 1).map_or(self.footer, |e| e.row);
        let wrapped = self.lines.get(entry.row + 1..end)?.join(" ");
        let rows = collapse(&format!("{} {wrapped}", entry.text));
        let rest = rows.strip_prefix(choice)?.strip_prefix(COMMENT_MARK)?;
        Some(rest.trim().to_owned())
    }

    /// The footer row, lower case.
    pub fn footer_text(&self) -> String {
        self.lines.get(self.footer).map(|s| s.to_lowercase()).unwrap_or_default()
    }
}

pub(super) fn menu(screen: &Screen) -> Option<Menu> {
    let rows: Vec<_> = (0..screen.rows()).map(|row| clean(&screen.row_text(row))).collect();
    let texts: Vec<String> = rows.iter().map(|(_, text)| text.clone()).collect();
    let footer = footer_row(screen, &texts)?;
    let mut entries: Vec<Entry> = Vec::new();
    for (row, (highlighted, text)) in rows.iter().enumerate().take(footer).skip(footer.saturating_sub(MENU_ROWS)) {
        let Some((n, rest)) = numbered(text) else { continue };
        if n == 1 {
            entries.clear();
        }
        if n == entries.len() + 1 {
            entries.push(Entry { row, highlighted: *highlighted, text: rest.to_owned() });
        } else {
            entries.clear();
        }
    }
    (!entries.is_empty()).then(|| Menu { entries, lines: rows.into_iter().map(|(_, t)| t).collect(), footer })
}

/// A permission prompt as the screen shows it: the question row right above its choices and the
/// tool description above that (including warnings such as a countdown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Permission {
    pub menu: Menu,
    pub question: String,
    pub body: Vec<String>,
    /// The decline choice that asks what to do instead.
    pub tell: Option<usize>,
}

pub(super) fn permission(screen: &Screen) -> Option<Permission> {
    if !crate::attention::screen_permission_prompt(screen) {
        return None;
    }
    let mut menu = menu(screen)?;
    let above = menu.above_box(BODY_ROWS);
    let (&question, body) = above.split_last()?;
    PERMISSION_ASKS.iter().any(|ask| question.starts_with(ask)).then_some(())?;
    let question = question.to_owned();
    let body = body.iter().map(|s| (*s).to_owned()).collect();
    // A long choice wraps onto the rows under it ("Yes, and always allow access to <path> from
    // this project"); a permission choice has no description rows, so they are its text.
    for i in 0..menu.entries.len() {
        let end = menu.entries.get(i + 1).map_or(menu.footer, |e| e.row);
        let rest: Vec<&str> = menu.lines[menu.entries[i].row + 1..end]
            .iter()
            .map(String::as_str)
            .filter(|text| !text.is_empty() && !rule(text))
            .collect();
        if !rest.is_empty() {
            let text = collapse(&format!("{} {}", menu.entries[i].text, rest.join(" ")));
            menu.entries[i].text = text;
        }
    }
    let tell = menu.entries.iter().position(|e| e.text.contains(TELL_CLAUDE));
    Some(Permission { question, body, tell, menu })
}

/// The permission mode the indicator under the input row names, if any.
pub(in crate::view::chat) fn mode(screen: &Screen) -> Option<&'static str> {
    let row = screen.cursor().row;
    (row + 1..(row + 1 + MODE_ROWS).min(screen.rows())).find_map(|i| {
        let text = clean(&screen.row_text(i)).1.to_lowercase().replace('’', "'");
        MODES.iter().find(|(indicator, _)| text.contains(indicator)).map(|(_, name)| *name)
    })
}

/// Any CLI menu, picker or dialog near the input: its title row, its numbered choices when it has
/// them, else its rows to show read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Panel {
    pub title: String,
    pub menu: Option<Menu>,
    pub rows: Vec<String>,
    /// The footer offers Esc.
    pub esc: bool,
    /// For a question Claude asks: what the screen shows under each choice (its description),
    /// one text per menu entry; empty for any other menu.
    pub notes: Vec<String>,
    /// The menu is a question of Claude's (`AskUserQuestion`), known by its tab row.
    pub question: bool,
}

/// The first row of a question Claude asks: its header with a box (" ☐ Colour"), or a form's row
/// of tabs. Boxes are drawn `[ ]` and `[×]` where Claude Code draws without Unicode.
fn question_tabs(text: &str) -> bool {
    tab_bar(text) || ["☐ ", "☒ ", "[ ] ", "[×] ", "[x] "].iter().any(|b| text.starts_with(b))
}

/// The numbered menu with the rows the arrow keys also stop on: a multi-select question's
/// unnumbered Submit or Next row, after its "Type something" row.
pub(super) fn walk_menu(screen: &Screen) -> Option<Menu> {
    let mut menu = menu(screen)?;
    // The last row with a box is "Type something", whatever was typed on it since.
    let other = menu.entries.iter().rposition(|e| checkbox(&e.text).is_some());
    if let Some(other) = other
        && let Some(entry) = submit_row(screen, &menu, other)
    {
        menu.entries.insert(other + 1, entry);
    }
    Some(menu)
}

/// The review of a form's answers as a menu of its two entries. It has no footer; it is known by
/// its own rows (`review`).
fn review_panel(screen: &Screen) -> Option<Panel> {
    let review = review(screen)?;
    let rows: Vec<(bool, String)> = (0..screen.rows()).map(|row| clean(&screen.row_text(row))).collect();
    let last = rows.iter().rposition(|(_, text)| !text.is_empty())?;
    let entries = (last - 1..=last)
        .map(|row| Entry { row, highlighted: rows[row].0, text: numbered(&rows[row].1).map_or("", |n| n.1).to_owned() })
        .collect();
    let answers: Vec<String> = review.answers.iter().map(|(q, a)| format!("{q} → {a}")).collect();
    let menu = Menu { entries, lines: rows.into_iter().map(|(_, text)| text).collect(), footer: last + 1 };
    Some(Panel {
        title: format!("{REVIEW}: {}", answers.join("; ")),
        menu: Some(menu),
        rows: Vec::new(),
        esc: false,
        notes: Vec::new(),
        question: true,
    })
}

/// The screen may show a permission prompt: recognised, or a menu with a permission prompt's rows
/// (or Yes and No choices over "Esc to cancel").
pub(in crate::view::chat) fn permission_like(screen: &Screen) -> bool {
    if crate::attention::screen_permission_prompt(screen) {
        return true;
    }
    let lines: Vec<String> =
        (0..screen.rows()).map(|row| clean(&screen.row_text(row)).1.to_lowercase().replace('’', "'")).collect();
    // The footer at the bottom of what the CLI draws. The cursor is no guide: with a menu open the
    // CLI parks it below the menu or in its corner (Claude Code 2.1.332).
    let Some(footer) = footer_row(screen, &lines) else {
        return false;
    };
    // The CLI draws a prompt or menu under a rule: rows above the nearest one are the conversation,
    // whose words say nothing about the menu. Without a rule every row near the footer counts.
    let first = footer.saturating_sub(LOOK_ROWS);
    let top = lines[first..footer].iter().rposition(|row| rule(row)).map_or(first, |at| first + at);
    let near = &lines[top..=footer];
    let yes_no = menu(screen).is_some_and(|m| {
        let texts = m.texts();
        let first = texts.first().map(|t| t.to_lowercase()).unwrap_or_default();
        (first.starts_with("yes") || first.starts_with("allow"))
            && texts.iter().any(|t| t.starts_with("No") || t.starts_with("Deny"))
    });
    near.iter().any(|row| PERMISSION_LOOKS.iter().any(|look| row.contains(look)))
        || yes_no && lines[footer].contains("esc to cancel")
}

pub(super) fn panel(screen: &Screen) -> Option<Panel> {
    if permission_like(screen) || screen.modes().alt_screen {
        return None;
    }
    if let Some(review) = review_panel(screen) {
        return Some(review);
    }
    let lines: Vec<String> = (0..screen.rows()).map(|row| clean(&screen.row_text(row)).1).collect();
    // Only a footer at the bottom of what the CLI draws: an old menu left in the output above has
    // the input and its status rows under it. The cursor is no guide (see `permission_like`).
    let footer = footer_row(screen, &lines)?;
    let esc = lines[footer].to_lowercase().contains("esc");
    let menu = walk_menu(screen);
    let mut title = menu.as_ref().and_then(|m| m.above(4).last().map(|s| (*s).to_owned())).unwrap_or_default();
    // A question Claude asks: its text is every row between its tab row and its choices, and
    // each choice has its description under it.
    let (mut notes, mut question) = (Vec::new(), false);
    if let Some(menu) = &menu {
        let above = menu.above_box(FORM_ABOVE);
        if let Some((first, text)) = above.split_first().filter(|(first, _)| question_tabs(first)) {
            question = true;
            if !text.is_empty() {
                title = collapse(&text.join(" "));
            } else if !tab_bar(first) {
                title = (*first).to_owned();
            }
            notes = (0..menu.entries.len())
                .map(|i| {
                    let end = menu.entries.get(i + 1).map_or(menu.footer, |e| e.row);
                    rows_between(menu, menu.entries[i].row + 1, end)
                })
                .collect();
        }
    }
    let top = panel_top(screen, &lines, footer);
    let mut rows: Vec<(usize, &String)> =
        (top..footer).map(|row| (row, &lines[row])).filter(|(_, text)| !text.is_empty() && !rule(text)).collect();
    // The settings dialog's tab row: the selected tab names the panel.
    if menu.is_none()
        && let Some(&(row, text)) = rows.first()
        && settings_tabs(text)
    {
        title = selected(screen, row);
        rows.remove(0);
    }
    let rows = rows.into_iter().map(|(_, text)| text.clone()).collect();
    Some(Panel { title, menu, rows, esc, notes, question })
}

/// Where a panel starts: under the nearest full-width solid rule above its footer. A panel
/// taller than the terminal has none on screen; it then starts at the top of the screen, unless
/// the rows above the footer show conversation (a `❯` prompt, `●` tool or `⎿` result row), in
/// which case only the last rows are taken.
fn panel_top(screen: &Screen, lines: &[String], footer: usize) -> usize {
    let full = |text: &str| solid_rule(text) && text.chars().count() >= screen.cols();
    if let Some(at) = lines[..footer].iter().rposition(|text| full(text)) {
        return at + 1;
    }
    let conversation = (0..footer).any(|row| {
        let text = screen.row_text(row);
        let text = text.trim_start();
        text.starts_with(['❯', '●', '⎿']) || text.starts_with("> ")
    });
    if conversation { footer.saturating_sub(PANEL_ROWS) } else { 0 }
}

/// The tab row of Claude Code's settings dialog ("Settings  Status  Config  Usage  Stats").
fn settings_tabs(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.first() == Some(&"Settings") && words.contains(&"Config") && words.contains(&"Usage")
}

/// The text of a row's inverse cells: the selected tab; empty when none is marked.
fn selected(screen: &Screen, row: usize) -> String {
    let Some(line) = screen.line(row) else { return String::new() };
    let text: String =
        line.cells().iter().filter(|c| c.attrs.contains(mdshared_term::Attrs::INVERSE)).map(|c| c.ch).collect();
    collapse(&text)
}

/// The live input row: a `❯` row between two solid rules, the lowest on the screen. It is found
/// without the cursor, which the CLI parks in its agent list while that has the keys.
pub(super) fn input_row(screen: &Screen) -> Option<usize> {
    let text = |r: usize| screen.row_text(r).trim().to_owned();
    (1..screen.rows().saturating_sub(1))
        .rev()
        .find(|&r| text(r).starts_with(prompt_mark) && solid_rule(&text(r - 1)) && solid_rule(&text(r + 1)))
}

/// The row that opens a tool call's block, `Tool(summary)`, with or without its bullet (it blinks
/// while the call runs): whether it names this shell command (cut with "…" when long).
fn names_command(row: &str, tool: &str, command: &str) -> bool {
    let row = collapse(row.trim().trim_start_matches('●'));
    let Some(inner) = row.strip_prefix(tool).and_then(|r| r.strip_prefix('(')) else { return false };
    let command = collapse(command.lines().next().unwrap_or_default());
    match inner.strip_suffix(')') {
        Some(whole) if whole == command => true,
        _ => {
            inner.trim_end_matches(')').strip_suffix('…').is_some_and(|cut| !cut.is_empty() && command.starts_with(cut))
        }
    }
}

/// The CLI offers Ctrl+B for this running shell command: its "(ctrl+b to run in background)" row
/// ends the call's own block, whose `Tool(command)` row is above it, a few rows over the input box
/// (Claude Code 2.1.332, `bg_hint`).
pub(super) fn background_hint(screen: &Screen, tool: &str, command: &str) -> bool {
    let Some(input) = input_row(screen) else { return false };
    let hint =
        (input.saturating_sub(HINT_ROWS)..input).rev().find(|&r| clean(&screen.row_text(r)).1 == BACKGROUND_HINT);
    let Some(hint) = hint else { return false };
    (hint.saturating_sub(BLOCK_ROWS)..hint).rev().any(|r| names_command(&screen.row_text(r), tool, command))
}

/// The call's block says the command now runs in the background (`bg_moved`).
pub(super) fn backgrounded(screen: &Screen, tool: &str, command: &str) -> bool {
    let Some(input) = input_row(screen) else { return false };
    (input.saturating_sub(MOVED_ROWS)..input).any(|r| {
        names_command(&screen.row_text(r), tool, command)
            && clean(&screen.row_text(r + 1)).1.trim_start_matches('⎿').trim_start().starts_with(BACKGROUNDED)
    })
}

/// Where the CLI's agent list has the keys, by the hint row under the input box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListKeys {
    /// The input has them; its hint row offers "↓ to manage".
    Input,
    /// The list has them, its highlight on the main session ("↑/↓ to select").
    Select,
    /// The highlight is on a running agent ("enter to view · x to stop").
    Stop,
    /// The highlight is on an agent that has ended ("enter to view · x to clear").
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentState {
    /// Its status starts with how long it has run ("5s · ↓ 37.8k tokens").
    Running,
    Stopped,
    /// Anything else (and the main session): Chat does not stop it.
    Other,
}

/// One row of the agent list: `● main` first, then `◯ type  description  status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentRow {
    pub name: String,
    pub description: String,
    pub state: AgentState,
}

/// The agent list the CLI draws under its input box while background agents exist (Claude Code
/// 2.1.332, the `agents_*` fixtures).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentList {
    pub keys: ListKeys,
    /// The main session first.
    pub rows: Vec<AgentRow>,
    pub highlight: Option<usize>,
}

impl AgentList {
    /// The rows without their states, to tell that the list is still the same one.
    pub fn names(&self) -> Vec<(&str, &str)> {
        self.rows.iter().map(|r| (r.name.as_str(), r.description.as_str())).collect()
    }
}

fn agent_row(row: &str) -> Option<(bool, AgentRow)> {
    let row = row.trim();
    let highlighted = row.starts_with(prompt_mark);
    let row = row.trim_start_matches(prompt_mark).trim_start();
    let rest = row.strip_prefix(['●', '◯'])?.strip_prefix(' ')?;
    if rest == "main" {
        let main = AgentRow { name: "main".into(), description: String::new(), state: AgentState::Other };
        return Some((highlighted, main));
    }
    let parts: Vec<&str> = rest.split("  ").map(str::trim).filter(|p| !p.is_empty()).collect();
    let [name, description, status] = parts[..] else { return None };
    let state = if status.starts_with(|c: char| c.is_ascii_digit()) {
        AgentState::Running
    } else if status.starts_with("stopped ") {
        AgentState::Stopped
    } else {
        AgentState::Other
    };
    Some((highlighted, AgentRow { name: name.into(), description: description.into(), state }))
}

/// The agent list under the input box, and where its keys are; `None` for anything else.
pub(super) fn agent_list(screen: &Screen) -> Option<AgentList> {
    let input = input_row(screen)?;
    let hint = screen.row_text(input + 2).trim().to_owned();
    let parts: Vec<String> = hint.split(" · ").map(collapse).collect();
    let has = |part: &str| parts.iter().any(|p| p == part);
    let keys = match () {
        _ if hint == LIST_SELECT => ListKeys::Select,
        _ if parts.first().is_some_and(|p| p == LIST_VIEW) && has(LIST_STOP) => ListKeys::Stop,
        _ if parts.first().is_some_and(|p| p == LIST_VIEW) && has(LIST_CLEAR) => ListKeys::Clear,
        // The input's hint row ends with the effort, after a gap.
        _ if parts.iter().any(|p| p == LIST_MANAGE || p.starts_with(&format!("{LIST_MANAGE} "))) => ListKeys::Input,
        _ => return None,
    };
    // A notice (usage, a tip) may sit between the hint row and the list.
    let first = (input + 3..(input + 6).min(screen.rows()))
        .find(|&r| agent_row(&screen.row_text(r)).is_some_and(|(_, row)| row.name == "main"))?;
    let mut rows = Vec::new();
    let mut highlight = None;
    for r in first..screen.rows() {
        let text = screen.row_text(r);
        if text.trim().is_empty() {
            break;
        }
        let (highlighted, row) = agent_row(&text)?;
        if highlighted {
            if highlight.is_some() {
                return None;
            }
            highlight = Some(rows.len());
        }
        rows.push(row);
    }
    let fits = match keys {
        ListKeys::Input => highlight.is_none(),
        ListKeys::Select => highlight == Some(0),
        ListKeys::Stop => highlight.is_some_and(|h| h > 0 && rows[h].state == AgentState::Running),
        ListKeys::Clear => highlight.is_some_and(|h| h > 0),
    };
    (fits && rows.len() > 1).then_some(AgentList { keys, rows, highlight })
}

/// The rewind list Esc Esc opens at an idle prompt (Claude Code 2.1.332, `rewind_*`): its prompts
/// oldest first, each over a row naming its code changes, then "(current)".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RewindList {
    /// Each prompt as the list shows it (its first line, cut with "…"), then "(current)".
    pub entries: Vec<String>,
    pub highlight: usize,
}

/// The last row reading `title` with `next` under it (blank rows between): the row after `next`.
fn titled(screen: &Screen, title: &str, next: &str) -> Option<usize> {
    let text = |r: usize| clean(&screen.row_text(r)).1;
    let under = |r: usize| (r + 1..screen.rows()).find(|&n| !text(n).is_empty());
    let top = (0..screen.rows()).rev().find(|&r| text(r) == title && under(r).is_some_and(|n| text(n) == next))?;
    under(top).map(|n| n + 1)
}

pub(super) fn rewind_list(screen: &Screen) -> Option<RewindList> {
    let start = titled(screen, REWIND_TITLE, REWIND_LIST)?;
    // Blank rows part the entries; they carry nothing.
    let rows: Vec<(bool, String)> =
        (start..screen.rows()).map(|r| clean(&screen.row_text(r))).filter(|(_, text)| !text.is_empty()).collect();
    let footer = rows.iter().position(|(_, text)| text == REWIND_LIST_FOOTER)?;
    // Nothing under the footer.
    if footer + 1 != rows.len() {
        return None;
    }
    let (current, rest) = rows[..footer].split_last()?;
    if current.1 != REWIND_CURRENT || rest.len() % 2 != 0 {
        return None;
    }
    let mut entries = Vec::new();
    let mut marked = Vec::new();
    for (i, pair) in rest.chunks(2).enumerate() {
        let [(mark, prompt), (summary_marked, summary)] = pair else { return None };
        if *summary_marked || prompt.is_empty() || summary.is_empty() {
            return None;
        }
        if *mark {
            marked.push(i);
        }
        entries.push(prompt.clone());
    }
    if current.0 {
        marked.push(entries.len());
    }
    entries.push(REWIND_CURRENT.to_owned());
    let [highlight] = marked[..] else { return None };
    Some(RewindList { entries, highlight })
}

/// The rewind's choices for one prompt, after Enter on it in the list (`rewind_actions*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RewindActions {
    /// The quoted prompt, its rows joined (without the "(… ago)" row).
    pub quoted: String,
    /// What each choice does to the conversation and the code, and any warning under the choices.
    pub body: Vec<String>,
    pub choices: Vec<String>,
    pub highlight: Option<usize>,
}

pub(super) fn rewind_actions(screen: &Screen) -> Option<RewindActions> {
    let start = titled(screen, REWIND_TITLE, REWIND_ACTIONS)?;
    let raw: Vec<String> =
        (start..screen.rows()).map(|r| screen.row_text(r).trim().to_owned()).filter(|row| !row.is_empty()).collect();
    let quote: Vec<String> =
        raw.iter().take_while(|row| row.starts_with('│')).map(|row| collapse(row.trim_start_matches('│'))).collect();
    let (ago, quote) = quote.split_last()?;
    if !(ago.starts_with('(') && ago.ends_with(" ago)")) || quote.is_empty() {
        return None;
    }
    let mut actions = RewindActions { quoted: collapse(&quote.join(" ")), ..RewindActions::default() };
    for row in &raw[quote.len() + 1..] {
        let (marked, text) = clean(row);
        match numbered(&text) {
            Some((n, choice)) if n == actions.choices.len() + 1 => {
                if marked {
                    if actions.highlight.is_some() {
                        return None;
                    }
                    actions.highlight = Some(actions.choices.len());
                }
                actions.choices.push(choice.to_owned());
            }
            Some(_) => return None,
            None if marked => return None,
            None if !text.is_empty() => actions.body.push(text),
            None => {}
        }
    }
    (!actions.choices.is_empty()).then_some(actions)
}

/// The rewind list shows `prompt` on this row: its first line, whole, or cut with "…" (a prompt of
/// several lines always is).
pub(super) fn rewind_row_is(row: &str, prompt: &str) -> bool {
    let first = collapse(prompt.trim().lines().next().unwrap_or_default());
    let several = prompt.trim().lines().nth(1).is_some();
    match row.strip_suffix('…').map(str::trim_end) {
        Some(cut) => !cut.is_empty() && first.starts_with(cut),
        None => !several && row == first,
    }
}

/// The plan approval menu: its question right above the choices, one of which keeps planning.
pub(super) fn plan(screen: &Screen) -> Option<Menu> {
    let menu = menu(screen)?;
    let asks = menu.above_box(4).last().is_some_and(|q| q.ends_with(PLAN_ASKS));
    (asks && plan_keep(&menu).is_some()).then_some(menu)
}

/// The plan approval's choice that keeps planning.
pub(super) fn plan_keep(menu: &Menu) -> Option<usize> {
    menu.entries.iter().position(|e| PLAN_KEEP.iter().any(|keep| e.text.contains(keep)))
}

/// The block characters a /usage bar is drawn with; its empty part is coloured cells, so a 0%
/// bar shows no characters at all (Claude Code 2.1.332).
const BAR_BLOCKS: [char; 8] = ['█', '▉', '▊', '▋', '▌', '▍', '▎', '▏'];

/// A usage bar row: block characters (none at 0%), then "N% used" (or "left"). The percentage
/// and the words after the bar.
pub(in crate::view::chat) fn usage_bar(row: &str) -> Option<(u8, &str)> {
    let rest = row.trim().trim_start_matches(BAR_BLOCKS).trim_start();
    let (digits, after) = rest.split_once('%')?;
    (!digits.is_empty() && digits.len() <= 3 && digits.chars().all(|c| c.is_ascii_digit())).then_some(())?;
    let percent: u8 = digits.parse().ok().filter(|p| *p <= 100)?;
    matches!(after.trim(), "used" | "left").then_some((percent, rest))
}

/// The row a multi-select question draws, unnumbered, under "Type something": "Next" while another
/// question follows, "Submit" on the last (Claude Code 2.1.332).
pub(super) const SUBMIT_ROWS: [&str; 2] = ["Submit", "Next"];
/// The free-text row of a multi-select question while it holds no text.
pub(super) const OTHER_BOX: &str = "[ ] Type something";
/// The heading of the step after a form's last question.
pub(super) const REVIEW: &str = "Review your answers";
const REVIEW_ENTRIES: [&str; 2] = ["Submit answers", "Cancel"];

/// A multi-select entry's box and the text after it.
pub(super) fn checkbox(text: &str) -> Option<(bool, &str)> {
    // The tick is `√` where Claude Code draws without Unicode.
    let ticked = text.strip_prefix("[✔] ").or_else(|| text.strip_prefix("[√] "));
    ticked.map(|rest| (true, rest)).or_else(|| text.strip_prefix("[ ] ").map(|rest| (false, rest)))
}

/// The row of question tabs over a form: "←  ☐ Size  ☐ Speed  ✔ Submit  →".
pub(super) fn tab_bar(text: &str) -> bool {
    text.starts_with('←') && text.ends_with('→')
}

/// A multi-select question's Submit or Next row, drawn without a number between entry `other`
/// ("Type something") and the next entry: the last non-empty row there that is not a rule. Rows
/// between `other` and it are the typed text wrapping on.
pub(super) fn submit_row(screen: &Screen, menu: &Menu, other: usize) -> Option<Entry> {
    let start = menu.entries.get(other)?.row + 1;
    let end = menu.entries.get(other + 1).map_or(menu.footer, |e| e.row);
    let row = (start..end).rev().find(|&row| {
        let text = &menu.lines[row];
        !text.is_empty() && !rule(text)
    })?;
    let (highlighted, text) = clean(&screen.row_text(row));
    SUBMIT_ROWS.contains(&text.as_str()).then_some(Entry { row, highlighted, text })
}

/// Rows from `start` up to `end` that are neither empty nor rules, joined.
pub(super) fn rows_between(menu: &Menu, start: usize, end: usize) -> String {
    let rows = menu.lines.get(start..end).unwrap_or_default();
    collapse(&rows.iter().filter(|t| !t.is_empty() && !rule(t)).cloned().collect::<Vec<_>>().join(" "))
}

/// The review a form shows after its last question, as the CLI draws it with no footer: the tab
/// row, "Review your answers", a "● question" and "→ answer" pair per question (each may wrap
/// onto the rows under it), then "1. Submit answers" and "2. Cancel" as the last rows drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Review {
    pub answers: Vec<(String, String)>,
    /// 0 on "Submit answers", 1 on "Cancel"; `None` unless exactly one of them is highlighted.
    pub highlight: Option<usize>,
}

pub(super) fn review(screen: &Screen) -> Option<Review> {
    let rows: Vec<(bool, String)> = (0..screen.rows()).map(|row| clean(&screen.row_text(row))).collect();
    let last = rows.iter().rposition(|(_, text)| !text.is_empty())?;
    let first = last.checked_sub(1)?;
    let entry = |at: usize, n: usize| numbered(&rows[at].1) == Some((n + 1, REVIEW_ENTRIES[n]));
    (entry(first, 0) && entry(last, 1)).then_some(())?;
    let anchor = rows[..first].iter().rposition(|(_, text)| text == REVIEW)?;
    let above = rows[..anchor].iter().rev().find(|(_, text)| !text.is_empty())?;
    (tab_bar(&above.1) && !rows[anchor].0).then_some(())?;
    let mut answers: Vec<(String, String)> = Vec::new();
    for (highlighted, text) in &rows[anchor + 1..first] {
        if *highlighted {
            return None;
        }
        if text.is_empty() {
            continue;
        }
        if let Some(question) = text.strip_prefix("● ") {
            answers.push((question.to_owned(), String::new()));
            continue;
        }
        let pair = answers.last_mut()?;
        match text.strip_prefix("→ ") {
            Some(answer) if pair.1.is_empty() => pair.1 = answer.to_owned(),
            Some(_) => return None,
            None if pair.1.is_empty() => pair.0 = format!("{} {text}", pair.0),
            None => pair.1 = format!("{} {text}", pair.1),
        }
    }
    (!answers.is_empty() && answers.iter().all(|(_, answer)| !answer.is_empty())).then_some(())?;
    let highlight = match (rows[first].0, rows[last].0) {
        (true, false) => Some(0),
        (false, true) => Some(1),
        _ => None,
    };
    Some(Review { answers, highlight })
}

#[cfg(test)]
pub(in crate::view::chat) mod recorded;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_prompt_gives_its_question_body_and_choices() {
        let mut screen = Screen::new(80, 16, 0);
        let rows = [
            "────────────────────",
            " Bash command",
            "",
            "   rm -rf build",
            "   Remove the build folder",
            "",
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   2. Yes, and don't ask again for rm commands",
            "   3. No, and tell Claude what to do differently (esc)",
            "",
            " Esc to cancel · Tab to amend",
        ];
        screen.feed(rows.join("\r\n").as_bytes());
        let p = permission(&screen).expect("a permission prompt");
        assert_eq!(p.question, "Do you want to proceed?");
        assert_eq!(p.body, ["Bash command", "rm -rf build", "Remove the build folder"]);
        assert_eq!(p.menu.texts()[1], "Yes, and don't ask again for rm commands");
        assert_eq!((p.tell, p.menu.highlight()), (Some(2), Some(0)));
    }
}
