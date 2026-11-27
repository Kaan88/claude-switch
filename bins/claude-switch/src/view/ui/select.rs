//! Selecting and copying the conversation, in the Chat view and the Session panel alike.
//!
//! A selection runs from an anchor to a focus, each a row and a byte in that row's shown text,
//! so it can cross messages. Rows are named by their stable [`Id`], so a selection survives the
//! relayout that appends a message. Copy joins the selected part of each row with a blank line.
//! The right-click menu offers Copy and Select all and, over a message, Copy message (its text as
//! shown) and Copy as Markdown (its source); over a typed prompt in a Chat tab, Rewind to here…
//! (`chat::question::rewind`). Code blocks carry their own Copy button (`paint`).

use std::ops::Range;

use mdshared_ui::gfx::Rect;
use mdshared_ui::host::Cx;
use mdshared_ui::menu::{Item, Menu};
use mdshared_ui::platform;

use super::rows::{Id, Row};
use super::{Page, Ui};
use crate::state::{AppState, MenuKind, Msg};

/// One end of a selection: a row and a byte offset in its body's text.
pub(super) type End = (Id, usize);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Selection {
    pub anchor: End,
    pub focus: End,
}

/// The menu's items, by id.
pub(super) const COPY: u32 = 0;
pub(super) const SELECT_ALL: u32 = 1;
pub(super) const COPY_MESSAGE: u32 = 2;
pub(super) const COPY_MARKDOWN: u32 = 3;
/// Over a prompt in a Chat tab: rewind the session to before it, through the CLI.
pub(super) const REWIND: u32 = 4;

fn text_of(row: &Row) -> Option<&str> {
    row.body.as_ref().map(|b| b.text())
}

