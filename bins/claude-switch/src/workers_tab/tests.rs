//! Made-up jobs in temporary folders; never the bridge's own folder.
use std::time::{Duration, Instant};

use super::*;
use crate::tabs::{SESSIONS, Tabs, WORKERS};
use crate::term::Term;

fn tab(tabs: &mut Tabs, label: &str) -> u64 {
    let new = crate::tabs::NewTab { label: label.into(), color: [1, 2, 3], session: None, account: None };
    tabs.open(new, Term::empty((10, 3)))
}

#[test]
fn the_workers_tab_sits_beside_sessions_while_the_bridges_folder_exists() {
    let mut tabs = Tabs::default();
    assert!(!tabs.strip_shown() && !tabs.activate(WORKERS), "no folder: no tab");
    tabs.set_workers_tab(true);
    assert!(tabs.strip_shown(), "the strip shows for it alone");
    assert_eq!((tabs.numbered(1), tabs.numbered(2), tabs.numbered(3)), (Some(SESSIONS), Some(WORKERS), None));
    let a = tab(&mut tabs, "a");
    assert_eq!(tabs.numbered(3), Some(a));
    assert!(tabs.activate(WORKERS) && tabs.workers_shown() && tabs.active.is_none());
    tabs.step(true);
    assert_eq!(tabs.active, Some(a));
    tabs.step(true);
    assert_eq!(tabs.shown_id(), SESSIONS);
    tabs.step(false);
    assert_eq!(tabs.active, Some(a));
    tabs.step(false);
    assert!(tabs.workers_shown());
    // Closing the first terminal tab from Workers' neighbour goes back to Workers.
    tabs.activate(a);
    tabs.close(a);
    assert!(tabs.workers_shown());
    // The folder went: Sessions shows.
    tabs.set_workers_tab(false);
    assert_eq!(tabs.shown_id(), SESSIONS);
    assert!(!tabs.strip_shown());
}

fn settle(w: &mut WorkersTab) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        w.take();
        let reading = w.scan.is_some() || w.output.as_ref().is_some_and(|o| o.reading.is_some());
        if !reading && w.scanned {
            return;
        }
        assert!(Instant::now() < deadline, "the reads finished");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_jobs_list_pending_first_and_the_selected_one_streams_its_output() {
    let scratch = crate::testing::Scratch::created("cs-workers-tab");
    demo_jobs(&scratch, now_ms()).unwrap();
    let mut w = WorkersTab::new(scratch.to_path_buf());
    // The made-up owners: this process is "there", DEMO_GONE is not (as the demo's scan says).
    w.demo = Some(DemoFolder(crate::testing::unique("cs-workers-none")));
    assert!(w.available());
    w.poll(|| {});
    settle(&mut w);
    let states: Vec<_> = w.jobs.iter().map(|j| (j.id.as_str(), j.state_word())).collect();
    assert_eq!(
        states,
        [
            ("job_k3x_5-lint", "Queued"),
            ("job_k3x_4-tile-cache", "Running"),
            ("job_k3x_3-readme", "Owner gone"),
            ("job_k3x_2-search-fuzz", "Done"),
            ("job_k3x_1-crlf", "Failed"),
        ]
    );
    assert_eq!(w.selected.as_deref(), Some("job_k3x_5-lint"), "the first job is selected");
    assert!(w.step(true));
    assert_eq!(w.selected.as_deref(), Some("job_k3x_4-tile-cache"));
    w.poll(|| {});
    settle(&mut w);
    let output = w.output.as_ref().unwrap();
    assert_eq!(output.id, "job_k3x_4-tile-cache");
    let kinds: Vec<_> = output.entries.iter().map(|e| e.kind).collect();
    assert_eq!(kinds.first(), Some(&log::Kind::Text));
    assert!(kinds.contains(&log::Kind::Error) && kinds.iter().filter(|k| **k == log::Kind::Tool).count() >= 4);
    // A line appended later comes on the next read, after the ones already there.
    let before = output.entries.len();
    let line = "{\"type\":\"text\",\"part\":{\"text\":\"All tests pass.\"}}\n";
    let path = scratch.join("job_k3x_4-tile-cache.jsonl");
    let text = std::fs::read_to_string(&path).unwrap() + line;
    std::fs::write(&path, text).unwrap();
    w.poll(|| {});
    settle(&mut w);
    let output = w.output.as_ref().unwrap();
    assert_eq!(output.entries.len(), before + 1);
    assert_eq!(output.entries.back().unwrap().text, "All tests pass.");
    // A job with no log yet says so.
    w.select("job_k3x_5-lint");
    w.poll(|| {});
    settle(&mut w);
    assert_eq!(w.output.as_ref().unwrap().error.as_deref(), Some("No output yet."));
}

