use super::*;

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