/// The nearest character boundary at or before `byte`, within the text.
fn floor_char(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The nearest character boundary at or after `byte`, within the text.
fn ceil_char(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

impl Selection {
    pub fn within(id: Id, a: usize, b: usize) -> Self {
        Self { anchor: (id.clone(), a), focus: (id, b) }
    }

    /// Everything with text, first row to last.
    pub fn all(rows: &[Row]) -> Option<Self> {
        let first = rows.iter().find(|r| r.body.is_some())?;
        let last = rows.iter().rev().find(|r| r.body.is_some())?;
        Some(Self { anchor: (first.id.clone(), 0), focus: (last.id.clone(), text_of(last).map_or(0, str::len)) })
    }

    /// Both ends as (row index, byte), in reading order. `None` when a row has gone.
    pub fn ends(&self, rows: &[Row]) -> Option<((usize, usize), (usize, usize))> {
        let at = |(id, byte): &End| Some((rows.iter().position(|r| r.id == *id)?, *byte));
        let (a, b) = (at(&self.anchor)?, at(&self.focus)?);
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The selected bytes of row `index`, given the ordered [`Self::ends`].
    pub fn range_in(ends: ((usize, usize), (usize, usize)), index: usize, text: &str) -> Option<Range<usize>> {
        let ((first, from), (last, to)) = ends;
        if index < first || index > last {
            return None;
        }
        // A stored byte can fall inside a character once the row's text has changed under the
        // selection; widen to whole characters rather than leave the row out of a copy.
        let start = if index == first { floor_char(text, from) } else { 0 };
        let end = if index == last { ceil_char(text, to) } else { text.len() };
        Some(start..end.max(start))
    }

    /// The selected text, the rows' parts joined by a blank line.
    pub fn text(&self, rows: &[Row]) -> String {
        let Some(ends) = self.ends(rows) else { return String::new() };
        let ((first, _), (last, _)) = ends;
        let mut parts = Vec::new();
        for (index, row) in rows.iter().enumerate().take(last + 1).skip(first) {
            if let Some(text) = text_of(row)
                && let Some(range) = Self::range_in(ends, index, text)
                && !range.is_empty()
            {
                parts.push(&text[range]);
            }
        }
        parts.join("\n\n")
    }
}

impl Ui {
    pub(super) fn selected_text(&self) -> Option<String> {
        let text = self.selection.as_ref()?.text(&self.rows);
        (!text.is_empty()).then_some(text)
    }

    pub(super) fn select_all(&mut self) -> bool {
        self.selection = Selection::all(&self.rows);
        self.selection.is_some()
    }

    /// The nearest text position to `(x, y)` in any row, for a drag that crosses rows: above or
    /// left of a row's text is its start, below or right of its last line is its end.
    pub(super) fn point_anywhere(&self, x: f32, y: f32) -> Option<End> {
        let yy = y - self.body.y + self.offset;
        let mut chosen = None;
        for row in self.rows.iter().filter(|r| r.body.is_some()) {
            let top = row.top + row.geo.ty;
            if chosen.is_some() && yy < top {
                break;
            }
            chosen = Some(row);
            if yy < top + row.body.as_ref().unwrap().height() {
                break;
            }
        }
        let row = chosen?;
        let body = row.body.as_ref()?;
        let top = row.top + row.geo.ty;
        let dy = yy - top;
        let at = if dy < 0.0 {
            0
        } else if dy >= body.height() {
            body.text().len()
        } else {
            let dx = (x - self.body.x - row.geo.tx).clamp(0.0, row.geo.tw);
            body.point(dx, dy).0
        };
        Some((row.id.clone(), at))
    }

    /// The message under the pointer, if any: the target of Copy message.
    fn message_at(&self, x: f32, y: f32) -> Option<&Row> {
        if self.page != Page::Conversation || !self.body.contains(x, y) {
            return None;
        }
        let yy = y - self.body.y + self.offset;
        let row = self.rows.iter().find(|r| yy >= r.top && yy < r.top + r.h)?;
        let frame = Rect::new(self.body.x + row.geo.x, 0.0, row.geo.w, 1.0);
        (matches!(row.id, Id::Message(_)) && row.body.is_some() && x >= frame.x && x < frame.right()).then_some(row)
    }

    /// A right press at `(x, y)`: the menu's items, or `None` outside the conversation.
    pub(super) fn context_menu(&mut self, x: f32, y: f32) -> Option<Vec<Item<u32>>> {
        if self.overlay_open() || !self.body.contains(x, y) {
            return None;
        }
        self.focused = true;
        self.dragging = false;
        self.start = None;
        self.pressed = None;
        self.menu_row = self.message_at(x, y).map(|r| r.id.clone());
        let mut items = vec![
            Item::new(COPY, "Copy").enabled(self.selected_text().is_some()),
            Item::new(SELECT_ALL, "Select all").enabled(self.rows.iter().any(|r| r.body.is_some())),
        ];
        if self.menu_row.is_some() {
            items.push(Item::new(COPY_MESSAGE, "Copy message").sep());
            items.push(Item::new(COPY_MARKDOWN, "Copy as Markdown"));
        }
        Some(items)
    }

    /// The transcript item of the message the menu was opened over.
    pub(super) fn menu_item(&self) -> Option<claude_switch_core::transcript::ItemId> {
        match self.menu_row {
            Some(super::rows::Id::Message(id)) => Some(id),
            _ => None,
        }
    }

    /// The text an item of the menu copies, or `None` for Select all.
    pub(super) fn menu_text(&mut self, id: u32) -> Option<String> {
        let message = || {
            let id = self.menu_row.as_ref()?;
            self.rows.iter().find(|r| r.id == *id)?.body.as_ref()
        };
        match id {
            COPY => self.selected_text(),
            COPY_MESSAGE => message().map(|b| b.text().to_owned()),
            COPY_MARKDOWN => message().map(|b| b.raw().to_owned()),
            _ => {
                self.select_all();
                None
            }
        }
    }
}

impl AppState {
    /// A right press over the shown conversation opens its menu. True when it did.
    pub fn conversation_menu(&mut self, x: f32, y: f32) -> bool {
        let width = self.session_view.area.w;
        let over = self.picture_overlay() || !self.session_view.pane_area.contains(x, y);
        let Some(tab) = self.tabs.active else { return false };
        if over {
            return false;
        }
        let Some(pane) = self.shown_pane(width) else { return false };
        let Some(mut items) = pane.ui.context_menu(x, y) else { return false };
        // Rewind goes through the tab's own CLI, from a prompt the user typed.
        let prompt = pane.ui.menu_item().and_then(|id| pane.model.items.get(&id)).is_some_and(|item| {
            matches!(&item.kind, claude_switch_core::transcript::ItemKind::Prompt { synthetic: false, .. })
        });
        if prompt && pane.chat.enabled {
            items.push(Item::new(REWIND, "Rewind to here…").sep());
        }
        self.session_view.text_focus = true;
        self.mouse = (x, y);
        self.menu = Some((MenuKind::Conversation(tab), Menu::new(items, x, y, false)));
        true
    }

    pub fn conversation_menu_chosen(&mut self, cx: &mut Cx<'_, Msg>, tab: u64, id: u32) {
        if self.tabs.active != Some(tab) {
            return;
        }
        let width = self.session_view.area.w;
        let Some(pane) = self.shown_pane(width) else { return };
        if id == REWIND {
            let Some(item) = pane.ui.menu_item() else { return };
            // As Send: the newest screen before the first Esc, then the tick that carries the rest.
            self.term_woke(cx);
            self.rewind_from_menu(tab, item, cx.now());
            return self.arm_chat_tick(cx);
        }
        if let Some(text) = pane.ui.menu_text(id) {
            platform::set_clipboard_text(&text);
        }
    }
}

impl AppState {
    /// Rewind to the prompt at `item` from the conversation menu of `tab`: only while that tab is
    /// shown with Chat on (the menu may have been opened before Chat was switched off). The caller
    /// has read the newest screen. True when the rewind started or said why it could not.
    pub(crate) fn rewind_from_menu(
        &mut self,
        tab: u64,
        item: claude_switch_core::transcript::ItemId,
        now: std::time::Instant,
    ) -> bool {
        if self.tabs.active != Some(tab) {
            return false;
        }
        let chat_on = |t: &&mut crate::tabs::Tab| t.pane.as_ref().is_some_and(|p| p.chat.enabled);
        let Some(t) = self.tabs.get_mut(tab).filter(chat_on) else { return false };
        crate::view::chat::question::rewind::start(t, now, item);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_covers_the_tail_of_the_first_row_whole_middle_rows_and_the_head_of_the_last() {
        let ends = ((1, 3), (3, 2));
        assert_eq!(Selection::range_in(ends, 0, "skip"), None);
        assert_eq!(Selection::range_in(ends, 1, "first row"), Some(3..9));
        assert_eq!(Selection::range_in(ends, 2, "middle"), Some(0..6));
        assert_eq!(Selection::range_in(ends, 3, "last"), Some(0..2));
        assert_eq!(Selection::range_in(ends, 4, "after"), None);
        // One row, and an end past a shortened row's text.
        assert_eq!(Selection::range_in(((2, 1), (2, 4)), 2, "abcdef"), Some(1..4));
        assert_eq!(Selection::range_in(((2, 1), (2, 99)), 2, "abc"), Some(1..3));
    }

    #[test]
    fn an_end_inside_a_character_widens_to_it_instead_of_dropping_the_row() {
        // The row's text changed under the selection: byte 1 is now inside "é".
        assert_eq!(Selection::range_in(((0, 1), (0, 2)), 0, "é"), Some(0..2));
        let ends = ((0, 1), (2, 1));
        let (first, middle, last) = ("éa", "middle", "üb");
        assert_eq!(Selection::range_in(ends, 0, first).map(|r| &first[r]), Some("éa"));
        assert_eq!(Selection::range_in(ends, 1, middle).map(|r| &middle[r]), Some("middle"));
        assert_eq!(Selection::range_in(ends, 2, last).map(|r| &last[r]), Some("ü"));
        // Ends in the middle of a four-byte character on one row.
        let wide = "a😺b";
        assert_eq!(Selection::range_in(((0, 2), (0, 3)), 0, wide).map(|r| &wide[r]), Some("😺"));
    }

    #[test]
    fn a_drag_selects_across_messages_and_the_menu_copies_a_message_or_its_markdown() {
        use crate::view::model::{Model, Source};
        use mdshared_text::bitmap::Bitmap;
        use mdshared_ui::gfx::{Gfx, Painter};
        use mdshared_ui::keys::{KeyName, Mods};
        use std::time::{Duration, Instant};

        let demo = crate::demo::Transcript::new().unwrap();
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(Source::File(demo.path().into()), demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(Duration::from_secs(3)).expect("transcript worker wake");
        assert!(model.collect());
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(760, 2600);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let mut ui = Ui::default();
        let view = Rect::new(0.0, 0.0, 740.0, 2600.0);
        ui.draw(&mut p, &mdshared_ui::theme::DARK, view, &mut model, (-1.0, -1.0), Instant::now(), false);

        let texts: Vec<usize> = (0..ui.rows.len()).filter(|&i| ui.rows[i].body.is_some()).collect();
        let at = |ui: &Ui, i: usize, dy: f32| {
            let row = &ui.rows[i];
            (ui.body.x + row.geo.tx + 2.0, ui.body.y + row.top - ui.offset + row.geo.ty + dy)
        };
        let (first, second) = (texts[0], texts[1]);
        let (x0, y0) = at(&ui, first, 4.0);
        ui.press(x0, y0, 1, Mods::default(), &mut model);
        let h = ui.rows[second].body.as_ref().unwrap().height();
        let (x1, y1) = at(&ui, second, h + 1.0);
        assert!(ui.moved(x1 + 400.0, y1));
        ui.release(x1, y1, Mods::default(), &mut model);
        let copied = ui.selected_text().expect("a selection");
        let whole = ui.rows[second].body.as_ref().unwrap().text().to_owned();
        assert!(copied.ends_with(&whole), "{copied:?}");
        assert!(copied.contains("\n\n"));

        assert!(ui.key(&KeyName::Char('a'), Mods { ctrl: true, ..Mods::default() }));
        let all = ui.selected_text().unwrap();
        assert!(all.len() > copied.len());
        assert!(all.matches("\n\n").count() + 1 >= texts.len());

        // Over a message: Copy, Select all, Copy message, Copy as Markdown.
        let answer = ui
            .rows
            .iter()
            .position(|r| matches!(r.id, Id::Message(_)) && r.body.as_ref().is_some_and(|b| b.raw() != b.text()))
            .expect("an answer with Markdown");
        let (x, y) = at(&ui, answer, 4.0);
        let items = ui.context_menu(x, y).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Copy", "Select all", "Copy message", "Copy as Markdown"]);
        let body = ui.rows[answer].body.as_ref().unwrap();
        let (text, raw) = (body.text().to_owned(), body.raw().to_owned());
        assert_eq!(ui.menu_text(COPY_MESSAGE), Some(text));
        assert_eq!(ui.menu_text(COPY_MARKDOWN), Some(raw));
        assert_eq!(ui.menu_text(COPY), Some(all));
        assert!(ui.context_menu(view.right() + 10.0, 5.0).is_none());

        // Code blocks carry their own Copy button with the code alone.
        let code: Vec<_> = ui
            .hits
            .iter()
            .filter_map(|(_, a)| if let super::super::Action::CopyCode(c) = a { Some(c.clone()) } else { None })
            .collect();
        assert!(code.iter().any(|c| c.starts_with("fn caption") && !c.contains("```")), "{code:?}");
    }
}
