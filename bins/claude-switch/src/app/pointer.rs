//! Only the front layer gets hover. Cursor choice is a total query of the current hit map,
//! independent of which controls handled the last motion event.
use super::*;
use mdshared_ui::cursor::Cursor;
use mdshared_ui::tooltip::Over;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Palette,
    Accounts,
    Menu,
    Dialog,
    Picture,
    Usage,
    Base,
}

impl Layer {
    fn accepts(self, target: Target) -> bool {
        let usage = matches!(target, Target::UsagePopup | Target::UsageRefresh | Target::UsageOpenAs);
        match self {
            Self::Dialog => matches!(target, Target::Dialog(_)),
            Self::Usage => usage || matches!(target, Target::Account | Target::UsageLines),
            Self::Base => !usage && !matches!(target, Target::Dialog(_)),
            _ => false, // These layers own their own geometry, not app targets.
        }
    }
}

impl AppState {
    fn pointer_layer(&self) -> Layer {
        if self.palette.is_some() {
            Layer::Palette
        } else if self.accounts_dd.is_open() {
            Layer::Accounts
        } else if self.menu.is_some() {
            Layer::Menu
        } else if self.dialog.is_some() {
            Layer::Dialog
        } else if self.picture_overlay() {
            Layer::Picture
        } else if self.usage.popup.open {
            Layer::Usage
        } else {
            Layer::Base
        }
    }

    fn hover_target(&self, layer: Layer) -> Option<Target> {
        let (x, y) = self.mouse;
        self.target_in(layer, x, y)
    }

    fn target_in(&self, layer: Layer, x: f32, y: f32) -> Option<Target> {
        self.hits.iter().rev().find(|(r, t)| layer.accepts(*t) && r.contains(x, y)).map(|(_, t)| *t)
    }

    /// The target at `(x, y)` in the front layer only.
    pub(crate) fn front_target_at(&self, x: f32, y: f32) -> Option<Target> {
        self.target_in(self.pointer_layer(), x, y)
    }

    fn pointer_for(&self, layer: Layer) -> (f32, f32) {
        if self.pointer_layer() == layer { self.mouse } else { (-1.0, -1.0) }
    }

    pub(crate) fn background_pointer(&self) -> (f32, f32) {
        self.pointer_for(Layer::Base)
    }

    pub(crate) fn picture_pointer(&self) -> (f32, f32) {
        self.pointer_for(Layer::Picture)
    }

    /// Drop covered hover before any part of the frame paints. The fresh hit map is used after paint.
    pub(crate) fn mask_covered_hover(&mut self, now: std::time::Instant) {
        let layer = self.pointer_layer();
        self.hover = self.hover.filter(|&t| layer.accepts(t));
        // A tooltip stays only while the layer of its control is in front.
        let tip = self.tip.shown().or(self.tip.waiting());
        // Settings' Peek button is drawn by the dialog, as its rows are.
        let home = |t: Target| match t {
            Target::Dialog(_) | Target::SettingsPeek => Layer::Dialog,
            _ => Layer::Base,
        };
        let own = tip.is_some_and(|o| home(o.key) == layer);
        if !own && (layer != Layer::Base || tip.is_some()) {
            self.tip.hover(now, None);
        }
    }

    pub(super) fn moved(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) -> bool {
        self.mouse = (x, y);
        if let Some(changed) = self.field_drag(x, y, cx.now()) {
            return changed | self.refresh_pointer(cx);
        }
        let moved = self.move_in_layer(cx, x, y);
        let usage = self.usage_hover(cx.now(), self.hover_target(self.pointer_layer()));
        moved | usage | self.refresh_pointer(cx)
    }

    fn move_in_layer(&mut self, cx: &mut Cx<'_, Msg>, x: f32, y: f32) -> bool {
        match self.pointer_layer() {
            Layer::Palette => return self.palette.as_mut().unwrap().pointer_move(x, y),
            Layer::Accounts => return self.accounts_dd.pointer(x, y),
            Layer::Menu => return self.menu.as_mut().unwrap().1.pointer(x, y),
            Layer::Dialog => {
                return if let Some(Dialog::Settings(form)) = &mut self.dialog {
                    form.pointer_move(x, y)
                } else {
                    self.folder_scroll_drag(y)
                };
            }
            Layer::Usage => return false,
            Layer::Picture => return self.session_move(x, y).unwrap_or(false),
            Layer::Base => {}
        }
        if let Some(changed) = self.chat_move(x, y) {
            return changed;
        }
        if self.tabs.drag.is_some() {
            return self.move_tab_drag(x, y);
        }
        let terminal_drag = self.tabs.shown().is_some_and(|tab| tab.term.selecting || tab.term.reported.is_some());
        if !terminal_drag
            && y >= TITLE_H
            && let Some(changed) = self.session_move(x, y)
        {
            return changed;
        }
        if self.splitter.dragging() {
            return self.splitter.drag(self.main_area(cx), Axis::Horizontal, x, y);
        }
        self.terminal_shown() && self.term_move(x, y, cx.mods())
    }

    pub(super) fn refresh_pointer(&mut self, cx: &mut Cx<'_, Msg>) -> bool {
        let layer = self.pointer_layer();
        let hover = self.hover_target(layer);
        let mut changed = std::mem::replace(&mut self.hover, hover) != hover;
        let over = hover
            .and_then(|key| {
                let text = match layer {
                    Layer::Base => topbar::tip_of(key)?.into(),
                    Layer::Dialog => self.dialog_tip(key)?,
                    _ => return None,
                };
                Some(Over { key, text, anchor: self.rect_of(key)? })
            })
            .or_else(|| {
                let anchor = self.settings_peek_tip().filter(|_| layer == Layer::Dialog)?;
                Some(Over { key: Target::SettingsPeek, text: crate::dialogs::PEEK_TIP.into(), anchor })
            });
        changed |= self.tip.hover_at(cx.now(), over, self.mouse);
        cx.set_cursor(self.pointer_cursor(layer, hover, cx.size()));
        changed
    }