#[test]
fn cancel_asks_first_refuses_a_queued_job_plainly_and_the_demo_ends_nothing() {
    let mut st = AppState::new(crate::source::Source::Demo, Some(false), Default::default());
    st.start_demo_workers();
    assert!(st.tabs.workers_shown());
    st.workers.poll(|| {});
    settle(&mut st.workers);
    st.workers.select("job_k3x_5-lint");
    st.workers_activate(Target::WorkerCancel);
    assert!(st.dialog.is_none());
    let notice = st.workers.notice.clone().unwrap();
    assert_eq!(notice.2, "Not cancelled");
    assert!(notice.3.contains("still queued"), "{}", notice.3);
    st.workers.select("job_k3x_4-tile-cache");
    st.workers_activate(Target::WorkerCancel);
    let Some(Dialog::Confirm { then: Pending::CancelWorker(id), text, .. }) = &st.dialog else { panic!("a confirm") };
    assert_eq!(id, "job_k3x_4-tile-cache");
    assert!(text.contains("process 40095"), "{text}");
    assert!(!Pending::CancelWorker(id.clone()).ok_is_primary(), "Enter takes Cancel, not End");
    st.dialog = None;
    st.cancel_worker("job_k3x_4-tile-cache");
    assert_eq!(st.workers.notice.as_ref().unwrap().3, "The demo ends nothing.");
}

/// The Workers tab with the demo's made-up jobs, a running one selected: into `CS_CHAT_SHOTS`.
#[test]
#[ignore = "writes pictures of the Workers tab; run by hand"]
fn workers_tab_shots() {
    use crate::view::chat::readme_shots::{self as shots, Size};
    use std::path::{Path, PathBuf};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_CHAT_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-workers-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    for (dark, theme) in [(true, "dark"), (false, "light")] {
        let mut st = shots::window(dark);
        st.start_demo_workers();
        st.workers.poll(|| {});
        settle(&mut st.workers);
        for (id, name) in [("job_k3x_4-tile-cache", "running"), ("job_k3x_3-readme", "owner-gone")] {
            st.workers.select(id);
            st.workers.poll(|| {});
            settle(&mut st.workers);
            if name == "owner-gone" {
                st.workers_activate(Target::WorkerCancel);
                st.dialog = None;
                st.cancel_worker(id);
            }
            for (size, wide) in [
                (Size { w: 1280.0, h: 860.0, scale: 1.5 }, "wide"),
                (Size { w: 760.0, h: 480.0, scale: 1.0 }, "narrow"),
            ] {
                st.workers.details_scroll = 0.0;
                let bmp = shots::settled_at(&mut st, Instant::now(), size);
                bmp.write_png(&dir.join(format!("10-workers-{name}-{wide}-{theme}.png"))).unwrap();
            }
            // The narrow window's details scrolled to their end: the output in full.
            let area = st.workers.details_area;
            assert!(st.workers.wheel(area.x + 10.0, area.y + 10.0, -10_000.0), "the short details scroll");
            let bmp = shots::settled_at(&mut st, Instant::now(), Size { w: 760.0, h: 480.0, scale: 1.0 });
            bmp.write_png(&dir.join(format!("10-workers-{name}-narrow-scrolled-{theme}.png"))).unwrap();
        }
        assert!(st.hits.iter().any(|(_, t)| matches!(t, Target::Worker(_))));
    }
}
