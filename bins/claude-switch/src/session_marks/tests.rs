use super::*;
use crate::tabs::NewTab;
use crate::term::Term;
use crate::view::model::Source as TranscriptSource;
use claude_switch_core::{App, Roots, marks::Marks};

const A: &str = "11111111-0000-0000-0000-000000000000";
const B: &str = "22222222-0000-0000-0000-000000000000";

fn open(st: &mut AppState, session: Option<String>, source: TranscriptSource, cwd: std::path::PathBuf) -> u64 {
    let pane = st.new_session_pane(source, cwd);
    let id = st.tabs.open(
        NewTab { label: "Synthetic".into(), color: [0; 3], session, account: Some("main".into()) },
        Term::empty((60, 20)),
    );
    st.tabs.get_mut(id).unwrap().pane = Some(pane);
    id
}

#[test]
fn identical_running_snapshot_retries_after_transcript_appears() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    let scratch = crate::testing::Scratch::new("cs-identical-running");
    let root = scratch.to_path_buf();
    std::fs::create_dir(&root).unwrap();
    let cwd = root.join("work");
    let file = crate::view::handover_transcript(&root, &cwd, A);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let source = TranscriptSource::Discover {
        config: root.clone(),
        cwd: cwd.clone(),
        started: std::time::UNIX_EPOCH,
        verified: Default::default(),
    };
    let tab = open(&mut st, None, source, cwd.clone());
    let record = Running { session_id: A.into(), app: App::Cli, account: "main".into(), pid: 42, cwd };
    let identify = |st: &mut AppState| {
        let id = &st.running[0].session_id;
        st.tabs.get_mut(tab).unwrap().apply_live_identity(Some(id))
    };
    assert!(st.update_running(vec![record.clone()], |_| true, identify));
    assert!(st.tabs.get(tab).unwrap().needs_identity());
    assert!(st.tabs.by_session(A, |_| true).is_none());
    std::fs::write(&file, b"{}\n").unwrap();
    assert!(st.update_running(vec![record.clone()], |_| true, identify));
    assert!(!st.tabs.get(tab).unwrap().needs_identity());
    assert_eq!(st.tabs.by_session(A, |_| true).unwrap().id, tab);
    assert!(!st.update_running(
        vec![record.clone()],
        |_| true,
        |_| { panic!("resolved tab need not retry unchanged snapshot") }
    ));
    let resumed = open(&mut st, Some(A.into()), TranscriptSource::File(file), record.cwd.clone());
    assert!(st.update_running(
        vec![record],
        |_| true,
        |st| {
            let id = &st.running[0].session_id;
            st.tabs.get_mut(resumed).unwrap().apply_live_identity(Some(id))
        }
    ));
    assert!(!st.tabs.get(resumed).unwrap().needs_identity());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mismatch_keeps_the_pinned_pane_but_removes_routing_and_seen_identity() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.window_focused = true;
    let s = st.sessions[0].clone();
    st.marks = Marks::default();
    let source = TranscriptSource::File(s.file.with_file_name(format!("{}.jsonl", s.id)));
    let tab_id = open(&mut st, Some(s.id.clone()), source, s.cwd.clone());
    let tab = st.tabs.get_mut(tab_id).unwrap();
    assert!(tab.apply_live_identity(Some(&s.id)));
    assert_eq!(tab.pane.as_ref().unwrap().pinned_id(), Some(s.id.as_str()));
    assert!(tab.apply_live_identity(None));
    assert!(tab.resume_id.is_none());
    assert!(st.tabs.by_session(&s.id, |_| true).is_some(), "an absent PID row must not permit a duplicate resume");
    st.see_shown();
    assert!(st.marks.unseen(&s.id, s.transcript_ms));
    let tab = st.tabs.get_mut(tab_id).unwrap();
    assert!(tab.apply_live_identity(Some(B)));
    assert!(tab.attention.transcript().is_none());
    assert!(tab.resume_id.is_none());
    assert!(tab.needs_identity());
    assert_eq!(tab.pane.as_ref().unwrap().pinned_id(), Some(s.id.as_str()));
    assert!(st.tabs.by_session(&s.id, |_| true).is_none());
    assert!(st.tabs.by_session(B, |_| true).is_none());
    st.see_shown();
    assert!(st.marks.unseen(&s.id, s.transcript_ms));
}

#[test]
fn a_competing_writer_merges_unless_it_changed_the_same_field_which_rolls_back() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.select_first();
    let s = st.selected_session().unwrap().clone();
    st.marks = Marks::default();
    let scratch = crate::testing::Scratch::new("cs-marks-conflict");
    let root = scratch.to_path_buf();
    std::fs::create_dir(&root).unwrap();
    let roots = Roots { home: root.clone(), appdata: root.clone() };
    let marks = roots.config_file().with_file_name("marks.json");
    std::fs::create_dir_all(marks.parent().unwrap()).unwrap();
    let now = Instant::now();
    let key = folder_key(&s.cwd);
    std::fs::write(&marks, b"{\"otherWindow\":true}").unwrap();
    st.source = Source::Real(roots.clone());
    st.marks_flush_at = Some(now);
    st.rename_session(&s.id, "Saved");
    st.toggle_pin(st.by_id[&s.id]);
    assert!(st.info.is_none(), "another window's unrelated change is merged");
    let disk = Marks::load(&roots).unwrap();
    assert_eq!((disk.name(&s.id), disk.pinned(&s.id)), (Some("Saved"), true));
    assert!(std::fs::read_to_string(&marks).unwrap().contains("otherWindow"));
    // Another window renames the same session meanwhile: a true conflict.
    let mut other = Marks::load(&roots).unwrap();
    other.rename(&s.id, &s.file, "Theirs");
    other.save(&roots).unwrap();
    st.rename_session(&s.id, "Not saved");
    assert_ne!(st.marks.name(&s.id), Some("Not saved"));
    assert_eq!(st.selected.as_deref(), Some(s.id.as_str()));
    assert!(st.info.is_some());
    assert_eq!(Marks::load(&roots).unwrap().name(&s.id), Some("Theirs"));
    st.info = None;
    st.toggle_hidden(now, false);
    assert!(st.info.is_none(), "the next change saves");
    assert!(Marks::load(&roots).unwrap().hidden(&s.id));
    st.toggle_group_hidden(now, key.clone());
    assert!(Marks::load(&roots).unwrap().folder_hidden(&key));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn handover_for_a_verified_tab_missing_from_the_index_refreshes_and_explains() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    let cwd = std::env::temp_dir();
    let source = TranscriptSource::File(cwd.join(format!("{A}.jsonl")));
    let tab_id = open(&mut st, Some(A.into()), source, cwd);
    assert!(st.tabs.get_mut(tab_id).unwrap().apply_live_identity(Some(A)));
    st.ask_tab_compact(tab_id, None);
    assert!(st.scan_job.is_some());
    assert!(st.info.is_some());
    assert!(st.dialog.is_none());
}
