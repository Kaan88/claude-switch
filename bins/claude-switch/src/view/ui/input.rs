use mdshared_ui::cursor::Cursor;
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::platform;

use super::super::links;
use super::scroll::Press;
use super::*;

impl Ui {
    fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a.clone())
    }

    fn text_at(&self, x: f32, y: f32) -> Option<(&Row, usize, Option<&str>)> {
        if self.overlay_open() || !self.body.contains(x, y) {
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
        if let Some((row, at, link)) = self.text_at(x, y) {
            let id = row.id.clone();
            let link = link.map(str::to_owned);
            let text = row.body.as_ref().unwrap().text();
            let (a, b) = if clicks >= 3 {
                (0, text.len())
            } else if clicks == 2 {
                word(text, at)
            } else if mods.shift {
                (self.selection.as_ref().filter(|(r, _, _)| *r == id).map_or(at, |s| s.1), at)
            } else {
                (at, at)
            };
            self.start = link.filter(|_| clicks == 1 && !mods.shift).map(|s| (x, y, s));
            self.selection = Some((id, a, b));
            self.dragging = clicks == 1;
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
        let mut changed = self.hot != hot || self.scroll_hot != track;
        self.hot = hot;
        self.scroll_hot = track;
        if self.dragging {
            if self.start.as_ref().is_some_and(|(sx, sy, _)| (x - sx).abs() + (y - sy).abs() > 4.0) {
                self.start = None;
            }
            if let Some((id, _, caret)) = &self.selection
                && let Some(row) = self.rows.iter().find(|r| r.id == *id)
                && let Some(text) = &row.body
            {
                let dx = x - self.body.x - row.geo.tx;
                let dy = y - self.body.y + self.offset - row.top - row.geo.ty;
                let next = text.point(dx, dy).0;
                if next != *caret {
                    self.selection.as_mut().unwrap().2 = next;
                    changed = true;
                    self.start = None;
                    self.pressed = None;
                }
            }
        }
        changed
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
            Action::Picture(id, index) => {
                let image = m.picture(id, index, true).or_else(|| m.picture(id, index, false));
                self.overlay = Some(Overlay { id, index, image, bounds: Rect::default() });
            }
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
            Destination::Picture(id, index) => {
                let image = m.picture(id, index, true).or_else(|| m.picture(id, index, false));
                self.overlay = Some(Overlay { id, index, image, bounds: Rect::default() });
            }
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

    pub(in crate::view) fn open_agent(&mut self, id: &str, model: &Model) {
        if model.agent.as_deref() != Some(id) {
            self.agent_origin = Some(self.page);
            model.select_agent(Some(id));
            self.show(Page::Conversation);
        }
    }

    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        if self.overlay_open() {
            return;
        }
        if let Some(pane) = &self.markdown {
            if pane.bounds.contains(x, y) {
                self.scroll_to(Surface::Markdown, pane.offset() - dy);
                return;
            }
            if self.body.w == 0.0 {
                return;
            }
        }
        self.scroll_to(Surface::Conversation, self.offset - dy);
    }

    pub fn key(&mut self, name: &KeyName, mods: Mods) -> bool {
        if self.overlay_open() {
            if *name == KeyName::Escape {
                self.overlay = None;
            }
            return true;
        }
        if !self.focused {
            return false;
        }
        match name {
            KeyName::Escape if self.markdown.is_some() => self.close_markdown(),
            KeyName::Char('c') if mods.ctrl && !mods.alt => {
                let Some((id, _, _)) = &self.selection else { return false };
                let Some(text) = self.rows.iter().find(|r| r.id == *id).and_then(|r| r.body.as_ref()).map(|b| b.text())
                else {
                    return false;
                };
                let Some(range) = selection(&self.selection, id, text).filter(|r| !r.is_empty()) else { return false };
                platform::set_clipboard_text(&text[range]);
            }
            KeyName::Char('a') if mods.ctrl && !mods.alt => {
                let Some((id, a, b)) = &mut self.selection else { return false };
                let Some(text) = self.rows.iter().find(|r| r.id == *id).and_then(|r| r.body.as_ref()) else {
                    return false;
                };
                *a = 0;
                *b = text.text().len();
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
    fn working_stop_in_both_surfaces_requires_the_same_turn_at_press_and_release() {
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
            let r = ui.hits.iter().find_map(|(r, action)| (*action == Action::Stop(one)).then_some(*r)).unwrap();
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
    fn jumping_to_an_edit_expands_its_group_and_scrolls_to_the_exact_tool() {
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
        assert!(ui.rows.iter().any(|row| row.style == Style::Group && row.title == "4 tool calls"));
        assert!(!ui.rows.iter().any(|row| row.id == Id::Tool(id)));
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
    }
}
