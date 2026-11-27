//! Host appearance and short-lived popup motion. Terminal opacity is independent of the chrome veil.

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::anim::{AsId, Id, Policy, document};
use mdshared_ui::appearance::Appearance;
use mdshared_ui::chrome::TITLE_H;
use mdshared_ui::gfx::{Painter, Rect, Rgb, rgb};
use mdshared_ui::host::Cx;
use mdshared_ui::menu::{Look, Menu};
use mdshared_ui::theme::Palette;
use mdshared_ui::tokens::{ACCENT_CONTRAST, accented, legible};

use crate::dialogs;
use crate::state::{AppState, Msg};
mod overlay;
pub(crate) use overlay::Cards;

/// `base` (one of `termtheme::palettes`) with Claude's clay as its own accent, then the accent
/// the appearance asks for put in: clay stays available on every palette.
pub fn palette(base: &Palette, look: &Appearance, system: Option<Rgb>) -> Palette {
    // The lighter clay target does not clear 4.5:1 on the light title and selected surfaces.
    let clay = rgb(if base.dark { 0xd97757 } else { 0x923d23 });
    let ratio = if crate::termtheme::palettes::of(base).high_contrast { 7.0 } else { ACCENT_CONTRAST };
    look.palette(&accented(base, legible(clay, base.page, ratio)), system, None)
}

pub fn apply(st: &AppState, cx: &mut Cx<'_, Msg>) {
    cx.set_dark(st.pal().dark);
    cx.set_backdrop(Backdrop::parse(&st.config.appearance.backdrop).unwrap_or_default(), st.pal());
    cx.set_backdrop_veil(Some(217)); // At most 15% compositor colour in frosted chrome.
    cx.anim().set_policy(if st.high_contrast { Policy::Never } else { st.appearance.motion.policy() });
}

fn popup<T: Clone + PartialEq>(
    cx: &mut Cx<'_, Msg>,
    p: &mut Painter,
    window: Rect,
    pal: &Palette,
    menu: &mut Menu<T>,
    style: (Look, bool),
    (id, switches): (impl AsId, &[(T, bool)]),
) {
    let id = Id::new(id);
    let (look, hc) = style;
    menu.set_look(look);
    menu.set_high_contrast(hc);
    let t = cx.anim().value_from(id, 0.0_f32, 1.0, document::MENU.enter);
    if t < 1.0 {
        menu.draw_entering_with(p, window, t, mdshared_ui::menu::Entrance::Drop, |p, m| {
            m.draw_in(p, pal, window);
            menu_switches(p, pal, m, switches);
        });
        cx.animating(menu.presence_bounds());
    } else {
        menu.draw_in_animated(p, pal, window, cx.anim(), id);
        menu_switches(p, pal, menu, switches);
        cx.animating(menu.drawn_bounds());
    }
}

fn menu_switches<T: Clone + PartialEq>(p: &mut Painter, pal: &Palette, menu: &Menu<T>, switches: &[(T, bool)]) {
    use mdshared_ui::toggle::{self, Toggle};
    if switches.is_empty() {
        return;
    }
    let clip = p.clip(menu.rect().inset(1.0));
    for (i, item) in menu.items.iter().enumerate() {
        if let Some((_, on)) = switches.iter().find(|(id, _)| id == &item.id) {
            let row = menu.row_rect(i);
            // Keep a column for Ctrl+D and Delete to the right of the switch.
            let r = Rect::new(
                row.right() - 64.0 - toggle::TOGGLE_W,
                row.cy() - toggle::TOGGLE_H / 2.0,
                toggle::TOGGLE_W,
                toggle::TOGGLE_H,
            );
            toggle::draw_toggle(
                p,
                pal,
                r,
                &Toggle::new(*on),
                toggle::Look { hovered: menu.hover == Some(i), disabled: !item.enabled, ..Default::default() },
            );
        }
    }
    p.restore(clip);
}

/// The tooltip that shows now, if any.
pub fn draw_tip(st: &AppState, p: &mut Painter, window: Rect) {
    if let Some(over) = st.tip.shown() {
        let pal = *st.pal();
        let tip = mdshared_ui::overlay::Bubble::tip(p, &over.text, over.anchor, window.w, window.h);
        Cards::for_frame(st, p).draw(p, tip.rect, pal.popup, |p| tip.draw(p, &pal));
    }
}

