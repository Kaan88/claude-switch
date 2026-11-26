use super::*;

fn state() -> AppState {
    let mut s = AppState::new(Source::Demo, Some(false), WindowFile::default());
    s.apply_scan(crate::demo::scan(&s.config));
    s
}

#[test]
fn the_demo_list_groups_every_unarchived_session_and_selects_the_first_pinned_row() {
    let s = state();
    let shown: usize = s.groups.iter().map(|g| g.rows.len()).sum();
    assert_eq!(shown, s.sessions.iter().filter(|x| !x.archived && !x.worker && !s.is_archived(x)).count());
    assert_eq!(s.selected.as_deref(), Some(s.sessions[2].id.as_str()));
    assert_eq!(s.groups[0].name, "Pinned");
    assert!(s.groups.iter().any(|g| g.name == "Older"));
}

#[test]
fn both_modes_promote_pins_and_filters_use_the_marked_state() {
    let mut s = state();
    for mode in [claude_switch_core::grouping::Mode::Recent, claude_switch_core::grouping::Mode::Projects] {
        s.config.sessions_mode = mode;
        s.config.group_by_folder = true;
        s.filter = Filter::All;
        s.rebuild_view();
        assert_eq!(s.groups[0].name, "Pinned");
        assert!(s.groups[0].rows.iter().all(|&i| s.marks.pinned(&s.sessions[i].id)));
        let count = s.groups.iter().map(|g| g.rows.len()).sum::<usize>();
        assert_eq!(count, s.sessions.iter().filter(|x| !x.archived && !x.worker && !s.is_archived(x)).count());
        s.filter = Filter::Running;
        s.rebuild_view();
        assert_eq!(s.groups.iter().map(|g| g.rows.len()).sum::<usize>(), 2);
        s.filter = Filter::Waiting;
        s.rebuild_view();
        let waiting: Vec<_> = s.groups.iter().flat_map(|g| &g.rows).collect();
        assert_eq!(waiting.len(), 1);
        assert!(s.sessions[*waiting[0]].turn_ended);
    }
}

#[test]
fn rename_hide_and_undo_update_the_projection_without_editing_a_transcript() {
    let mut s = state();
    let idx = 10;
    let id = s.sessions[idx].id.clone();
    s.filter = Filter::Unseen;
    s.rebuild_view();
    assert!(s.entry_of(&id).is_some());
    s.select(&id);
    assert!(s.entry_of(&id).is_none(), "showing the details clears the Unseen row");
    s.filter = Filter::All;
    s.rebuild_view();
    s.rename_session(&id, "local alias");
    assert_eq!(s.title_of(&s.sessions[idx]), "local alias");
    assert_ne!(s.sessions[idx].title, "local alias");
    s.rename_session(&id, "");
    assert_eq!(s.title_of(&s.sessions[idx]), s.sessions[idx].title);
    let now = Instant::now();
    s.hide_from_keyboard(now);
    assert!(s.marks.hidden(&id));
    assert!(s.entry_of(&id).is_none());
    s.config.show_hidden = true;
    s.filter = Filter::Hidden;
    s.rebuild_view();
    assert!(s.entry_of(&id).is_some());
    s.undo_hidden(now);
    assert!(s.entry_of(&id).is_none());
    assert!(!s.marks.hidden(&id));
}

#[test]
fn a_search_keeps_only_matching_sessions_and_an_empty_one_brings_all_back() {
    let mut s = state();
    let before = s.sections.len();
    s.search.set_text("splitter");
    s.rebuild_view();
    let shown: Vec<&Session> = s.groups.iter().flat_map(|g| g.rows.iter().map(|&i| &s.sessions[i])).collect();
    assert!(!shown.is_empty());
    assert!(shown.iter().all(|x| {
        crate::list::model::score("splitter", s.marks.name(&x.id), x, &crate::session_marks::folder_label(x)).is_some()
    }));
    s.search.clear();
    s.rebuild_view();
    assert_eq!(s.sections.len(), before);
}

#[test]
fn rows_map_to_their_sessions_and_back() {
    let s = state();
    for e in 0..s.sections.len() {
        if let Some(i) = s.session_at(e) {
            assert_eq!(s.entry_of(&s.sessions[i].id), Some(e));
        }
    }
}

#[test]
fn thousands_of_sessions_still_yield_only_a_viewport_of_draw_entries() {
    let mut s = AppState::new(Source::Demo, Some(false), WindowFile::default());
    let mut scan = crate::demo::scan(&s.config);
    let seed = scan.sessions.clone();
    for run in 0..14 {
        scan.sessions.extend(seed.iter().cloned().map(|mut session| {
            session.id.push_str(&format!("-{run}"));
            session
        }));
    }
    s.apply_scan(scan);
    s.sections.view = 380.0;
    assert!(s.sections.len() > 4_000);
    assert!(s.sections.visible().len() <= 16);
    s.sections.scroll = s.sections.reveal(s.sections.len() - 1);
    assert!(s.sections.visible().len() <= 16);
}

#[test]
fn worker_sessions_show_only_with_the_setting() {
    let mut s = state();
    let workers = s.sessions.iter().filter(|x| x.worker && !x.archived && !s.is_archived(x)).count();
    assert!(workers > 0);
    let shown = |s: &AppState| s.groups.iter().map(|g| g.rows.len()).sum::<usize>();
    let hidden = shown(&s);
    s.config.show_workers = true;
    s.rebuild_view();
    assert_eq!(shown(&s), hidden + workers);
}

#[test]
fn a_session_open_in_a_tab_counts_as_open_there() {
    let mut s = state();
    let id = s.sessions[3].id.clone();
    assert!(s.tab_of_session(&id).is_none());
    let new = crate::tabs::NewTab {
        label: "work · x · Resume".into(),
        color: [0, 0, 0],
        session: Some(id.clone()),
        account: Some("work".into()),
    };
    let tab = s.tabs.open(new, crate::term::Term::empty((10, 3)));
    assert_eq!(s.tab_of_session(&id), Some(tab));
    assert_eq!(s.tab_account(&id).as_deref(), Some("work"));
    s.tabs.activate(crate::tabs::SESSIONS);
    s.open(Instant::now(), &id, "main".into(), claude_switch_core::App::Cli);
    assert_eq!(s.tabs.active, Some(tab), "opening it again shows its tab");
    assert!(s.dialog.is_none() && s.info.is_none());
    s.close_tab_now(tab);
    assert!(s.tab_of_session(&id).is_none());
}

#[test]
fn a_collapsed_folder_stays_collapsed_after_a_rebuild() {
    let mut s = state();
    let key = s.groups[0].key.clone();
    s.collapsed.insert(key);
    s.rebuild_view();
    assert!(s.sections.is_collapsed(0));
}
