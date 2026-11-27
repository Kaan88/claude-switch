use super::*;

#[test]
fn shift_enter_uses_the_shown_tabs_launch_executable_not_its_label_or_modes() {
    let mut app = AppState::new(crate::source::Source::Demo, None, Default::default());
    app.usage.demo_echo = true;
    let shift_enter = KeyPress {
        name: Some(KeyName::Enter),
        mods: Mods { shift: true, ..Mods::default() },
        text: Some("\r".into()),
        repeat: false,
    };
    let cases: &[(&str, &[u8])] = &[
        ("claude", b"\x1b[13;2u"),
        (r"C:\Tools\Claude.EXE", b"\x1b[13;2u"),
        ("pwsh.exe", b"\r"),
        ("cmd.exe", b"\r"),
        (r"C:\Tools\claude-helper.exe", b"\r"),
    ];
    for &(program, bytes) in cases {
        let mut term = Term::for_command(&mdshared_pty::Command::new(program), (30, 4));
        term.feed(b"\x1b[?2004h");
        app.tabs.open(NewTab { label: "Claude".into(), color: [0; 3], account: None, session: None }, term);
        app.term_key(Instant::now(), &shift_enter);
        assert_eq!(std::mem::take(&mut app.usage.demo_keys), bytes, "{program}");
    }
}

#[test]
fn ended_tab_does_not_block_resume_and_typed_input_never_falls_back_to_it() {
    let mut st = AppState::new(crate::source::Source::Demo, None, Default::default());
    let mut term = Term::empty((10, 3));
    term.ended = Some(mdshared_pty::Exit { code: Some(0), signal: None });
    let tab =
        st.tabs.open(NewTab { label: "ended".into(), color: [0; 3], account: None, session: Some("id".into()) }, term);
    assert_eq!(st.tab_of_session("id"), None);
    assert_eq!(st.tabs.list[0].id, tab);
    st.debug_tab = Some(tab + 1);
    assert_eq!(st.debug_input_tab(), None);
}
#[test]
fn ended_tabs_still_handle_local_scrollback() {
    let mut app = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
    let mut term = Term::empty((10, 3));
    term.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
    term.ended = Some(mdshared_pty::Exit { code: Some(0), signal: None });
    app.tabs.open(NewTab { label: "ended".into(), color: [0; 3], account: None, session: None }, term);
    let key = KeyPress {
        name: Some(KeyName::PageUp),
        mods: Mods { shift: true, ..Mods::default() },
        text: None,
        repeat: false,
    };
    app.term_key(Instant::now(), &key);
    assert!(app.tabs.shown().unwrap().term.scroll > 0);
}

fn mouse_terminal() -> AppState {
    let mut app = AppState::new(crate::source::Source::Demo, None, Default::default());
    app.tabs.open(NewTab { label: "mouse".into(), color: [0; 3], account: None, session: None }, Term::empty((10, 3)));
    let area = Rect::new(0.0, 0.0, 400.0, 180.0);
    app.layout_terminals(area, area, 1.0);
    app.tabs.shown_mut().unwrap().term.feed(b"one\r\ntwo");
    app
}

fn cell(app: &AppState, row: usize, col: usize) -> (f32, f32) {
    let r = app.metrics.unwrap().cell_rect(app.term_area, row, col);
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

#[test]
fn a_terminal_drag_keeps_its_selection_until_right_click_copies_and_clears_it() {
    let mut app = mouse_terminal();
    let (x, y) = cell(&app, 0, 0);
    let (end_x, end_y) = cell(&app, 1, 2);
    let plain = Mods::default();
    assert_eq!(app.term_pointer_press(x, y, Button::Left, 1, plain), None);
    assert!(app.term_move(end_x, end_y, plain));
    assert!(app.term_release(end_x, end_y, Button::Left, plain));
    assert_eq!(app.tabs.shown().unwrap().term.selected_text().as_deref(), Some("one\ntwo"));
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, plain), Some(MouseClipboard::Copy("one\ntwo".into())));
    assert!(app.tabs.shown().unwrap().term.selection.is_none());
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, plain), Some(MouseClipboard::Paste));

    // Empty copied text is not a reason to paste old clipboard contents.
    let (blank_x, blank_y) = cell(&app, 2, 2);
    assert_eq!(app.term_pointer_press(x, blank_y, Button::Left, 1, plain), None);
    assert!(app.term_move(blank_x, blank_y, plain));
    assert!(app.term_release(blank_x, blank_y, Button::Left, plain));
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, plain), Some(MouseClipboard::Copy(String::new())));
}