pub fn draw_overlays(st: &mut AppState, cx: &mut Cx<'_, Msg>, p: &mut Painter, window: Rect) {
    crate::usage_ui::draw_popup(st, p, window, cx.now());
    let pal = *st.pal();
    let mut look = st.appearance.menu_look();
    look.soft_shadow = !st.high_contrast;
    look.shadow &= !st.high_contrast;
    let cards = Cards::for_frame(st, p);
    let style = (cards.menu(look), st.high_contrast);
    if let Some(menu) = st.accounts_dd.menu_mut() {
        popup(cx, p, window, &pal, menu, style, ("accounts-menu", &[]));
    }
    // A field's menu over a dialog belongs above it.
    let over_dialog = st.dialog.is_some();
    let switches = matches!(st.menu, Some((crate::state::MenuKind::Row, _))).then(|| st.row_switches());
    let switches = switches.as_ref().map_or(&[][..], |rows| rows.as_slice());
    if !over_dialog && let Some((kind, menu)) = &mut st.menu {
        popup(cx, p, window, &pal, menu, style, (("popup", *kind), switches));
    }
    let t = if st.dialog.is_some() {
        cx.animating(window);
        cx.anim().value_from("dialog-enter", 0.0_f32, 1.0, document::DIALOG.enter)
    } else {
        1.0
    };
    dialogs::draw(st, cx, p, window, t);
    if over_dialog {
        draw_tip(st, p, window);
    }
    if over_dialog && let Some((kind, menu)) = &mut st.menu {
        popup(cx, p, window, &pal, menu, style, (("popup", *kind), switches));
    }
    if let Some(palette) = &mut st.palette {
        let t = cx.anim().value_from("palette-enter", 0.0_f32, 1.0, document::MENU.enter);
        let r = palette.layout(window, TITLE_H);
        mdshared_ui::anim::pop(p, r.inset(-18.0), t, (0.0, -mdshared_ui::menu::MENU_DROP), |p| {
            cards.draw(p, r, pal.popup_line, |p| palette.draw(p, &pal, window, TITLE_H, true));
        });
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
    fn a_menu_switch_keeps_the_row_label_and_shortcut_in_place() {
        use mdshared_text::Bitmap;
        use mdshared_ui::{
            gfx::{Face, Gfx},
            menu::Item,
        };
        let window = Rect::new(0.0, 0.0, 320.0, 220.0);
        let pal = theme::DARK;
        let mut bmp = Bitmap::new(320, 220);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let mut m = Menu::new(
            vec![Item::new(1, "Pinned").hint("Ctrl+D"), Item::new(2, "Session hidden").hint("Delete")],
            16.0,
            16.0,
            false,
        )
        .min_width(244.0);
        m.draw_in(&mut p, &pal, window);
        let before = p.bmp.pixels.clone();
        menu_switches(&mut p, &pal, &m, &[(1, true), (2, false)]);
        let mut changed = [false; 2];
        for (i, item) in m.items.iter().enumerate() {
            let r = m.row_rect(i);
            let label_end = r.x + 10.0 + p.measure(Face::Ui, m.look().text, &item.label);
            let hint_left = r.right() - 10.0 - p.measure(Face::Ui, 12.0, &item.hint);
            for y in r.y as usize..r.bottom() as usize {
                for x in r.x as usize..r.right() as usize {
                    let off = (y * 320 + x) * 4;
                    if before[off..off + 4] != p.bmp.pixels[off..off + 4] {
                        assert!(x as f32 > label_end && (x as f32) < hint_left);
                        changed[i] = true;
                    }
                }
            }
        }
        assert_eq!(changed, [true, true]);
        if let Some(dir) = std::env::var_os("CS_THEME_SHOTS") {
            p.bmp.write_png(&std::path::Path::new(&dir).join("menu-switches.png")).unwrap();
        }
    }

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
    fn clay_reads_on_every_palette_surface_and_high_contrast_holds_seven_to_one() {
        use crate::termtheme::palettes::{DARK_THEMES, LIGHT_THEMES};
        for t in DARK_THEMES.iter().chain(LIGHT_THEMES) {
            let pal = palette(&t.palette, &Appearance::DEFAULT, None);
            let ratio = if t.high_contrast { 7.0 } else { 4.5 };
            assert!(contrast(pal.accent, pal.page) >= ratio, "{}", t.id);
            for surface in [pal.title, pal.bar, pal.pane, pal.field, pal.popup] {
                assert!(contrast(pal.accent, surface) >= 3.0, "{} {surface:?}", t.id);
            }
            assert!(contrast(pal.accent, pal.on_accent) >= 4.5, "{}", t.id);
            assert!(contrast(pal.text, pal.selection) >= 3.0, "{}", t.id);
            // The same hue on every palette: red leads.
            assert!(pal.accent[0] > pal.accent[1] && pal.accent[1] > pal.accent[2] / 2, "{} {:?}", t.id, pal.accent);
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
