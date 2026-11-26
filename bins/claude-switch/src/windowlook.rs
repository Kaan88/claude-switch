//! Host appearance and short-lived popup motion. The terminal grid stays out of both paths.

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::anim::{AsId, Id, Policy, document};
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::host::Cx;
use mdshared_ui::menu::{Look, Menu};
use mdshared_ui::theme::Palette;

use crate::dialogs;
use crate::state::{AppState, Msg};

pub fn apply(st: &AppState, cx: &mut Cx<'_, Msg>) {
    cx.set_dark(st.pal().dark);
    cx.set_backdrop(Backdrop::parse(&st.config.appearance.backdrop).unwrap_or_default(), st.pal());
    cx.set_backdrop_veil(Some(217)); // At most 15% compositor colour in frosted chrome.
    cx.anim().set_policy(if st.high_contrast { Policy::Never } else { st.appearance.motion.policy() });
}

fn popup<T: Clone>(
    cx: &mut Cx<'_, Msg>,
    p: &mut Painter,
    window: Rect,
    pal: &Palette,
    menu: &mut Menu<T>,
    style: (Look, bool),
    id: impl AsId,
) {
    let id = Id::new(id);
    menu.set_look(style.0);
    menu.set_high_contrast(style.1);
    let t = cx.anim().value_from(id, 0.0_f32, 1.0, document::MENU.enter);
    if t < 1.0 {
        menu.draw_in_presence(p, pal, window, t);
        cx.animating(menu.presence_bounds());
    } else {
        menu.draw_in_animated(p, pal, window, cx.anim(), id);
        cx.animating(menu.drawn_bounds());
    }
}

pub fn draw_overlays(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter, window: Rect) {
    let pal = *st.pal();
    let mut look = st.appearance.menu_look();
    look.soft_shadow = !st.high_contrast;
    look.shadow &= !st.high_contrast;
    let style = (look, st.high_contrast);
    if let Some(menu) = st.accounts_dd.menu_mut() {
        popup(cx, p, window, &pal, menu, style, "accounts-menu");
    }
    if let Some((kind, menu)) = &mut st.menu {
        popup(cx, p, window, &pal, menu, style, ("popup", *kind as u8));
    }
    let t = if st.dialog.is_some() {
        cx.animating(window);
        cx.anim().value_from("dialog-enter", 0.0_f32, 1.0, document::DIALOG.enter)
    } else {
        1.0
    };
    dialogs::draw(st, cx, p, window, t);
    if let Some(palette) = &mut st.palette {
        let t = cx.anim().value_from("palette-enter", 0.0_f32, 1.0, document::MENU.enter);
        palette.draw_presence(p, &pal, window, TITLE_H, true, t);
        if t < 1.0 {
            cx.animating(palette.presence_bounds());
        }
    }
}