    fn pointer_cursor(&self, layer: Layer, hover: Option<Target>, (w, h): (f32, f32)) -> Cursor {
        let (x, y) = self.mouse;
        if !Rect::new(0.0, 0.0, w, h).contains(x, y) {
            return Cursor::Default;
        }
        if layer == Layer::Palette && self.palette.as_ref().is_some_and(|p| p.field_rect().contains(x, y)) {
            return Cursor::Text;
        }
        if self.field_mouse.held().is_some() || self.over_field(x, y) {
            return Cursor::Text;
        }
        if (layer == Layer::Picture
            || layer == Layer::Base && !matches!(hover, Some(Target::Chat(_) | Target::Terminal)) && y >= self.main_top)
            && let Some(cursor) = self.session_pointer_cursor(x, y)
        {
            return cursor;
        }
        match hover {
            Some(Target::Splitter) => Cursor::ResizeEw,
            Some(Target::Chat(crate::view::chat::Control::Editor)) => Cursor::Text,
            Some(Target::Chat(crate::view::chat::Control::Grip)) => Cursor::ResizeNs,
            Some(Target::Chat(crate::view::chat::Control::Detail(_) | crate::view::chat::Control::Shells)) => {
                Cursor::Hand
            }
            Some(Target::Dialog(crate::compact_dialog::EDITOR)) if matches!(self.dialog, Some(Dialog::Compact(_))) => {
                Cursor::Text
            }
            _ => Cursor::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;

    #[test]
    fn a_dialog_row_tooltip_stays_under_its_dialog_and_goes_with_it() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        app.dialog = Some(Dialog::About);
        let (key, now) = (Target::Dialog(1000), std::time::Instant::now());
        let anchor = Rect::new(0.0, 0.0, 10.0, 10.0);
        app.tip.hover(now, Some(Over { key, text: "path".into(), anchor }));
        app.mask_covered_hover(now);
        assert!(app.tip.waiting().is_some());
        app.dialog = None;
        app.mask_covered_hover(now);
        assert!(app.tip.waiting().is_none() && app.tip.shown().is_none());
    }

    #[test]
    fn the_peek_tooltip_stays_while_settings_is_open() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        app.dialog = Some(Dialog::About);
        let now = std::time::Instant::now();
        let over =
            Over { key: Target::SettingsPeek, text: dialogs::PEEK_TIP.into(), anchor: Rect::new(0.0, 0.0, 10.0, 10.0) };
        app.tip.hover(now, Some(over));
        app.mask_covered_hover(now);
        assert!(app.tip.waiting().is_some(), "the dialog is in front: its Peek tip stays");
        app.dialog = None;
        app.mask_covered_hover(now);
        assert!(app.tip.waiting().is_none() && app.tip.shown().is_none());
    }

    #[test]
    fn newly_opened_overlays_mask_existing_row_hover_even_before_the_pointer_moves() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        let now = std::time::Instant::now();
        let area = Rect::new(0.0, 0.0, 120.0, 100.0);
        app.mouse = (30.0, 40.0);
        app.hits = vec![(area, Target::Entry(1)), (area, Target::Dialog(dialogs::OK))];
        assert_eq!(app.hover_target(app.pointer_layer()), Some(Target::Entry(1)));
        app.hover = Some(Target::Entry(1));
        app.main_menu(area);
        app.mask_covered_hover(now);
        assert_eq!(app.hover, None);
        assert_eq!(app.background_pointer(), (-1.0, -1.0));
        assert_eq!(app.hover_target(app.pointer_layer()), None);
        app.menu = None;
        app.open_palette();
        assert_eq!(app.hover_target(app.pointer_layer()), None);
        app.palette = None;
        app.dialog = Some(Dialog::About);
        assert_eq!(app.hover_target(app.pointer_layer()), Some(Target::Dialog(dialogs::OK)));
        app.dialog = None;
        assert_eq!(app.hover_target(app.pointer_layer()), Some(Target::Entry(1)));
    }

    #[test]
    fn cursor_follows_current_hit_after_tab_or_menu_changes_and_on_leave() {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        let window = (640.0, 480.0);
        let at = Rect::new(100.0, 100.0, 200.0, 150.0);
        app.mouse = (125.0, 130.0);
        let cursor = |app: &AppState| {
            let layer = app.pointer_layer();
            app.pointer_cursor(layer, app.hover_target(layer), window)
        };
        app.hits = vec![(at, Target::Terminal)];
        assert_eq!(cursor(&app), Cursor::Default, "the terminal always has an arrow");
        app.hits = vec![(at, Target::Tab(2))];
        assert_eq!(cursor(&app), Cursor::Default, "same point, but the new layout is not a terminal");
        app.hits = vec![(at, Target::Chat(crate::view::chat::Control::Editor))];
        assert_eq!(cursor(&app), Cursor::Text);
        app.main_menu(at);
        assert_eq!(cursor(&app), Cursor::Default);
        app.menu = None;
        assert_eq!(cursor(&app), Cursor::Text);
        app.hits = vec![(at, Target::Terminal)];
        assert_eq!(cursor(&app), Cursor::Default);
        app.session_pointer_left();
        assert_eq!(cursor(&app), Cursor::Default);
    }
}
