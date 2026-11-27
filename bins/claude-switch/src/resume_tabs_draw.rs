use std::cell::RefCell;

use mdshared_ui::anim::pop;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::host::Cx;
use mdshared_ui::notice::{bar_button, bar_button_w};
use mdshared_ui::overlay::{Bubble, TOAST_H, TOAST_RISE};

use crate::state::{AppState, Msg, Target};
use crate::winfile::RestoreTabs;

use super::TOAST_PREFIX;

impl AppState {
    pub fn draw_tab_restore(&mut self, p: &mut Painter, top: f32, w: f32) -> f32 {
        let n = self.restore.pending.len();
        if n == 0 || self.restore.started || self.win_file.restore_tabs != RestoreTabs::Ask || !self.scanned {
            return top;
        }
        let tabs = if n == 1 { "tab" } else { "tabs" };
        let r = Rect::new(0.0, top, w, 38.0);
        let pal = *self.pal();
        p.fill(r, pal.bar);
        p.fill(Rect::new(r.x, r.bottom() - p.px(), r.w, p.px()), pal.line);
        let mut x = r.right() - 12.0;
        for (label, target, primary) in [("Dismiss", Target::DismissTabs, false), ("Reopen", Target::RestoreTabs, true)]
        {
            x -= bar_button_w(p, label);
            let rect = bar_button(
                p,
                &pal,
                x,
                r.cy(),
                label,
                primary,
                self.hover == Some(target),
                self.pressed == Some(target),
            );
            self.hits.push((rect, target));
            x -= 8.0;
        }
        p.text(
            Face::Ui,
            12.5,
            14.0,
            r.cy(),
            &format!("Reopen {n} {tabs} from last time"),
            pal.muted,
            (x - 14.0).max(0.0),
        );
        r.bottom()
    }
}

fn height(p: &mut Painter, text: &str, window: Rect) -> f32 {
    if text.starts_with(TOAST_PREFIX) {
        let w = window.w - 68.0;
        (p.paragraph_height_wrapped(Face::Ui, 13.0, w, text, Wrap::Anywhere) + 20.0)
            .clamp(TOAST_H, (window.h - 120.0).max(TOAST_H))
    } else {
        TOAST_H
    }
}

pub fn draw_toasts(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter, window: Rect) {
    let pal = *st.pal();
    let cards = crate::windowlook::Cards::for_frame(st, p);
    let frames = {
        // The toolkit asks for a pure height callback; the painter caches text shaping.
        let measure = RefCell::new(&mut *p);
        st.toasts.frame(cx.anim(), "toasts", |text| height(&mut measure.borrow_mut(), text, window), 8.0)
    };
    for shown in frames {
        let mut b = Bubble::toast(p, shown.item, window);
        b.rect.y -= shown.lift;
        if shown.item.starts_with(TOAST_PREFIX) {
            let h = height(p, shown.item, window);
            b.rect = Rect::new(window.x + 16.0, b.rect.bottom() - h, window.w - 32.0, h);
        }
        pop(p, b.bounds(), shown.t, (0.0, TOAST_RISE), |p| {
            cards.draw(p, b.rect, pal.popup, |p| {
                if shown.item.starts_with(TOAST_PREFIX) {
                    let r = b.rect;
                    p.shadow(r, 8.0, pal.dark);
                    p.frame(r, 8.0, pal.popup, pal.popup_line);
                    let inset = p.clip(r.inset(8.0));
                    p.paragraph_wrapped(
                        Face::Ui,
                        13.0,
                        r.x + 18.0,
                        r.y + 10.0,
                        r.w - 36.0,
                        shown.item,
                        pal.text,
                        Wrap::Anywhere,
                    );
                    p.restore(inset);
                } else {
                    b.draw(p, &pal);
                }
            })
        });
        cx.animating(b.presence_bounds().inset(-8.0));
    }
}
