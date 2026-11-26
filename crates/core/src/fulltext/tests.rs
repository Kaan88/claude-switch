use super::*;
use std::cell::Cell;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[test]
fn index_search_and_stats_agree_on_the_clean_main_prompt() {
    let folder = Folder::new();
    let file = folder.0.join("session.jsonl");
    let text = concat!(
        "{\"type\":\"user\",\"cwd\":\"C:/fixture\",\"message\":{\"content\":[",
        "{\"type\":\"text\",\"text\":\"<system-reminder>hidden \"},",
        "{\"type\":\"text\",\"text\":\"private</system-reminder>needle alpha\\nbeta gamma<system-reminder>secret</system-reminder>\"}]}}\n",
        "{\"type\":\"user\",\"parent_tool_use_id\":\"nested\",\"message\":{\"content\":\"secret\"}}\n"
    );
    fs::write(&file, text).unwrap();
    let indexed = crate::sessions::read_transcript(&file).unwrap();
    assert_eq!(indexed.last_prompt, "needle alpha beta gamma");
    let (first, _) = crate::sessions::naming_prompts(&file);
    assert_eq!(first.as_deref(), Some("needle alpha beta gamma"));
    for (query, expected) in [("needle alpha", 1), ("alpha beta", 0), ("secret", 0)] {
        let mut count = 0;
        Query::new(query).unwrap().scan(text.as_bytes(), || false, |hits| count = hits.shown.len()).unwrap();
        assert_eq!(count, expected);
    }
    let stats = crate::stats::collect(text.as_bytes(), || false).unwrap();
    assert_eq!(stats.turns, 1);
    assert_eq!(stats.context.count, "needle alpha\nbeta gamma".chars().count() as u64 / 4);
}

