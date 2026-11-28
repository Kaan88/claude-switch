//! One native settings form at a time. Its controls (including swatches and popovers) keep
//! their existing input handling; the app owns only the category rail and page lifetime.
//!
//! A category switch cross-fades the old page into the new one with the toolkit's
//! `PageTransition`. The old page is drawn once more in the first frame after the switch (the
//! window's bitmap no longer holds it by then), saved, and blended away over `SWITCH`. The
//! animator decides whether anything moves: the app's Reduce motion, Windows' reduced-motion
//! setting and high contrast all make the switch instant.

use super::{
    AppState, Form, PAGES, SettingsEvent, appearance, page_form, peek, terminal_appearance, theme_editor::Editor, tiles,
};
use crate::windowlook::Cards;
use mdshared_ui::anim::{Animator, PageEffect, PageTransition, fluent};
use mdshared_ui::cursor::Cursor;
use mdshared_ui::gfx::{Face, Gfx, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::menu::Item;
use mdshared_ui::settings::{Outcome, Style, Value};
use mdshared_ui::textfield::ContextAction;
use std::time::Instant;

const NAV_W: f32 = 146.0;
const NAV_ROW: f32 = 32.0;
const NAV_TOP: f32 = 54.0;
const GAP: f32 = 8.0;
// Allow the native form's own dialog margins (16 on each side and 24 above/below).
const SIDE: f32 = 16.0;
const TOP: f32 = 24.0;
const MAX_PAGE_W: f32 = 600.0;
const MAX_PAGE_H: f32 = 560.0;
/// A category switch's cross-fade: short, as a page of a navigation view.
const SWITCH: std::time::Duration = std::time::Duration::from_millis(140);
/// Room round the page for its card's soft shadow, which the switch blends too.
const SHADOW_ROOM: f32 = 16.0;

#[derive(Default)]
struct Layout {
    rail: Rect,
    viewport: Rect,
}

impl Layout {
    fn new(window: Rect) -> Self {
        let width = (window.w - 2.0 * SIDE).min(NAV_W + GAP + MAX_PAGE_W);
        let x = window.x + ((window.w - width) / 2.0).round();
        let height = window.h.min(MAX_PAGE_H + 2.0 * TOP);
        let y = window.y + ((window.h - height) / 2.0).round();
        let rail = Rect::new(x, y + TOP, NAV_W, NAV_TOP + PAGES.len() as f32 * NAV_ROW + 12.0);
        let viewport = Rect::new(rail.right() + GAP - SIDE, y, width - NAV_W - GAP + 2.0 * SIDE, height);
        Self { rail, viewport }
    }

    fn category(&self, index: usize) -> Rect {
        Rect::new(self.rail.x + 8.0, self.rail.y + NAV_TOP + index as f32 * NAV_ROW, self.rail.w - 16.0, NAV_ROW)
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        (self.rail.w > 0.0).then_some(())?;
        (0..PAGES.len()).find(|&i| self.category(i).contains(x, y))
    }
}

/// Lays `form` out in `viewport`, less the height its card does not use: the form centres the
/// card, and a short page keeps the top of its controls level with the rail.
fn fit(form: &mut Form, p: &mut Painter, mut viewport: Rect) -> Rect {
    let card = form.layout(p, viewport);
    viewport.h = card.h + 2.0 * TOP;
    viewport
}

fn draw_form(
    form: &mut Form,
    p: &mut Painter,
    st: &AppState,
    viewport: Rect,
    caret: bool,
    anim: &mut Animator,
    category: &str,
) {
    // A native dropdown/picker can extend into the form's margins. Give its
    // whole viewport an opaque backing only while that nested popup is open.
    let cards = if form.popup_open() { Cards::for_frame(st, p) } else { Cards::Native };
    cards.draw(p, viewport, st.pal().popup, |p| {
        form.draw_animated(p, st.pal(), viewport, caret, anim, ("settings", category))
    });
}

pub struct Settings {
    index: usize,
    pub(in super::super) form: Form,
    pub(super) editor: Option<Editor>,
    rail_focus: bool,
    key_used: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
    /// The Appearance page's palette swatch under the pointer, and the one pressed.
    swatch_hover: Option<(u32, usize)>,
    swatch_pressed: Option<(u32, usize)>,
    layout: Layout,
    /// The page being left, until the next frame draws it once more for the switch.
    leaving: Option<(usize, Form)>,
    switch: PageTransition,
    pub(super) peek: peek::Peek,
}

impl Settings {
    pub(super) fn new(st: &AppState, index: usize) -> Self {
        Self {
            index,
            form: page_form(st, index),
            editor: None,
            rail_focus: true,
            key_used: false,
            hover: None,
            pressed: None,
            swatch_hover: None,
            swatch_pressed: None,
            layout: Layout::default(),
            leaving: None,
            switch: PageTransition::new("settings-page").with_timing(SWITCH, fluent::DECELERATE),
            peek: peek::Peek::default(),
        }
    }

    pub(in super::super) fn category(&self) -> &'static str {
        PAGES[self.index].0
    }

    pub(super) fn show_page(&mut self, st: &AppState, index: usize) {
        if index != self.index {
            let old = std::mem::replace(&mut self.form, page_form(st, index));
            // A switch over a switch starts from the page that was being left first.
            if self.leaving.is_none() {
                self.leaving = Some((self.index, old));
            }
            self.index = index;
            self.editor = None;
            self.swatch_hover = None;
        }
    }

    fn place(&mut self, p: &mut Painter, window: Rect) -> Rect {
        self.layout = Layout::new(window);
        fit(&mut self.form, p, self.layout.viewport)
    }

    /// Where the page switch blends: every page's viewport, its shadow and nothing outside the
    /// window.
    fn switch_region(&self, window: Rect) -> Rect {
        let v = self.layout.viewport.inset(-SHADOW_ROOM);
        let (x, y) = (v.x.max(window.x), v.y.max(window.y));
        Rect::new(x, y, v.right().min(window.right()) - x, v.bottom().min(window.bottom()) - y)
    }

    /// The first frame after a switch: draws the page being left in the region, saves it for
    /// the cross-fade and puts the region back as it was, for the new page to be drawn on.
    fn begin_switch(&mut self, p: &mut Painter, st: &AppState, region: Rect, anim: &mut Animator) {
        let Some((index, mut old)) = self.leaving.take() else { return };
        if !anim.animates() {
            return self.switch.cancel(anim);
        }
        let under = p.snapshot(region);
        let viewport = fit(&mut old, p, self.layout.viewport);
        draw_form(&mut old, p, st, viewport, false, anim, PAGES[index].0);
        if index == 0 || PAGES[index].0 == super::TERMINAL {
            appearance::draw(p, st, &old, None, false);
        }
        let snapshot = p.snapshot(region);
        under.restore(p.bmp);
        self.switch.begin_with(anim, snapshot, p.scale, PageEffect::Crossfade);
    }

    /// Whether a page switch is blending.
    #[cfg(test)]
    fn switching(&self) -> bool {
        self.switch.running() || self.leaving.is_some()
    }

    pub(in super::super) fn draw(
        &mut self,
        p: &mut Painter,
        st: &AppState,
        window: Rect,
        presence: f32,
        anim: &mut Animator,
    ) {
        let pal = st.pal();
        let cards = Cards::for_frame(st, p);
        let radius = cards.decoration(st.appearance.radius);
        let shadow = if st.high_contrast { 0.0 } else { cards.decoration(st.appearance.shadow) };
        if self.category() == super::TERMINAL {
            terminal_appearance::sync(&mut self.form, &st.config.appearance, st.high_contrast, p.frost_key().is_some());
        }
        let viewport = self.place(p, window);
        cards.dim(p, pal, window, presence);
        let rail = self.layout.rail;
        let rail_shadow = shadow * if pal.dark { 1.0 } else { mdshared_ui::gfx::LIGHT_SHADOW };
        p.soft_shadow(rail, radius, &mdshared_ui::gfx::Shade::POPUP, rail_shadow);
        p.frame(rail, radius, pal.popup, pal.popup_line);
        p.text(Face::Semibold, 17.0, rail.x + 18.0, rail.y + 33.0, "Settings", pal.text, rail.w - 36.0);
        for (i, &(label, _)) in PAGES.iter().enumerate() {
            let rect = self.layout.category(i).inset(1.0);
            let selected = i == self.index;
            if selected {
                if self.rail_focus && self.key_used {
                    p.frame(rect, 6.0, pal.on, pal.accent);
                } else {
                    p.rbox(rect, 6.0, pal.on);
                }
                p.rbox(Rect::new(rect.x + 4.0, rect.cy() - 8.0, 3.0, 16.0), 1.5, pal.accent);
            } else if self.hover == Some(i) {
                p.rbox(rect, 6.0, pal.hover);
            }
            p.text(
                Face::Ui,
                13.0,
                rect.x + 14.0,
                rect.cy(),
                label,
                if selected { pal.text } else { pal.muted },
                rect.w - 24.0,
            );
        }
        let mut style = Style::CLASSIC;
        style.radius = radius;
        style.soft_shadow = Some(shadow);
        style.backdrop = 0.0;
        let region = self.switch_region(window);
        for form in std::iter::once(&mut self.form).chain(self.leaving.as_mut().map(|(_, f)| f)) {
            form.set_style(style);
            form.set_high_contrast(st.high_contrast);
        }
        self.begin_switch(p, st, region, anim);
        let category = self.category();
        draw_form(&mut self.form, p, st, viewport, true, anim, category);
        if let Some(editor) = &self.editor {
            let note = editor.note(&self.form);
            let r = Rect::new(rail.x, rail.bottom() + 8.0, rail.w, 60.0);
            p.frame(r, cards.decoration(6.0), pal.popup, pal.popup_line);
            let (summary, detail) = note.split_once(" · ").unwrap_or((&note, ""));
            p.text(Face::Semibold, 12.0, r.x + 8.0, r.y + 18.0, summary, pal.text, r.w - 16.0);
            p.text(Face::Ui, 11.5, r.x + 8.0, r.y + 40.0, detail, pal.muted, r.w - 16.0);
        } else if self.has_palettes() {
            appearance::draw(p, st, &self.form, self.swatch_hover, self.key_used && !self.rail_focus);
        }
        // After the page and before anything over it: the menus and pickers are the form's own
        // and are drawn inside it, so only the blend is left.
        self.switch.apply(anim, p, region);
        self.peek.button = peek::draw_button(p, pal, self.form.rect(), &self.peek);
    }

    /// The dim and the dialog, faded by how far a peek has taken them away, then the peek's
    /// pill. Without a peek this is [`Self::draw`]. True while the fade is moving.
    pub(in super::super) fn draw_layer(
        &mut self,
        p: &mut Painter,
        st: &AppState,
        window: Rect,
        presence: f32,
        anim: &mut Animator,
    ) -> bool {
        let shown = anim.toggle(peek::SHOWN, !self.peeking(), peek::MOTION);
        if shown >= 1.0 {
            self.draw(p, st, window, presence, anim);
        } else if shown > 0.0 {
            let under = p.snapshot(window);
            self.draw(p, st, window, presence, anim);
            p.fade(&under, shown);
        }
        peek::draw_pill(p, st, window, 1.0 - shown);
        !anim.settled(peek::SHOWN)
    }

    /// The Peek button as last drawn.
    #[cfg(test)]
    pub(crate) fn peek_button(&self) -> Rect {
        self.peek.button
    }

    /// Whether a peek is holding Settings out of sight.
    pub(crate) fn peeking(&self) -> bool {
        self.peek.by.is_some()
    }

    /// Ends the peek `by` started; another peek, or none, stays as it is. True if it ended.
    pub(crate) fn end_peek(&mut self, by: peek::By) -> bool {
        self.peek.by.take_if(|b| *b == by).is_some()
    }

    /// Ends any peek: the window lost the keyboard. True if one ended.
    pub(crate) fn cancel_peek(&mut self) -> bool {
        self.peek.by.take().is_some()
    }

    /// The Peek button while the pointer rests on it, for its tooltip.
    pub(crate) fn peek_tip(&self) -> Option<Rect> {
        (self.peek.hover && !self.peeking()).then_some(self.peek.button)
    }

    /// Space peeks only when what has the keyboard does nothing with it: the category list, no
    /// control, or a number. A text box, switch, choice, palette card, button or an open
    /// dropdown or picker keeps its Space.
    fn space_peeks(&self) -> bool {
        use mdshared_ui::settings::Row;
        if self.form.popup_open() {
            return false;
        }
        if self.rail_focus {
            return true;
        }
        if self.form.focus_on_close() {
            return false;
        }
        let Some(id) = self.form.focus_id() else { return true };
        matches!(self.form.rows().iter().find(|r| r.id() == Some(id)), Some(Row::Number { .. }) | None)
    }

    fn swatch_at(&self, x: f32, y: f32) -> Option<(u32, usize)> {
        self.has_palettes().then(|| tiles::hit(&self.form, x, y)).flatten()
    }
    fn has_palettes(&self) -> bool {
        self.editor.is_none() && (self.index == 0 || self.category() == super::TERMINAL)
    }

    pub(crate) fn key(
        &mut self,
        config: &claude_switch_core::Config,
        key: &KeyName,
        mods: Mods,
        text: Option<&str>,
    ) -> SettingsEvent {
        if self.peeking() {
            return SettingsEvent::Form(Outcome::None);
        }
        if *key == KeyName::Space && !(mods.ctrl || mods.alt || mods.shift) && self.space_peeks() {
            self.peek.by = Some(peek::By::Space);
            return SettingsEvent::Form(Outcome::Redraw);
        }
        self.key_used = true;
        if self.rail_focus {
            let next = match key {
                KeyName::Up => self.index.saturating_sub(1),
                KeyName::Down => (self.index + 1).min(PAGES.len() - 1),
                KeyName::Home => 0,
                KeyName::End => PAGES.len() - 1,
                KeyName::Tab | KeyName::Right | KeyName::Enter => {
                    self.enter_form(*key == KeyName::Tab && mods.shift);
                    return SettingsEvent::Form(Outcome::Redraw);
                }
                KeyName::Escape => return SettingsEvent::Form(Outcome::Closed),
                _ => return SettingsEvent::Form(Outcome::Redraw),
            };
            return SettingsEvent::Page(next);
        }
        if self.has_palettes()
            && self.form.colour_open().is_none()
            && let Some(outcome) = tiles::key(&mut self.form, config, key)
        {
            tiles::reveal(&mut self.form);
            return SettingsEvent::Form(outcome);
        }
        let first = self.first_row();
        let was_first = self.form.focus_id() == first;
        let was_close = self.form.focus_on_close();
        let was = self.form.focus_id();
        let outcome = self.form.key(key, mods, text);
        if self.has_palettes() && self.form.focus_id() != was {
            tiles::reveal(&mut self.form);
        }
        // Insert the rail between Close and the first control in the form's Tab ring.
        // A first Tab that only reveals focus or dismisses a popup hasn't moved focus.
        if *key == KeyName::Tab
            && if mods.shift {
                was_first && self.form.focus_on_close()
            } else {
                was_close && self.form.focus_id() == first
            }
        {
            self.rail_focus = true;
            // The form has no blur API. Its outside press clears the keyboard ring;
            // ignore Closed here, since focus is only moving to the rail.
            let _ = self.form.press(self.layout.rail.x, self.layout.rail.y, Mods::default(), 1);
        }
        SettingsEvent::Form(outcome)
    }

    fn first_row(&self) -> Option<u32> {
        self.form.rows().iter().find_map(|row| row.id())
    }

    fn enter_form(&mut self, reverse: bool) {
        self.rail_focus = false;
        if let Some(id) = self.first_row() {
            // The native form may use the first Tab just to reveal focus. Put the
            // ring on the first control, or on Close if entering backwards.
            self.form.focus_row(id);
            let _ = self.form.key(&KeyName::Tab, Mods { shift: reverse, ..Mods::default() }, None);
            if reverse {
                if !self.form.focus_on_close() {
                    let _ = self.form.key(&KeyName::Tab, Mods { shift: true, ..Mods::default() }, None);
                }
            } else {
                self.form.focus_row(id);
            }
        }
    }

    /// A press without text editing, for tests that only click controls.
    #[cfg(test)]
    pub(crate) fn press(&mut self, x: f32, y: f32, mods: Mods, clicks: u8) -> SettingsEvent {
        self.press_with(x, y, |form| form.press(x, y, mods, clicks))
    }

    /// A press through the toolkit's text editing: in a text row it places the caret, extends
    /// with Shift, takes a word on a double click and the whole row on a triple click; anywhere
    /// else it is [`Self::press`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pointer_press(
        &mut self,
        x: f32,
        y: f32,
        mods: Mods,
        clicks: u8,
        now: Instant,
        gfx: &mut Gfx,
        scale: f32,
    ) -> SettingsEvent {
        self.press_with(x, y, |form| form.pointer_press((x, y), mods, clicks, now, gfx, scale))
    }

    fn press_with(&mut self, x: f32, y: f32, press: impl FnOnce(&mut Form) -> Outcome) -> SettingsEvent {
        if self.peeking() {
            return SettingsEvent::Form(Outcome::None);
        }
        if !self.form.popup_open() && self.peek.button.contains(x, y) {
            self.peek.by = Some(peek::By::Mouse);
            return SettingsEvent::Form(Outcome::Redraw);
        }
        self.pressed = None;
        self.key_used = false;
        self.swatch_pressed = self.swatch_at(x, y);
        if let Some((id, _)) = self.swatch_pressed {
            self.rail_focus = false;
            self.form.focus_row(id);
            return SettingsEvent::Form(Outcome::Redraw);
        }
        if self.has_palettes() && tiles::covers(&self.form, x, y) {
            // Between the cards: the grid row's own dropdown is under them and stays shut.
            return SettingsEvent::Form(Outcome::None);
        }
        let outcome = press(&mut self.form);
        if self.layout.rail.contains(x, y) && outcome == Outcome::Closed {
            // The native form dismisses its open dropdown or picker first, returning
            // Redraw (or a reverted colour), not Closed. Do not click through them.
            self.pressed = self.layout.hit(x, y);
            self.rail_focus = true;
            SettingsEvent::Form(Outcome::Redraw)
        } else {
            if self.form.rect().contains(x, y) {
                self.rail_focus = false;
            }
            SettingsEvent::Form(outcome)
        }
    }

    pub(crate) fn release(&mut self, x: f32, y: f32) -> SettingsEvent {
        if self.end_peek(peek::By::Mouse) {
            return SettingsEvent::Form(Outcome::Redraw);
        }
        if self.peeking() {
            return SettingsEvent::Form(Outcome::None);
        }
        if let Some((id, k)) = self.swatch_pressed.take()
            && self.swatch_at(x, y) == Some((id, k))
        {
            self.rail_focus = false;
            self.form.set_value(id, Value::Choice(k));
            return SettingsEvent::Form(Outcome::Changed { id, value: Value::Choice(k) });
        }
        if let Some(index) = self.pressed.take()
            && self.layout.hit(x, y) == Some(index)
        {
            return SettingsEvent::Page(index);
        }
        SettingsEvent::Form(self.form.release(x, y))
    }

    pub(crate) fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        if self.peeking() {
            return false;
        }
        let over = self.peek.button.contains(x, y);
        let peek_changed = std::mem::replace(&mut self.peek.hover, over) != over;
        let hover = self.layout.hit(x, y);
        let swatch = self.swatch_at(x, y);
        let swatch_changed = std::mem::replace(&mut self.swatch_hover, swatch) != swatch;
        let changed = (std::mem::replace(&mut self.hover, hover) != hover) | swatch_changed;
        self.form.pointer_move(x, y) || changed || peek_changed
    }

    /// Whether a colour row's picker is open.
    pub(crate) fn colour_open(&self) -> bool {
        self.form.colour_open().is_some()
    }

    /// The text row under the pointer, for the I-beam and the edit menu. The toolkit decides:
    /// only an uncovered, editable text area counts, never one under an open dropdown or picker.
    pub(crate) fn text_row_at(&self, x: f32, y: f32) -> Option<u32> {
        if self.peeking()
            || self.layout.rail.contains(x, y)
            || !matches!(self.form.pointer_cursor_at(x, y), Some(Cursor::Text))
        {
            return None;
        }
        self.form.rows().iter().enumerate().find_map(|(i, row)| match row {
            mdshared_ui::settings::Row::Text { id, .. } if self.form.control_rect(i).contains(x, y) => Some(*id),
            _ => None,
        })
    }

    /// A move while a text drag started by [`Self::pointer_press`] is held. True to redraw.
    pub(crate) fn text_motion(&mut self, x: f32, y: f32, now: Instant, gfx: &mut Gfx) -> bool {
        !matches!(self.form.pointer_motion((x, y), Mods::default(), now, gfx), Outcome::None | Outcome::Unused)
    }

    /// The button went up on a held text drag.
    pub(crate) fn text_release(&mut self, x: f32, y: f32, now: Instant, gfx: &mut Gfx) -> SettingsEvent {
        SettingsEvent::Form(self.form.pointer_release((x, y), now, gfx))
    }

    /// When a held text drag next scrolls its row, if it rests past an end.
    pub(crate) fn text_deadline(&self) -> Option<Instant> {
        self.form.text_scroll_deadline()
    }

    pub(crate) fn text_tick(&mut self, now: Instant, gfx: &mut Gfx) -> bool {
        self.form.tick_text_scroll(now, gfx)
    }

    /// Lost focus or another button: the drag ends and the next click is a first one.
    pub(crate) fn cancel_text(&mut self) {
        self.form.cancel_text_pointer();
    }

    /// The edit menu of the text row at `(x, y)`, from its real selection. Secret rows get
    /// Cut and Copy disabled. `None` off a text row or while a popup is open.
    pub(crate) fn text_menu(&mut self, x: f32, y: f32) -> Option<Vec<Item<ContextAction>>> {
        self.text_row_at(x, y)?;
        let items = self.form.context_menu(Some((x, y)))?;
        self.rail_focus = false;
        Some(items)
    }

    /// A chosen edit menu action, on the row the menu was opened for.
    pub(crate) fn text_action(&mut self, action: ContextAction) -> SettingsEvent {
        SettingsEvent::Form(self.form.apply_context_action(action))
    }

    pub(in super::super) fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        if !self.peeking() && !self.layout.rail.contains(x, y) {
            self.form.wheel(x, y, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    use mdshared_ui::settings::Row;

    const MIN_WINDOW: Rect = Rect::new(0.0, 0.0, 760.0, 480.0);

    #[test]
    fn rail_controls_and_close_fit_at_minimum_size_with_tab_revealing_every_row() {
        let st = AppState::new(Source::Demo, Some(true), Default::default());
        let mut gfx = Gfx::new();
        for scale in [1.0, 1.5] {
            let mut bitmap = Bitmap::new((MIN_WINDOW.w * scale) as u32, (MIN_WINDOW.h * scale) as u32);
            let mut p = Painter::new(&mut bitmap, &mut gfx, scale);
            for (index, (label, _)) in PAGES.iter().enumerate() {
                let mut settings = Settings::new(&st, index);
                let viewport = settings.place(&mut p, MIN_WINDOW);
                let card = settings.form.layout(&mut p, viewport);
                let rail = settings.layout.rail;
                assert_eq!(card.y, rail.y);
                assert!(rail.right() < card.x && card.right() < MIN_WINDOW.right());
                assert!(card.bottom() < MIN_WINDOW.bottom() && rail.bottom() < MIN_WINDOW.bottom());
                let nav = settings.layout.category(index).inset(1.0);
                assert!(nav.bottom() < rail.bottom());
                assert!(p.measure(Face::Ui, 13.0, label) <= nav.w - 24.0, "{label}");
                let body = settings.form.body_rect();
                let controls: Vec<_> =
                    settings.form.rows().iter().enumerate().filter_map(|(i, row)| row.id().map(|id| (i, id))).collect();
                let mods = Mods::default();
                for (row, id) in controls {
                    settings.key(&st.config, &KeyName::Tab, mods, None);
                    assert_eq!(settings.form.focus_id(), Some(id), "{label}");
                    let control = settings.form.control_rect(row);
                    assert!(control.w > 0.0 && control.h > 0.0);
                    assert!(control.x >= body.x && control.right() <= body.right(), "{label}: {id}");
                    assert!(control.y >= body.y && control.bottom() <= body.bottom(), "{label}: {id}");
                    let (name, help) = match &settings.form.rows()[row] {
                        Row::Toggle { label, help, .. }
                        | Row::Choice { label, help, .. }
                        | Row::Number { label, help, .. }
                        | Row::Text { label, help, .. } => (label.as_str(), help.as_str()),
                        Row::Action { label, .. } => (label.as_str(), ""),
                        Row::Header(_) => unreachable!(),
                    };
                    // Match the toolkit form's single-line label slot.
                    let room = (control.x - settings.form.rect_of_row(row).x - 28.0).max(40.0);
                    assert!(p.measure(Face::Ui, 13.5, name) <= room, "{label}: {name}");
                    assert!(p.measure(Face::Ui, 11.5, help) <= room, "{label}: {help} (room {room})");
                }
                settings.key(&st.config, &KeyName::Tab, mods, None);
                let close = settings.form.close_rect();
                assert!(settings.form.focus_on_close());
                assert!(close.y >= body.bottom() && close.bottom() < card.bottom());
                settings.key(&st.config, &KeyName::Tab, mods, None);
                assert!(settings.rail_focus);
            }
        }
    }

    #[test]
    fn mouse_navigation_requires_matching_release_and_does_not_click_through_a_picker() {
        let st = AppState::new(Source::Demo, None, Default::default());
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(760, 480);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let mut settings = Settings::new(&st, 0);
        let viewport = settings.place(&mut p, MIN_WINDOW);
        settings.form.layout(&mut p, viewport);
        let first = settings.layout.category(0);
        let second = settings.layout.category(1);
        let mods = Mods::default();

        let swatch_index =
            settings.form.rows().iter().position(|r| r.id() == Some(super::super::SET_ACCENT_COLOUR)).unwrap();
        // Below the palette cards at this size: scroll it into view as Tab would.
        settings.form.focus_row(super::super::SET_ACCENT_COLOUR);
        let swatch = settings.form.swatch_rect(swatch_index);
        settings.press(swatch.cx(), swatch.cy(), mods, 1);
        settings.release(swatch.cx(), swatch.cy());
        assert!(settings.form.colour_open().is_some());
        assert!(matches!(settings.key(&st.config, &KeyName::Escape, mods, None), SettingsEvent::Form(Outcome::Redraw)));
        assert!(settings.form.colour_open().is_none());
        settings.press(swatch.cx(), swatch.cy(), mods, 1);
        settings.release(swatch.cx(), swatch.cy());

        assert!(matches!(settings.press(second.cx(), second.cy(), mods, 1), SettingsEvent::Form(Outcome::Redraw)));
        assert!(matches!(settings.release(second.cx(), second.cy()), SettingsEvent::Form(_)));
        assert!(settings.form.colour_open().is_none());
        settings.press(second.cx(), second.cy(), mods, 1);
        assert!(matches!(settings.release(first.cx(), first.cy()), SettingsEvent::Form(_)));
        settings.press(second.cx(), second.cy(), mods, 1);
        assert!(matches!(settings.release(second.cx(), second.cy()), SettingsEvent::Page(1)));
    }
    #[test]
    fn colour_rows_change_only_when_applied_and_an_outside_press_applies_without_clicking_through() {
        let st = AppState::new(Source::Demo, None, Default::default());
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(760, 480);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let mut settings = Settings::new(&st, 0);
        let viewport = settings.place(&mut p, MIN_WINDOW);
        settings.form.layout(&mut p, viewport);
        let mods = Mods::default();
        let id = super::super::SET_ACCENT_COLOUR;
        let before = settings.form.colour(id);
        let index = settings.form.rows().iter().position(|r| r.id() == Some(id)).unwrap();
        // Below the palette cards at this size: scroll it into view as Tab would.
        settings.form.focus_row(id);
        let open = |settings: &mut Settings, p: &mut Painter| {
            let opened = settings.form.colour(id);
            let swatch = settings.form.swatch_rect(index);
            settings.press(swatch.cx(), swatch.cy(), mods, 1);
            settings.release(swatch.cx(), swatch.cy());
            assert!(settings.colour_open());
            settings.form.layout(p, viewport);
            let buttons = settings.form.picker_confirmation_layout().expect("a confirming picker");
            assert!(buttons.apply.w > 0.0 && buttons.cancel.w > 0.0);
            // A press in the square changes the preview only.
            let square = buttons.picker.square;
            let (x, y) = (square.x + 4.0, square.y + 4.0);
            assert!(!matches!(settings.press(x, y, mods, 1), SettingsEvent::Form(Outcome::Changed { .. })));
            assert!(!matches!(settings.release(x, y), SettingsEvent::Form(Outcome::Changed { .. })));
            assert_eq!(settings.form.colour(id), opened, "staged, not applied");
        };

        open(&mut settings, &mut p);
        settings.key(&st.config, &KeyName::Escape, mods, None);
        assert!(!settings.colour_open());
        assert_eq!(settings.form.colour(id), before, "Escape restores");

        open(&mut settings, &mut p);
        assert!(
            matches!(settings.key(&st.config,&KeyName::Enter, mods, None), SettingsEvent::Form(Outcome::Changed { id: c, .. }) if c == id)
        );
        assert!(!settings.colour_open());
        let applied = settings.form.colour(id);
        assert_ne!(applied, before);

        // An outside press on the rail applies and does not open that page.
        settings.form.set_value(id, mdshared_ui::settings::Value::Text("#123456".into()));
        open(&mut settings, &mut p);
        let second = settings.layout.category(1);
        assert!(matches!(
            settings.press(second.cx(), second.cy(), mods, 1),
            SettingsEvent::Form(Outcome::Changed { .. })
        ));
        assert!(!settings.colour_open());
        assert!(matches!(settings.release(second.cx(), second.cy()), SettingsEvent::Form(_)));
    }

    #[test]
    fn a_category_switch_cross_fades_briefly_and_not_at_all_with_reduced_motion() {
        use mdshared_ui::anim::Policy;
        use std::time::{Duration, Instant};

        let st = AppState::new(Source::Demo, Some(true), Default::default());
        let window = Rect::new(0.0, 0.0, 900.0, 560.0);
        // Draws a frame `ms` after `t0` and gives the pixels.
        let frame = |settings: &mut Settings, anim: &mut Animator, at: Instant| {
            let mut gfx = Gfx::new();
            let mut bitmap = Bitmap::new(900, 560);
            let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
            anim.begin_frame(at);
            settings.draw(&mut p, &st, window, 1.0, anim);
            anim.end_frame();
            bitmap.pixels
        };
        let t0 = Instant::now();
        for (policy, reduced) in [(None, false), (Some(Policy::Never), false), (None, true)] {
            let mut anim = Animator::new();
            if let Some(policy) = policy {
                anim.set_policy(policy);
            }
            anim.set_system_reduced(reduced);
            let mut settings = Settings::new(&st, 0);
            frame(&mut settings, &mut anim, t0);
            settings.show_page(&st, 1);
            let during = frame(&mut settings, &mut anim, t0 + Duration::from_millis(40));
            let mut plain = Settings::new(&st, 1);
            let new_page = frame(&mut plain, &mut Animator::new(), t0);
            let moves = policy.is_none() && !reduced;
            assert_eq!(settings.switching(), moves);
            assert_eq!(during != new_page, moves, "the old page shows only while it fades");
            // Frames 16 ms apart, as the window draws them while something moves.
            let mut at = t0 + Duration::from_millis(40);
            let mut after = during;
            while at < t0 + Duration::from_millis(40) + SWITCH + Duration::from_millis(32) {
                at += Duration::from_millis(16);
                after = frame(&mut settings, &mut anim, at);
                if at <= t0 + Duration::from_millis(40) + SWITCH / 2 {
                    assert_eq!(settings.switching(), moves, "still blending halfway");
                }
            }
            assert!(!settings.switching());
            assert_eq!(after, new_page);
        }
        assert!((120..=160).contains(&SWITCH.as_millis()));
    }
    #[test]
    fn text_rows_select_with_the_mouse_and_the_menu_follows_the_real_selection() {
        use std::time::Duration;
        let st = AppState::new(Source::Demo, None, Default::default());
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(900, 700);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let index = PAGES.iter().position(|(name, _)| *name == super::super::HANDOVERS).unwrap();
        let mut settings = Settings::new(&st, index);
        let window = Rect::new(0.0, 0.0, 900.0, 700.0);
        let viewport = settings.place(&mut p, window);
        settings.form.layout(&mut p, viewport);
        let mods = Mods::default();
        let (row, id) = settings
            .form
            .rows()
            .iter()
            .enumerate()
            .find_map(|(i, r)| match r {
                Row::Text { id, value, .. } if !value.is_empty() => Some((i, *id)),
                _ => None,
            })
            .expect("a text row with text");
        let field = settings.form.control_rect(row);
        let (x, y) = (field.x + 24.0, field.cy());
        assert_eq!(settings.text_row_at(x, y), Some(id));
        let mut tk = Gfx::new();
        let t0 = Instant::now();

        // A single click places the caret: nothing to cut or copy.
        settings.pointer_press(x, y, mods, 1, t0, &mut tk, 1.0);
        settings.text_release(x, y, t0, &mut tk);
        let enabled = |items: Vec<Item<ContextAction>>, action| items.iter().any(|i| i.id == action && i.enabled);
        let menu = settings.text_menu(x, y).expect("a menu on a text row");
        assert!(!enabled(menu, ContextAction::Copy));

        // A triple click takes the whole row, which the menu now offers to copy.
        for (k, clicks) in [(1, 2u8), (2, 3)] {
            let at = t0 + Duration::from_millis(100 * k);
            settings.pointer_press(x, y, mods, clicks, at, &mut tk, 1.0);
            settings.text_release(x, y, at, &mut tk);
        }
        let menu = settings.text_menu(x, y).unwrap();
        assert!(enabled(menu.clone(), ContextAction::Copy) && enabled(menu, ContextAction::Cut));

        // An open dropdown covers the text rows: no I-beam, no edit menu.
        // Enter opens a dropdown choice (a segmented one picks its next option instead).
        let choices: Vec<u32> = settings
            .form
            .rows()
            .iter()
            .filter_map(|r| matches!(r, Row::Choice { .. }).then(|| r.id()).flatten())
            .collect();
        for choice in choices {
            settings.form.focus_row(choice);
            settings.form.key(&KeyName::Enter, mods, None);
            if settings.form.popup_open() {
                break;
            }
        }
        assert!(settings.form.popup_open(), "a dropdown on the page opened");
        assert_eq!(settings.text_row_at(x, y), None);
        assert!(settings.text_menu(x, y).is_none());
    }

    // ---- Peek ---------------------------------------------------------------------------------

    const PEEK_WINDOW: Rect = Rect::new(0.0, 0.0, 1000.0, 700.0);

    /// The demo's sessions, for a window that looks like one.
    fn peek_state(dark: bool) -> AppState {
        let mut st = AppState::new(Source::Demo, Some(dark), Default::default());
        st.apply_scan(crate::demo::scan(&st.config));
        st
    }

    /// The window under Settings: its page, title bar, session list and details.
    fn window_under(st: &mut AppState, p: &mut Painter) {
        use mdshared_ui::splitter::Axis;
        let pal = *st.pal();
        if st.config.appearance.backdrop != "off" {
            // Mica or Acrylic: the key colour is where the compositor's material shows.
            let key = mdshared_ui::frost::key_colour(&pal);
            p.set_frost_key(Some(mdshared_ui::frost::Key::new(key, 217)));
        }
        p.fill(PEEK_WINDOW, pal.page);
        crate::topbar::draw(st, p, PEEK_WINDOW.w, false, false, false);
        let top = mdshared_ui::chrome::TITLE_H;
        let area = Rect::new(0.0, top, PEEK_WINDOW.w, PEEK_WINDOW.h - top);
        let rects = st.splitter.rects(area, Axis::Horizontal);
        crate::list::draw_in(st, &mut Animator::new(), p, rects.0);
        crate::details::draw(st, p, rects.2);
        mdshared_ui::splitter::draw(p, &pal, rects, Axis::Horizontal, false);
    }

    /// One frame: the window, then Settings (or what a peek leaves of it). True while the
    /// peek's fade moves.
    fn peek_frame(st: &mut AppState, s: &mut Settings, anim: &mut Animator, at: Instant, scale: f32) -> (Bitmap, bool) {
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new((PEEK_WINDOW.w * scale) as u32, (PEEK_WINDOW.h * scale) as u32);
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        anim.begin_frame(at);
        window_under(st, &mut p);
        let moving = s.draw_layer(&mut p, st, PEEK_WINDOW, 1.0, anim);
        anim.end_frame();
        (bmp, moving)
    }

    fn base_frame(st: &mut AppState, scale: f32) -> Bitmap {
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new((PEEK_WINDOW.w * scale) as u32, (PEEK_WINDOW.h * scale) as u32);
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        window_under(st, &mut p);
        bmp
    }

    fn pill_at(scale: f32) -> Rect {
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(8, 8);
        peek::pill_rect(&mut Painter::new(&mut bmp, &mut gfx, scale), PEEK_WINDOW)
    }

    /// Pixels that differ between two frames, outside `except` (logical px).
    fn differing(a: &Bitmap, b: &Bitmap, except: Rect, scale: f32) -> usize {
        let e = Rect::new(except.x * scale, except.y * scale, except.w * scale, except.h * scale);
        let mut n = 0;
        for y in 0..a.height {
            for x in 0..a.width {
                if e.contains(x as f32 + 0.5, y as f32 + 0.5) {
                    continue;
                }
                let i = ((y * a.width + x) * 4) as usize;
                n += usize::from(a.pixels[i..i + 3] != b.pixels[i..i + 3]);
            }
        }
        n
    }

    fn shot(bmp: &Bitmap, name: &str) {
        if let Some(dir) = std::env::var_os("CS_PEEK_SHOTS") {
            bmp.write_png(&std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
        }
    }

    #[test]
    fn holding_peek_shows_the_window_exactly_and_letting_go_brings_settings_back_as_it_was() {
        use std::time::Duration;
        for (dark, name) in [(true, "dark"), (false, "light")] {
            let mut st = peek_state(dark);
            for scale in [1.0, 1.5] {
                let mut s = Settings::new(&st, 0);
                let mut anim = Animator::new();
                let t0 = Instant::now();
                let (open, _) = peek_frame(&mut st, &mut s, &mut anim, t0, scale);
                let base = base_frame(&mut st, scale);
                if scale == 1.0 {
                    shot(&open, &format!("peek-{name}-1-settings-open"));
                }
                assert!(differing(&open, &base, Rect::default(), scale) > 10_000, "Settings and its dim are drawn");

                // The button: at the card's top right, on the title's line, above the rows.
                let card = s.form.rect();
                let b = s.peek.button;
                assert!(b.right() <= card.right() - 20.0 && b.x > card.cx() && b.y > card.y);
                assert!(b.bottom() < s.form.body_rect().y);

                // Some state to keep: a scroll, keyboard focus on a row.
                s.form.wheel(card.cx(), card.cy(), 60.0);
                s.form.focus_row(super::super::SET_CORNERS);
                s.rail_focus = false;
                let kept = (s.index, s.form.scroll(), s.form.focus_id(), s.rail_focus);

                assert!(s.pointer_move(b.cx(), b.cy()));
                assert_eq!(s.peek_tip(), Some(b));
                assert!(matches!(s.press(b.cx(), b.cy(), Mods::default(), 1), SettingsEvent::Form(Outcome::Redraw)));
                assert!(s.peeking() && s.peek_tip().is_none());

                // Halfway both the window and Settings show; then the window alone and the pill.
                let (_, moving) = peek_frame(&mut st, &mut s, &mut anim, t0 + Duration::from_millis(16), scale);
                assert!(moving);
                let (half, moving) = peek_frame(&mut st, &mut s, &mut anim, t0 + Duration::from_millis(60), scale);
                assert!(moving, "the fade takes about 120 ms");
                assert!(differing(&half, &open, Rect::default(), scale) > 1_000);
                assert!(differing(&half, &base, Rect::default(), scale) > 1_000);
                let mut at = t0 + Duration::from_millis(60);
                let mut peeking = None;
                while at < t0 + Duration::from_millis(200) {
                    at += Duration::from_millis(16);
                    peeking = Some(peek_frame(&mut st, &mut s, &mut anim, at, scale));
                }
                let (peeking, moving) = peeking.unwrap();
                assert!(!moving);
                let pill = pill_at(scale);
                // The window as it is now: the list's times ("2 min ago") follow the clock, and a
                // slow run can cross a minute since the first frame.
                let base = base_frame(&mut st, scale);
                assert_eq!(
                    differing(&peeking, &base, peek::pill_bounds(pill), scale),
                    0,
                    "exactly the window, but the pill"
                );
                assert!(differing(&peeking, &base, Rect::default(), scale) > 0, "the pill shows");
                if scale == 1.0 {
                    shot(&half, &format!("peek-{name}-2-fading"));
                    shot(&peeking, &format!("peek-{name}-3-peeking"));
                }

                // Nothing else does anything while peeking.
                let second = s.layout.category(1);
                let mods = Mods::default();
                assert!(matches!(s.press(second.cx(), second.cy(), mods, 1), SettingsEvent::Form(Outcome::None)));
                for key in [KeyName::Tab, KeyName::Down, KeyName::Escape, KeyName::Space, KeyName::Right] {
                    assert!(matches!(s.key(&st.config, &key, mods, None), SettingsEvent::Form(Outcome::None)));
                }
                s.wheel(card.cx(), card.cy(), 200.0);
                assert!(!s.pointer_move(card.cx(), card.cy()));
                assert_eq!(s.text_row_at(card.cx(), card.cy()), None);

                // Let go anywhere, outside the window too: Settings comes back as it was.
                assert!(matches!(s.release(-40.0, -40.0), SettingsEvent::Form(Outcome::Redraw)));
                assert!(!s.peeking());
                assert_eq!((s.index, s.form.scroll(), s.form.focus_id(), s.rail_focus), kept);
                let (_, moving) = peek_frame(&mut st, &mut s, &mut anim, t0 + Duration::from_millis(232), scale);
                assert!(moving, "it fades back in");
                let mut at = t0 + Duration::from_millis(232);
                let mut back = None;
                while at < t0 + Duration::from_millis(400) {
                    at += Duration::from_millis(16);
                    back = Some(peek_frame(&mut st, &mut s, &mut anim, at, scale));
                }
                let (back, moving) = back.unwrap();
                assert!(!moving);
                assert!(differing(&back, &base, Rect::default(), scale) > 10_000);
            }
        }
    }

    #[test]
    fn reduce_motion_and_the_systems_reduced_motion_snap_the_peek() {
        use mdshared_ui::anim::Policy;
        let mut st = peek_state(true);
        for (policy, reduced) in [(Some(Policy::Never), false), (None, true)] {
            let mut anim = Animator::new();
            if let Some(policy) = policy {
                anim.set_policy(policy);
            }
            anim.set_system_reduced(reduced);
            let mut s = Settings::new(&st, 0);
            let t0 = Instant::now();
            peek_frame(&mut st, &mut s, &mut anim, t0, 1.0);
            let b = s.peek.button;
            s.press(b.cx(), b.cy(), Mods::default(), 1);
            let (peeking, moving) = peek_frame(&mut st, &mut s, &mut anim, t0, 1.0);
            assert!(!moving);
            let base = base_frame(&mut st, 1.0);
            assert_eq!(differing(&peeking, &base, peek::pill_bounds(pill_at(1.0)), 1.0), 0);
            s.release(b.cx(), b.cy());
            let (_, moving) = peek_frame(&mut st, &mut s, &mut anim, t0, 1.0);
            assert!(!moving);
        }
    }

    #[test]
    fn space_peeks_only_when_what_has_the_keyboard_does_not_use_space() {
        let st = AppState::new(Source::Demo, Some(true), Default::default());
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(1000, 700);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let mut s = Settings::new(&st, 0);
        let mut anim = Animator::new();
        anim.begin_frame(Instant::now());
        s.draw(&mut p, &st, PEEK_WINDOW, 1.0, &mut anim);
        let mods = Mods::default();
        let space = |s: &mut Settings, mods: Mods| s.key(&st.config, &KeyName::Space, mods, Some(" "));

        // The category list does nothing with Space: it peeks, repeats do nothing, release ends.
        assert!(s.rail_focus);
        assert!(matches!(space(&mut s, mods), SettingsEvent::Form(Outcome::Redraw)));
        assert_eq!(s.peek.by, Some(peek::By::Space));
        assert!(matches!(space(&mut s, mods), SettingsEvent::Form(Outcome::None)));
        assert!(!s.end_peek(peek::By::Mouse), "the mouse's release doesn't end Space's peek");
        assert!(s.end_peek(peek::By::Space) && !s.peeking());
        // With a modifier it is someone else's shortcut.
        for m in [Mods { ctrl: true, ..mods }, Mods { alt: true, ..mods }, Mods { shift: true, ..mods }] {
            space(&mut s, m);
            assert!(!s.peeking());
        }

        // A switch, a segmented choice and a palette card keep their Space.
        s.rail_focus = false;
        for id in [super::super::SET_REDUCE_MOTION, super::super::SET_THEME, tiles::DARK] {
            s.form.focus_row(id);
            let o = space(&mut s, mods);
            assert!(matches!(o, SettingsEvent::Form(Outcome::Changed { id: c, .. }) if c == id), "{id}");
            assert!(!s.peeking(), "{id}");
        }
        // A colour row's text box keeps it too.
        s.form.focus_row(super::super::SET_ACCENT_COLOUR);
        space(&mut s, mods);
        assert!(!s.peeking());
        s.form.key(&KeyName::Escape, mods, None);
        // A number does nothing with Space: it peeks.
        s.form.focus_row(super::super::SET_CORNERS);
        assert!(matches!(space(&mut s, mods), SettingsEvent::Form(Outcome::Redraw)));
        assert!(s.peeking());
        assert!(s.cancel_peek());
        // The Close button keeps it.
        s.form.focus_row(super::super::SET_CORNERS);
        while !s.form.focus_on_close() {
            s.form.key(&KeyName::Tab, mods, None);
        }
        space(&mut s, mods);
        assert!(!s.peeking());
    }

    /// Pixels of the frost key colour: where Mica or Acrylic shows through.
    fn see_through(bmp: &Bitmap, key: mdshared_ui::gfx::Rgb) -> usize {
        bmp.pixels.as_chunks::<4>().0.iter().filter(|px| [px[2], px[1], px[0]] == key).count()
    }

    #[test]
    fn with_mica_or_acrylic_a_peek_leaves_the_material_showing_as_without_settings() {
        use std::time::Duration;
        for (dark, backdrop) in [(true, "mica"), (false, "acrylic")] {
            let mut st = peek_state(dark);
            st.config.appearance.backdrop = backdrop.into();
            let key = mdshared_ui::frost::key_colour(st.pal());
            let mut s = Settings::new(&st, 0);
            let mut anim = Animator::new();
            let t0 = Instant::now();
            let (open, _) = peek_frame(&mut st, &mut s, &mut anim, t0, 1.0);
            let base = base_frame(&mut st, 1.0);
            assert!(see_through(&base, key) > 10_000, "{backdrop}: the window has material showing");
            assert!(see_through(&open, key) < see_through(&base, key) / 4, "{backdrop}: the dim covers it");
            let b = s.peek.button;
            s.press(b.cx(), b.cy(), Mods::default(), 1);
            let mut at = t0;
            let mut last = None;
            while at < t0 + Duration::from_millis(200) {
                at += Duration::from_millis(16);
                last = Some(peek_frame(&mut st, &mut s, &mut anim, at, 1.0));
            }
            let (peeking, moving) = last.unwrap();
            assert!(!moving);
            assert_eq!(differing(&peeking, &base, peek::pill_bounds(pill_at(1.0)), 1.0), 0, "{backdrop}");
            let pill = peek::pill_bounds(pill_at(1.0));
            let pill_area = (pill.w * pill.h) as usize;
            assert!(see_through(&peeking, key) + pill_area >= see_through(&base, key), "{backdrop}: no dim left");
        }
    }

    #[test]
    fn every_page_has_peek_at_the_same_place_and_it_peeks_there() {
        let mut st = peek_state(true);
        let mut place = None;
        for (index, (name, _)) in PAGES.iter().enumerate() {
            let mut s = Settings::new(&st, index);
            let mut anim = Animator::new();
            peek_frame(&mut st, &mut s, &mut anim, Instant::now(), 1.0);
            let b = s.peek.button;
            let card = s.form.rect();
            assert!(b.w > 0.0 && b.y > card.y && b.bottom() < s.form.body_rect().y, "{name}");
            // Short pages have a shorter card; its top and right edge, and so the button, stay.
            assert_eq!(*place.get_or_insert(b), b, "{name}: the same place on every page");
            assert!(b.right() <= card.right() - 20.0, "{name}");
            s.press(b.cx(), b.cy(), Mods::default(), 1);
            assert!(s.peeking(), "{name}");
            s.release(b.cx(), b.cy());
            assert!(!s.peeking() && s.index == index, "{name}");
        }
    }
}
