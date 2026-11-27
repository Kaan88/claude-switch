//! Peek: while the Peek button is held, or Space with nothing focused that uses it, the dim and
//! the Settings dialog fade away and the window shows as it is without them, so a palette,
//! transparency, terminal colours or the Chat reading width can be judged. Letting go brings
//! Settings back as it was. Settings itself never changes while peeking: the app swallows every
//! other input (see `AppState::peek_event`), and focus loss ends the peek.
use mdshared_ui::anim::{Motion, fluent};
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::{Face, Painter, Rect, Shade};
use mdshared_ui::glyph;
use mdshared_ui::theme::Palette;

use crate::state::AppState;
use crate::windowlook::Cards;

/// What holds the peek open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum By {
    Mouse,
    Space,
}

#[derive(Default)]
pub(super) struct Peek {
    pub(super) by: Option<By>,
    pub(super) hover: bool,
    /// The button as last drawn.
    pub(super) button: Rect,
}

pub(crate) const PEEK_TIP: &str = "Hold to see the window without Settings";
pub(crate) const PILL: &str = "Peeking · let go to return to Settings";
const LABEL: &str = "Peek";
/// About 120 ms each way; the animator snaps it under Reduce motion.
pub(super) const MOTION: Motion = Motion::tween_ms(120, fluent::DECELERATE);
/// The animator's name for how present Settings is: 1 shown, 0 peeking.
pub(super) const SHOWN: &str = "settings-peek";
const BUTTON_H: f32 = 28.0;
const PILL_H: f32 = 32.0;

/// The button at the top right of the page's card, centred on its title's line.
pub(super) fn draw_button(p: &mut Painter, pal: &Palette, card: Rect, peek: &Peek) -> Rect {
    let w = (12.0 + 16.0 + 6.0 + p.measure(Face::Ui, 13.0, LABEL) + 14.0).ceil();
    let r = Rect::new(card.right() - mdshared_ui::overlay::DIALOG_PAD - w, card.y + 20.0, w, BUTTON_H);
    let fill = if peek.by == Some(By::Mouse) {
        pal.press
    } else if peek.hover {
        pal.hover
    } else {
        pal.popup
    };
    p.frame(r, 6.0, fill, pal.popup_line);
    p.icon(glyph::VIEW, 15.0, r.x + 12.0 + 8.0, r.cy(), pal.text);
    p.text(Face::Ui, 13.0, r.x + 12.0 + 16.0 + 6.0, r.cy(), LABEL, pal.text, r.w);
    r
}

/// Where the pill sits: centred at the top of the window, under the title bar.
pub(super) fn pill_rect(p: &mut Painter, window: Rect) -> Rect {
    let w = (14.0 + 16.0 + 8.0 + p.measure(Face::Ui, 13.0, PILL) + 16.0).ceil().min(window.w - 16.0);
    Rect::new((window.x + (window.w - w) / 2.0).round(), window.y + TITLE_H + 10.0, w, PILL_H)
}

/// Everything the pill touches, its soft shadow (8 px down, 24 px of blur) included.
pub(super) fn pill_bounds(pill: Rect) -> Rect {
    Rect::new(pill.x - 48.0, pill.y - 48.0, pill.w + 96.0, pill.h + 104.0)
}

/// The pill, `presence` of the way in.
pub(super) fn draw_pill(p: &mut Painter, st: &AppState, window: Rect, presence: f32) {
    if presence <= 0.0 {
        return;
    }
    let pal = st.pal();
    let cards = Cards::for_frame(st, p);
    let r = pill_rect(p, window);
    let radius = cards.decoration(PILL_H / 2.0);
    let room = pill_bounds(r);
    let under = (presence < 1.0).then(|| p.snapshot(room));
    let shadow = if st.high_contrast { 0.0 } else { cards.decoration(st.appearance.shadow) };
    if shadow > 0.0 {
        let k = shadow * if pal.dark { 1.0 } else { mdshared_ui::gfx::LIGHT_SHADOW };
        p.soft_shadow(r, radius, &Shade::POPUP, k);
    }
    cards.draw(p, r, pal.popup, |p| {
        p.rbox(r, radius, pal.accent);
        p.rbox(r.inset(1.5), (radius - 1.5).max(0.0), pal.popup);
        p.icon(glyph::VIEW, 15.0, r.x + 14.0 + 8.0, r.cy(), pal.accent);
        let x = r.x + 14.0 + 16.0 + 8.0;
        p.text(Face::Ui, 13.0, x, r.cy(), PILL, pal.text, r.right() - 12.0 - x);
    });
    if let Some(under) = under {
        p.fade(&under, presence);
    }
}

/// What a window event does while Settings is open, as far as Peek is concerned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PeekInput {
    /// Not Peek's business: handle it as usual.
    Pass,
    /// Peeking: the event does nothing at all.
    Swallow,
    /// The peek ended; draw Settings again.
    Redraw,
}

impl AppState {
    fn peek_settings(&mut self) -> Option<&mut super::Settings> {
        match &mut self.dialog {
            Some(crate::dialogs::Dialog::Settings(s)) => Some(s),
            _ => None,
        }
    }

    /// Whether a peek is holding Settings out of sight.
    pub(crate) fn settings_peeking(&self) -> bool {
        matches!(&self.dialog, Some(crate::dialogs::Dialog::Settings(s)) if s.peeking())
    }

