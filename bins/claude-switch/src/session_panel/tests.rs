use super::*;
use crate::source::Source;
use crate::state::MenuKind;
use crate::winfile::WindowFile;

fn demo_state() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(true), WindowFile::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st
}

#[test]
fn batch_without_trustworthy_marks_cannot_prepare_or_start_naming() {
    let mut st = crate::state::AppState::new(crate::source::Source::Demo, None, Default::default());
    st.marks_read_only = true;
    st.ask_naming(true);
    assert!(st.dialog.is_none() && st.info.is_some());
    st.start_naming(vec!["anything".into()], true);
    assert!(st.naming_job.is_none());
}

#[test]
fn switcher_matches_the_prompt_and_ctrl_enter_can_override_the_folder_default() {
    let mut st = demo_state();
    let target = st.sessions[2].id.clone();
    let now = Instant::now();
    st.quick_switcher();
    crate::dialogs::text_input(&mut st, "compare a clean build");
    crate::dialogs::key(&mut st, now, &KeyName::Enter, Mods { ctrl: true, ..Mods::default() }, None);
    assert_eq!(st.selected.as_deref(), Some(target.as_str()));
    let Some((kind, menu)) = &st.menu else { panic!("Open as menu not shown") };
    assert_eq!(*kind, MenuKind::OpenAs);
    assert!(menu.items.iter().any(|item| item.label.contains("Work")));
    assert!(menu.items.iter().any(|item| item.label.contains("Main")));
}

#[test]
fn batch_confirms_the_cap_and_review_never_saves_before_accept() {
    let mut st = demo_state();
    st.ask_naming(true);
    let Some(Dialog::Confirm { then: Pending::Suggest { ids, .. }, .. }) = st.dialog.take() else {
        panic!("batch requires explicit confirmation");
    };
    assert_eq!(ids.len(), naming::MAX_BATCH);
    let first = ids[0].clone();
    let second = ids[1].clone();
    let sample =
        |id: &str| Suggestion { id: id.to_string(), original: "Before".into(), title: "Generated".into(), topic: None };
    st.dialog = review(VecDeque::from([sample(&first), sample(&second)]), 1, 2, 0);
    assert!(st.marks.name(&first).is_none() && st.marks.name(&second).is_none());
    crate::dialogs::click(&mut st, Instant::now(), SKIP, 0.0);
    crate::dialogs::text_input(&mut st, "Edited proposal");
    crate::dialogs::click(&mut st, Instant::now(), OK, 0.0);
    assert!(st.marks.name(&first).is_none());
    assert_eq!(st.marks.name(&second), Some("Edited proposal"));
}

#[test]
fn edit_topic_in_the_same_review_and_reject_an_invalid_edit_before_saving_either_field() {
    let mut st = demo_state();
    let id = st.sessions[0].id.clone();
    let old = st.marks.topic(&id).unwrap().to_string();
    let example = Suggestion {
        id: id.clone(),
        original: "Before".into(),
        title: "Generated".into(),
        topic: Some("Suggested".into()),
    };
    st.dialog = review(VecDeque::from([example]), 1, 1, 0);
    let now = Instant::now();
    crate::dialogs::key(&mut st, now, &KeyName::Tab, Mods::default(), None);
    crate::dialogs::text_input(&mut st, "this is too long");
    crate::dialogs::click(&mut st, now, OK, 0.0);
    assert_eq!(st.marks.topic(&id), Some(old.as_str()));
    assert_eq!(st.marks.name(&id), None);
    if let Some(Dialog::Session(panel)) = &mut st.dialog {
        assert!(panel.invalid_topic);
        panel.topic.select_all();
    } else {
        panic!("the review should stay open");
    }
    crate::dialogs::text_input(&mut st, "Navigation regression");
    crate::dialogs::click(&mut st, now, OK, 0.0);
    assert_eq!(st.marks.topic(&id), Some("Navigation regression"));
    assert_eq!(st.marks.name(&id), Some("Generated"));
}
