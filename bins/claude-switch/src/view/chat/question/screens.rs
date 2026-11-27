//! Every piece of Claude Code screen text the Chat cards rely on, in one place. None of it is a
//! protocol: each pattern is a conservative reading of the classic renderer, to be confirmed in a
//! live session. A screen that does not match exactly is not answered.
use mdshared_term::Screen;

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
/// The hint the CLI shows under a running shell command that can move to the background, lower case.
const BACKGROUND_HINT: &str = "ctrl+b to run in background";
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

/// The footer of whatever the CLI draws at the bottom: the last non-empty row, or the row right
/// above it when the footer wrapped (it then ends with its `·` separator). `rows` are the screen's
/// cleaned rows; the last row is found on the screen itself, where an empty input (`❯`) counts.
fn footer_row(screen: &Screen, rows: &[String]) -> Option<usize> {
    let last = (0..screen.rows()).rposition(|row| !screen.row_text(row).trim().is_empty())?;
    let footer = if menu_footer(&rows[last]) {
        last
    } else {
        let prev = last.checked_sub(1)?;
        (menu_footer(&rows[prev]) && rows[prev].ends_with('·')).then_some(prev)?
    };
    (!input_box_above(screen, footer)).then_some(footer)
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
    first.starts_with('❯') && numbered(first.trim_start_matches('❯').trim()).is_none()
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
    let lines: Vec<String> = (0..screen.rows()).map(|row| clean(&screen.row_text(row)).1).collect();
    // Only a footer at the bottom of what the CLI draws: an old menu left in the output above has
    // the input and its status rows under it. The cursor is no guide (see `permission_like`).
    let footer = footer_row(screen, &lines)?;
    let esc = lines[footer].to_lowercase().contains("esc");
    let menu = menu(screen);
    let mut title = menu.as_ref().and_then(|m| m.above(4).last().map(|s| (*s).to_owned())).unwrap_or_default();
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
    Some(Panel { title, menu, rows, esc })
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
        text.starts_with(['❯', '●', '⎿'])
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

/// The CLI offers Ctrl+B for the running command: its hint is by the input.
pub(super) fn background_hint(screen: &Screen) -> bool {
    let cursor = screen.cursor().row;
    (cursor.saturating_sub(6)..(cursor + 6).min(screen.rows()))
        .any(|row| screen.row_text(row).to_lowercase().contains(BACKGROUND_HINT))
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
    text.strip_prefix("[✔] ").map(|rest| (true, rest)).or_else(|| text.strip_prefix("[ ] ").map(|rest| (false, rest)))
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
