//! The composer's own conveniences, all local:
//! - `@` completes a path in the session folder. The folder is listed once, on a worker, bounded
//!   (depth, entry count, no hidden or build folders, no links), and the list is kept per tab.
//! - Ctrl+R searches the earlier prompts the transcript reader has already loaded, newest first.
//! - The CLI's grey suggestion for the next prompt, read conservatively from the prompt row's
//!   faint text, becomes a chip; choosing it fills the composer and never sends Tab to the CLI.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use mdshared_term::{Attrs, Cell, Screen};
use mdshared_ui::gfx::Rect;
use mdshared_ui::keys::{KeyName, Mods};

use super::Chat;
use crate::view::model::Model;

/// How deep and how much of the session folder `@` lists.
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 5000;
/// Rows a pop-up offers at most.
pub(super) const MAX_ROWS: usize = 50;
/// Folders never listed: version control, dependencies and build output.
const SKIP: [&str; 8] = ["node_modules", "target", "dist", "build", "out", "__pycache__", "venv", "bin"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Files,
    History,
}

#[derive(Default)]
struct FileIndex {
    root: Option<PathBuf>,
    list: Vec<String>,
    loading: Option<Receiver<Vec<String>>>,
}

#[derive(Default)]
pub(super) struct Popups {
    pub open: Option<Kind>,
    pub selected: usize,
    /// What Ctrl+R searches for; typed while its pop-up is open.
    pub query: String,
    files: FileIndex,
    /// The CLI's suggested next prompt, while its prompt row shows one.
    pub next_prompt: Option<String>,
}

/// The `@word` the caret is in or just after: its byte range, `@` included.
pub(super) fn at_token(text: &str, caret: usize) -> Option<Range<usize>> {
    let caret = caret.min(text.len());
    let start = text[..caret]
        .rfind(|c: char| c.is_whitespace())
        .map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
    let end = caret + text[caret..].find(char::is_whitespace).unwrap_or(text.len() - caret);
    text[start..].starts_with('@').then_some(start..end)
}

/// The completion for `path`: `@path`, quoted when it holds a space, then a space.
pub(super) fn mention(path: &str) -> String {
    if path.contains(' ') { format!("@\"{path}\" ") } else { format!("@{path} ") }
}

/// The entries matching `query`, best first: a name that starts with it, then a path that
/// contains it, shorter paths first within each. Case does not matter.
pub(super) fn rank_files<'a>(list: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim_start_matches(['"', '/']).to_lowercase();
    let mut found: Vec<(u8, usize, &str)> = list
        .iter()
        .filter_map(|path| {
            let lower = path.to_lowercase();
            let name = lower.trim_end_matches('/').rsplit('/').next().unwrap_or(&lower);
            let rank = if q.is_empty() || name.starts_with(&q) {
                0
            } else if lower.contains(&q) {
                1
            } else {
                return None;
            };
            Some((rank, path.len(), path.as_str()))
        })
        .collect();
    found.sort();
    found.into_iter().take(MAX_ROWS).map(|(_, _, p)| p).collect()
}

/// Earlier prompts containing every word of `query`, newest first, each once.
pub(super) fn rank_prompts<'a>(newest_first: impl Iterator<Item = &'a str>, query: &str) -> Vec<&'a str> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut seen = std::collections::HashSet::new();
    newest_first
        .filter(|p| {
            let lower = p.to_lowercase();
            words.iter().all(|w| lower.contains(w.as_str()))
        })
        .filter(|p| seen.insert(p.trim().to_owned()))
        .take(MAX_ROWS)
        .collect()
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIP.iter().any(|s| s.eq_ignore_ascii_case(name))
}

