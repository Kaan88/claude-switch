use super::*;

#[test]
fn archive_row_explains_each_block_and_does_not_block_unarchiving() {
    for (pinned, running, hint) in
        [(false, false, ""), (true, false, "Pinned"), (false, true, "Running"), (true, true, "Pinned and running")]
    {
        let flags = crate::list::model::Flags { pinned, running, ..Default::default() };
        let item = archive_row_item(false, flags);
        assert_eq!(item.enabled, hint.is_empty());
        assert_eq!(item.hint, hint);
        let archived = archive_row_item(true, flags);
        assert_eq!(archived.id, UNARCHIVE_ROW);
        assert!(archived.enabled);
    }
}

#[test]
fn terminal_palette_uses_the_visible_verified_tab_not_the_identically_named_selected_session() {
    let mut app = AppState::new(crate::source::Source::Demo, None, Default::default());
    app.apply_scan(crate::demo::scan(&app.config));
    app.sessions[1].title = app.sessions[0].title.clone();
    let selected = app.sessions[0].id.clone();
    let visible = app.sessions[1].id.clone();
    app.select(&selected);
    let tab = app.tabs.open(
        crate::tabs::NewTab {
            label: app.sessions[1].title.clone(),
            color: app.account_color("main"),
            session: Some(visible.clone()),
            account: Some("main".into()),
        },
        crate::term::Term::empty((80, 24)),
    );
    app.show_tab(tab);
    assert_eq!(app.selected.as_deref(), Some(selected.as_str()));
    assert_eq!(app.tabs.shown().unwrap().id, tab);
    app.command(Instant::now(), crate::compact_dialog::COMMAND, None);
    assert_eq!(app.compact_source(), Some((visible.as_str(), Some(tab))));
    app.dialog = None;
    app.tabs.get_mut(tab).unwrap().resume_id = None;
    app.command(Instant::now(), crate::compact_dialog::COMMAND, None);
    assert_eq!(app.compact_source(), None);
}

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
    assert!(Pending::Copy { id: cli.clone(), account: "main".into(), delete: false }.ok_is_primary());
}

#[test]
fn a_session_continued_in_another_folder_is_listed_there_and_stays_there_after_a_scan() {
    let mut app = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
    app.apply_scan(crate::demo::scan(&app.config));
    let (id, old) = (app.sessions[1].id.clone(), app.sessions[1].cwd.clone());
    let new = std::path::PathBuf::from(r"C:\work\moved-here");
    assert_ne!(old, new);
    assert!(app.set_session_folder(&id, &new));
    let cwd = |app: &AppState| app.sessions[app.by_id[&id]].cwd.clone();
    assert_eq!(cwd(&app), new);
    assert!(app.all_groups.iter().any(|g| g.cwd == new));
    // The next scan reads the transcript's first folder again; the mark wins.
    app.apply_scan(crate::demo::scan(&app.config));
    assert_eq!(cwd(&app), new);
    assert_eq!(app.marks.folder(&id), Some(r"C:\work\moved-here"));
}

#[test]
fn a_session_open_in_a_tab_is_not_moved() {
    let mut app = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
    app.apply_scan(crate::demo::scan(&app.config));
    let (id, old) = (app.sessions[1].id.clone(), app.sessions[1].cwd.clone());
    app.tabs.open(
        crate::tabs::NewTab { label: "Tab".into(), color: [0; 3], session: Some(id.clone()), account: None },
        crate::term::Term::empty((80, 24)),
    );
    let folder = std::env::temp_dir();
    app.continue_in_folder(std::time::Instant::now(), &id, &folder);
    assert_eq!(app.sessions[app.by_id[&id]].cwd, old);
    assert_eq!(app.marks.folder(&id), None);
}
