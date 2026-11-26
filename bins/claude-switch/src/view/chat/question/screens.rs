//! Every piece of Claude Code screen text the Chat cards rely on, in one place. None of it is a
//! protocol: each pattern is a conservative reading of the classic renderer, to be confirmed in a
//! live session. A screen that does not match exactly is not answered.
use mdshared_term::Screen;

/// Footer hints under a CLI menu (the last non-empty row), lower case.
pub(super) const MENU_FOOTERS: [&str; 2] = ["esc to cancel", "enter to select"];
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
/// The plan approval question shown for `ExitPlanMode`.
pub(super) const PLAN_ASKS: &str = "Would you like to proceed?";
/// The plan approval choice that declines (keeps planning).
pub(super) const PLAN_KEEP: &str = "keep planning";
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

/// A box's top or bottom edge, or a rule: never part of a prompt's text.
fn rule(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| matches!(c, '─' | '━' | '═' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘'))
}

fn numbered(text: &str) -> Option<(usize, &str)> {
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

    /// The footer row, lower case.
    pub fn footer_text(&self) -> String {
        self.lines.get(self.footer).map(|s| s.to_lowercase()).unwrap_or_default()
    }
}

pub(super) fn menu(screen: &Screen) -> Option<Menu> {
    let rows: Vec<_> = (0..screen.rows()).map(|row| clean(&screen.row_text(row))).collect();
    let footer = rows.iter().rposition(|(_, text)| !text.is_empty())?;
    let hint = rows[footer].1.to_lowercase();
    if !MENU_FOOTERS.iter().any(|f| hint.contains(f)) {
        return None;
    }
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
    let menu = menu(screen)?;
    let above = menu.above(BODY_ROWS);
    let (&question, body) = above.split_last()?;
    PERMISSION_ASKS.iter().any(|ask| question.starts_with(ask)).then_some(())?;
    let tell = menu.entries.iter().position(|e| e.text.contains(TELL_CLAUDE));
    let body = body.iter().map(|s| (*s).to_owned()).collect();
    Some(Permission { question: question.to_owned(), body, tell, menu })
}

/// The permission mode the indicator under the input row names, if any.
pub(in crate::view::chat) fn mode(screen: &Screen) -> Option<&'static str> {
    let row = screen.cursor().row;
    (row + 1..(row + 1 + MODE_ROWS).min(screen.rows())).find_map(|i| {
        let text = clean(&screen.row_text(i)).1.to_lowercase().replace('’', "'");
        MODES.iter().find(|(indicator, _)| text.contains(indicator)).map(|(_, name)| *name)
    })
}

/// The plan approval menu: its question right above the choices, one of which keeps planning.
pub(super) fn plan(screen: &Screen) -> Option<Menu> {
    let menu = menu(screen)?;
    let asks = menu.above(4).last().is_some_and(|q| *q == PLAN_ASKS);
    (asks && menu.entries.iter().any(|e| e.text.contains(PLAN_KEEP))).then_some(menu)
}

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
