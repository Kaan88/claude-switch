use mdshared_ui::cursor::Cursor;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::super::links;
use super::scroll::Press;
use super::select::Selection;
use super::*;

impl Ui {
    fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a.clone())
    }

    fn text_at(&self, x: f32, y: f32) -> Option<(&Row, usize, Option<&str>)> {
        if self.overlay_open() || !self.shown_body().contains(x, y) {
            return None;
        }
        let yy = y - self.body.y + self.offset;
        let i = self.rows.partition_point(|r| r.top + r.h < yy);
        let row = self.rows.get(i)?;
        let layout = row.body.as_ref()?;
        let dx = x - self.body.x - row.geo.tx;
        let dy = yy - row.top - row.geo.ty;
        if dx < 0.0 || dx >= row.geo.tw || dy < 0.0 || dy >= layout.height() {
            return None;
        }
        let (at, link) = layout.point(dx, dy);
        Some((row, at, link))
    }

    fn scrollbar(&self, surface: Surface) -> Option<Scrollbar> {
        match surface {
            Surface::Conversation => self.scrollbar,
            Surface::Markdown => self.markdown.as_ref()?.scroll,
        }
    }

    fn scroll_surface(&self, x: f32, y: f32) -> Option<Surface> {
        if self.overlay_open() {
            return None;
        }
        [Surface::Markdown, Surface::Conversation]
            .into_iter()
            .find(|&surface| self.scrollbar(surface).is_some_and(|bar| bar.track.contains(x, y)))
    }

    fn scroll_to(&mut self, surface: Surface, offset: f32) -> bool {
        match surface {
            Surface::Conversation => {
                let offset = offset.clamp(0.0, self.max);
                let changed = std::mem::replace(&mut self.offset, offset) != offset;
                let followed = self.follow.active();
                if self.page == Page::Conversation {
                    self.follow.scrolled(offset, self.max);
                }
                changed || followed != self.follow.active()
            }
            Surface::Markdown => self.markdown.as_mut().is_some_and(|pane| pane.set_offset(offset)),
        }
    }

    fn scroll_press(&mut self, x: f32, y: f32) -> bool {
        let Some(surface) = self.scroll_surface(x, y) else { return false };
        let offset = match surface {
            Surface::Conversation => self.offset,
            Surface::Markdown => self.markdown.as_ref().unwrap().offset(),
        };
        match self.scrollbar(surface).expect("visible track").press(y, offset) {
            Press::Page(to) => {
                self.scroll_to(surface, to);
            }
            Press::Thumb(fraction) => self.scroll_drag = Some(Drag { surface, fraction }),
        }
        true
    }

    fn drag_to(&mut self, drag: Drag, y: f32) -> bool {
        let Some(bar) = self.scrollbar(drag.surface) else { return false };
        self.scroll_to(drag.surface, bar.drag(y, drag.fraction))
    }

    pub fn scroll_dragging(&self) -> bool {
        self.scroll_drag.is_some()
    }

    pub fn pointer_left(&mut self) {
        self.pointed = None;
        self.hot = None;
        self.scroll_hot = None;
        self.scroll_drag = None;
        self.pressed = None;
        self.start = None;
        self.dragging = false;
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.dragging = false;
        self.start = None;
        self.pressed = None;
        self.scroll_drag = None;
    }

    pub fn press(&mut self, x: f32, y: f32, clicks: u32, mods: Mods, _: &mut Model) -> bool {
        self.focused = true;
        self.start = None;
        self.dragging = false;
        self.snap = None;
        self.scroll_drag = None;
        if self.divider_press(x, y) {
            self.pressed = None;
            return false;
        }
        if self.scroll_press(x, y) {
            self.pressed = None;
            return false;
        }
        self.pressed = self.hit(x, y);
        if self.overlay_open() {
            if self.pressed.is_none() && self.overlay.as_ref().is_some_and(|o| !o.bounds.contains(x, y)) {
                self.overlay = None;
            }
            return false;
        }
        if self.pressed == Some(Action::Attach) {
            self.selection = None;
            return true;
        }
        if matches!(self.pressed, Some(Action::CopyCode(_))) {
            return false;
        }
        if let Some((row, at, link)) = self.text_at(x, y) {
            let id = row.id.clone();
            let row_id = row.id.clone();
            let link = link.map(str::to_owned);
            let text = row.body.as_ref().unwrap().text();
            let (a, b) = if clicks >= 3 {
                (0, text.len())
            } else if clicks == 2 {
                word(text, at)
            } else {
                (at, at)
            };
            self.start = link.filter(|_| clicks == 1 && !mods.shift).map(|s| (x, y, s));
            self.selection = Some(match self.selection.take() {
                // Shift+click extends from the old anchor, in this message or another.
                Some(old) if mods.shift && clicks == 1 => Selection { anchor: old.anchor, focus: (id, at) },
                _ => Selection::within(id, a, b),
            });
            // A drag after a double or triple click goes on by whole words or whole rows.
            self.snap = (clicks >= 2).then(|| (Selection::within(row_id, a, b), clicks >= 3));
            self.dragging = true;
        } else if mods.shift
            && self.selection.is_some()
            && let Some(end) = self.point_anywhere(x, y)
        {
            self.selection.as_mut().unwrap().focus = end;
            self.dragging = true;
        } else {
            self.selection = None;
        }
        false
    }

    pub fn moved(&mut self, x: f32, y: f32) -> bool {
        if let Some(changed) = self.divider_move(x, y) {
            return changed;
        }
        if let Some(drag) = self.scroll_drag {
            return self.drag_to(drag, y);
        }
        let track = self.scroll_surface(x, y);
        let hot = self.hit(x, y);
        let pointed = self.pointed_message(x, y);
        let mut changed = self.hot != hot || self.scroll_hot != track || self.pointed != pointed;
        self.pointed = pointed;
        self.hot = hot;
        self.scroll_hot = track;
        if self.dragging {
            if self.start.as_ref().is_some_and(|(sx, sy, _)| (x - sx).abs() + (y - sy).abs() > 4.0) {
                self.start = None;
            }
            // Dragging past the top or bottom scrolls the conversation along with the selection.
            let beyond = if y < self.body.y {
                y - self.body.y
            } else if y > self.body.bottom() {
                y - self.body.bottom()
            } else {
                0.0
            };
            if beyond != 0.0 && self.selection.is_some() {
                changed |= self.scroll_to(Surface::Conversation, self.offset + beyond * 0.5);
            }
            if let Some(next) = self.point_anywhere(x, y)
                && let Some(next) = self.snapped(next)
                && self.selection.as_ref().is_some_and(|selection| *selection != next)
            {
                self.selection = Some(next);
                changed = true;
                self.start = None;
                self.pressed = None;
            }
        }
        changed
    }

    /// The message with buttons under it that `(x, y)` is over, with the room for its buttons.
    fn pointed_message(&self, x: f32, y: f32) -> Option<ItemId> {
        if self.overlay_open() || !self.shown_body().contains(x, y) {
            return None;
        }
        let yy = y - self.body.y + self.offset;
        let row = self.rows.iter().find(|r| yy >= r.top && yy < r.top + r.h + super::rows::ACTIONS_ROOM)?;
        match (row.actions, &row.id) {
            (Some(_), Id::Message(id)) => Some(*id),
            _ => None,
        }
    }

    /// The selection a drag to `end` makes: from the anchor to `end`, or after a double or triple
    /// click from what it took to the whole word or row at `end`, whichever way the drag goes.
    fn snapped(&self, end: select::End) -> Option<Selection> {
        let anchor = self.selection.as_ref()?.anchor.clone();
        let Some((first, rows)) = &self.snap else { return Some(Selection { anchor, focus: end }) };
        let (id, at) = end;
        let index = |id: &Id| self.rows.iter().position(|row| row.id == *id);
        let text = self.rows.get(index(&id)?)?.body.as_ref()?.text();
        let (a, b) = if *rows { (0, text.len()) } else { word(text, at) };
        let before = (index(&id)?, a) < (index(&first.anchor.0)?, first.anchor.1);
        Some(if before {
            Selection { anchor: first.focus.clone(), focus: (id, a) }
        } else {
            Selection { anchor: first.anchor.clone(), focus: (id, b) }
        })
    }

    pub fn owns_pointer(&self, x: f32, y: f32) -> bool {
        self.dragging
            || self.scroll_dragging()
            || self.panel.split.dragging()
            || self.overlay_open()
            || self.area.contains(x, y)
    }

    pub fn pointer_cursor(&self, x: f32, y: f32) -> Option<Cursor> {
        if self.divider_hot(x, y) {
            return Some(Cursor::ResizeEw);
        }
        if self.scroll_dragging() || self.scroll_surface(x, y).is_some() {
            return Some(Cursor::Default);
        }
        if self.dragging {
            return Some(Cursor::Text);
        }
        let text = self.text_at(x, y);
        if self.hit(x, y).is_some() || text.as_ref().is_some_and(|(_, _, link)| link.is_some()) {
            Some(Cursor::Hand)
        } else if text.is_some() {
            Some(Cursor::Text)
        } else if self.area.contains(x, y) || self.overlay_open() {
            Some(Cursor::Default)
        } else {
            None
        }
    }

    pub(in crate::view) fn release(&mut self, x: f32, y: f32, mods: Mods, m: &mut Model) -> Release {
        if self.divider_release() {
            return Release::Handled;
        }
        if let Some(drag) = self.scroll_drag.take() {
            self.drag_to(drag, y);
            return Release::Handled;
        }
        let drag = std::mem::take(&mut self.dragging);
        let pressed = self.pressed.take();
        if let Some((sx, sy, target)) = self.start.take()
            && (sx - x).abs() + (sy - y).abs() < 5.0
            && let Some((row, _, Some(link))) = self.text_at(x, y)
            && link == target
        {
            if links::url(&target) {
                platform::open_url(&target);
            } else if let Some(file) = row.body.as_ref().and_then(|text| text.files.get(&target)).cloned() {
                if mods.ctrl || mods.shift || !file.markdown() {
                    file.open(mods.ctrl || mods.shift);
                } else {
                    self.open_markdown(file, m);
                }
            }
            return Release::Handled;
        }
        let Some(action) = pressed.filter(|a| Some(a) == self.hit(x, y).as_ref()) else {
            return if drag || self.overlay_open() || self.area.contains(x, y) {
                Release::Handled
            } else {
                Release::Miss
            };
        };
        match action {
            Action::Page(page) => {
                if m.agent.is_some() {
                    m.select_agent(None);
                }
                self.agent_origin = None;
                self.show(page);
            }
            Action::Attach => {}
            Action::Earlier => {
                self.offset = 0.0;
                self.follow.scrolled(0.0, self.max.max(2.0));
                m.earlier();
            }
            Action::Latest => {
                self.tool_jump = None;
                self.follow.latest();
            }
            Action::Stop(turn) => return Release::Stop(turn),
            Action::Shell(id) => return Release::Shell(id),
            Action::ShellList => return Release::ShellList,
            Action::Output(output) => self.open_output(output.to, m),
            Action::Group(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
                self.cache = None;
            }
            Action::Tool(id) => {
                if !self.tool_expanded.remove(&id) {
                    self.tool_expanded.insert(id);
                }
                self.cache = None;
            }
            Action::Agent(id) => self.open_agent(&id, m),
            Action::Back => {
                m.select_agent(None);
                let page = self.agent_origin.take().unwrap_or(Page::Agents);
                self.show(page);
            }
            Action::Picture(id, index) => self.overlay = Some(Overlay::open(m, id, index)),
            Action::File(file) => self.open_markdown(file, m),
            Action::Editor => {
                if let Some(pane) = &mut self.markdown {
                    pane.editor();
                }
            }
            Action::Reading => self.next_reading(),
            Action::CloseMarkdown => self.close_markdown(),
            Action::Open => {
                if let Some(o) = &self.overlay {
                    m.open_picture(o.id, o.index);
                }
            }
            Action::Close => self.overlay = None,
            Action::Previous | Action::Next => {
                if let Some(o) = &mut self.overlay {
                    o.step(action == Action::Next);
                }
            }
            Action::CopyCode(code) => {
                if platform::set_clipboard_text(&code) {
                    self.copied = Some(code);
                }
            }
            Action::CopyMessage(id) => {
                // As the right-click menu's Copy message: the text as shown.
                let text = self.rows.iter().find(|r| r.id == Id::Message(id)).and_then(|r| r.body.as_ref());
                if let Some(text) = text.map(|b| crate::view::preview::copyable(b.text()).into_owned())
                    && platform::set_clipboard_text(&text)
                {
                    self.copied = Some(text);
                }
            }
            Action::Rewind(id) => return Release::Rewind(id),
            Action::Branch(id) => return Release::Branch(id),
        }
        Release::Handled
    }

    pub(in crate::view) fn open_markdown(&mut self, file: NamedFile, m: &mut Model) {
        if self.detail_side().is_some() {
            self.show(Page::Conversation);
        }
        self.selection = None;
        self.focused = true;
        m.request_markdown(file.clone());
        self.markdown = Some(markdown::Markdown::new(file));
    }

    /// Shows Markdown the transcript holds (an MCP task's result) in the viewer.
    pub(in crate::view) fn open_text(&mut self, title: String, subtitle: String, markdown: String) {
        if self.detail_side().is_some() {
            self.show(Page::Conversation);
        }
        self.selection = None;
        self.focused = true;
        self.markdown = Some(markdown::Markdown::text(title, subtitle, markdown));
    }

    pub(in crate::view) fn open_output(&mut self, output: Destination, m: &mut Model) {
        if self.detail_side().is_some() && !matches!(&output, Destination::Web(..)) {
            self.show(Page::Conversation);
        }
        match output {
            Destination::Web(url, _) if links::url(&url) => {
                platform::open_url(&url);
            }
            Destination::File(file) => self.open_markdown(file, m),
            Destination::Picture(id, index) => self.overlay = Some(Overlay::open(m, id, index)),
            _ => {}
        }
    }

    pub(super) fn close_markdown(&mut self) {
        self.markdown = None;
        self.scroll_drag = None;
    }

    pub(super) fn show(&mut self, page: Page) {
        self.close_markdown();
        self.page = page;
        self.selection = None;
        self.tool_jump = None;
        self.rows.clear();
        self.cache = None;
        self.offset = 0.0;
        self.follow.latest();
    }

    /// The Agents page, for its pictures.
    #[cfg(test)]
    pub(in crate::view) fn show_agents(&mut self) {
        self.show(Page::Agents);
    }

    pub(in crate::view) fn open_agent(&mut self, id: &str, model: &Model) {
        if model.agent.as_deref() != Some(id) {
            self.agent_origin = Some(self.page);
            model.select_agent(Some(id));
            self.show(Page::Conversation);
        }
    }

    /// Scrolls what is under the pointer: true when it moved.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        if self.overlay_open() {
            return false;
        }
        if let Some(pane) = &self.markdown {
            if pane.bounds.contains(x, y) {
                return self.scroll_to(Surface::Markdown, pane.offset() - dy);
            }
            if self.body.w == 0.0 {
                return false;
            }
        }
        self.scroll_to(Surface::Conversation, self.offset - dy)
    }

    pub fn key(&mut self, name: &KeyName, mods: Mods) -> bool {
        if let Some(overlay) = &mut self.overlay {
            // Every key stays with the viewer while it is open; none reaches the terminal.
            match name {
                KeyName::Escape => self.overlay = None,
                KeyName::Left | KeyName::Right if mods.none() => {
                    overlay.step(*name == KeyName::Right);
                }
                _ => {}
            }
            return true;
        }
        if !self.focused {
            return false;
        }
        match name {
            KeyName::Escape if self.markdown.is_some() => self.close_markdown(),
            KeyName::Char('c' | 'C') if mods.ctrl && !mods.alt && !mods.shift => {
                let Some(text) = self.selected_text() else { return false };
                platform::set_clipboard_text(&text);
            }
            KeyName::Char('a' | 'A') if mods.ctrl && !mods.alt && !mods.shift => {
                if !self.select_all() {
                    return false;
                }
            }
            KeyName::Escape if self.selection.is_some() => {
                self.selection = None;
                self.focused = false;
            }
            _ => return false,
        }
        true
    }
}

