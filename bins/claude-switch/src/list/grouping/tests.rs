use super::*;
use crate::source::Source;

fn demo() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st
}

#[test]
fn project_rows_include_the_worktree_and_pins_but_the_folder_option_splits_them() {
    let mut st = demo();
    st.config.sessions_mode = Mode::Projects;
    let key = format!("project:{}", st.project(6).unwrap().key);
    st.rebuild_view();
    let group = st.groups.iter().find(|g| g.key == key).unwrap();
    assert_eq!(group.name, "kweb-mail");
    assert!(group.rows.contains(&0) && group.rows.contains(&3) && group.rows.contains(&6));
    assert!(group.rows.windows(2).all(|w| st.sessions[w[0]].last_active_ms >= st.sessions[w[1]].last_active_ms));
    assert!(st.grouping.project_summary[&key].contains("1 running · 0 waiting"));
    st.config.group_by_folder = true;
    st.rebuild_view();
    assert!(!st.groups.iter().any(|g| g.rows.contains(&6) && g.rows.contains(&0)));
}

#[test]
fn threads_fold_only_earlier_matches_and_related_navigation_reveals_them() {
    let mut st = demo();
    st.config.sessions_mode = Mode::Threads;
    st.rebuild_view();
    let first = st.sessions[0].id.clone();
    let earlier = st.sessions[3].id.clone();
    assert_eq!(st.grouping.earlier.get(&0), Some(&3));
    assert!(st.entry_of(&first).is_some() && st.entry_of(&earlier).is_none());
    st.select(&first);
    assert_eq!(st.grouping.related[0], (earlier.clone(), Relation::Earlier));
    st.select(&earlier);
    st.reveal_selected();
    assert!(st.entry_of(&earlier).is_some());
    assert_eq!(st.grouping.related[0], (first.clone(), Relation::Later));
    st.toggle_thread(0);
    assert!(st.entry_of(&earlier).is_none());
    assert_eq!(st.selected.as_deref(), Some(first.as_str()));
    st.filter = Filter::Running;
    st.rebuild_view();
    assert!(st.grouping.earlier.is_empty());
    assert_eq!(st.groups.iter().map(|g| g.rows.len()).sum::<usize>(), 2);
}

#[test]
fn related_worker_returns_up_to_five_for_the_current_selection_only() {
    let mut st = demo();
    let project = st.project(0).unwrap().key.clone();
    let eligible: Vec<_> = (10..st.sessions.len())
        .filter(|&i| st.project(i).is_some_and(|p| p.key == project) && st.related_visible(&st.sessions[i]))
        .take(7)
        .collect();
    assert_eq!(eligible.len(), 7);
    for i in std::iter::once(0).chain(eligible.iter().copied()) {
        st.sessions[i].grouping.first_prompt = Some("quux needle frobnicate".into());
        st.sessions[i].last_prompt = "quux needle frobnicate".into();
    }
    let mut select = |i: usize| {
        let id = st.sessions[i].id.clone();
        st.select(&id);
    };
    select(1);
    select(0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while st.grouping.pending.as_ref().is_some_and(Job::running) {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert!(st.take_related());
    let similar: Vec<_> =
        st.grouping.related.iter().filter(|(_, r)| *r == Relation::Similar).map(|(id, _)| st.by_id[id]).collect();
    assert_eq!(similar, eligible[..5]);
}

#[test]
fn topic_mode_is_conditional_and_filtering_and_local_edits_preserve_hiding_and_archive() {
    let mut st = demo();
    st.config.sessions_mode = Mode::Topics;
    st.rebuild_view();
    assert_eq!(st.list_mode(), Mode::Topics);
    assert!(st.groups.iter().any(|g| g.name == "No topic"));
    st.search.set_text("interface");
    st.rebuild_view();
    assert!(st.groups.iter().any(|g| g.name == "Interface"));
    st.search.clear();
    st.filter = Filter::Archived;
    st.rebuild_view();
    assert!(st.groups.iter().flat_map(|g| &g.rows).all(|&i| st.is_archived(&st.sessions[i])));
    for s in &st.sessions {
        st.marks.set_topic(&s.id, &s.file, "");
    }
    st.rebuild_view();
    assert_eq!(st.list_mode(), Mode::Recent);
}
