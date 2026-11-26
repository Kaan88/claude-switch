//! What a key, a paste or a wheel notch does while a terminal tab is shown: one of the window's
//! own few shortcuts, a copy or a paste, a scroll of the scrollback, or bytes for the program
//! (`mdshared_term::input`). Pure functions of the event and the screen's modes, so every
//! decision is tested here without a window or a program.

use mdshared_term::{Key, Modes, MouseMode, input};
use mdshared_ui::keys::{KeyName, Mods};

use crate::actions;

/// What a key press in a terminal tab comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// One of the window's commands ([`actions`]): the shortcuts that stay with the window.
    Command(u32),
    /// Copy the selection.
    Copy,
    /// Paste the clipboard.
    Paste,
    /// Scroll the scrollback by this many pages, up being positive.
    Page(i32),
    /// Bytes for the program.
    Send(Vec<u8>),
    /// Nothing (a key the terminal has no bytes for).
    None,
}

/// The shortcuts that stay with the window in every tab. Ctrl+K alone is the program's: the
/// command palette is Ctrl+Shift+K here.
pub fn shortcut(name: &KeyName, mods: Mods) -> Option<u32> {
    let plain_ctrl = mods.ctrl && !mods.alt;
    if !plain_ctrl {
        return None;
    }
    Some(match (name, mods.shift) {
        (KeyName::Tab, false) => actions::NEXT_TAB,
        (KeyName::Tab, true) => actions::PREV_TAB,
        (KeyName::Char('w'), true) => actions::CLOSE_TAB,
        (KeyName::Char('t'), true) => actions::NEW_SHELL,
        (KeyName::Char('k'), true) => actions::PALETTE,
        _ => return None,
    })
}

/// The terminal's key for the toolkit's.
pub fn term_key(name: &KeyName) -> Key {
    match *name {
        KeyName::Char(c) => Key::Char(c),
        KeyName::F(n) => Key::F(n),
        KeyName::Tab => Key::Tab,
        KeyName::Escape => Key::Escape,
        KeyName::Enter => Key::Enter,
        KeyName::Space => Key::Space,
        KeyName::Backspace => Key::Backspace,
        KeyName::Delete => Key::Delete,
        KeyName::Home => Key::Home,
        KeyName::End => Key::End,
        KeyName::PageUp => Key::PageUp,
        KeyName::PageDown => Key::PageDown,
        KeyName::Up => Key::Up,
        KeyName::Down => Key::Down,
        KeyName::Left => Key::Left,
        KeyName::Right => Key::Right,
    }
}

pub fn term_mods(mods: Mods) -> mdshared_term::Mods {
    mdshared_term::Mods { ctrl: mods.ctrl || mods.control, shift: mods.shift, alt: mods.alt }
}

/// Text that types something: not empty and no control characters in it.
fn printable(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
}

/// What a key press does in a terminal tab. `name` and `text` are the host's
/// ([`mdshared_ui::host::KeyPress`]); `selected` says whether there is a selection to copy.
pub fn key(name: Option<&KeyName>, mods: Mods, text: Option<&str>, modes: &Modes, selected: bool) -> KeyAction {
    let Some(name) = name else {
        // A key the toolkit has no name for: whatever text it typed.
        return printable(text).map_or(KeyAction::None, |t| KeyAction::Send(t.as_bytes().to_vec()));
    };
    if let Some(id) = shortcut(name, mods) {
        return KeyAction::Command(id);
    }
    let ctrl_shift = mods.ctrl && mods.shift && !mods.alt;
    match name {
        KeyName::Char('c') if ctrl_shift => return KeyAction::Copy,
        KeyName::Char('v') if ctrl_shift => return KeyAction::Paste,
        KeyName::Char('c') if mods.ctrl && !mods.shift && !mods.alt && selected => return KeyAction::Copy,
        KeyName::PageUp if mods.shift && !mods.ctrl && !mods.alt && !modes.alt_screen => return KeyAction::Page(1),
        KeyName::PageDown if mods.shift && !mods.ctrl && !mods.alt && !modes.alt_screen => {
            return KeyAction::Page(-1);
        }
        _ => {}
    }
    let tm = term_mods(mods);
    match name {
        KeyName::Char(c) => {
            // AltGr is Ctrl+Alt on Windows: a key that typed a character with both held typed
            // that character (`@`, `€`, `{` on many layouts).
            if tm.ctrl
                && tm.alt
                && let Some(t) = printable(text)
            {
                return KeyAction::Send(t.as_bytes().to_vec());
            }
            if tm.ctrl || tm.alt {
                return KeyAction::Send(input::encode_key(Key::Char(*c), tm, modes));
            }
            match printable(text) {
                Some(t) => KeyAction::Send(t.as_bytes().to_vec()),
                None => KeyAction::Send(input::encode_key(Key::Char(*c), tm, modes)),
            }
        }
        other => {
            let bytes = input::encode_key(term_key(other), tm, modes);
            if bytes.is_empty() { KeyAction::None } else { KeyAction::Send(bytes) }
        }
    }
}

