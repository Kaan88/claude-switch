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
