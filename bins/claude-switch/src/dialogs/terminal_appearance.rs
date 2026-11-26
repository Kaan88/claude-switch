use claude_switch_core::config::{
    AppearancePrefs, TERMINAL_OPACITY_MAX, TERMINAL_OPACITY_MIN, TERMINAL_OPACITY_STEP, TerminalCursor,
};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::settings::{Form, Row};

pub(super) const CURSOR: u32 = 92;
pub(super) const OPACITY: u32 = 93;

pub(super) fn cursor_row(a: &AppearancePrefs) -> Row {
    Row::choice(CURSOR, "Cursor", &TerminalCursor::OPTIONS.map(|(_, _, label)| label), a.cursor_shape.index())
        .help("Used unless the terminal program sets its own shape.")
}

fn unavailable(backdrop: Backdrop, supported: bool, high_contrast: bool, active: bool) -> Option<&'static str> {
    if backdrop == Backdrop::Off {
        Some("Choose Mica or Acrylic under Appearance.")
    } else if high_contrast {
        Some("Unavailable with Windows high contrast.")
    } else if !supported {
        Some("Requires a Windows 11 22H2 or later backdrop.")
    } else if !active {
        Some("The window backdrop is not active.")
    } else {
        None
    }
}

pub(super) fn opacity_row(a: &AppearancePrefs, high_contrast: bool, active: bool) -> Row {
    let backdrop = Backdrop::parse(&a.backdrop).unwrap_or_default();
    let value = i32::from(a.terminal_opacity);
    let why = unavailable(backdrop, mdshared_ui::platform::backdrop_available(), high_contrast, active);
    let (min, max) = if why.is_some() { (value, value) } else { (TERMINAL_OPACITY_MIN, TERMINAL_OPACITY_MAX) };
    Row::number(OPACITY, "Background opacity", value, min, max)
        .unit("%")
        .step(TERMINAL_OPACITY_STEP)
        .help(why.unwrap_or("Applies to the default terminal background and padding."))
}

pub(super) fn sync(form: &mut Form, a: &AppearancePrefs, high_contrast: bool, active: bool) {
    let row = opacity_row(a, high_contrast, active);
    if let Some(index) = form.rows().iter().position(|r| r.id() == Some(OPACITY))
        && form.rows()[index] != row
    {
        // The grouped form has no disabled-row variant. Equal stepper bounds disable both
        // buttons and keyboard changes. Updating rows preserves the scroll and focus.
        let mut rows = form.rows().to_vec();
        rows[index] = row;
        form.set_rows(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_ui::keys::{KeyName, Mods};
    use mdshared_ui::settings::{Outcome, Value};

    #[test]
    fn off_unsupported_high_contrast_and_inactive_each_disable_opacity() {
        assert!(unavailable(Backdrop::Off, true, false, true).is_some());
        assert!(unavailable(Backdrop::Mica, false, false, false).is_some());
        assert!(unavailable(Backdrop::Acrylic, true, true, false).is_some());
        assert!(unavailable(Backdrop::Mica, true, false, false).is_some());
        assert!(unavailable(Backdrop::Mica, true, false, true).is_none());
        assert!(unavailable(Backdrop::Acrylic, true, false, true).is_none());
        let a = AppearancePrefs { terminal_opacity: 80, ..Default::default() };
        let mut form = Form::new("Terminal", vec![opacity_row(&a, false, false)]);
        assert!(form.focus_row(OPACITY));
        for key in [KeyName::Left, KeyName::Right, KeyName::Home, KeyName::End] {
            assert!(!matches!(form.key(&key, Mods::default(), None), Outcome::Changed { .. }));
        }
        assert_eq!(form.value(OPACITY), Some(Value::Number(80)));
    }
}
