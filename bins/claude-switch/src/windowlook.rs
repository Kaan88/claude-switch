//! Host appearance and short-lived popup motion. The terminal grid stays out of both paths.

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::anim::{AsId, Id, Policy, document};
use mdshared_ui::appearance::Appearance;
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::{Painter, Rect, Rgb, rgb};
use mdshared_ui::host::Cx;
use mdshared_ui::menu::{Look, Menu};
use mdshared_ui::theme::Palette;
use mdshared_ui::tokens::accented;

use crate::dialogs;
use crate::state::{AppState, Msg};

pub fn palette(base: &Palette, look: &Appearance, system: Option<Rgb>) -> Palette {
    // The lighter clay target does not clear 4.5:1 on the light title and selected surfaces.
    let clay = rgb(if base.dark { 0xd97757 } else { 0x923d23 });
    look.palette(&accented(base, clay), system, None)
}

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
    crate::usage_ui::draw_popup(st, p, window, cx.now());
    let pal = *st.pal();
    let mut look = st.appearance.menu_look();
    look.soft_shadow = !st.high_contrast;
    look.shadow &= !st.high_contrast;
    let style = (look, st.high_contrast);
    if let Some(menu) = st.accounts_dd.menu_mut() {
        popup(cx, p, window, &pal, menu, style, "accounts-menu");
    }
    if let Some((kind, menu)) = &mut st.menu {
        popup(cx, p, window, &pal, menu, style, ("popup", *kind));
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

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_ui::{appearance::AccentFrom, cover::contrast, gfx::mix, theme};

    #[test]
    fn clay_is_readable_on_light_surfaces_and_its_button_label_is_readable_in_both_themes() {
        let light = palette(&theme::LIGHT, &Appearance::DEFAULT, None);
        for surface in [
            light.page,
            light.title,
            light.bar,
            light.pane,
            light.field,
            light.popup,
            light.hover,
            light.press,
            light.on,
        ] {
            assert!(contrast(light.accent, surface) >= 4.5, "{surface:?}");
        }
        let dark = palette(&theme::DARK, &Appearance::DEFAULT, None);
        assert_eq!(dark.accent, rgb(0xd97757));
        assert_ne!(light.selection, theme::LIGHT.selection);
        for pal in [light, dark] {
            for t in [0.0, 0.1, 0.2] {
                assert!(contrast(mix(pal.accent, pal.popup, t), pal.on_accent) >= 4.5);
            }
        }
    }

    #[test]
    fn a_saved_custom_accent_wins() {
        let custom = rgb(0x004080);
        let mut look = Appearance::DEFAULT;
        look.accent = AccentFrom::Custom(custom);
        assert_eq!(palette(&theme::LIGHT, &look, None).accent, custom);
    }
}
