use super::*;
use crate::util::testing::{TempDir, config_with, put};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::{Duration, UNIX_EPOCH};

fn prompt(content: &str) -> String {
    format!("{}\n", serde_json::json!({"type":"user","cwd":"C:\\fixture","message":{"role":"user","content":content}}))
}

fn set_fixed_time(path: &Path) {
    let fixed = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    OpenOptions::new().write(true).open(path).unwrap().set_modified(fixed).unwrap();
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), fixed);
}

#[test]
fn second_scan_reads_no_unchanged_transcript_or_desktop_bodies() {
    let t = TempDir::new("scan-cache");
    let roots = t.roots();
    let config = config_with(&roots, &[]);
    let account = &config.accounts[0];
    let folder = account.cli_dir.join("projects").join("fixture");
    let a = folder.join("a.jsonl");
    let b = folder.join("b.jsonl");
    let desktop = account.desktop_dir.join("claude-code-sessions/acct/org/local_a.json");
    put(&a, &prompt("first prompt"));
    put(&b, &prompt("second prompt"));
    put(&folder.join("not-a-session.jsonl"), "not JSON\n");
    put(&desktop, r#"{"cliSessionId":"a","title":"Desktop title"}"#);
    let mut cache = ScanCache::default();

    let first = cache.scan(&roots, &config);
    assert_eq!(cache.body_reads, 4);
    assert_eq!(first.len(), 2);
    assert_eq!(first.iter().find(|s| s.id == "a").unwrap().title, "Desktop title");
    assert_eq!(cache.scan(&roots, &config), first);
    assert_eq!(cache.body_reads, 0);

    fs::remove_file(&desktop).unwrap();
    fs::remove_file(&b).unwrap();
    let remaining = cache.scan(&roots, &config);
    assert_eq!(cache.body_reads, 0);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].title, "first prompt");
    assert!(cache.transcript_stamp(&b).is_none() && cache.desktop.is_empty());
}

#[test]
fn scan_invalidates_on_identity_size_or_precise_modification_time() {
    let t = TempDir::new("scan-stamps");
    let roots = t.roots();
    let config = config_with(&roots, &[]);
    let a = config.accounts[0].cli_dir.join("projects/fixture/a.jsonl");
    put(&a, &prompt("first"));
    set_fixed_time(&a);
    let mut cache = ScanCache::default();
    cache.scan(&roots, &config);
    let initial = cache.transcript_stamp(&a).unwrap();

    let replaced = a.with_extension("old");
    fs::rename(&a, &replaced).unwrap();
    put(&a, &prompt("newer"));
    set_fixed_time(&a);
    assert_eq!(cache.scan(&roots, &config)[0].last_prompt, "newer");
    assert_eq!(cache.body_reads, 1);
    let replacement = cache.transcript_stamp(&a).unwrap();
    assert_ne!(replacement.identity, initial.identity);
    assert_eq!((replacement.size, replacement.modified), (initial.size, initial.modified));

    OpenOptions::new().append(true).open(&a).unwrap().write_all(prompt("appended").as_bytes()).unwrap();
    set_fixed_time(&a);
    assert_eq!(cache.scan(&roots, &config)[0].last_prompt, "appended");
    assert_eq!(cache.body_reads, 1);
    let appended = cache.transcript_stamp(&a).unwrap();
    assert_eq!((appended.identity, appended.modified), (replacement.identity, replacement.modified));
    assert!(appended.size > replacement.size);

    // Change only the modification time, within the same millisecond.
    OpenOptions::new()
        .write(true)
        .open(&a)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000) + Duration::from_micros(100))
        .unwrap();
    cache.scan(&roots, &config);
    assert_eq!(cache.body_reads, 1);
    let touched = cache.transcript_stamp(&a).unwrap();
    assert_ne!(touched, appended);
    assert_eq!(touched.modified_ms(), appended.modified_ms());
}

#[test]
fn append_during_a_bounded_tail_read_does_not_advance_the_poll_stamp() {
    let t = TempDir::new("scan-race");
    let path = t.0.join("race.jsonl");
    put(&path, &prompt("first"));
    set_fixed_time(&path);
    let (previous, _) = transcript_snapshot(&path).unwrap();
    let append = |text| {
        OpenOptions::new().append(true).open(&path).unwrap().write_all(prompt(text).as_bytes()).unwrap();
        set_fixed_time(&path);
    };
    append("middle");
    let raced = changed_row_activity_with(&path, previous, |f, size| {
        let lines = Lines::read_from(f, size, 0)?;
        assert_eq!(prompts::activity(&lines).0, "middle");
        append("final");
        Some(lines)
    });
    assert!(raced.is_none(), "must not publish stale activity with a fresh stamp");
    let (current, last_prompt, ended) = changed_row_activity(&path, previous).unwrap();
    assert_eq!(current.modified_ms(), previous.modified_ms());
    assert!(current.size() > previous.size());
    assert_eq!((last_prompt.as_str(), ended), ("final", false));
    assert!(changed_row_activity(&path, current).is_none());
}