#[cfg(windows)]
fn is_link(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    meta.file_type().is_symlink() || meta.file_attributes() & REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// The folder's files and folders (folders end in `/`), relative with `/`, breadth first, bounded.
pub(super) fn list_folder(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), String::new(), 0)]);
    while let Some((dir, prefix, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<_> = entries.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for entry in names {
            if out.len() >= MAX_ENTRIES {
                return out;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if is_link(&meta) || skipped(&name) {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if meta.is_dir() {
                out.push(format!("{rel}/"));
                if depth + 1 < MAX_DEPTH {
                    queue.push_back((entry.path(), format!("{rel}/"), depth + 1));
                }
            } else if meta.is_file() {
                out.push(rel);
            }
        }
    }
    out
}

impl Popups {
    /// Starts listing `root` unless it is already listed or being listed.
    pub fn want_files(&mut self, root: &Path) {
        let index = &mut self.files;
        if index.root.as_deref() == Some(root) {
            return;
        }
        index.root = Some(root.to_path_buf());
        index.list.clear();
        let (tx, rx) = mpsc::channel();
        let root = root.to_path_buf();
        if std::thread::Builder::new()
            .name("chat-files".into())
            .spawn(move || {
                let _ = tx.send(list_folder(&root));
            })
            .is_ok()
        {
            index.loading = Some(rx);
        }
    }

    /// Takes a finished listing; true when there is a new one to show.
    pub fn collect(&mut self) -> bool {
        let Some(rx) = &self.files.loading else { return false };
        match rx.try_recv() {
            Ok(list) => {
                self.files.list = list;
                self.files.loading = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.files.loading = None;
                true
            }
        }
    }

    pub fn loading_files(&self) -> bool {
        self.files.loading.is_some()
    }

    pub fn files(&self) -> &[String] {
        &self.files.list
    }

    pub fn close(&mut self) {
        self.open = None;
        self.selected = 0;
        self.query.clear();
    }

    pub fn step(&mut self, count: usize, down: bool) {
        if count == 0 {
            self.selected = 0;
        } else if down {
            self.selected = (self.selected + 1) % count;
        } else {
            self.selected = (self.selected + count - 1) % count;
        }
    }

    /// Reads the prompt row; true when the suggestion changed.
    pub fn read_next_prompt(&mut self, screen: &Screen) -> bool {
        let next = next_prompt(screen);
        let changed = self.next_prompt != next;
        self.next_prompt = next;
        changed
    }
}

fn faint(cell: &Cell) -> bool {
    !cell.attrs.contains(Attrs::INVERSE) && cell.attrs.contains(Attrs::DIM)
}

/// The CLI's grey next-prompt suggestion on its live, empty prompt row, if one is shown. Only an
/// unbroken faint run right where typing would start counts, and the CLI's fixed placeholder
/// hints (`Try "…"`, `<filepath>`, an ellipsis) never do.
pub(super) fn next_prompt(screen: &Screen) -> Option<String> {
    if !super::submission::screen::clean_prompt(screen) {
        return None;
    }
    let cursor = screen.cursor();
    let cells = screen.line(cursor.row)?.cells();
    let glyph = cells.iter().position(|c| super::submission::screen::prompt_mark(c.ch))?;
    let start = glyph + 2;
    // Claude Code 2.1.332 draws a no-break space after the mark.
    if cursor.col != start || cells.get(glyph + 1).is_none_or(|c| !matches!(c.ch, ' ' | '\u{a0}')) {
        return None;
    }
    let rest = cells.get(start..)?;
    let run = rest.iter().take_while(|c| faint(c) || c.is_spacer()).count();
    let text: String = rest[..run].iter().filter(|c| !c.is_spacer()).map(|c| c.ch).collect();
    let after = &rest[run..];
    // Only blanks or the box's border may follow the suggestion.
    if !after.iter().all(|c| c.ch == ' ' || c.is_spacer() || matches!(c.ch, '│' | '┃' | '║' | '┆' | '┊')) {
        return None;
    }
    let text = text.trim();
    let placeholder = text.starts_with("Try \"")
        || text.contains('<')
        || text.ends_with('…')
        || text.ends_with("...")
        || text.to_ascii_lowercase().contains("for shortcuts");
    let fits = (3..=200).contains(&text.chars().count()) && text.chars().any(char::is_alphanumeric);
    (fits && !placeholder && rest.first().is_some_and(|c| faint(c) && c.ch != ' ')).then(|| text.to_owned())
}

/// What a pop-up shows this frame, placed in the slash picker's envelope.
pub(super) struct Plan {
    pub kind: Kind,
    pub bounds: Rect,
    pub items: Vec<String>,
    pub selected: usize,
    pub first: usize,
    pub visible: usize,
}

impl Plan {
    pub fn pointer_behind(&self, mouse: (f32, f32)) -> (f32, f32) {
        if self.bounds.contains(mouse.0, mouse.1) { (-1.0, -1.0) } else { mouse }
    }
}

pub(super) const HEADER: f32 = 22.0;
pub(super) const ROW: f32 = 25.0;

impl Chat {
    /// The rows of the open pop-up, or `None` when it has nothing to stand on (the caret left
    /// the `@word`, or the slash picker is open).
    pub(super) fn popup_items(&self, model: &Model) -> Option<(Kind, Vec<String>)> {
        if self.commands.open || self.locked() {
            return None;
        }
        let kind = self.popups.open?;
        let items = match kind {
            Kind::Files => {
                let token = at_token(&self.editor.text, self.editor.caret)?;
                let query = self.editor.text[token].trim_start_matches('@').to_owned();
                rank_files(self.popups.files(), &query).into_iter().map(str::to_owned).collect()
            }
            Kind::History => {
                let prompts = model.items.values().rev().filter_map(super::recall::prompt_text);
                rank_prompts(prompts, &self.popups.query).into_iter().map(str::to_owned).collect()
            }
        };
        Some((kind, items))
    }

    pub(super) fn popup_plan(&self, model: &Model) -> Option<Plan> {
        let (kind, items) = self.popup_items(model)?;
        let r = self.layout.suggestions;
        let max_rows = ((r.h - HEADER - 4.0) / ROW).max(0.0) as usize;
        if max_rows == 0 {
            return None;
        }
        let visible = items.len().max(1).min(max_rows);
        let selected = self.popups.selected.min(items.len().saturating_sub(1));
        let first = selected.saturating_sub(visible - 1);
        let h = HEADER + ROW * visible as f32 + 4.0;
        Some(Plan { kind, bounds: Rect::new(r.x, r.bottom() - h, r.w, h), items, selected, first, visible })
    }

    /// The draft changed: an `@word` at the caret opens file completion, leaving it closes it.
    pub(super) fn popup_edited(&mut self, cwd: &Path) {
        if self.popups.open == Some(Kind::History) {
            return;
        }
        if !self.commands.open && at_token(&self.editor.text, self.editor.caret).is_some() {
            if self.popups.open != Some(Kind::Files) {
                self.popups.selected = 0;
            }
            self.popups.open = Some(Kind::Files);
            self.popups.want_files(cwd);
        } else {
            self.popups.close();
        }
    }

    /// Puts row `index` of the open pop-up into the draft.
    pub(super) fn popup_choose(&mut self, model: &Model, index: usize) -> bool {
        let Some((kind, items)) = self.popup_items(model) else { return false };
        let Some(item) = items.get(index) else { return false };
        let (text, caret) = match kind {
            Kind::Files => {
                let Some(token) = at_token(&self.editor.text, self.editor.caret) else { return false };
                let mut insert = mention(item);
                if self.editor.text[token.end..].starts_with(char::is_whitespace) {
                    insert.pop(); // The draft already has the space after it.
                }
                let caret = token.start + insert.len();
                let mut text = self.editor.text.clone();
                text.replace_range(token, &insert);
                (text, caret)
            }
            Kind::History => (item.clone(), item.len()),
        };
        self.recall.end();
        self.editor.set(text);
        self.editor.move_to(caret, false);
        self.popups.close();
        true
    }

    /// The CLI's suggestion goes into an empty draft; nothing is sent to the terminal.
    pub(super) fn fill_next_prompt(&mut self) -> bool {
        let Some(text) = self.popups.next_prompt.clone() else { return false };
        if self.locked() || !self.editor.text.is_empty() {
            return false;
        }
        self.recall.end();
        let at = text.len();
        self.editor.set(text);
        self.editor.move_to(at, false);
        true
    }

    /// Ctrl+R, and the keys of an open pop-up. True when the key was used here.
    pub(super) fn popup_key(&mut self, model: &Model, key: &KeyName, mods: Mods, typed: Option<&str>) -> bool {
        if self.locked() {
            return false;
        }
        if matches!(key, KeyName::Char('r' | 'R')) && mods.ctrl && !mods.alt && !mods.shift {
            if self.popups.open == Some(Kind::History) {
                let count = self.popup_items(model).map_or(0, |(_, items)| items.len());
                self.popups.step(count, true);
            } else {
                self.popups.close();
                self.commands.open = false;
                self.popups.open = Some(Kind::History);
            }
            return true;
        }
        let Some((kind, items)) = self.popup_items(model) else {
            return *key == KeyName::Tab && mods.none() && self.fill_next_prompt();
        };
        let plain = mods.none();
        match key {
            KeyName::Escape => self.popups.close(),
            KeyName::Up | KeyName::Down if plain => self.popups.step(items.len(), *key == KeyName::Down),
            KeyName::Enter | KeyName::Tab if plain && !items.is_empty() => {
                let selected = self.popups.selected.min(items.len() - 1);
                self.popup_choose(model, selected);
            }
            KeyName::Enter | KeyName::Tab if plain => {
                // Nothing to choose: Enter on a file name goes on to send, Tab only closes.
                self.popups.close();
                return *key == KeyName::Tab || kind == Kind::History;
            }
            KeyName::Backspace if kind == Kind::History => {
                self.popups.query.pop();
                self.popups.selected = 0;
            }
            _ if kind == Kind::History => {
                if let Some(text) = typed.filter(|t| !t.chars().any(char::is_control) && !mods.ctrl && !mods.alt) {
                    self.popups.query.push_str(text);
                    self.popups.selected = 0;
                }
            }
            _ => return false,
        }
        true
    }

    /// Typed text while Ctrl+R is open goes to its search.
    pub(super) fn popup_text(&mut self, text: &str) -> bool {
        if self.popups.open != Some(Kind::History) || self.locked() {
            return false;
        }
        self.popups.query.push_str(text);
        self.popups.selected = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_at_token_is_the_word_around_the_caret_and_completes_with_quotes_for_spaces() {
        let text = "see @src/ma and more";
        assert_eq!(at_token(text, 11), Some(4..11));
        assert_eq!(at_token(text, 6), Some(4..11));
        assert_eq!(at_token(text, 2), None);
        assert_eq!(at_token("mail a@b", 8), None, "an @ inside a word is not a mention");
        assert_eq!(at_token("é @x", "é @x".len()), Some(3..5));
        assert_eq!(mention("src/main.rs"), "@src/main.rs ");
        assert_eq!(mention("My Docs/a.md"), "@\"My Docs/a.md\" ");
    }

    #[test]
    fn files_rank_name_prefixes_first_and_prompts_need_every_word() {
        let list: Vec<String> =
            ["docs/", "docs/main-notes.md", "src/main.rs", "src/domain.rs", "README.md"].map(String::from).into();
        assert_eq!(rank_files(&list, "main"), ["src/main.rs", "docs/main-notes.md", "src/domain.rs"]);
        assert_eq!(rank_files(&list, "READ"), ["README.md"]);
        assert_eq!(rank_files(&list, "").len(), 5);
        let prompts = ["fix the push", "Fix the PUSH", "push tags", "something else"];
        assert_eq!(rank_prompts(prompts.into_iter(), "push fix"), ["fix the push", "Fix the PUSH"]);
        assert_eq!(rank_prompts(["a", " a", "b"].into_iter(), ""), ["a", "b"], "the same prompt once");
    }

    #[test]
    fn the_listing_skips_hidden_and_build_folders_and_stays_bounded() {
        let scratch = crate::testing::Scratch::new("cs-popups");
        let root = scratch.to_path_buf();
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["src/deep", ".git", "node_modules/pkg", "Target"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["src/deep/a.rs", ".git/HEAD", "node_modules/pkg/i.js", "Target/x", "top.md"] {
            std::fs::write(root.join(file), b"x").unwrap();
        }
        let list = list_folder(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(list, ["src/", "top.md", "src/deep/", "src/deep/a.rs"]);
    }

    fn row(content: &str) -> Screen {
        let mut screen = Screen::new(60, 6, 0);
        screen.feed(format!("❯ {content}\x1b[1;3H").as_bytes());
        screen
    }

    #[test]
    fn only_a_faint_suggestion_at_the_empty_prompt_becomes_a_chip() {
        assert_eq!(next_prompt(&row("\x1b[2mrun the tests again\x1b[22m")).as_deref(), Some("run the tests again"));
        assert_eq!(next_prompt(&row("typed text")), None, "real input is not a suggestion");
        assert_eq!(next_prompt(&row("\x1b[2mTry \"fix lint errors\"\x1b[22m")), None);
        assert_eq!(next_prompt(&row("\x1b[2mrefactor <filepath>\x1b[22m")), None);
        assert_eq!(next_prompt(&row("\x1b[2mType a message…\x1b[22m")), None);
        assert_eq!(next_prompt(&row("")), None);
        let mut moved = row("\x1b[2mrun the tests again\x1b[22m");
        moved.feed(b"\x1b[1;6H");
        assert_eq!(next_prompt(&moved), None, "the caret must sit where typing starts");
        let mut menu = row("\x1b[2mrun the tests again\x1b[22m");
        menu.feed("\r\n\r\n↑↓ navigate · Enter to select · Esc to cancel\x1b[1;3H".as_bytes());
        assert_eq!(next_prompt(&menu), None);
    }

    #[test]
    fn at_completion_ctrl_r_search_and_the_suggestion_chip_only_edit_the_draft() {
        use claude_switch_core::transcript::{Item, ItemId, ItemKind};
        let mut model = Model::new(crate::view::model::Source::File(Default::default()), Default::default(), || {});
        for (offset, text) in [(10, "fix the push"), (20, "run the tests"), (30, "push tags")] {
            model.items.insert(
                ItemId { offset, block: 0 },
                Item {
                    timestamp: None,
                    kind: ItemKind::Prompt { text: text.into(), pictures: Vec::new(), synthetic: false, queued: false },
                },
            );
        }
        let none = Mods::default();
        let mut chat = Chat::default();
        chat.popups.files.root = Some(PathBuf::from("unused"));
        chat.popups.files.list = ["src/main.rs", "docs/My notes.md"].map(String::from).into();
        chat.editor.insert("look at @ma please");
        chat.editor.move_to("look at @ma".len(), false);
        chat.popup_edited(Path::new("unused"));
        assert_eq!(chat.popups.open, Some(Kind::Files));
        assert!(chat.popup_key(&model, &KeyName::Tab, none, None));
        assert_eq!(chat.editor.text, "look at @src/main.rs please");
        assert_eq!(chat.popups.open, None);

        chat.editor.set("@notes".into());
        chat.editor.move_to(6, false);
        chat.popup_edited(Path::new("unused"));
        assert!(chat.popup_choose(&model, 0));
        assert_eq!(chat.editor.text, "@\"docs/My notes.md\" ");

        // Ctrl+R: newest first, typed text narrows, Enter fills without sending.
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        assert!(chat.popup_key(&model, &KeyName::Char('r'), ctrl, None));
        assert_eq!(chat.popup_items(&model).unwrap().1, ["push tags", "run the tests", "fix the push"]);
        assert!(chat.popup_text("push"));
        assert!(chat.popup_key(&model, &KeyName::Char('r'), ctrl, None), "Ctrl+R again steps to the next match");
        assert!(chat.popup_key(&model, &KeyName::Enter, none, None));
        assert_eq!(chat.editor.text, "fix the push");

        // The chip fills only an empty draft; Tab elsewhere stays the editor's.
        chat.popups.next_prompt = Some("run the tests again".into());
        assert!(!chat.fill_next_prompt());
        chat.editor.set(String::new());
        assert!(chat.popup_key(&model, &KeyName::Tab, none, None));
        assert_eq!(chat.editor.text, "run the tests again");
        assert!(!chat.popup_key(&model, &KeyName::Tab, none, None));
    }
}