fn word(text: &str, at: usize) -> (usize, usize) {
    let class = |c: char| {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    let mut left = at.min(text.len());
    if left == text.len() {
        left = text.char_indices().next_back().map_or(0, |(i, _)| i);
    }
    let Some(c) = text[left..].chars().next() else { return (0, 0) };
    let k = class(c);
    let right = left + text[left..].chars().take_while(|&x| class(x) == k).map(char::len_utf8).sum::<usize>();
    left -= text[..left].chars().rev().take_while(|&x| class(x) == k).map(char::len_utf8).sum::<usize>();
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_text::bitmap::Bitmap;
    use mdshared_ui::gfx::Gfx;

    #[test]
    fn the_session_views_stop_requires_the_same_turn_at_press_and_release_and_chat_has_none_in_the_pane() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let mut model = Model::new(crate::view::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(740, 220);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        for (chat, width) in [(false, 350.0), (true, 740.0)] {
            let mut ui = Ui::default();
            ui.set_chat(chat);
            let draw = |ui: &mut Ui, p: &mut Painter, model: &mut Model| {
                ui.draw(
                    p,
                    &mdshared_ui::theme::DARK,
                    Rect::new(0.0, 0.0, width, 220.0),
                    model,
                    (-1.0, -1.0),
                    Instant::now(),
                    false,
                );
            };
            let one = Turn(1, 2);
            let two = Turn(1, 3);
            ui.set_working(Some(Footer {
                label: "✻ Shimmying… (53m 37s · ↓ 27.1k tokens)".into(), stop: Some(one)
            }));
            draw(&mut ui, &mut p, &mut model);
            let stop = ui.hits.iter().find_map(|(r, action)| (*action == Action::Stop(one)).then_some(*r));
            if chat {
                // Chat's Stop is on the composer's Send button (`composer::tests`), not in the pane.
                assert_eq!(stop, None);
                assert_eq!(ui.footer_height(width, true), 0.0, "Chat has no strip under the conversation");
                continue;
            }
            let r = stop.unwrap();
            let (x, y) = (r.cx(), r.cy());
            ui.press(x, y, 1, Mods::default(), &mut model);
            assert_eq!(ui.release(x, y, Mods::default(), &mut model), Release::Stop(one));
            ui.press(x, y, 1, Mods::default(), &mut model);
            ui.set_working(Some(Footer { label: "Working… (0s)".into(), stop: Some(two) }));
            draw(&mut ui, &mut p, &mut model);
            assert_eq!(ui.release(x, y, Mods::default(), &mut model), Release::Handled);
            ui.set_working(None);
            draw(&mut ui, &mut p, &mut model);
            assert!(ui.hits.iter().all(|(_, action)| !matches!(action, Action::Stop(_))));
        }
    }

    #[test]
    fn chat_reading_widths_render_centered_at_narrow_and_wide_sizes() {
        let demo = crate::demo::Transcript::new().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let mut model = Model::new(source, demo.cwd().into(), || {});
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(1500, 600);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        for width in [600.0, 1400.0] {
            for reading in Reading::ALL {
                let mut ui = Ui::default();
                ui.set_chat(true);
                ui.set_chat_reading(reading);
                ui.draw(
                    &mut p,
                    &mdshared_ui::theme::DARK,
                    Rect::new(0.0, 0.0, width, 600.0),
                    &mut model,
                    (-1.0, -1.0),
                    Instant::now(),
                    false,
                );
                // Chat keeps room beside the column for the composer's box, which is wider.
                let available = width - 2.0 * super::chat_pad(width) - GUTTER;
                let expected = match reading {
                    Reading::Narrow => available.min(560.0),
                    Reading::Medium => available.min(720.0),
                    Reading::Wide => available.min(900.0),
                    Reading::Full => available,
                };
                assert_eq!(ui.body.w, expected);
                assert!((ui.body.cx() - (width - GUTTER) / 2.0).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn the_working_line_ends_the_conversation_at_the_reading_column_and_the_error_line_starts_there() {
        let demo = crate::demo::Transcript::new().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let mut model = Model::new(source, demo.cwd().into(), || {});
        model.error = Some("The transcript could not be read.".into());
        let pal = mdshared_ui::theme::DARK;
        let mut gfx = Gfx::new();
        // The leftmost and rightmost drawn pixel in rows `y0..y1`.
        let ink = |bitmap: &Bitmap, y0: f32, y1: f32, width: f32| {
            let mut span = (f32::MAX, 0.0_f32);
            for y in y0 as u32..y1 as u32 {
                for x in 0..width as u32 {
                    let i = (y * bitmap.width + x) as usize * 4;
                    if [bitmap.pixels[i + 2], bitmap.pixels[i + 1], bitmap.pixels[i]] != pal.page {
                        span = (span.0.min(x as f32), span.1.max(x as f32));
                    }
                }
            }
            span
        };
        for width in [520.0, 900.0, 1400.0] {
            for reading in Reading::ALL {
                let mut bitmap = Bitmap::new(1400, 400);
                let mut ui = Ui::default();
                ui.set_chat(true);
                ui.set_chat_reading(reading);
                ui.set_working(Some(Footer {
                    label: "✶ Sprouting… (40s · ↓ 3.2k tokens)".into(),
                    stop: Some(Turn(1, 2)),
                }));
                let area = Rect::new(0.0, 0.0, width, 400.0);
                let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
                ui.draw(&mut p, &pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
                let body = ui.body;
                assert_eq!(ui.column(), (body.x, body.w), "{width} {reading:?}");
                // The working line is the conversation's last row: right after the last message,
                // inside the conversation, with no strip under it.
                assert_eq!(ui.footer_height(width, true), 0.0);
                assert!(ui.follow.active() && ui.offset == ui.max);
                let top = body.y + ui.rows_end() - ui.offset;
                assert!(top + crate::view::working::CHAT_LINE <= body.bottom() + 0.5, "{width} {reading:?}");
                let (left, right) = ink(&bitmap, top + 4.0, top + crate::view::working::CHAT_LINE, width);
                assert!(
                    left >= body.x && left <= body.x + 8.0,
                    "{width} {reading:?}: the working line starts at {left}, the text at {}",
                    body.x
                );
                assert!(
                    right > left + 100.0 && right <= body.right() + 1.0,
                    "{width} {reading:?}: the working line ends at {right}, the column at {}",
                    body.right()
                );
                assert!(ui.hits.iter().all(|(_, a)| !matches!(a, Action::Stop(_))), "Stop is the composer's");
                let (left, _) = ink(&bitmap, 9.0, 24.0, width);
                assert!(left >= body.x && left <= body.x + 4.0, "{width} {reading:?}: the error at {left}");
            }
        }
    }

    #[test]
    fn conversation_scrollbar_pages_drags_resumes_follow_and_releases_outside() {
        let demo = crate::demo::Transcript::new().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let mut model = Model::new(source, demo.cwd().into(), || {});
        for chat in [false, true] {
            let bar = Scrollbar::new(Rect::new(290.0, 100.0, GUTTER, 200.0), 800.0);
            let mut ui = Ui {
                chat,
                scrollbar: bar,
                max: 800.0,
                offset: 800.0,
                area: Rect::new(0.0, 0.0, 330.0, 400.0),
                ..Default::default()
            };
            assert!(!ui.press(295.0, 140.0, 1, Mods::default(), &mut model));
            assert_eq!(ui.offset, 600.0);
            assert!(!ui.follow.active());
            assert!(!ui.scroll_dragging());
            // The thumb is now at y=220..260. Take it by its middle, move back up
            // and release outside the pane below the track.
            ui.press(295.0, 240.0, 1, Mods::default(), &mut model);
            assert!(ui.scroll_dragging());
            assert!(ui.owns_pointer(800.0, 500.0));
            assert!(matches!(ui.pointer_cursor(800.0, 500.0), Some(Cursor::Default)));
            assert!(ui.moved(100.0, 120.0));
            assert_eq!(ui.offset, 0.0);
            assert_eq!(ui.release(800.0, 500.0, Mods::default(), &mut model), Release::Handled);
            assert_eq!(ui.offset, 800.0);
            assert!(ui.follow.active());
            assert!(!ui.scroll_dragging());
            assert!(!ui.owns_pointer(800.0, 500.0));
            assert!(ui.pointer_cursor(800.0, 500.0).is_none());
        }
    }

    #[test]
    fn file_chips_open_the_worker_loaded_viewer_in_both_conversation_surfaces() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        let wait = || events.recv_timeout(std::time::Duration::from_secs(3)).expect("transcript worker wake");
        wait();
        assert!(model.collect());
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(920, 720);
        let pal = &mdshared_ui::theme::DARK;
        let ts = Instant::now();
        for (chat, width) in [(true, 900.0), (false, 360.0)] {
            let view = Rect::new(0.0, 0.0, width, 680.0);
            let mut ui = Ui::default();
            ui.set_chat(chat);
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            ui.draw(&mut p, pal, view, &mut model, (0.0, 0.0), ts, false);
            let chip = ui.hits.iter().find_map(|(r, a)| matches!(a, Action::File(_)).then_some(*r)).unwrap();
            let (x, y) = (chip.cx(), chip.cy());
            ui.press(x, y, 1, Mods::default(), &mut model);
            assert_eq!(ui.release(x, y, Mods::default(), &mut model), Release::Handled);
            // Visible pictures can wake the same worker while the named file is
            // still queued. Drive collection until the requested viewer is drawn.
            for _ in 0..8 {
                wait();
                model.collect();
                ui.draw(&mut p, pal, view, &mut model, (x, y), ts, false);
                if ui.markdown.as_ref().is_some_and(markdown::Markdown::rendered) {
                    break;
                }
            }
            assert!(ui.hits.iter().any(|(_, a)| *a == Action::Editor));
            assert!(ui.markdown.as_ref().unwrap().rendered());
            assert_eq!(ui.body.w > 0.0, chat);
            assert!(ui.key(&KeyName::Escape, Mods::default()));
            assert!(ui.markdown.is_none());
        }
    }

    #[test]
    fn jumping_to_an_edit_opens_it_and_jumping_to_a_step_opens_its_run() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut m = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        m.poll();
        events.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(m.collect());
        let id = m
            .items
            .iter()
            .find_map(|(&id, item)| match &item.kind {
                claude_switch_core::transcript::ItemKind::Tool { id: tool, .. } if tool == "demo-index" => Some(id),
                _ => None,
            })
            .unwrap();
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(700, 240);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let area = Rect::new(0.0, 0.0, 700.0, 240.0);
        let pal = &mdshared_ui::theme::DARK;
        let now = Instant::now();
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.draw(&mut p, pal, area, &mut m, (0.0, 0.0), now, false);
        // An edit is a row of its own: there before the jump, closed.
        let row = ui.rows.iter().find(|row| row.id == Id::Tool(id)).unwrap();
        assert!(row.style == Style::Edit && !row.open);
        assert!(ui.jump_to_tool(&m, id));
        ui.draw(&mut p, pal, area, &mut m, (0.0, 0.0), now, false);
        let row = ui.rows.iter().find(|row| row.id == Id::Tool(id)).unwrap();
        assert!(row.open);
        assert_eq!(ui.tool_jump, Some((id, false)));
        assert!(!ui.follow.active());
        assert!(ui.hits.iter().any(|(_, a)| *a == Action::Tool(id)));
        let offset = ui.offset;
        assert!(!ui.jump_to_tool(&m, ItemId { offset: 0, block: 1000 }));
        assert_eq!(ui.offset, offset);

        // A call inside a run is not shown until its run opens: the jump opens both.
        let demo = crate::demo::Transcript::new_showcase(None).unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut m = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        m.poll();
        events.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(m.collect());
        let id = m
            .items
            .iter()
            .find_map(|(&id, item)| match &item.kind {
                claude_switch_core::transcript::ItemKind::Tool { id: tool, .. } if tool == "grep" => Some(id),
                _ => None,
            })
            .unwrap();
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.draw(&mut p, pal, area, &mut m, (0.0, 0.0), now, false);
        assert!(ui.rows.iter().any(|row| row.style == Style::Group && row.title.starts_with("Ran a command, read 3")));
        assert!(!ui.rows.iter().any(|row| row.id == Id::Tool(id)));
        assert!(ui.jump_to_tool(&m, id));
        ui.draw(&mut p, pal, area, &mut m, (0.0, 0.0), now, false);
        let row = ui.rows.iter().find(|row| row.id == Id::Tool(id)).unwrap();
        assert!(row.open && row.step == Some((false, true)));
        assert_eq!(ui.tool_jump, Some((id, false)));
    }

    #[test]
    fn a_drag_after_a_double_or_triple_click_goes_by_whole_words_or_rows() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(model.collect());
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(700, 900);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        for chat in [true, false] {
            let mut ui = Ui::default();
            ui.set_chat(chat);
            let width = if chat { 700.0 } else { 380.0 };
            ui.draw(
                &mut p,
                &mdshared_ui::theme::DARK,
                Rect::new(0.0, 0.0, width, 900.0),
                &mut model,
                (-1.0, -1.0),
                Instant::now(),
                false,
            );
            ui.follow.scrolled(0.0, ui.max.max(1.0));
            ui.offset = 0.0;
            // Two points on the text of two different rows, the first above.
            let x = ui.body.x + 24.0;
            let on_text = |ui: &Ui, from: f32| {
                (from as u32..ui.body.bottom() as u32)
                    .map(|y| y as f32)
                    .find_map(|y| ui.text_at(x, y).map(|(row, at, _)| (y, row.id.clone(), at)))
                    .unwrap()
            };
            let (y1, row1, _) = on_text(&ui, ui.body.y);
            let below = ui.rows.iter().find(|r| r.id == row1).map(|r| r.top + r.h - ui.offset + ui.body.y).unwrap();
            let (y2, row2, _) = on_text(&ui, below + 1.0);
            assert!(row1 != row2);
            let text = |ui: &Ui, id: &Id| {
                ui.rows.iter().find(|r| r.id == *id).unwrap().body.as_ref().unwrap().text().to_owned()
            };
            let (text1, text2) = (text(&ui, &row1), text(&ui, &row2));
            let plain = Mods::default();

            // A double click, then a drag down: from the word's start to the end of a word.
            ui.press(x, y1, 2, plain, &mut model);
            let first = ui.selection.clone().unwrap();
            assert!(ui.dragging && first.anchor.1 < first.focus.1);
            ui.moved(x, y1);
            assert!(ui.selection.as_ref() == Some(&first), "within the word: the word");
            ui.moved(x, y2);
            let s = ui.selection.clone().unwrap();
            assert!(s.anchor == first.anchor && s.focus.0 == row2);
            assert_eq!(word(&text2, s.focus.1.saturating_sub(1)).1, s.focus.1, "ends a word");
            ui.release(x, y2, plain, &mut model);

            // From a lower word up: from that word's end to the start of a word above.
            ui.press(x, y2, 2, plain, &mut model);
            let first = ui.selection.clone().unwrap();
            ui.moved(x, y1);
            let s = ui.selection.clone().unwrap();
            assert!(s.anchor == first.focus && s.focus.0 == row1);
            assert_eq!(word(&text1, s.focus.1).0, s.focus.1, "starts a word");
            ui.release(x, y1, plain, &mut model);

            // A triple click takes the row, and a drag whole rows.
            ui.press(x, y2, 3, plain, &mut model);
            ui.moved(x, y1);
            let s = ui.selection.clone().unwrap();
            assert!(s.anchor == (row2.clone(), text2.len()) && s.focus == (row1.clone(), 0));
            ui.release(x, y1, plain, &mut model);

            // A single click drags by characters from where it was pressed, as before.
            ui.press(x, y1, 1, plain, &mut model);
            let at = ui.selection.clone().unwrap().anchor;
            ui.moved(x, y2);
            let s = ui.selection.clone().unwrap();
            assert!(s.anchor == at && s.focus.0 == row2);
            ui.release(x, y2, plain, &mut model);
        }
    }

    #[test]
    fn the_buttons_under_a_message_show_on_pointing_and_rewind_and_branch_go_to_the_app() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(model.collect());
        // The demo's newest turn as one that has ended: Branch is offered at its end.
        use claude_switch_core::transcript::{ChatPhase, ItemKind};
        let last =
            model.items.iter().rev().find(|(_, i)| matches!(i.kind, ItemKind::Assistant { .. })).unwrap().0.offset;
        (model.chat.latest, model.chat.pending) = (Some((last, ChatPhase::EndTurn)), None);
        // An ended turn's calls have their results.
        for item in model.items.values_mut() {
            if let ItemKind::Tool { state, .. } = &mut item.kind {
                *state = claude_switch_core::transcript::ToolState::Ok;
            }
        }
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(800, 2400);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let area = Rect::new(0.0, 0.0, 800.0, 2400.0);
        let pal = &mdshared_ui::theme::DARK;
        for chat in [true, false] {
            let mut ui = Ui::default();
            ui.set_chat(chat);
            ui.draw(&mut p, pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
            let rewinds = |ui: &Ui| ui.hits.iter().filter(|(_, a)| matches!(a, Action::Rewind(_))).count();
            let branches = |ui: &Ui| ui.hits.iter().filter(|(_, a)| matches!(a, Action::Branch(_))).count();
            if !chat {
                assert_eq!((rewinds(&ui), branches(&ui)), (0, 0), "not in the Session view");
                continue;
            }
            // Away from every message: only the newest answer's buttons.
            assert_eq!((rewinds(&ui), branches(&ui)), (0, 1));
            let (x, y) = ui.prompt_point().unwrap();
            ui.moved(x, y);
            ui.draw(&mut p, pal, area, &mut model, (x, y), Instant::now(), false);
            assert_eq!((rewinds(&ui), branches(&ui)), (1, 2), "the pointed prompt's too");
            let (r, action) = ui.hits.iter().find(|(_, a)| matches!(a, Action::Rewind(_))).cloned().unwrap();
            let Action::Rewind(prompt) = action else { unreachable!() };
            assert!(matches!(model.items[&prompt].kind, ItemKind::Prompt { .. }));
            // Moving onto the button keeps it: it is under the pointed message.
            let (bx, by) = (r.cx(), r.cy());
            assert_eq!(ui.pointed_message(bx, by), Some(prompt));
            ui.draw(&mut p, pal, area, &mut model, (bx, by), Instant::now(), false);
            ui.press(bx, by, 1, Mods::default(), &mut model);
            assert_eq!(ui.release(bx, by, Mods::default(), &mut model), Release::Rewind(prompt));
            let (r, action) =
                ui.hits.iter().find(|(_, a)| matches!(a, Action::Branch(id) if *id == prompt)).cloned().unwrap();
            ui.press(r.cx(), r.cy(), 1, Mods::default(), &mut model);
            assert_eq!(ui.release(r.cx(), r.cy(), Mods::default(), &mut model), Release::Branch(prompt));
            assert!(action == Action::Branch(prompt));
            // While the newest turn runs, its answer has Copy and no Branch.
            model.chat.latest = Some((last, ChatPhase::Active));
            model.revision += 1;
            ui.draw(&mut p, pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
            let copies = ui.hits.iter().filter(|(_, a)| matches!(a, Action::CopyMessage(_))).count();
            assert_eq!((copies, branches(&ui)), (1, 0));
        }
    }

    #[test]
    fn no_text_is_drawn_or_taken_under_the_jump_button() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(model.collect());
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(700, 420);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let area = Rect::new(0.0, 0.0, 700.0, 420.0);
        let pal = &mdshared_ui::theme::DARK;
        let mut ui = Ui::default();
        ui.set_chat(true);
        ui.draw(&mut p, pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
        assert!(ui.max > 200.0, "a conversation longer than the window");
        assert_eq!(ui.covered, 0.0, "following: no button, nothing kept");
        ui.wheel(ui.body.cx(), ui.body.cy(), 150.0);
        ui.draw(&mut p, pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
        assert!(!ui.follow.active() && ui.covered > JUMP);
        let button = ui.hits.iter().find(|(_, a)| *a == Action::Latest).map(|(r, _)| *r).unwrap();
        let foot = ui.shown_body().bottom();
        assert!(button.y >= foot, "the button stands in the kept foot");
        // Nothing else is there to point at, and no text is found under it.
        assert!(ui.hits.iter().filter(|(_, a)| *a != Action::Latest).all(|(r, _)| r.bottom() <= foot + 0.5));
        let x = ui.body.x + 24.0;
        assert!((foot as u32..ui.body.bottom() as u32).all(|y| ui.text_at(x, y as f32).is_none()));
        // Back at the end it follows again: the foot is the conversation's once more.
        ui.wheel(ui.body.cx(), ui.body.cy(), -10_000.0);
        ui.draw(&mut p, pal, area, &mut model, (-1.0, -1.0), Instant::now(), false);
        assert!(ui.follow.active() && ui.covered == 0.0);
    }
}
