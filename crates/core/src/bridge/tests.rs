//! Made-up job files in temporary folders; never the bridge's own folder.
use super::log::{Entry, Kind, Tail, parse_line};
use super::*;
use crate::util::testing::{TempDir, put};
use serde_json::json;

fn state(id: &str, status: &str, extra: Value) -> String {
    let mut v = json!({
        "id": id, "owner": 4242, "status": status, "started": 1_000_000, "directory": "C:\\work\\demo",
        "lastTool": "read", "files": [], "reply": "", "ok": false, "log": "C:\\elsewhere\\x.jsonl",
        "worker": "flash", "model": "provider/model-a"
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    v.to_string()
}

#[test]
fn only_the_bridges_state_file_names_are_jobs() {
    assert_eq!(job_id("job_k3x_2.json"), Some("job_k3x_2"));
    assert_eq!(job_id("job_k3x_2-card-fixes.json"), Some("job_k3x_2-card-fixes"));
    for name in [
        "job_k3x_2.jsonl",
        "job_k3x_2.cancel",
        "job_K3x_2.json",
        "job_k3x_x.json",
        "job__2.json",
        "job_k3x_2-Card.json",
        "job_k3x_2.json.4242.tmp",
        "catalog-1a2b.json",
        "clients.jsonl",
    ] {
        assert_eq!(job_id(name), None, "{name}");
    }
}

#[test]
fn the_job_files_texts_are_one_bounded_line_without_control_characters() {
    let t = TempDir::new("bridge-plain");
    let long = format!("C:\\work\\{}", "d".repeat(5_000));
    let text = state(
        "job_a_1",
        "running",
        json!({"directory": "C:\\x\u{1b}[31m\nred", "label": "a\u{7}b\r\nc", "worker": long}),
    );
    let job = Job::parse("job_a_1", text.as_bytes(), &t.0, 1).unwrap();
    assert_eq!(job.directory, PathBuf::from("C:\\x [31m red"));
    assert_eq!(job.label.as_deref(), Some("a b c"));
    assert_eq!(job.worker.as_ref().unwrap().chars().count(), 200);
    let text = state("job_a_1", "running", json!({"directory": long}));
    let job = Job::parse("job_a_1", text.as_bytes(), &t.0, 1).unwrap();
    assert_eq!(job.directory.as_os_str().len(), 1024);
}

#[test]
fn a_process_tree_is_walked_only_through_held_parents_and_never_into_switch() {
    // 10 is the job's worker; 20 is Switch (say, run by the worker's `cargo run`), 21 under it.
    let all = [(10, 1), (11, 10), (12, 11), (20, 11), (21, 20), (30, 99)];
    assert_eq!(process::next_round(&all, &[10], 20), [(11, 10)]);
    assert_eq!(process::next_round(&all, &[10, 11], 20), [(12, 11)], "Switch is skipped");
    assert!(process::next_round(&all, &[10, 11, 12], 20).is_empty(), "and so is all under it");
    // A pid found in a snapshot is under its parent only if it existed then and is no older.
    assert!(process::accept(150, 200, 100));
    assert!(!process::accept(250, 200, 100), "given out again after the snapshot");
    assert!(!process::accept(50, 200, 100), "older than its parent: a stale parent pid");
    // Only the user's own processes, and no elevated one for an elevated Switch.
    assert!(process::may_end(true, false, false) && process::may_end(true, false, true));
    assert!(process::may_end(true, true, false));
    assert!(!process::may_end(true, true, true) && !process::may_end(false, false, false));
}

#[test]
fn a_state_file_reads_into_a_job_and_its_log_is_the_file_beside_it() {
    let t = TempDir::new("bridge-parse");
    let text = state(
        "job_a_1-fix",
        "running",
        json!({"label": "fix the scan", "running": 1_000_500, "childPid": 77, "lastEvent": 1_060_000,
               "inFlight": {"label": "bash cargo test", "since": 1}}),
    );
    let job = Job::parse("job_a_1-fix", text.as_bytes(), &t.0, 1_070_000).unwrap();
    assert_eq!(job.name(), "fix the scan");
    assert_eq!((job.worker.as_deref(), job.model.as_deref()), (Some("flash"), Some("provider/model-a")));
    assert_eq!(job.state, State::Running);
    assert_eq!((job.owner, job.child_pid), (Some(4242), Some(77)));
    assert_eq!(job.in_flight.as_deref(), Some("bash cargo test"));
    assert_eq!(job.log, t.0.join("job_a_1-fix.jsonl"), "never the path the file names");
    assert_eq!(job.elapsed_ms(1_100_500), 100_000, "from when it started running");
    assert!(Job::parse("job_a_2", text.as_bytes(), &t.0, 0).is_none(), "the id must be the file's");
    assert!(Job::parse("job_a_1-fix", b"{not json", &t.0, 0).is_none());
}

#[test]
fn a_job_whose_owner_is_gone_says_so_and_stops_its_clock_at_its_last_save() {
    let t = TempDir::new("bridge-owner");
    put(&t.0.join("job_a_1.json"), &state("job_a_1", "running", json!({"running": 1_000_000, "lastEvent": 1_030_000})));
    let mut scanner = Scanner::default();
    let jobs = scanner.scan(&t.0, 10, |_, _| false).unwrap();
    assert!(jobs[0].owner_gone && !jobs[0].live());
    assert_eq!(jobs[0].state_word(), "Owner gone");
    assert_eq!(jobs[0].elapsed_ms(u64::MAX), jobs[0].saved_ms.max(1_030_000) - 1_000_000);
    let jobs = scanner.scan(&t.0, 10, |pid, started| pid == 4242 && started == 1_000_000).unwrap();
    assert!(jobs[0].live() && jobs[0].state_word() == "Running");
}

#[test]
fn the_scan_lists_pending_jobs_first_then_the_newest_ended_ones_and_rereads_only_changes() {
    let t = TempDir::new("bridge-scan");
    put(&t.0.join("job_a_1.json"), &state("job_a_1", "done", json!({"finished": 2_000_000})));
    put(&t.0.join("job_a_2.json"), &state("job_a_2", "failed", json!({"finished": 3_000_000})));
    put(&t.0.join("job_a_3.json"), &state("job_a_3", "done", json!({"finished": 1_500_000})));
    put(&t.0.join("job_b_1.json"), &state("job_b_1", "queued", json!({"started": 500})));
    put(&t.0.join("job_b_2.json"), &state("job_b_2", "running", json!({"started": 900})));
    put(&t.0.join("job_b_2.jsonl"), "{}\n");
    put(&t.0.join("job_b_3.json"), "{ half a file");
    put(&t.0.join("notes.json"), &state("notes", "running", json!({})));
    let mut scanner = Scanner::default();
    let ids = |jobs: Vec<Job>| jobs.into_iter().map(|j| j.id).collect::<Vec<_>>();
    assert_eq!(ids(scanner.scan(&t.0, 2, |_, _| true).unwrap()), ["job_b_2", "job_b_1", "job_a_2", "job_a_1"]);
    // A job that ended moves down; a removed file goes.
    put(&t.0.join("job_b_2.json"), &state("job_b_2", "done", json!({"started": 900, "finished": 4_000_000})));
    std::fs::remove_file(t.0.join("job_a_2.json")).unwrap();
    assert_eq!(ids(scanner.scan(&t.0, 3, |_, _| true).unwrap()), ["job_b_1", "job_b_2", "job_a_1", "job_a_3"]);
    assert!(scanner.scan(&t.0.join("missing"), 3, |_, _| true).is_err());
}

#[test]
fn a_cancel_says_why_it_cannot_end_a_job_that_is_not_running() {
    let t = TempDir::new("bridge-cancel");
    put(&t.0.join("job_a_1.json"), &state("job_a_1", "done", json!({"childPid": 5})));
    put(&t.0.join("job_a_2.json"), &state("job_a_2", "queued", json!({})));
    put(&t.0.join("job_a_3.json"), &state("job_a_3", "running", json!({})));
    let alive = |_, _| true;
    assert_eq!(cancel(&t.0, "job_a_1", alive), Err(Refused::NotRunning("Done".into())));
    assert_eq!(cancel(&t.0, "job_a_2", alive), Err(Refused::Queued));
    assert_eq!(cancel(&t.0, "job_a_3", alive), Err(Refused::NoProcess));
    assert!(matches!(cancel(&t.0, "job_a_9", alive), Err(Refused::Unreadable(_))));
    for refusal in
        [Refused::Queued, Refused::NotTheJobs(9), Refused::Ended(9), Refused::Unknown(9), Refused::Protected(9)]
    {
        assert!(refusal.to_string().ends_with('.'), "{refusal}");
    }
}

#[test]
fn a_pid_is_the_jobs_only_as_a_child_of_its_owner_created_while_the_job_ran() {
    let t = TempDir::new("bridge-identity");
    let text = state("job_a_1", "running", json!({"started": 1_000_000, "lastEvent": 1_050_000}));
    let job = Job::parse("job_a_1", text.as_bytes(), &t.0, 1_060_000).unwrap();
    assert!(process::is_jobs(&job, 4242, 1_000_100));
    assert!(process::is_jobs(&job, 4242, 999_000), "a little clock skew");
    assert!(!process::is_jobs(&job, 4243, 1_000_100), "another parent");
    assert!(!process::is_jobs(&job, 4242, 990_000), "older than the job");
    assert!(!process::is_jobs(&job, 4242, 1_070_000), "newer than the state that names it");
}

#[cfg(windows)]
#[test]
fn a_cancel_refuses_a_pid_that_is_not_the_jobs_and_ends_one_that_is_with_its_children() {
    use std::time::{Duration, Instant};
    let t = TempDir::new("bridge-end");
    let now = unix_ms(SystemTime::now());
    // This test's own process, named as a job's worker by a stranger: never ended.
    let me = std::process::id();
    put(
        &t.0.join("job_a_1.json"),
        &state("job_a_1", "running", json!({"owner": 1, "childPid": me, "started": now - 1_000})),
    );
    assert_eq!(cancel(&t.0, "job_a_1", |_, _| true), Err(Refused::NotTheJobs(me)));
    // A worker this test started, with a child of its own (a ping under cmd).
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let pid = child.id();
    put(
        &t.0.join("job_a_2.json"),
        &state("job_a_2", "running", json!({"owner": me, "childPid": pid, "started": now - 1_000})),
    );
    // Too old a state to name it: a pid it names must be older than the state's last save.
    let stale = Job::parse(
        "job_a_2",
        state("job_a_2", "running", json!({"owner": me, "childPid": pid, "started": now - 60_000})).as_bytes(),
        &t.0,
        now - 30_000,
    )
    .unwrap();
    assert_eq!(process::end_job_tree(&stale), Err(Refused::NotTheJobs(pid)));
    assert!(child.try_wait().unwrap().is_none(), "a refused cancel ends nothing");
    let ended = cancel(&t.0, "job_a_2", |_, _| true).unwrap();
    assert_eq!(ended.pid, pid);
    assert!(ended.count >= 2, "the worker and the ping under it: {ended:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the worker ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(cancel(&t.0, "job_a_2", |_, _| true), Err(Refused::Ended(pid)));
    assert_eq!(process::lookup(pid), process::Lookup::Gone);
    assert!(process::owner_alive(me, now));
    assert!(!process::owner_alive(me, 0), "a process newer than the job is not its owner");
}

#[test]
fn opencode_and_claude_code_events_read_as_entries() {
    let lines = [
        json!({"type":"step_start","part":{"type":"step-start"}}),
        json!({"type":"text","part":{"type":"text","text":"I'll read the file."}}),
        json!({"type":"tool_use","part":{"tool":"read","state":{"status":"completed","title":"src/a.rs","input":{"filePath":"C:/x/src/a.rs"}}}}),
        json!({"type":"tool_use","part":{"tool":"bash","state":{"status":"error","input":{"command":"cargo  test"},"error":"exit 101"}}}),
        json!({"type":"error","error":{"name":"APIError","data":{"message":"rate limited"}}}),
        json!({"type":"system","subtype":"init","model":"claude-model-x"}),
        json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Done."},{"type":"tool_use","name":"Edit","input":{"file_path":"b.rs"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"no such file"}]}]}}),
        json!({"type":"result","is_error":false,"duration_ms":125_000}),
        json!({"type":"rate_limit_event"}),
    ];
    let entries: Vec<Entry> = lines.iter().flat_map(|l| parse_line(l.to_string().as_bytes())).collect();
    let shown: Vec<(Kind, &str, &str)> = entries.iter().map(|e| (e.kind, e.title.as_str(), e.text.as_str())).collect();
    assert_eq!(
        shown,
        [
            (Kind::Text, "", "I'll read the file."),
            (Kind::Tool, "read", "src/a.rs"),
            (Kind::Error, "bash", "cargo test — exit 101"),
            (Kind::Error, "Error", "rate limited"),
            (Kind::Note, "Started", "claude-model-x"),
            (Kind::Thought, "", "hm"),
            (Kind::Text, "", "Done."),
            (Kind::Tool, "Edit", "b.rs"),
            (Kind::Error, "Tool failed", "no such file"),
            (Kind::Result, "Finished", "after 2m 05s"),
        ]
    );
    assert!(parse_line(b"not json").is_empty());
}

fn text_line(text: &str) -> String {
    format!("{}\n", json!({"type":"text","part":{"text":text}}))
}

#[test]
fn the_tail_reads_appends_and_waits_for_a_line_to_end() {
    let t = TempDir::new("bridge-tail");
    let path = t.0.join("job_a_1.jsonl");
    put(&path, &text_line("one"));
    let mut tail = Tail::new(&path);
    assert!(tail.read().unwrap());
    assert!(!tail.read().unwrap(), "nothing new");
    let mut more = text_line("two");
    let half = more.split_off(10);
    let append = |s: &str| {
        use std::io::Write as _;
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(s.as_bytes()).unwrap();
    };
    append(&more);
    assert!(!tail.read().unwrap(), "half a line is not an entry yet");
    append(&half);
    assert!(tail.read().unwrap());
    let texts: Vec<_> = tail.entries.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["one", "two"]);
    assert!(!tail.cut);
    assert_eq!((tail.pushed, tail.resets), (2, 0));
    // A shorter file was replaced: read again from its start.
    put(&path, &text_line("new"));
    assert!(tail.read().unwrap());
    assert_eq!(tail.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["new"]);
    assert_eq!((tail.pushed, tail.resets), (1, 1));
}

#[test]
fn the_tail_starts_near_the_end_of_a_long_stream_reads_in_steps_and_skips_huge_lines() {
    let t = TempDir::new("bridge-long");
    let path = t.0.join("job_a_1.jsonl");
    let mut text = String::new();
    let mut n = 0;
    while text.len() < (log::START as usize) * 2 {
        text.push_str(&text_line(&format!("line {n}")));
        n += 1;
    }
    put(&path, &text);
    let mut tail = Tail::new(&path);
    tail.read().unwrap();
    assert!(tail.cut, "the start was not read");
    assert_eq!(tail.entries.back().unwrap().text, format!("line {}", n - 1));
    assert!(tail.entries.len() < n / 2 + 10, "only the last {} bytes", log::START);
    // A line longer than the limit is skipped whole; the line after it reads.
    let huge = format!("{{\"type\":\"text\",\"part\":{{\"text\":\"{}\"}}}}\n", "x".repeat(log::MAX_LINE + 10));
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    use std::io::Write as _;
    file.write_all(huge.as_bytes()).unwrap();
    file.write_all(text_line("after").as_bytes()).unwrap();
    drop(file);
    let mut reads = 0;
    while tail.behind() {
        tail.read().unwrap();
        reads += 1;
    }
    assert!(reads >= 4, "at most {} bytes a read", log::STEP);
    assert_eq!(tail.entries.back().unwrap().text, "after");
    assert!(tail.entries.iter().all(|e| e.text.len() < 10_000));
    assert!(tail.entries.len() <= log::KEEP);
}
