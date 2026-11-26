use super::*;

#[test]
fn ambiguous_desktop_delivery_does_not_claim_an_account() {
    assert_eq!(resume_status(App::Desktop, "Session", "Work"), "“Session” handed to the desktop app");
}

#[test]
fn palette_entry_keeps_its_session_after_reordering_and_refuses_a_removed_id() {
    let mut st = crate::state::AppState::new(crate::source::Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.open_palette();
    let (&entry, id) = st.palette_sessions.iter().next().unwrap();
    let id = id.clone();
    st.sessions.reverse();
    st.by_id = st.sessions.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
    assert_eq!(st.palette_session(entry), Some(id.as_str()));
    st.by_id.remove(&id);
    assert_eq!(st.palette_session(entry), None);
}

#[test]
fn one_confirmation_holds_whichever_warnings_apply() {
    assert_eq!(open_warning(None, false), None);
    assert_eq!(open_warning(Some("Cold cache."), false).as_deref(), Some("Cold cache."));
    assert_eq!(open_warning(None, true).as_deref(), Some(DESKTOP_WARNING));
    let both = open_warning(Some("Cold cache."), true).unwrap();
    assert!(both.starts_with("Cold cache.") && both.ends_with(DESKTOP_WARNING));
}

#[test]
fn a_desktop_session_opened_in_the_cli_asks_first_and_a_cli_one_does_not() {
    let mut s =
        crate::state::AppState::new(crate::source::Source::Demo, Some(false), crate::winfile::WindowFile::default());
    s.apply_scan(crate::demo::scan(&s.config));
    let running: Vec<String> = s.running.iter().map(|r| r.session_id.clone()).collect();
    let free = |want: App| {
        s.sessions
            .iter()
            .find(|x| x.last_app == want && x.desktop_id.is_none() == (want == App::Cli) && !running.contains(&x.id))
            .filter(|x| crate::demo::last_account(x) == "main")
            .map(|x| x.id.clone())
    };
    let (desk, cli) = (free(App::Desktop).unwrap(), free(App::Cli).unwrap());
    s.open(Instant::now(), &desk, "main".into(), App::Cli);
    match &s.dialog {
        Some(Dialog::Confirm { text, then: Pending::Open { .. }, .. }) => assert_eq!(text, DESKTOP_WARNING),
        _ => panic!("no desktop warning"),
    }
    s.dialog = None;
    s.open(Instant::now(), &cli, "main".into(), App::Cli);
    assert!(s.dialog.is_none());

    s.open(Instant::now(), &desk, "work".into(), App::Cli);
    match &s.dialog {
        Some(Dialog::Confirm { text, then: Pending::Open { id, account, app: App::Cli }, .. }) => {
            assert_eq!((id, account.as_str()), (&desk, "work"));
            assert!(text.contains("full price") && text.ends_with(DESKTOP_WARNING));
        }
        _ => panic!("expected one confirmation containing both warnings"),
    }
    crate::dialogs::click(&mut s, Instant::now(), crate::dialogs::OK, 0.0);
    assert!(s.dialog.is_none(), "Continue does not open a second confirmation");

    s.open(Instant::now(), &desk, "main".into(), App::Desktop);
    assert!(s.dialog.is_none(), "the Desktop warning only applies when opening in the CLI");
    let i = s.by_id[&desk];
    s.sessions[i].desktop_id = None;
    s.sessions[i].desktop_account = None;
    s.open(Instant::now(), &desk, "main".into(), App::Cli);
    assert!(s.dialog.is_none(), "a Desktop entrypoint without a Desktop record is not a listing");

    // A copy of a running session is not refused.
    s.open_copy(Instant::now(), &running[0]);
    assert!(s.info.is_none());
    // Enter chooses the primary button: Cancel for closing, Continue for opening.
    assert!(!Pending::Quit.ok_is_primary() && !Pending::CloseTab(1).ok_is_primary());
    assert!(Pending::Copy { id: cli.clone(), account: "main".into() }.ok_is_primary());
}