#[test]
fn a_record_with_extreme_json_width_fails_instead_of_reporting_a_partial_scan() {
    let padding = "0,".repeat(65_536);
    let input = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"hello\",\"padding\":[{padding}0]}}}}\n");
    let error = records(input.as_bytes(), || false, |_, _| panic!("oversized structure reached parser")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(bounded_structure(br#"{"message": "commas, braces { and escaped quote \" in a string"}"#, 2));
}

pub(crate) struct Folder(pub PathBuf);

impl Folder {
    pub(crate) fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("cs-search-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn searches_mixed_case_in_any_order_on_one_decoded_line() {
    let q = Query::new("  BETA  áLpHa  ").unwrap();
    let src = concat!(
        r#"{"message":{"content":"ÁLPHA separate\nBETA line"},"type":"user"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"BETA before \u00c1lpha"}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"Álpha then beta"}}"#,
        "\n",
    );
    let mut hits = Vec::new();
    q.scan(src.as_bytes(), || false, |h| hits.push(h.clone())).unwrap();
    assert_eq!(hits.iter().map(|h| h.shown.len()).collect::<Vec<_>>(), [1, 2]);
    let hit = &hits[0].shown[0];
    assert_eq!(hit.line, "BETA before Álpha");
    assert_eq!(hit.matches.iter().map(|r| &hit.line[r.clone()]).collect::<Vec<_>>(), ["BETA", "Álpha"]);
    assert!(Query::new(" \t ").is_none());
}

#[test]
fn skips_non_conversation_blocks_and_bad_records_and_caps_hits() {
    let src = concat!(
        r#"{"type":"api-request-blob","message":{"type":"user","content":"needle"}}"#,
        "\n",
        r#"{"message":{"type":"assistant","content":"needle"},"type":"api-request-blob"}"#,
        "\n",
        r#"{"type":"user","message":{"content":"needle"},"type":"api-request-blob"}"#,
        "\n",
        r#"{"type":"user","isMeta":true,"message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"assistant","isSidechain":true,"message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"user","isCompactSummary":true,"message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"assistant","isApiErrorMessage":true,"message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"assistant","parent_tool_use_id":"agent","message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"<local-command-stdout>needle</local-command-stdout>"}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"image","source":{"data":"needle"}},{"type":"tool_result","content":[{"type":"text","text":"needle"}]}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","text":"needle"},{"type":"tool_use","input":{"value":"needle"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"needle"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":"Needle 2\nNeedle 3\nNeedle 4\nNeedle 5"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":"needle (unfinished)"}}"#,
    );
    let mut progress = Vec::new();
    Query::new("needle").unwrap().scan(src.as_bytes(), || false, |hits| progress.push(hits.clone())).unwrap();
    assert_eq!(progress.iter().map(|h| (h.shown.len(), h.more)).collect::<Vec<_>>(), [(1, 0), (2, 0), (3, 0), (3, 2)]);
    assert_eq!(
        progress[3].shown.iter().map(|h| h.line.as_str()).collect::<Vec<_>>(),
        ["needle", "Needle 2", "Needle 3"]
    );
}

#[test]
fn chunks_skip_an_oversized_record_and_cancellation_stops_inside_one() {
    let start = br#"{"type":"user","message":{"content":"needle "#;
    let end = b"\"}}\n{\"type\":\"user\",\"message\":{\"content\":\"nEeDle next\"}}\n";
    let input = || Cursor::new(start).chain(io::repeat(b'a').take(MAX_LINE as u64)).chain(Cursor::new(end));
    let q = Query::new("needle").unwrap();
    let mut found = Vec::new();
    q.scan(input(), || false, |h| found.push(h.clone())).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].shown[0].line, "nEeDle next");
    let checks = Cell::new(0);
    q.scan(
        input(),
        || {
            checks.set(checks.get() + 1);
            checks.get() > 8
        },
        |_| panic!("published after cancel"),
    )
    .unwrap();
    assert!(checks.get() <= 10);
}

#[test]
fn long_excerpt_keeps_the_first_match_and_valid_unicode_boundaries() {
    let line = format!("{} \tİSTANBUL then {}", "好".repeat(400), "a".repeat(600));
    let body = format!("{}\n", serde_json::json!({"type":"assistant","message":{"content":line}}));
    let mut hit = None;
    Query::new("i\u{307}stanbul").unwrap().scan(body.as_bytes(), || false, |h| hit = Some(h.shown[0].clone())).unwrap();
    let hit = hit.unwrap();
    assert!(hit.line.starts_with('…') && hit.line.ends_with('…'));
    assert_eq!(&hit.line[hit.matches[0].clone()], "İSTANBUL");
    assert!(!hit.line.contains('\t'));
}

#[test]
#[ignore = "writes and scans approximately 1 GiB of synthetic JSONL"]
fn search_one_gib() {
    let temp = Folder::new();
    let name = |i| temp.0.join(format!("transcript-{i}.jsonl"));
    let block = [
        format!("{{\"type\":\"api-request-blob\",\"payload\":\"{}\"}}\n", "x".repeat(16_384)),
        format!("{{\"type\":\"user\",\"message\":{{\"content\":\"Can you inspect the {}?\"}}}}\n", "layout ".repeat(175)),
        format!("{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"I checked the {}\"}}]}}}}\n", "measurements ".repeat(450)),
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"thinking\",\"thinking\":\"Private\"}]}}\n".into(),
    ].concat();
    let per_file = 16 * 1024 * 1024 / block.len();
    for i in 0..64 {
        let mut file = io::BufWriter::new(File::create(name(i)).unwrap());
        writeln!(file, "{}", serde_json::json!({"type":"user","message":{"content":"Unique benchmark needle"}}))
            .unwrap();
        for _ in 0..per_file {
            file.write_all(block.as_bytes()).unwrap();
        }
        file.flush().unwrap();
    }
    let bytes: u64 = (0..64).map(|i| fs::metadata(name(i)).unwrap().len()).sum();
    let q = Query::new("needle benchmark").unwrap();
    let start = Instant::now();
    let mut hits = 0;
    for i in 0..64 {
        q.scan(File::open(name(i)).unwrap(), || false, |h| hits += h.shown.len()).unwrap();
    }
    println!("{bytes} bytes across 64 synthetic transcripts; {hits} hits in {:.3?}", start.elapsed());
    assert_eq!(hits, 64);
}
