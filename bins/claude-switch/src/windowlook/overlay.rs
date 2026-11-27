//! The native painter draws rounded edges and shadows assuming an opaque destination.
//! On a translucent terminal, confine those widgets to their own solid card. Do this
//! *inside* entrance animations: the toolkit can fade a finished card in premultiplied BGRA.

use mdshared_ui::{
    gfx::{Painter, Rect, Rgb},
    menu::Look,
    overlay::draw_backdrop_presence,
    theme::Palette,
};

use crate::state::AppState;

#[derive(Clone, Copy)]
pub(crate) enum Cards {
    Native,
    OnTerminal,
}

impl Cards {
    pub(crate) fn for_frame(st: &AppState, p: &Painter) -> Self {
        if st.terminal_shown()
            && (!st.chat_active() || st.term_area.h > 0.0)
            && st.config.appearance.terminal_opacity < 100
            && p.frost_key().is_some()
        {
            Self::OnTerminal
        } else {
            Self::Native
        }
    }

    // A square frame needs no exterior antialiasing or shadow.
    pub(crate) fn decoration(self, normal: f32) -> f32 {
        match self {
            Self::Native => normal,
            Self::OnTerminal => 0.0,
        }
    }

    pub(crate) fn menu(self, mut look: Look) -> Look {
        if let Self::OnTerminal = self {
            look.radius = 0.0;
            look.shadow = false;
        }
        look
    }

    pub(crate) fn draw<T>(self, p: &mut Painter, rect: Rect, ground: Rgb, draw: impl FnOnce(&mut Painter) -> T) -> T {
        if let Self::Native = self {
            return draw(p);
        }
        let clip = p.clip(rect);
        p.fill(rect, ground);
        let value = draw(p);
        p.restore(clip);
        value
    }

    pub(crate) fn dim(self, p: &mut Painter, pal: &Palette, window: Rect, presence: f32) {
        if let Self::Native = self {
            return draw_backdrop_presence(p, pal, window.w, window.h, presence);
        }
        // Fading a solid black fill over the saved destination is black source-over.
        // Painter::fade also knows how to decode a marked chrome pixel before mixing it.
        let strength = if pal.dark { 0.45 } else { 0.28 } * presence;
        if strength > 0.0 {
            let under = p.snapshot(window);
            p.fill(window, [0; 3]);
            p.fade(&under, strength);
        }
    }
}