/// The bytes of a paste for the program's modes.
pub fn paste(text: &str, modes: &Modes) -> Vec<u8> {
    input::encode_paste(text, modes.bracketed_paste)
}

/// Lines of scrollback one wheel notch moves.
pub const WHEEL_LINES: f32 = 3.0;

/// What the wheel does over a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
    /// Scroll the scrollback.
    Scroll,
    /// Report it to the program (it asked for the mouse).
    Report,
    /// Send arrow keys: a full-screen program on the alternate screen that did not ask for the
    /// mouse, as terminals do.
    Arrows,
}

/// What the wheel does with these modes. Shift keeps the wheel the terminal's own, also while
/// the program has the mouse.
pub fn wheel(modes: &Modes, shift: bool) -> Wheel {
    if modes.mouse != MouseMode::Off && !shift {
        Wheel::Report
    } else if modes.alt_screen && !shift {
        Wheel::Arrows
    } else {
        Wheel::Scroll
    }
}

/// Whole lines out of a wheel movement of `dy` logical px (positive: away from the user, which
/// looks back), keeping the part of a line left over in `rest` for the next movement.
pub fn wheel_lines(dy: f32, rest: &mut f32) -> i32 {
    let total = *rest + dy / mdshared_ui::host::WHEEL_LINE * WHEEL_LINES;
    let lines = total.trunc();
    *rest = total - lines;
    lines as i32
}

