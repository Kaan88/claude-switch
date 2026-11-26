//! Settings › Appearance: the window theme, the dark and light palettes (each shown as a row of
//! swatches beside its choice), the accent and the motion and shape controls.
//!
//! The swatches are drawn by the app over the native form, in the free part of the palette rows
//! between the label and the control; a click on one chooses it as the row's choice would.

use super::{
    AppState, Form, Row, SET_ACCENT, SET_ACCENT_COLOUR, SET_BACKDROP, SET_CORNERS, SET_REDUCE_MOTION, SET_SHADOWS,
    SET_THEME, colour,
};
use crate::termtheme::palettes::{self, Theme};
use crate::termtheme::{ACCENT_SOURCES, WINDOW_THEMES};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::gfx::{Painter, Rect, Rgb, mix, rgb};
use mdshared_ui::theme::Palette;

pub(super) const SET_DARK_THEME: u32 = 20;
pub(super) const SET_LIGHT_THEME: u32 = 21;

const SWATCH_W: f32 = 26.0;
const SWATCH_H: f32 = 22.0;
const SWATCH_GAP: f32 = 7.0;
/// Between the last swatch and the row's control.
const BEFORE_CONTROL: f32 = 14.0;

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let themes = WINDOW_THEMES.map(|(_, name)| name);
    let theme = WINDOW_THEMES.iter().position(|(id, _)| *id == st.config.window_theme).unwrap_or(0);
    let mut theme_row = Row::choice(SET_THEME, "Theme", &themes, theme);
    if st.forced_dark.is_some() {
        theme_row = theme_row.help("The --theme launch flag overrides this setting.");
    } else if st.high_contrast {
        theme_row = theme_row.help("Windows high contrast is on and takes priority.");
    }
    let a = &st.config.appearance;
    let accent = ACCENT_SOURCES.iter().position(|(id, _)| *id == a.accent_source).unwrap_or(0);
    let accent_names = ACCENT_SOURCES.map(|(_, label)| label);
    let names = |dark| palettes::list(dark).iter().map(|t| t.name).collect::<Vec<_>>();
    vec![
        theme_row,
        Row::choice(SET_DARK_THEME, "Dark palette", &names(true), palettes::index(true, &a.dark_theme)),
        Row::choice(SET_LIGHT_THEME, "Light palette", &names(false), palettes::index(false, &a.light_theme)),
        Row::choice(
            SET_BACKDROP,
            Backdrop::TITLE,
            &Backdrop::LABELS,
            Backdrop::parse(&a.backdrop).unwrap_or_default().index(),
        )
        .help("Windows 11 22H2 or later. Terminal opacity is set separately."),
        Row::choice(SET_ACCENT, "Accent source", &accent_names, accent).help("Claude’s clay works on every palette."),
        colour(SET_ACCENT_COLOUR, "Window accent", &a.accent_colour, st.pal().accent)
            .help("Used when Accent source is Custom."),
        Row::toggle(SET_REDUCE_MOTION, "Reduce motion", a.reduce_motion)
            .help("Windows’ reduced-motion preference also applies."),
        Row::number(SET_CORNERS, "Dialog and menu corners", i32::from(a.corner_radius), 0, 16).unit("px"),
        Row::toggle(SET_SHADOWS, "Soft shadows", a.soft_shadows),
    ]
}

/// A palette row's choice: saves the id. False for any other row or a choice out of range.
pub(super) fn choose(st: &mut AppState, id: u32, index: usize) -> bool {
    let dark = match id {
        SET_DARK_THEME => true,
        SET_LIGHT_THEME => false,
        _ => return false,
    };
    let Some(theme) = palettes::list(dark).get(index) else { return false };
    let a = &mut st.config.appearance;
    *(if dark { &mut a.dark_theme } else { &mut a.light_theme }) = theme.id.into();
    true
}

/// The palette rows' swatches: the row id, the swatch's place in its list and where it is.
pub(super) fn swatches(form: &Form) -> Vec<(u32, usize, Rect)> {
    let mut out = Vec::new();
    for (row, r) in form.rows().iter().enumerate() {
        let (id, dark) = match r.id() {
            Some(SET_DARK_THEME) => (SET_DARK_THEME, true),
            Some(SET_LIGHT_THEME) => (SET_LIGHT_THEME, false),
            _ => continue,
        };
        let control = form.control_rect(row);
        let n = palettes::list(dark).len();
        let right = control.x - BEFORE_CONTROL;
        let y = (control.cy() - SWATCH_H / 2.0).round();
        for k in 0..n {
            let x = right - (n - k) as f32 * (SWATCH_W + SWATCH_GAP) + SWATCH_GAP;
            out.push((id, k, Rect::new(x.round(), y, SWATCH_W, SWATCH_H)));
        }
    }
    out
}

/// The swatch under the pointer, if it is in the form's visible body.
pub(super) fn hit(form: &Form, x: f32, y: f32) -> Option<(u32, usize)> {
    let body = form.body_rect();
    body.contains(x, y).then_some(())?;
    swatches(form).into_iter().find(|(_, _, r)| r.inset(-2.0).contains(x, y)).map(|(id, k, _)| (id, k))
}

/// The palette's look as the window would have it, with the current accent choice.
fn preview(st: &AppState, t: &Theme) -> Palette {
    // The system accent as the current palette holds it stands in for asking the system again.
    let system = (st.config.appearance.accent_source == "system").then_some(st.pal().accent);
    crate::windowlook::palette(&t.palette, &st.appearance, system)
}

