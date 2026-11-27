use super::*;
use crate::source::Source;
use crate::winfile::WindowFile;
use claude_switch_core::grouping::Mode;
use std::time::{Duration, Instant};

/// The demo source: its "model" is `demo_answer`, so nothing leaves the computer.
fn demo() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(true), WindowFile::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st
}

fn finish(st: &mut AppState) {
    let started = Instant::now();
    while st.meaning.job.is_some() {
        st.take_meaning_job();
        assert!(started.elapsed() < Duration::from_secs(20), "the job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn confirm(st: &mut AppState) {
    assert!(matches!(st.dialog, Some(Dialog::Confirm { .. })), "a run that sends something is confirmed first");
    crate::dialogs::click(st, Instant::now(), crate::dialogs::OK, 0.0);
}

#[test]
fn off_by_default_and_nothing_starts_while_off() {
    let mut st = demo();
    assert!(!st.config.meaning.on());
    st.ask_meaning_index();
    st.ask_meaning_question();
    assert!(st.dialog.is_none() && st.info.is_some());
    assert!(st.meaning.job.is_none());
}

#[test]
fn the_confirmation_names_what_is_sent_and_to_which_model() {
    let t = std::env::temp_dir().join(format!("cs-meaning-confirm-{}", std::process::id()));
    let mut st = demo();
    st.source = Source::Real(claude_switch_core::Roots { home: t.clone(), appdata: t.clone() });
    st.config.meaning.enabled = true;
    st.config.meaning.engine = Engine::OpenCode("prov/model-x".into());
    st.ask_meaning_index();
    let Some(Dialog::Confirm { text, then: Pending::MeaningIndex, .. }) = &st.dialog else { panic!("no confirmation") };
    assert!(text.contains("OpenCode prov/model-x"), "{text}");
    assert!(text.contains("first and up to three recent prompts (up to 500 characters each)"));
    assert!(text.contains("hidden sessions are never sent"));
    assert!(st.meaning.job.is_none(), "nothing runs before the confirmation");
    st.config.meaning.engine = Engine::Claude;
    st.ask_meaning_index();
    let Some(Dialog::Confirm { text, .. }) = &st.dialog else { panic!("no confirmation") };
    assert!(text.contains("Claude Haiku (the Main account)"), "{text}");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn summaries_make_search_and_topics_work_by_meaning_without_another_request() {
    let mut st = demo();
    st.config.meaning.enabled = true;
    st.ask_meaning_index();
    confirm(&mut st);
    assert!(matches!(&st.dialog, Some(Dialog::Session(p)) if p.meaning_working_now()));
    finish(&mut st);
    assert!(st.dialog.is_none());
    let kept = st.meaning.cache.digests.len();
    assert!(kept >= meaning::BATCH, "{kept}");
    assert!(st.info.as_ref().is_some_and(|i| i.title().contains("summarized")));

    // A session found by its summary's words, though they need not be in its title.
    let (id, digest) = st.meaning.cache.digests.iter().next().map(|(id, d)| (id.clone(), d.clone())).unwrap();
    assert!(st.meaning_match(&id, &digest.keywords[0]));
    st.config.meaning.enabled = false;
    assert!(!st.meaning_match(&id, &digest.keywords[0]), "nothing by meaning while it is off");
    st.config.meaning.enabled = true;

    // Topics groups by subject, across projects.
    st.config.sessions_mode = Mode::Topics;
    st.config.meaning.group_topics = true;
    st.rebuild_view();
    assert_eq!(st.list_mode(), Mode::Topics);
    assert!(st.groups.iter().any(|g| g.key.starts_with("subject:")));
    assert!(st.groups.iter().any(|g| g.key == "subject-none"), "sessions not summarized yet stay listed");

    // Asking again sends only sessions that are not current; with none left, nothing is asked.
    let before = st.meaning.cache.clone();
    for _ in 0..10 {
        st.dialog = None;
        st.ask_meaning_index();
        if !matches!(st.dialog, Some(Dialog::Confirm { .. })) {
            break;
        }
        confirm(&mut st);
        finish(&mut st);
    }
    assert!(st.meaning.cache.digests.len() > before.digests.len());
    assert!(st.info.as_ref().is_some_and(|i| i.title() == "Nothing to summarize"), "every run ends");
    assert_eq!(st.meaning.cache.digests[&id], before.digests[&id], "a current summary is not redone");
}

#[test]
fn a_question_is_confirmed_answered_and_then_answered_from_the_cache() {
    let mut st = demo();
    st.config.meaning.enabled = true;
    st.ask_meaning_question();
    st.submit_meaning_question("where did we fix the swipe?");
    assert!(st.dialog.is_none() && st.info.is_some(), "no summaries yet: nothing to ask over");

    st.ask_meaning_index();
    confirm(&mut st);
    finish(&mut st);
    st.ask_meaning_question();
    crate::dialogs::text_input(&mut st, "where did we fix the swipe?");
    crate::dialogs::click(&mut st, Instant::now(), crate::dialogs::OK, 0.0);
    let Some(Dialog::Confirm { then: Pending::MeaningAsk { question }, .. }) = &st.dialog else {
        panic!("asking is confirmed")
    };
    assert_eq!(question, "where did we fix the swipe?");
    confirm(&mut st);
    finish(&mut st);
    assert!(matches!(&st.dialog, Some(Dialog::Session(_))), "the answer is shown");
    assert_eq!(st.meaning.cache.answers.len(), 1);
    let answer = st.meaning.cache.answers[0].clone();
    assert_eq!(answer.ids.len(), 3);

    // The same question again: the kept answer, at once, with no confirmation and no job.
    st.dialog = None;
    st.submit_meaning_question("Where did we  fix the SWIPE?");
    assert!(matches!(&st.dialog, Some(Dialog::Session(_))));
    assert!(st.meaning.job.is_none());

    // Opening a named session from the answer selects it.
    crate::dialogs::click(&mut st, Instant::now(), crate::dialogs::OK, 0.0);
    assert_eq!(st.selected.as_deref(), Some(answer.ids[0].as_str()));
}

#[test]
fn cancel_stops_the_job_and_shows_no_result() {
    let mut st = demo();
    st.config.meaning.enabled = true;
    st.ask_meaning_index();
    confirm(&mut st);
    crate::dialogs::click(&mut st, Instant::now(), crate::dialogs::CANCEL, 0.0);
    assert!(st.dialog.is_none());
    assert!(st.meaning.job.as_ref().is_some_and(|job| job.cancelled()));
    finish(&mut st);
    assert!(st.dialog.is_none());
}

#[test]
fn the_demo_model_answers_both_kinds_of_request() {
    let items = [Excerpts { first: Some("Fix the swipe gesture".into()), last: vec![] }];
    let index = demo_answer(&meaning::index_request(&items, &[]));
    let parsed = meaning::parse_index(&index, 1, &[]);
    assert_eq!(parsed[&0].2.as_deref(), Some("Gestures"));
}

#[test]
fn a_session_hidden_after_it_was_summarized_lends_no_subject_to_the_next_request() {
    let mut st = demo();
    st.config.meaning.enabled = true;
    let hidden = st.sessions[0].clone();
    let shown = st.sessions[1].clone();
    let kept = |subject: &str| meaning::Digest {
        basis: 1,
        active_ms: u64::MAX,
        engine: "claude".into(),
        summary: "kept".into(),
        keywords: vec![],
        subject: Some(subject.into()),
    };
    st.meaning.cache.digests.insert(hidden.id.clone(), kept("Secret area"));
    st.meaning.cache.digests.insert(shown.id.clone(), kept("Open area"));
    st.marks.hide(&hidden.id, &hidden.file, true);
    assert!(st.hidden_session(&hidden));
    let visible: std::collections::HashSet<_> =
        st.meaning_sessions().into_iter().map(|i| st.sessions[i].id.clone()).collect();
    assert!(!visible.contains(&hidden.id) && visible.contains(&shown.id));
    let subjects = st.meaning.cache.subjects(|id| visible.contains(id));
    assert_eq!(subjects, ["Open area"]);
}