    /// The Peek button while it shows its tooltip.
    pub(crate) fn settings_peek_tip(&self) -> Option<Rect> {
        match &self.dialog {
            Some(crate::dialogs::Dialog::Settings(s)) => s.peek_tip(),
            _ => None,
        }
    }

    /// While peeking, only the release of what started it does anything: the left button for
    /// the button (wherever it is let go), Space for Space (see [`Self::space_released`]).
    /// Losing the keyboard (Alt+Tab, a click in another window) ends any peek and is then
    /// handled as usual.
    pub(crate) fn peek_event(&mut self, event: &mdshared_ui::host::Event) -> PeekInput {
        use mdshared_ui::host::{Button, Event};
        let Some(settings) = self.peek_settings() else { return PeekInput::Pass };
        if !settings.peeking() {
            return PeekInput::Pass;
        }
        match *event {
            Event::Focus(false) => {
                settings.cancel_peek();
                PeekInput::Pass
            }
            Event::Release { button: Button::Left, .. } if settings.end_peek(By::Mouse) => PeekInput::Redraw,
            Event::PointerMove { x, y } => {
                self.mouse = (x, y);
                PeekInput::Swallow
            }
            Event::Press { .. }
            | Event::Release { .. }
            | Event::Wheel { .. }
            | Event::Key(_)
            | Event::Text(_)
            | Event::Preedit(_) => PeekInput::Swallow,
            _ => PeekInput::Pass,
        }
    }

    /// Space was let go: ends a peek Space started. True if one ended.
    pub(crate) fn space_released(&mut self) -> bool {
        self.peek_settings().is_some_and(|s| s.end_peek(By::Space))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use mdshared_ui::host::{Button, Event, KeyPress};
    use mdshared_ui::keys::{KeyName, Mods};

    fn open(dark: bool) -> AppState {
        let mut st = AppState::new(Source::Demo, Some(dark), Default::default());
        st.dialog = Some(crate::dialogs::settings(&mut st, None));
        st
    }

    /// Space on the category list (where Settings opens) starts a peek.
    fn space_down(st: &mut AppState) {
        let config = st.config.clone();
        let s = st.peek_settings().unwrap();
        s.key(&config, &KeyName::Space, Mods::default(), Some(" "));
        assert!(st.settings_peeking());
    }

    fn key(name: KeyName) -> Event {
        Event::Key(KeyPress { name: Some(name), mods: Mods::default(), text: None, repeat: false })
    }

    #[test]
    fn while_peeking_only_the_release_does_anything_and_focus_loss_ends_it() {
        let mut st = open(true);
        assert_eq!(st.peek_event(&key(KeyName::Space)), PeekInput::Pass, "not peeking: as usual");
        space_down(&mut st);
        let swallowed = [
            Event::Press { x: 10.0, y: 10.0, button: Button::Left, clicks: 1 },
            Event::Press { x: 10.0, y: 10.0, button: Button::Right, clicks: 1 },
            // Space's peek is not the button's: a left release does nothing.
            Event::Release { x: 10.0, y: 10.0, button: Button::Left },
            Event::Wheel { dx: 0.0, dy: 40.0, precise: false },
            key(KeyName::Escape),
            Event::Key(KeyPress { name: Some(KeyName::Space), mods: Mods::default(), text: None, repeat: true }),
            Event::Text("x".into()),
            Event::PointerMove { x: 300.0, y: 200.0 },
        ];
        for event in &swallowed {
            assert_eq!(st.peek_event(event), PeekInput::Swallow, "{event:?}");
            assert!(st.settings_peeking(), "{event:?}");
        }
        assert_eq!(st.mouse, (300.0, 200.0));
        assert_eq!(st.peek_event(&Event::Resize { w: 900.0, h: 600.0 }), PeekInput::Pass);
        assert!(st.space_released());
        assert!(!st.settings_peeking());
        assert!(!st.space_released(), "nothing left to end");
        assert!(matches!(st.dialog, Some(crate::dialogs::Dialog::Settings(_))), "Settings stays open");

        // Alt+Tab, a click in another window: the keyboard goes and the peek with it.
        space_down(&mut st);
        assert_eq!(st.peek_event(&Event::Focus(false)), PeekInput::Pass);
        assert!(!st.settings_peeking());
        assert!(matches!(st.dialog, Some(crate::dialogs::Dialog::Settings(_))));
    }

    #[test]
    fn the_buttons_peek_ends_on_a_left_release_anywhere() {
        let mut st = open(false);
        let s = st.peek_settings().unwrap();
        // Lay the button out.
        let mut gfx = mdshared_ui::gfx::Gfx::new();
        let mut bmp = mdshared_text::Bitmap::new(1000, 700);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let mut anim = mdshared_ui::anim::Animator::new();
        anim.begin_frame(std::time::Instant::now());
        let window = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let st_view = AppState::new(Source::Demo, Some(false), Default::default());
        s.draw_layer(&mut p, &st_view, window, 1.0, &mut anim);
        let b = s.peek.button;
        s.press(b.cx(), b.cy(), Mods::default(), 1);
        assert!(st.settings_peeking());
        assert!(!st.space_released(), "Space's release doesn't end the button's peek");
        assert_eq!(st.peek_event(&Event::Release { x: 0.0, y: 0.0, button: Button::Right }), PeekInput::Swallow);
        assert_eq!(st.peek_event(&Event::Release { x: -80.0, y: 900.0, button: Button::Left }), PeekInput::Redraw);
        assert!(!st.settings_peeking());
        assert!(st.settings_peek_tip().is_none() || st.settings_peek_tip() == Some(b));
    }
}