pub(super) fn draw(p: &mut Painter, st: &AppState, form: &Form, hover: Option<(u32, usize)>) {
    if form.colour_open().is_some() {
        return; // The picker's flyout may lie over the rows.
    }
    let pal = st.pal();
    let clip = p.clip(form.body_rect());
    for (id, k, r) in swatches(form) {
        let dark = id == SET_DARK_THEME;
        let theme = &palettes::list(dark)[k];
        let chosen = form.value(id) == Some(mdshared_ui::settings::Value::Choice(k));
        let ring = if chosen {
            Some(pal.accent)
        } else if hover == Some((id, k)) {
            Some(pal.muted)
        } else {
            None
        };
        if let Some(c) = ring {
            p.rbox(r.inset(-3.0), 7.0, c);
            p.rbox(r.inset(-1.5), 5.5, pal.popup);
        }
        paint(p, r, &preview(st, theme), theme, mix(pal.popup, pal.text, 0.25));
    }
    p.restore(clip);
}

/// One swatch: the window's bar and page with a line of text and the accent, and the terminal's
/// background with its text at the right.
fn paint(p: &mut Painter, r: Rect, look: &Palette, theme: &Theme, edge: Rgb) {
    let term = theme.terminal;
    p.frame(r, 4.0, look.page, edge);
    let inner = r.inset(1.0);
    p.fill(Rect::new(inner.x + 3.0, inner.y, inner.w - 6.0, 5.0), look.bar);
    let split = (inner.x + inner.w * 0.62).round();
    p.fill(Rect::new(inner.x + 4.0, inner.y + 9.0, split - inner.x - 7.0, 2.0), look.text);
    p.fill(Rect::new(inner.x + 4.0, inner.y + 14.0, 6.0, 3.0), look.accent);
    let terminal = Rect::new(split, inner.y + 5.0, inner.right() - split - 2.0, inner.h - 7.0);
    p.rbox(terminal, 2.0, rgb(term.background));
    p.fill(Rect::new(terminal.x + 2.0, terminal.y + 4.0, terminal.w - 4.0, 1.5), rgb(term.foreground));
    p.fill(Rect::new(terminal.x + 2.0, terminal.y + 8.0, 3.0, 1.5), rgb(term.ansi[2]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::{Face, Gfx};
    use mdshared_ui::settings::{Outcome, Value};

    #[test]
    fn swatches_sit_between_label_and_control_and_a_click_chooses_and_saves_the_palette() {
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        let mut gfx = Gfx::new();
        for (w, scale) in [(760.0, 1.0), (760.0, 1.5), (1280.0, 1.0)] {
            let window = Rect::new(0.0, 0.0, w, 480.0);
            let mut bitmap = Bitmap::new((w * scale) as u32, (480.0 * scale) as u32);
            let mut p = Painter::new(&mut bitmap, &mut gfx, scale);
            let mut form = super::super::page_form(&st, 0);
            form.layout(&mut p, Rect::new(window.x + 150.0, 0.0, window.w - 160.0, window.h));
            let all = swatches(&form);
            assert_eq!(all.len(), palettes::DARK_THEMES.len() + palettes::LIGHT_THEMES.len());
            for (row, r) in form.rows().iter().enumerate() {
                let Some(id @ (SET_DARK_THEME | SET_LIGHT_THEME)) = r.id() else { continue };
                let Row::Choice { label, .. } = r else { unreachable!() };
                let label_end = form.rect_of_row(row).x + 14.0 + p.measure(Face::Ui, 13.5, label);
                let control = form.control_rect(row);
                let mine: Vec<_> = all.iter().filter(|s| s.0 == id).collect();
                assert!(
                    mine.first().unwrap().2.x > label_end + 8.0,
                    "{label} at {w}: {label_end} {:?} {control:?}",
                    mine.first().unwrap().2
                );
                assert!(mine.last().unwrap().2.right() < control.x, "{label} at {w}");
                assert!(
                    mine.iter()
                        .all(|s| s.2.y > form.rect_of_row(row).y && s.2.bottom() < form.rect_of_row(row).bottom())
                );
            }
        }
        let mut gfx = Gfx::new();
        let mut bitmap = Bitmap::new(760, 480);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let mut form = super::super::page_form(&st, 0);
        form.layout(&mut p, Rect::new(150.0, 0.0, 600.0, 480.0));
        let (_, _, warm) = swatches(&form).into_iter().find(|s| s.0 == SET_DARK_THEME && s.1 == 2).unwrap();
        assert_eq!(hit(&form, warm.cx(), warm.cy()), Some((SET_DARK_THEME, 2)));
        let before = st.pal().page;
        super::super::settings_outcome(&mut st, Outcome::Changed { id: SET_DARK_THEME, value: Value::Choice(2) });
        assert_eq!(st.config.appearance.dark_theme, "warm-dark");
        assert_ne!(st.pal().page, before, "the forced dark window takes the warm palette");
        assert_eq!(st.pal().page, palettes::DARK_THEMES[2].palette.page);
        // The light palette is saved too, but the forced dark window keeps its dark one.
        super::super::settings_outcome(&mut st, Outcome::Changed { id: SET_LIGHT_THEME, value: Value::Choice(3) });
        assert_eq!(st.config.appearance.light_theme, "contrast-light");
        assert!(st.pal().dark);
        assert!(!choose(&mut st, SET_LIGHT_THEME, 99) && !choose(&mut st, SET_THEME, 0));
    }

    #[test]
    fn the_light_palette_applies_when_the_window_is_light() {
        let mut st = AppState::new(Source::Demo, Some(false), Default::default());
        assert_eq!(st.pal().page, palettes::LIGHT_THEMES[0].palette.page);
        assert!(choose(&mut st, SET_LIGHT_THEME, 2));
        st.retheme();
        assert_eq!(st.pal().page, palettes::LIGHT_THEMES[2].palette.page);
        assert_eq!(st.term_colours.0.background, rgb(palettes::LIGHT_THEMES[2].terminal.background));
    }
}