#[test]
fn mouse_reporting_gets_clicks_unless_shift_starts_a_local_gesture() {
    let mut app = mouse_terminal();
    app.tabs.shown_mut().unwrap().term.feed(b"\x1b[?1002h\x1b[?1006h");
    let (x, y) = cell(&app, 0, 0);
    let (end_x, end_y) = cell(&app, 1, 2);
    let plain = Mods::default();
    let shift = Mods { shift: true, ..plain };

    assert_eq!(app.term_pointer_press(x, y, Button::Left, 1, plain), None);
    assert_eq!(app.tabs.shown().unwrap().term.reported, Some(MouseButton::Left));
    assert!(!app.term_move(end_x, end_y, plain));
    assert!(app.term_release(end_x, end_y, Button::Left, plain));
    assert!(app.tabs.shown().unwrap().term.selection.is_none());

    assert_eq!(app.term_pointer_press(x, y, Button::Left, 1, shift), None);
    assert!(app.tabs.shown().unwrap().term.reported.is_none());
    assert!(app.term_move(end_x, end_y, shift));
    assert!(app.term_release(end_x, end_y, Button::Left, plain), "letting go of Shift does not send a release");
    assert_eq!(
        app.term_pointer_press(x, y, Button::Right, 1, plain),
        None,
        "right-click reports even with a selection"
    );
    assert_eq!(app.tabs.shown().unwrap().term.reported, Some(MouseButton::Right));
    assert!(app.term_release(x, y, Button::Right, plain));
    assert_eq!(app.tabs.shown().unwrap().term.selected_text().as_deref(), Some("one\ntwo"));
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, shift), Some(MouseClipboard::Copy("onetwo".into())));
    assert!(app.tabs.shown().unwrap().term.selection.is_none());
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, shift), Some(MouseClipboard::Paste));
}

#[test]
fn terminal_clipboard_paste_refuses_unbracketed_lines_and_skips_ended_tabs() {
    let mut app = mouse_terminal();
    let multiline = || Some("one\r\ntwo".to_string());
    app.paste_terminal(multiline);
    assert!(app.info.take().is_some());
    let term = &mut app.tabs.shown_mut().unwrap().term;
    term.feed(b"\x1b[?2004h\x1b[?1002h");
    app.paste_terminal(multiline);
    assert!(app.info.is_none());

    app.tabs.shown_mut().unwrap().term.finish(mdshared_pty::Exit { code: Some(0), signal: None });
    let (x, y) = cell(&app, 0, 0);
    assert_eq!(app.term_pointer_press(x, y, Button::Right, 1, Mods::default()), None);
    assert!(app.tabs.shown().unwrap().term.reported.is_none());
    app.paste_terminal(|| panic!("an ended tab should not read the clipboard"));
}

#[test]
fn claude_tabs_get_the_classic_renderer_only_when_the_setting_is_on() {
    let launch = |program: &str| Launch {
        program: program.into(),
        args: vec![],
        cwd: None,
        env: Default::default(),
        env_remove: vec![],
        place: claude_switch_core::Place::Terminal,
        label: String::new(),
    };
    let mut claude = launch("claude");
    classic_renderer(&mut claude, true);
    assert_eq!(claude.env.get(CLASSIC_RENDERER).map(String::as_str), Some("1"));
    assert_eq!(CLASSIC_RENDERER, "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN");
    let mut off = launch("claude");
    classic_renderer(&mut off, false);
    assert!(off.env.is_empty());
    for handover in [r"C:\Users\me\.local\bin\claude.exe", r"C:\Tools\Claude.EXE", "claude.cmd"] {
        let mut tab = launch(handover);
        classic_renderer(&mut tab, true);
        assert_eq!(tab.env.get(CLASSIC_RENDERER).map(String::as_str), Some("1"), "{handover}");
    }
    let mut lookalike = launch(r"C:\Tools\claude-helper.exe");
    classic_renderer(&mut lookalike, true);
    assert!(lookalike.env.is_empty());
    let mut shell = launch("cmd.exe");
    classic_renderer(&mut shell, true);
    assert!(shell.env.is_empty(), "other programs keep their own full-screen views");
}