/// Whether the press goes to the program: it asked for the mouse and Shift is not held (Shift
/// keeps selecting the terminal's own).
pub fn mouse_to_program(modes: &Modes, shift: bool) -> bool {
    modes.mouse != MouseMode::Off && !shift
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false, control: false };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const SHIFT: Mods = Mods { shift: true, ..NONE };
    const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, ..NONE };
    const ALT: Mods = Mods { alt: true, ..NONE };
    const ALTGR: Mods = Mods { ctrl: true, alt: true, ..NONE };

    fn k(name: KeyName, mods: Mods, text: Option<&str>) -> KeyAction {
        key(Some(&name), mods, text, &Modes::default(), false)
    }

    fn send(b: &[u8]) -> KeyAction {
        KeyAction::Send(b.to_vec())
    }

    #[test]
    fn typed_text_goes_to_the_program_as_utf8() {
        assert_eq!(k(KeyName::Char('a'), NONE, Some("a")), send(b"a"));
        assert_eq!(k(KeyName::Char('a'), SHIFT, Some("A")), send(b"A"));
        assert_eq!(k(KeyName::Char('\u{e9}'), NONE, Some("\u{e9}")), send("\u{e9}".as_bytes()));
        assert_eq!(k(KeyName::Space, NONE, Some(" ")), send(b" "));
        assert_eq!(key(None, NONE, Some("\u{20ac}"), &Modes::default(), false), send("\u{20ac}".as_bytes()));
        assert_eq!(key(None, NONE, None, &Modes::default(), false), KeyAction::None);
    }

    #[test]
    fn control_and_alt_make_the_bytes_of_a_vt_keyboard_and_altgr_types_its_character() {
        assert_eq!(k(KeyName::Char('c'), CTRL, Some("\u{3}")), send(b"\x03"));
        assert_eq!(k(KeyName::Char('k'), CTRL, None), send(b"\x0b"), "Ctrl+K is the program's");
        assert_eq!(k(KeyName::Char('d'), CTRL, Some("\u{4}")), send(b"\x04"));
        assert_eq!(k(KeyName::Char('b'), ALT, Some("b")), send(b"\x1bb"));
        assert_eq!(k(KeyName::Char('q'), ALTGR, Some("@")), send(b"@"));
        assert_eq!(k(KeyName::Char('x'), ALTGR, None), send(b"\x1b\x18"));
    }

    #[test]
    fn named_keys_follow_the_modes() {
        assert_eq!(k(KeyName::Enter, NONE, Some("\r")), send(b"\r"));
        assert_eq!(k(KeyName::Backspace, NONE, Some("\u{8}")), send(b"\x7f"));
        assert_eq!(k(KeyName::Tab, NONE, Some("\t")), send(b"\t"));
        assert_eq!(k(KeyName::Tab, SHIFT, None), send(b"\x1b[Z"));
        assert_eq!(k(KeyName::Escape, NONE, None), send(b"\x1b"));
        assert_eq!(k(KeyName::Up, NONE, None), send(b"\x1b[A"));
        assert_eq!(k(KeyName::Left, CTRL, None), send(b"\x1b[1;5D"));
        assert_eq!(k(KeyName::F(5), NONE, None), send(b"\x1b[15~"));
        assert_eq!(k(KeyName::Delete, NONE, None), send(b"\x1b[3~"));
        let app = Modes { app_cursor: true, ..Modes::default() };
        assert_eq!(key(Some(&KeyName::Up), NONE, None, &app, false), send(b"\x1bOA"));
        assert_eq!(k(KeyName::F(30), NONE, None), KeyAction::None);
    }

    #[test]
    fn the_window_keeps_its_few_shortcuts_and_the_copy_and_paste_keys() {
        assert_eq!(k(KeyName::Tab, CTRL, None), KeyAction::Command(actions::NEXT_TAB));
        assert_eq!(k(KeyName::Tab, CTRL_SHIFT, None), KeyAction::Command(actions::PREV_TAB));
        assert_eq!(k(KeyName::Char('w'), CTRL_SHIFT, None), KeyAction::Command(actions::CLOSE_TAB));
        assert_eq!(k(KeyName::Char('t'), CTRL_SHIFT, None), KeyAction::Command(actions::NEW_SHELL));
        assert_eq!(k(KeyName::Char('k'), CTRL_SHIFT, None), KeyAction::Command(actions::PALETTE));
        assert_eq!(k(KeyName::Char('w'), CTRL, Some("\u{17}")), send(b"\x17"), "Ctrl+W is the program's");
        assert_eq!(k(KeyName::Char('c'), CTRL_SHIFT, None), KeyAction::Copy);
        assert_eq!(k(KeyName::Char('v'), CTRL_SHIFT, None), KeyAction::Paste);
        // Ctrl+C copies a selection, and is the program's interrupt without one.
        assert_eq!(key(Some(&KeyName::Char('c')), CTRL, None, &Modes::default(), true), KeyAction::Copy);
        assert_eq!(key(Some(&KeyName::Char('c')), CTRL, None, &Modes::default(), false), send(b"\x03"));
    }

    #[test]
    fn shift_page_keys_scroll_the_scrollback_except_on_the_alternate_screen() {
        assert_eq!(k(KeyName::PageUp, SHIFT, None), KeyAction::Page(1));
        assert_eq!(k(KeyName::PageDown, SHIFT, None), KeyAction::Page(-1));
        assert_eq!(k(KeyName::PageUp, NONE, None), send(b"\x1b[5~"));
        let alt = Modes { alt_screen: true, ..Modes::default() };
        assert_eq!(key(Some(&KeyName::PageUp), SHIFT, None, &alt, false), send(b"\x1b[5;2~"));
    }

    #[test]
    fn a_paste_is_bracketed_when_the_program_asked() {
        assert_eq!(paste("a\r\nb", &Modes::default()), b"a\rb");
        let br = Modes { bracketed_paste: true, ..Modes::default() };
        assert_eq!(paste("x\n", &br), b"\x1b[200~x\r\x1b[201~");
    }

    #[test]
    fn the_wheel_scrolls_reports_or_sends_arrows_by_the_modes() {
        let plain = Modes::default();
        let alt = Modes { alt_screen: true, ..plain };
        let mouse = Modes { mouse: MouseMode::Press, ..plain };
        let both = Modes { mouse: MouseMode::Drag, alt_screen: true, ..plain };
        assert_eq!(wheel(&plain, false), Wheel::Scroll);
        assert_eq!(wheel(&alt, false), Wheel::Arrows);
        assert_eq!(wheel(&mouse, false), Wheel::Report);
        assert_eq!(wheel(&both, false), Wheel::Report);
        assert_eq!(wheel(&mouse, true), Wheel::Scroll);
        assert_eq!(wheel(&alt, true), Wheel::Scroll);
        assert!(mouse_to_program(&mouse, false));
        assert!(!mouse_to_program(&mouse, true));
        assert!(!mouse_to_program(&plain, false));
    }

    #[test]
    fn a_notch_is_three_lines_and_touchpad_fractions_add_up() {
        let mut rest = 0.0;
        assert_eq!(wheel_lines(mdshared_ui::host::WHEEL_LINE, &mut rest), 3);
        assert_eq!(wheel_lines(-2.0 * mdshared_ui::host::WHEEL_LINE, &mut rest), -6);
        let mut rest = 0.0;
        let step = mdshared_ui::host::WHEEL_LINE / 9.0;
        let got: i32 = (0..9).map(|_| wheel_lines(step, &mut rest)).sum();
        assert_eq!(got, 3);
    }
}
