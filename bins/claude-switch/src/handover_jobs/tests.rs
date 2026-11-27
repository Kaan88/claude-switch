use super::*;
use crate::compact_dialog::{self as dialog, Compact};
use crate::source::Source;
use mdshared_ui::keys::{KeyName, Mods};

fn app() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.select_first();
    st
}

/// The setup dialog for the `n`th session, as **Compact and open…** opens it.
fn ask(st: &mut AppState, n: usize) {
    let session = st.sessions[n].clone();
    st.ask_compact_for(session, Some("work"), false);
}

/// Presses Make handover with a worker that returns `result` once `gate` lets it (or is
/// cancelled), as the real one returns the model's text.
fn make(st: &mut AppState, gate: std::sync::mpsc::Receiver<Result<String, String>>) {
    let wake = st.wake_fn();
    let Some(Dialog::Compact(d)) = &mut st.dialog else { panic!("no handover dialog") };
    d.start_test_generation(wake, gate);
    crate::dialogs::click(st, Instant::now(), crate::dialogs::OK, 0.0);
}

/// Waits for every worker to end and takes what it wrote.
fn settle(st: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while st.compact.jobs.list.iter().any(|j| j.state == State::Writing) {
        st.take_background_handovers();
        assert!(Instant::now() < deadline, "the worker ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn states(st: &AppState) -> Vec<State> {
    st.compact.jobs.list.iter().map(|j| j.state.clone()).collect()
}

fn review(st: &AppState) -> &Compact {
    let Some(Dialog::Compact(d)) = &st.dialog else { panic!("expected the review") };
    d
}

#[test]
fn generation_runs_behind_and_its_review_opens_by_itself_with_every_choice_kept() {
    let mut st = app();
    ask(&mut st, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(Dialog::Compact(d)) = &mut st.dialog {
        d.delete_original = true;
    }
    make(&mut st, rx);
    assert!(st.dialog.is_none(), "the dialog closes at once: the app is usable");
    assert_eq!(states(&st), [State::Writing]);
    assert!(!st.take_background_handovers(), "still writing");
    assert!(!st.open_finished_handover(Instant::now()));
    tx.send(Ok("## Goal\n\nWritten behind.".into())).unwrap();
    settle(&mut st);
    assert_eq!(states(&st), [State::Finished]);
    assert!(st.compact.jobs.flash.as_deref().is_some_and(|t| t.starts_with("Handover ready")));
    assert!(st.open_finished_handover(Instant::now()));
    assert!(st.compact.jobs.list.is_empty(), "the review holds it now");
    let d = review(&st);
    assert!(d.is_review());
    assert_eq!(d.preview_text(), "## Goal\n\nWritten behind.");
    assert_eq!(d.account_slug(), Some("work"), "the account chosen before is kept");
    assert!(!d.delete_original, "the delete box is off each time the review opens, as before");
}

#[test]
fn a_finished_review_waits_for_modal_things_and_several_open_one_after_another() {
    let mut st = app();
    let mut gates = Vec::new();
    for n in 0..2 {
        ask(&mut st, n);
        let (tx, rx) = std::sync::mpsc::channel();
        make(&mut st, rx);
        gates.push(tx);
    }
    assert_eq!(states(&st), [State::Writing, State::Writing], "two at once");
    for (i, tx) in gates.iter().enumerate() {
        tx.send(Ok(format!("## Goal\n\nNumber {i}"))).unwrap();
    }
    settle(&mut st);
    st.dialog = Some(Dialog::About);
    assert!(!st.open_finished_handover(Instant::now()), "never over another dialog");
    st.dialog = None;
    st.menu = Some((crate::state::MenuKind::Main, mdshared_ui::menu::Menu::new(Vec::new(), 0.0, 0.0, false)));
    assert!(!st.open_finished_handover(Instant::now()), "nor over a menu");
    st.menu = None;
    assert!(st.open_finished_handover(Instant::now()));
    assert!(review(&st).preview_text().ends_with("Number 0"));
    assert!(!st.open_finished_handover(Instant::now()), "one at a time");
    // Acting on it (the demo opens nothing) closes it for good; the next one follows.
    let past = Instant::now() + SETTLE;
    crate::dialogs::click(&mut st, past, crate::dialogs::OK, 0.0);
    assert!(st.dialog.is_none());
    assert!(st.open_finished_handover(Instant::now()));
    assert!(review(&st).preview_text().ends_with("Number 1"));
    assert_eq!(states(&st), [], "acted-on and open ones leave the strip");
}

#[test]
fn a_review_that_opened_by_itself_ignores_keys_in_flight_then_answers_them() {
    let mut st = app();
    ask(&mut st, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx.send(Ok("## Goal\n\nKeep.".into())).unwrap();
    settle(&mut st);
    let opened = Instant::now();
    assert!(st.open_finished_handover(opened));
    let none = Mods::default();
    // The Enter half-typed into the composer, and the next letters, do nothing.
    assert!(crate::dialogs::key(&mut st, opened, &KeyName::Enter, none, None));
    crate::dialogs::text_input(&mut st, "x");
    assert!(st.dialog.is_some(), "Enter did not open anything");
    assert_eq!(review(&st).preview_text(), "## Goal\n\nKeep.");
    // A click on Open in CLI right away is ignored too.
    crate::dialogs::click(&mut st, opened, crate::dialogs::OK, 0.0);
    assert!(st.dialog.is_some());
    if let Some(Dialog::Compact(d)) = &mut st.dialog {
        d.settle_now();
    }
    crate::dialogs::text_input(&mut st, "y");
    assert!(review(&st).preview_text().contains('y'), "after it settles, typing edits");
}

#[test]
fn cancel_or_escape_parks_the_review_in_the_strip_and_review_brings_it_back() {
    let mut st = app();
    ask(&mut st, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx.send(Ok("## Goal\n\nParked.".into())).unwrap();
    settle(&mut st);
    st.open_finished_handover(Instant::now() - SETTLE * 2);
    if let Some(Dialog::Compact(d)) = &mut st.dialog {
        d.settle_now();
        d.edit_for_test(" Edited.");
    }
    let now = Instant::now();
    crate::dialogs::key(&mut st, now, &KeyName::Escape, Mods::default(), None);
    assert!(st.dialog.is_none());
    assert_eq!(states(&st), [State::Ready]);
    assert!(!st.open_finished_handover(now), "a parked one does not reopen by itself");
    let id = st.compact.jobs.list[0].id;
    st.handover_act(id, Act::Review, now);
    assert!(review(&st).preview_text().ends_with("Edited."), "the edit was kept");
    crate::dialogs::click(&mut st, now, crate::dialogs::CANCEL, 0.0);
    assert_eq!(states(&st), [State::Ready]);
    let id = st.compact.jobs.list[0].id;
    st.handover_act(id, Act::Dismiss, now);
    assert!(st.compact.jobs.list.is_empty() && st.dialog.is_none(), "dismiss discards it");
}

#[test]
fn cancel_stops_the_worker_and_a_failure_shows_its_reason_and_retries() {
    let mut st = app();
    ask(&mut st, 0);
    let (_keep, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    let id = st.compact.jobs.list[0].id;
    st.handover_act(id, Act::Cancel, Instant::now());
    assert!(st.compact.jobs.list.is_empty());
    assert!(st.toasts.find(|t| t.contains("canceled")).is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    while st.cancel_compact_work() {
        assert!(Instant::now() < deadline, "a cancelled worker ends");
        std::thread::sleep(Duration::from_millis(5));
    }

    ask(&mut st, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx.send(Err("OpenCode stopped: the model is not available.".into())).unwrap();
    settle(&mut st);
    assert_eq!(states(&st), [State::Failed("OpenCode stopped: the model is not available.".into())]);
    assert!(st.compact.jobs.flash.as_deref().is_some_and(|t| t.starts_with("Handover failed")));
    assert!(!st.open_finished_handover(Instant::now()), "a failure does not pop up");
    // Retry rechecks consent like the first try: without it the dialog asks again. (The demo
    // then writes at once, so allowing ends in the review.)
    let id = st.compact.jobs.list[0].id;
    st.handover_act(id, Act::Retry, Instant::now());
    assert!(!review(&st).is_review() && st.compact.jobs.list.is_empty(), "asks for consent");
    crate::dialogs::click(&mut st, Instant::now(), crate::dialogs::OK, 0.0);
    assert!(review(&st).is_review());
    // With consent recorded, Retry goes straight back to writing (here: the demo's instant text).
    assert!(st.config.compact.consents.contains("demo"));
}

#[test]
fn asking_again_for_a_session_with_a_handover_in_progress_does_not_start_another() {
    let mut st = app();
    ask(&mut st, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    ask(&mut st, 0);
    assert!(st.dialog.is_none(), "no second dialog for the same session");
    assert!(st.toasts.find(|t| t.contains("being written")).is_some());
    tx.send(Ok("## Goal\n\nOnce.".into())).unwrap();
    settle(&mut st);
    ask(&mut st, 0);
    assert!(review(&st).preview_text().ends_with("Once."), "asking again opens the written one");
    assert!(st.compact.jobs.list.is_empty());
}

#[test]
fn closing_the_window_says_what_happens_to_handovers_and_cancels_running_ones() {
    let mut st = app();
    assert_eq!(st.handover_close_note(), None);
    assert!(st.ask_quit(), "nothing running: closes without asking");
    ask(&mut st, 0);
    let (_keep, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    ask(&mut st, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx.send(Ok("## Goal\n\nUnread.".into())).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while states(&st).iter().all(|s| *s == State::Writing) {
        st.take_background_handovers();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    st.open_finished_handover(Instant::now());
    let note = st.handover_close_note().unwrap();
    assert!(note.contains("still being written") && note.contains("canceled"), "{note}");
    assert!(note.contains("not been opened") && note.contains("lost"), "the open review counts: {note}");
    assert!(!st.ask_quit(), "asks first");
    let Some(Dialog::Confirm { text, .. }) = &st.dialog else { panic!("a confirmation") };
    assert!(text.contains("being written"), "{text}");
    assert_eq!(states(&st), [State::Writing, State::Ready], "the open review was parked, not lost, if you stay");
    // Closing: the running worker is cancelled and the exit waits for it.
    st.cancel_compact_work();
    let deadline = Instant::now() + Duration::from_secs(5);
    while st.cancel_compact_work() {
        assert!(Instant::now() < deadline, "the exit is not held up");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_strip_takes_no_room_when_empty_and_folds_above_two() {
    let mut st = app();
    assert_eq!(st.compact.jobs.height(), 0.0);
    for n in 0..3 {
        ask(&mut st, n);
        let (_tx, rx) = std::sync::mpsc::channel();
        make(&mut st, rx);
        if n < 2 {
            assert_eq!(st.compact.jobs.height(), (n + 1) as f32 * ROW_H + 8.0);
        }
        std::mem::forget(_tx);
    }
    assert_eq!(st.compact.jobs.height(), ROW_H + 8.0, "three fold into one line");
    st.handover_act(0, Act::Fold, Instant::now());
    assert_eq!(st.compact.jobs.height(), 4.0 * ROW_H + 8.0);
    assert_eq!(elapsed(Duration::from_secs(42)), "0:42");
    assert_eq!(elapsed(Duration::from_secs(725)), "12:05");
    assert_eq!(elapsed(Duration::from_secs(3723)), "1:02:03");
    st.cancel_background_handovers();
}

/// The Sessions column with the strip in each state, drawn through the list's own code;
/// `CS_SHOT_DIR` keeps the pictures.
#[test]
fn sessions_column_draws_the_strip_between_the_chips_and_the_list() {
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    let mut st = app();
    let (w, h) = (380.0, 520.0);
    let column = Rect::new(0.0, 0.0, w, h);
    let mut shots = Vec::new();
    let mut draw = |st: &mut AppState, name: &str| -> Vec<(Rect, Target)> {
        let mut gfx = Gfx::new();
        let mut bmp = Bitmap::new(w as u32 * 2, h as u32 * 2);
        st.hits.clear();
        {
            let mut p = Painter::new(&mut bmp, &mut gfx, 2.0);
            let mut anim = mdshared_ui::anim::Animator::new();
            crate::list::draw_with(st, &mut anim, &mut p, column, Instant::now());
        }
        if let Ok(dir) = std::env::var("CS_SHOT_DIR") {
            bmp.write_png(&std::path::Path::new(&dir).join(format!("cs-strip-{name}.png"))).unwrap();
        }
        shots.push(name.to_owned());
        st.hits.clone()
    };
    let list_top = |st: &AppState| st.list_area.y;
    draw(&mut st, "empty");
    let bare = list_top(&st);

    ask(&mut st, 0);
    let (tx0, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    st.compact.jobs.list[0].started = Instant::now() - Duration::from_secs(42);
    let hits = draw(&mut st, "writing");
    assert_eq!(list_top(&st), bare + ROW_H + 8.0, "the list moves down by the strip");
    let cancel = hits.iter().find(|(_, t)| matches!(t, Target::HandoverJob(_, Act::Cancel))).unwrap().0;
    assert!(cancel.y >= bare && cancel.bottom() <= list_top(&st) && cancel.right() <= w);

    ask(&mut st, 1);
    let (tx1, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx1.send(Err("OpenCode is not installed or not on PATH.".into())).unwrap();
    while !matches!(states(&st).get(1), Some(State::Failed(_))) {
        st.take_background_handovers();
        std::thread::sleep(Duration::from_millis(5));
    }
    let hits = draw(&mut st, "writing-and-failed");
    assert!(hits.iter().any(|(_, t)| matches!(t, Target::HandoverJob(_, Act::Retry))));

    ask(&mut st, 2);
    let (tx2, rx) = std::sync::mpsc::channel();
    make(&mut st, rx);
    tx2.send(Ok("## Goal\n\nReady.".into())).unwrap();
    while states(&st).get(2) != Some(&State::Finished) {
        st.take_background_handovers();
        std::thread::sleep(Duration::from_millis(5));
    }
    st.open_finished_handover(Instant::now());
    crate::dialogs::click(&mut st, Instant::now(), crate::dialogs::CANCEL, 0.0);
    draw(&mut st, "three-folded");
    assert_eq!(list_top(&st), bare + ROW_H + 8.0);
    st.handover_act(0, Act::Fold, Instant::now());
    let hits = draw(&mut st, "three-open");
    assert_eq!(list_top(&st), bare + 3.0 * ROW_H + FAILED_H + 8.0, "a failure has a line for its reason");
    assert!(hits.iter().any(|(_, t)| matches!(t, Target::HandoverJob(_, Act::Review))));
    for (r, t) in &hits {
        if matches!(t, Target::HandoverJob(..)) {
            assert!(r.x >= 0.0 && r.right() <= w, "{t:?} {r:?}");
        }
    }
    st.forced_dark = Some(true);
    st.retheme();
    draw(&mut st, "three-open-dark");
    drop(tx0);
    st.cancel_background_handovers();
    let _ = dialog::COMMAND;
}
