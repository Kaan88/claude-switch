//! The form owns scrolling and Tab stops. Each palette row has one hit map for its
//! previews and captions; Left/Right and Home/End address the same choices.
use super::{Form, Row};
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb};
use mdshared_ui::keys::KeyName;
use mdshared_ui::settings::{Outcome, Value};
use mdshared_ui::theme::Palette;

pub(super) const DARK: u32 = 20;
pub(super) const LIGHT: u32 = 21;
const EXTRA: u32 = 4000;
pub(super) const ACROSS: usize = 5;
pub(super) const MORE_TERMINAL: u32 = 386;
#[derive(Clone, Copy)]
pub(super) enum Family {
    Window(bool),
    Terminal,
}

pub(super) fn row_id(dark: bool, group: usize) -> u32 {
    match group {
        0 => {
            if dark {
                DARK
            } else {
                LIGHT
            }
        }
        n => EXTRA + 2 * n as u32 + u32::from(!dark),
    }
}

pub(super) fn group(id: u32) -> Option<(Family, usize)> {
    match id {
        DARK => Some((Family::Window(true), 0)),
        LIGHT => Some((Family::Window(false), 0)),
        super::SET_TERM_COLOURS => Some((Family::Terminal, 0)),
        MORE_TERMINAL => Some((Family::Terminal, 1)),
        n if n >= EXTRA + 2 => Some((Family::Window(n.is_multiple_of(2)), ((n - EXTRA) / 2) as usize)),
        _ => None,
    }
}

fn rack(form: &Form, i: usize) -> Rect {
    let r = form.rect_of_row(i);
    let width = (r.w - 114.0).clamp(1.0, 393.0);
    Rect::new(r.right() - 10.0 - width, r.y + 2.0, width, r.h - 4.0)
}

fn tile(rack: Rect, k: usize, n: usize) -> Rect {
    let w = rack.w / n as f32;
    Rect::new(rack.x + k as f32 * w, rack.y, w, rack.h)
}

pub(super) fn hit(form: &Form, x: f32, y: f32) -> Option<(u32, usize)> {
    if form.colour_open().is_some() || !form.body_rect().contains(x, y) {
        return None;
    }
    form.rows().iter().enumerate().find_map(|(i, row)| {
        let Row::Choice { id, options, .. } = row else { return None };
        group(*id)?;
        let r = rack(form, i);
        (0..options.len()).find(|&k| tile(r, k, options.len()).contains(x, y)).map(|k| (*id, k))
    })
}

pub(super) fn key(form: &mut Form, key: &KeyName) -> Option<Outcome> {
    let id = form.focus_id()?;
    group(id)?;
    let row = form.rows().iter().find(|r| r.id() == Some(id))?;
    let Row::Choice { options, value, .. } = row else { return None };
    let n = options.len();
    let next = match key {
        KeyName::Right | KeyName::Enter | KeyName::Space => (value + 1) % n,
        KeyName::Left => (value + n - 1) % n,
        KeyName::Home => 0,
        KeyName::End => n - 1,
        _ => return None,
    };
    form.set_value(id, Value::Choice(next));
    Some(Outcome::Changed { id, value: Value::Choice(next) })
}

pub(super) struct Sample {
    pub page: Rgb,
    pub text: Rgb,
    pub bar: Rgb,
    pub accent: Rgb,
    pub terminal: Rgb,
    pub ink: Rgb,
    pub selected: bool,
}

pub(super) fn draw(
    p: &mut Painter,
    pal: &Palette,
    form: &Form,
    hover: Option<(u32, usize)>,
    focus: bool,
    sample: impl Fn(Family, usize) -> Option<Sample>,
) {
    // The confirming picker has already been drawn by Form on top of its rows.
    let picker = form.colour_open().map(|_| p.snapshot(form.picker_rect().inset(-16.0)));
    let clip = p.clip(form.body_rect());
    for (i, row) in form.rows().iter().enumerate() {
        let Row::Choice { id, options, .. } = row else { continue };
        let Some((family, group)) = group(*id) else { continue };
        let r = rack(form, i);
        if r.bottom() <= form.body_rect().y || r.y >= form.body_rect().bottom() {
            continue;
        }
        p.fill(r, pal.popup);
        for (k, name) in options.iter().enumerate() {
            let Some(s) = sample(family, group * ACROSS + k) else { continue };
            let t = tile(r, k, options.len()).inset(1.0);
            if s.selected || hover == Some((*id, k)) {
                p.rbox(t, 4.0, if s.selected { pal.on } else { pal.hover });
            }
            let chip = Rect::new(t.cx() - 15.0, t.cy() - 15.0, 30.0, 16.0);
            p.frame(chip, 3.0, s.page, if s.selected { pal.accent } else { pal.popup_line });
            p.fill(Rect::new(chip.x + 2.0, chip.y + 2.0, 26.0, 3.0), s.bar);
            p.fill(Rect::new(chip.x + 3.0, chip.y + 7.0, 10.0, 2.0), s.text);
            p.fill(Rect::new(chip.x + 3.0, chip.y + 11.0, 6.0, 2.0), s.accent);
            p.fill(Rect::new(chip.x + 16.0, chip.y + 5.0, 12.0, 9.0), s.terminal);
            p.fill(Rect::new(chip.x + 18.0, chip.y + 8.0, 6.0, 1.0), s.ink);
            let w = p.measure(Face::Ui, 11.5, name).min(t.w - 4.0);
            p.text(
                Face::Ui,
                11.5,
                t.cx() - w / 2.0,
                t.cy() + 9.0,
                name,
                if s.selected { pal.text } else { pal.muted },
                w + 1.0,
            );
        }
        if focus && form.focus_id() == Some(*id) {
            p.fill(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), pal.accent);
        }
    }
    p.restore(clip);
    if let Some(saved) = picker {
        saved.restore(p.bmp);
    }
}
