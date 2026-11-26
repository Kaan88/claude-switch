//! One native settings form at a time. Its controls (including swatches and popovers) keep
//! their existing input handling; the app owns only the category rail and page lifetime.

use super::{AppState, Form, PAGES, SettingsEvent, page_form, terminal_appearance};
use mdshared_ui::anim::Animator;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::overlay::draw_backdrop_presence;
use mdshared_ui::settings::{Outcome, Style};

const NAV_W: f32 = 146.0;
const NAV_ROW: f32 = 32.0;
const NAV_TOP: f32 = 54.0;
const GAP: f32 = 8.0;
// Allow the native form's own dialog margins (16 on each side and 24 above/below).
const SIDE: f32 = 16.0;
const TOP: f32 = 24.0;
const MAX_PAGE_W: f32 = 600.0;
const MAX_PAGE_H: f32 = 560.0;

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

pub struct Settings {
    index: usize,
    pub(in super::super) form: Form,
    rail_focus: bool,
    key_used: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
    layout: Layout,
}

impl Settings {
    pub(super) fn new(st: &AppState, index: usize) -> Self {
        Self {
            index,
            form: page_form(st, index),
            rail_focus: true,
            key_used: false,
            hover: None,
            pressed: None,
            layout: Layout::default(),
        }
    }

    pub(in super::super) fn category(&self) -> &'static str {
        PAGES[self.index].0
    }

    pub(super) fn show_page(&mut self, st: &AppState, index: usize) {
        if index != self.index {
            self.form = page_form(st, index);
            self.index = index;
        }
    }

    fn place(&mut self, p: &mut Painter, window: Rect) -> Rect {
        self.layout = Layout::new(window);
        let mut viewport = self.layout.viewport;
        // The form centers the card in its viewport. Remove unused height to keep the
        // top of the controls aligned with the rail even on a short page.
        let card = self.form.layout(p, viewport);
        viewport.h = card.h + 2.0 * TOP;
        viewport
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
        let radius = st.appearance.radius;
        let shadow = if st.high_contrast { 0.0 } else { st.appearance.shadow };
        if self.category() == super::TERMINAL {
            terminal_appearance::sync(&mut self.form, &st.config.appearance, st.high_contrast, p.frost_key().is_some());
        }
        let viewport = self.place(p, window);
        draw_backdrop_presence(p, pal, window.w, window.h, presence);
        let rail = self.layout.rail;
        let shadow = shadow * if pal.dark { 1.0 } else { mdshared_ui::gfx::LIGHT_SHADOW };
        p.soft_shadow(rail, radius, &mdshared_ui::gfx::Shade::POPUP, shadow);
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
        style.soft_shadow = Some(if st.high_contrast { 0.0 } else { st.appearance.shadow });
        style.backdrop = 0.0;
        self.form.set_style(style);
        self.form.set_high_contrast(st.high_contrast);
        self.form.draw_animated(p, pal, viewport, true, anim, ("settings", self.category()));
    }

    pub(crate) fn key(&mut self, key: &KeyName, mods: Mods, text: Option<&str>) -> SettingsEvent {
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
        let first = self.first_row();
        let was_first = self.form.focus_id() == first;
        let was_close = self.form.focus_on_close();
        let outcome = self.form.key(key, mods, text);
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

    pub(crate) fn press(&mut self, x: f32, y: f32, mods: Mods, clicks: u8) -> SettingsEvent {
        self.pressed = None;
        self.key_used = false;
        let outcome = self.form.press(x, y, mods, clicks);
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
        if let Some(index) = self.pressed.take()
            && self.layout.hit(x, y) == Some(index)
        {
            return SettingsEvent::Page(index);
        }
        SettingsEvent::Form(self.form.release(x, y))
    }

    pub(crate) fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let hover = self.layout.hit(x, y);
        let changed = std::mem::replace(&mut self.hover, hover) != hover;
        self.form.pointer_move(x, y) || changed
    }

    pub(in super::super) fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        if !self.layout.rail.contains(x, y) {
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
                    settings.key(&KeyName::Tab, mods, None);
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
                settings.key(&KeyName::Tab, mods, None);
                let close = settings.form.close_rect();
                assert!(settings.form.focus_on_close());
                assert!(close.y >= body.bottom() && close.bottom() < card.bottom());
                settings.key(&KeyName::Tab, mods, None);
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
        let swatch = settings.form.swatch_rect(swatch_index);
        settings.press(swatch.cx(), swatch.cy(), mods, 1);
        settings.release(swatch.cx(), swatch.cy());
        assert!(settings.form.colour_open().is_some());
        assert!(matches!(settings.key(&KeyName::Escape, mods, None), SettingsEvent::Form(Outcome::Redraw)));
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
}
