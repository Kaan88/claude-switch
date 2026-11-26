#[test]
fn truncate_and_regrow_on_the_same_handle_invalidates_the_cursor() {
    let dir = crate::util::testing::TempDir::new("tail-checkpoint");
    let path = dir.0.join("log.jsonl");
    for rewritten in [b"new\n".as_slice(), b"new\nfollowup\n".as_slice()] {
        std::fs::write(&path, b"old\n").unwrap();
        let mut tail = Tail::open(&path, 4096).unwrap();
        tail.initial(|_, _| {}).unwrap();
        std::fs::write(&path, rewritten).unwrap();
        assert!(tail.replaced().unwrap());
    }
}

#[test]
fn same_size_modified_transcript_is_invalidated_even_with_an_unchanged_checkpoint() {
    let dir = crate::util::testing::TempDir::new("tail-timestamp");
    let path = dir.0.join("log.jsonl");
    fs::write(&path, b"same\n").unwrap();
    let f = OpenOptions::new().write(true).open(&path).unwrap();
    let timestamp = SystemTime::now() - std::time::Duration::from_secs(90);
    f.set_times(fs::FileTimes::new().set_modified(timestamp)).unwrap();
    let mut tail = Tail::open(&path, 4096).unwrap();
    tail.initial(|_, _| {}).unwrap();
    assert!(!tail.replaced().unwrap());
    f.set_times(fs::FileTimes::new().set_modified(timestamp + std::time::Duration::from_secs(30))).unwrap();
    assert!(tail.replaced().unwrap());
}

#[test]
fn oversized_sidecar_is_ignored_before_parsing() {
    let dir = crate::util::testing::TempDir::new("tail-sidecar");
    let path = dir.0.join("agent.meta.json");
    let mut old = br#"{"model":"not used"}"#.to_vec();
    old.resize(MAX_SIDECAR as usize + 1, b' ');
    fs::write(&path, old).unwrap();
    assert_eq!(read_sidecar(&path).unwrap(), Value::Null);
}
#[test]
fn oversized_partial_line_is_discarded_until_newline_then_offsets_resume() {
    let mut line = b"part".to_vec();
    let mut oversized = false;
    let mut first = std::io::Cursor::new(b"1234567890");
    assert_eq!(bounded_line(&mut first, &mut line, &mut oversized, 8).unwrap(), (10, false));
    assert!(oversized && line.is_empty());
    let mut next = std::io::Cursor::new(b"abc\nok\n");
    assert_eq!(bounded_line(&mut next, &mut line, &mut oversized, 8).unwrap(), (4, true));
    assert!(line.is_empty());
    assert_eq!(bounded_line(&mut next, &mut line, &mut false, 8).unwrap(), (3, true));
    assert_eq!(line, b"ok\n");
}
use super::*;
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{Instant, SystemTime};

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("cs-transcript-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn session(&self) -> PathBuf {
        self.0.join("session.jsonl")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(value: Value) -> String {
    format!("{value}\n")
}

fn append(path: &Path, bytes: impl AsRef<[u8]>) {
    OpenOptions::new().append(true).open(path).unwrap().write_all(bytes.as_ref()).unwrap();
}

fn prompt(text: &str) -> String {
    record(json!({"type":"user","message":{"content":text}}))
}

fn tool(id: &str, name: &str, input: Value) -> String {
    record(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}))
}

fn image(data: &str) -> Value {
    json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":data}})
}

#[test]
fn filters_hidden_invalid_and_repeated_content_and_keeps_order() {
    let folder = Folder::new();
    let path = folder.session();
    let lines = [
        r#"{"message":{"content":"ignore"},"type":"api-request-blob"}"#,
        r#"{"type":"attachment","message":{"content":"ignore"}}"#,
        r#"{"type":"user","isMeta":true,"message":{"content":"hide"}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"content":"hide"}}"#,
        r#"{"type":"user","message":{"content":"  <system-reminder>hide</system-reminder>"}}"#,
        r#"{"type":"user","message":{"content":"<command-name>/x</command-name><command-args>ignore</command-args>"}}"#,
        r#"{"type":"user","message":{"content":"bad"#,
        r#"{"type":"user","message":{}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hide"},{"type":"text","text":"**hi**\n  there"},{"type":"text"},{"type":"tool_use","name":"Bash"}]},"timestamp":"2026-11-26T01:00:00Z"}"#,
        r#"{"t\u0079pe":"u\u0073er","message":{"content":"works"}}"#,
        r#"{"type":"user","message":{"content":"hide"},"type":"api-request-blob"}"#,
        r#"{"type":"user","isCompactSummary":true,"message":{"content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}}"#,
        r#"{"type":"system","isApiErrorMessage":true,"error":{"message":"unavailable"}}"#,
        r#"{"type":"system","content":"ordinary system record"}"#,
    ];
    fs::write(&path, lines.join("\r\n") + "\r\n").unwrap();
    let reader = Reader::open(&path, 4096).unwrap();
    let items: Vec<_> = reader.items().map(|(_, item)| item).collect();
    assert_eq!(items.len(), 5);
    assert!(matches!(&items[0].kind, ItemKind::Prompt { text, .. } if text == "/x"));
    assert_eq!(items[1].timestamp.as_deref(), Some("2026-11-26T01:00:00Z"));
    assert!(matches!(&items[1].kind, ItemKind::Assistant { markdown } if markdown == "**hi**\n  there"));
    assert!(matches!(&items[2].kind, ItemKind::Prompt { text, .. } if text == "works"));
    assert_eq!(items[3].kind, ItemKind::Notice { kind: NoticeKind::Compaction, text: "one\ntwo".into() });
    assert_eq!(items[4].kind, ItemKind::Notice { kind: NoticeKind::ApiError, text: "unavailable".into() });
    assert!(!reader.has_earlier());
}

#[test]
fn split_result_updates_only_its_call_and_result_picture_uses_real_offsets() {
    let folder = Folder::new();
    let path = folder.session();
    let calls = tool("good", "Bash", json!({"command":"echo first\n echo second"}))
        + &tool("bad", "Read", json!({"file_path":"example.rs"}));
    fs::write(&path, &calls).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    let ids: Vec<_> = reader.items().map(|(id, _)| id).collect();
    assert!(matches!(&reader.item(ids[0]).unwrap().kind, ItemKind::Tool { summary, state: ToolState::Running, .. }
        if summary == "echo first echo second"));
    let result = record(json!({"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"good","content":[{"type":"tool_reference"},{"type":"text","text":"ok"},image("AAH+/w==")]},
        {"type":"tool_result","tool_use_id":"bad","is_error":true,"content":"error"}
    ]}}));
    let split = result.len() / 2;
    append(&path, &result[..split]);
    assert_eq!(reader.poll().unwrap(), Changes::default());
    append(&path, &result[split..result.len() - 1]);
    assert_eq!(reader.poll().unwrap(), Changes::default());
    append(&path, "\n");
    let change = reader.poll().unwrap();
    assert_eq!(change.updated, BTreeSet::from([ids[0], ids[1]]));
    assert!(change.added.is_empty());
    let ItemKind::Tool { pictures, state: ToolState::Ok, .. } = &reader.item(ids[0]).unwrap().kind else { panic!() };
    let pic = pictures[0].clone();
    assert_eq!(
        (pic.line_offset, pic.line_len, pic.media_type.as_str(), pic.base64_len),
        (calls.len() as u64, result.len(), "image/png", 8)
    );
    assert_eq!(reader.load_picture(&pic).unwrap(), [0, 1, 254, 255]);
    assert!(matches!(&reader.item(ids[1]).unwrap().kind, ItemKind::Tool { state: ToolState::Failed, .. }));
    assert_eq!(reader.poll().unwrap(), Changes::default());
}

#[test]
fn prompt_pictures_are_lazy_and_bad_data_is_reported_only_when_loaded() {
    let folder = Folder::new();
    let path = folder.session();
    let prefix = prompt("first");
    let line = record(json!({"type":"user","message":{"content":[
        {"type":"text","text":"look"},
        {"type":"text","text":"<hidden>"},
        image("aGk="),
        image("!!!!")
    ]}}));
    fs::write(&path, prefix.clone() + &line).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    let items: Vec<_> = reader.items().map(|(_, item)| item).collect();
    let ItemKind::Prompt { text, pictures, .. } = &items[1].kind else { panic!() };
    assert_eq!(text, "look");
    let pictures = pictures.clone();
    assert_eq!(pictures[0].line_offset, prefix.len() as u64);
    assert_eq!(pictures[0].line_len, line.len());
    assert_eq!(reader.load_picture(&pictures[0]).unwrap(), b"hi");
    assert_eq!(reader.load_picture(&pictures[1]).unwrap_err().kind(), io::ErrorKind::InvalidData);
    let mut oversized = pictures[0].clone();
    oversized.base64_len = MAX_PICTURE.div_ceil(3) * 4 + 4;
    assert!(reader.load_picture(&oversized).unwrap_err().to_string().contains("32 MiB"));
    oversized.base64_len = pictures[0].base64_len;
    oversized.line_len = MAX_LINE + 1;
    assert!(reader.load_picture(&oversized).unwrap_err().to_string().contains("containing line"));
}

#[test]
fn base64_validates_padding_bits_alphabet_and_size_before_decoding() {
    for (encoded, decoded) in [
        (b"".as_slice(), b"".as_slice()),
        (b"Zg==", b"f"),
        (b"Zm8=", b"fo"),
        (b"Zm9v", b"foo"),
        (b"Zm9vYg", b"foob"),
        (b"/+/9", &[255, 239, 253]),
    ] {
        assert_eq!(decode_base64(encoded, decoded.len()).unwrap(), decoded);
    }
    for bad in [b"Z".as_slice(), b"Zg=", b"Zm9v=", b"Z===", b"Zh==", b"Zm9=", b"Z g=", b"__8=", b"Z=g="] {
        assert!(decode_base64(bad, MAX_PICTURE).is_err(), "{bad:?}");
    }
    // This has the same encoded length as an image at the limit but decodes one byte larger.
    let big = vec![b'A'; MAX_PICTURE.div_ceil(3) * 4];
    assert!(decode_base64(&big, MAX_PICTURE).unwrap_err().to_string().contains("32 MiB"));
}

#[test]
fn bounded_tail_and_history_keep_ids_and_apply_results_that_were_read_first() {
    let folder = Folder::new();
    let path = folder.session();
    let first = prompt(&"long".repeat(50_000));
    let call = tool("old", "Grep", json!({"pattern":"old\nvalue"}));
    let result = record(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"old"}]}}));
    let last = prompt("recent");
    fs::write(&path, first.clone() + &call + &result + &last).unwrap();
    let mut reader = Reader::open(&path, result.len() + last.len() + 5).unwrap();
    assert!(reader.has_earlier());
    assert_eq!(reader.items().len(), 1);
    let recent = reader.items().next().unwrap().0;
    let change = reader.earlier().unwrap();
    assert_eq!(change.added, BTreeSet::from([ItemId { offset: first.len() as u64, block: 0 }]));
    assert!(matches!(&reader.items().next().unwrap().1.kind, ItemKind::Tool { state: ToolState::Ok, summary, .. }
        if summary == "old value"));
    assert!(reader.has_earlier());
    assert_eq!(reader.earlier().unwrap().added.len(), 1);
    assert!(!reader.has_earlier());
    assert_eq!(reader.items().next_back().unwrap().0, recent);
    assert_eq!(reader.earlier().unwrap(), Changes::default());
}

#[test]
fn clipped_partial_line_survives_poll_or_interleaved_history_load() {
    for load_history_first in [false, true] {
        let folder = Folder::new();
        let path = folder.session();
        let text = "long".repeat(4000);
        let line = prompt(&text);
        fs::write(&path, prompt("previous") + &line[..line.len() - 4]).unwrap();
        let mut reader = Reader::open(&path, 64).unwrap();
        assert_eq!(reader.items().len(), 0);
        assert!(reader.has_earlier());
        if load_history_first {
            assert_eq!(reader.earlier().unwrap().added.len(), 1);
        }
        append(&path, &line[line.len() - 4..line.len() - 1]);
        assert_eq!(reader.poll().unwrap(), Changes::default());
        append(&path, "\n".to_owned() + &prompt("new"));
        assert_eq!(reader.poll().unwrap().added.len(), if load_history_first { 2 } else { 1 });
        if !load_history_first {
            assert_eq!(reader.earlier().unwrap().added.len(), 1);
            assert_eq!(reader.earlier().unwrap().added.len(), 1);
        }
        assert!(!reader.has_earlier());
        assert!(matches!(&reader.items().nth(1).unwrap().1.kind, ItemKind::Prompt { text: got, .. } if *got == text));
    }
}

#[test]
fn initial_partial_first_line_is_kept_until_its_newline() {
    let folder = Folder::new();
    let path = folder.session();
    let line = prompt("a  b");
    fs::write(&path, &line[..line.len() - 1]).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    assert_eq!(reader.items().len(), 0);
    assert!(!reader.has_earlier());
    assert_eq!(reader.poll().unwrap(), Changes::default());
    let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
    writer.write_all(b"\n").unwrap();
    assert_eq!(reader.poll().unwrap().added, BTreeSet::from([ItemId { offset: 0, block: 0 }]));
    drop(writer);
    assert_eq!(reader.poll().unwrap(), Changes::default());
    assert!(matches!(&reader.items().next().unwrap().1.kind, ItemKind::Prompt { text, .. } if text == "a  b"));
}

#[test]
fn summaries_use_tool_specific_fields_and_preserve_inline_spaces() {
    for (name, input, expected) in [
        (
            "PowerShell",
            json!({"command":"Write-Output 'two  spaces'\r\nGet-Date"}),
            "Write-Output 'two  spaces' Get-Date",
        ),
        ("Write", json!({"file_path":"a  b.txt","content":"hidden"}), "a  b.txt"),
        ("Edit", json!({"path":"a.txt","new_string":"hidden"}), "a.txt"),
        ("Glob", json!({"pattern":"src/**/*.rs"}), "src/**/*.rs"),
        ("Task", json!({"description":"Describe work","prompt":"hidden"}), "Describe work"),
        ("WebSearch", json!({"query":"search phrase"}), "search phrase"),
        ("WebFetch", json!({"url":"https://example.com"}), "https://example.com"),
        ("Skill", json!({"skill":"sample"}), "sample"),
        ("mcp__server_name__tool__suffix", json!({"private":"hidden"}), "server_name / tool__suffix"),
    ] {
        assert_eq!(tool_summary(name, &input), expected);
    }
}

#[test]
fn latest_task_snapshot_wins_and_actions_replay_in_file_order() {
    let folder = Folder::new();
    let path = folder.session();
    let plan = "mcp__ccd_turn__update_plan";
    let old = tool("old", "TodoWrite", json!({"todos":[{"content":"old","status":"completed"}]}));
    let snapshot = tool(
        "p",
        plan,
        json!({"items":[{"content":"first","status":"pending"},
        {"content":"second","status":"pending"},{"content":"invalid","status":"unknown"}]}),
    );
    fs::write(&path, old + &snapshot).unwrap();
    let mut reader = Reader::open(&path, snapshot.len() + 10).unwrap();
    assert_eq!(reader.tasks().len(), 2);
    assert!(reader.has_earlier());
    assert!(!reader.earlier().unwrap().tasks_changed);
    append(&path, tool("a", plan, json!({"start":true})));
    assert!(reader.poll().unwrap().tasks_changed);
    assert_eq!(reader.tasks()[0].status, TaskStatus::InProgress);
    append(&path, tool("b", plan, json!({"action":"advance"})));
    assert!(reader.poll().unwrap().tasks_changed);
    assert_eq!(reader.tasks()[0].status, TaskStatus::Completed);
    assert_eq!(reader.tasks()[1].status, TaskStatus::InProgress);
    append(&path, tool("c", plan, json!({"done":true})));
    assert!(reader.poll().unwrap().tasks_changed);
    assert!(reader.tasks().iter().all(|task| task.status == TaskStatus::Completed));
    append(&path, tool("d", "TodoWrite", json!({"todos":[]})));
    assert!(reader.poll().unwrap().tasks_changed);
    assert!(reader.tasks().is_empty());
}

#[test]
fn subagent_without_meta_follows_activity_and_links_when_meta_appears() {
    let folder = Folder::new();
    let path = folder.session();
    let dir = path.with_extension("").join("subagents");
    fs::create_dir_all(&dir).unwrap();
    let agent_path = dir.join("agent-abc.jsonl");
    fs::write(&path, tool("spawn", "Agent", json!({"description":"Find\nthis","prompt":"private input"}))).unwrap();
    fs::write(
        &agent_path,
        record(json!({"type":"user","isSidechain":true,"timestamp":"2026-11-26T01:00:00Z",
        "message":{"content":"agent prompt"}})),
    )
    .unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    let spawn = reader.items().next().unwrap().0;
    assert!(
        matches!(&reader.item(spawn).unwrap().kind, ItemKind::Tool { summary, subagent: None, .. } if summary == "Find this")
    );
    let info = reader.subagents().next().unwrap();
    assert_eq!(info.id, "abc");
    assert_eq!(info.finished, None);
    assert_eq!(info.started_at.as_deref(), Some("2026-11-26T01:00:00Z"));
    assert_eq!(info.tool_calls, 0);
    let mut child = reader.open_subagent("abc", 4096).unwrap();
    assert_eq!(child.items().len(), 1);
    append(
        &agent_path,
        record(json!({"type":"assistant","isSidechain":true,"timestamp":"2026-11-26T01:02:00Z",
        "message":{"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"file.rs"}},
        {"type":"tool_use","id":"mcp","name":"mcp__server__tool","input":{}}]}})),
    );
    fs::write(
        dir.join("agent-abc.meta.json"),
        record(json!({"agentType":"explore","description":"Find this",
        "model":"model-x","finished":true,"toolUseId":"spawn","spawnDepth":1})),
    )
    .unwrap();
    let change = reader.poll().unwrap();
    assert!(change.subagents_changed);
    assert_eq!(change.updated, BTreeSet::from([spawn]));
    assert!(matches!(&reader.item(spawn).unwrap().kind, ItemKind::Tool { subagent: Some(id), .. } if id == "abc"));
    let info = reader.subagents().next().unwrap();
    assert_eq!(info.finished, Some(true));
    assert_eq!(info.spawn_depth, Some(1));
    assert_eq!(info.model.as_deref(), Some("model-x"));
    assert_eq!(info.agent_type.as_deref(), Some("explore"));
    assert_eq!(info.description.as_deref(), Some("Find this"));
    assert_eq!(info.last_activity_at.as_deref(), Some("2026-11-26T01:02:00Z"));
    assert_eq!(info.tool_calls, 2);
    assert_eq!(info.last_tool_summary.as_deref(), Some("server / tool"));
    assert_eq!(reader.poll().unwrap(), Changes::default());
    assert_eq!(child.poll().unwrap().added.len(), 2);
    drop(child);
    fs::remove_file(agent_path).unwrap();
    let change = reader.poll().unwrap();
    assert!(change.subagents_changed);
    assert_eq!(change.updated, BTreeSet::from([spawn]));
    assert_eq!(reader.subagents().len(), 0);
}

#[test]
fn shrink_and_larger_replacement_reset_history() {
    let folder = Folder::new();
    let path = folder.session();
    let line = record(json!({"type":"user","message":{"content":[image("aGk=")]}}));
    fs::write(&path, line + &prompt("old")).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    let ItemKind::Prompt { pictures, .. } = &reader.items().next().unwrap().1.kind else { panic!() };
    let stale_picture = pictures[0].clone();
    fs::write(&path, prompt("new")).unwrap();
    let change = reader.poll().unwrap();
    assert!(change.reset);
    assert_eq!(change.added.len(), 1);
    assert_eq!(reader.items().len(), 1);
    assert!(reader.load_picture(&stale_picture).is_err());
    fs::rename(&path, folder.0.join("old.jsonl")).unwrap();
    assert_eq!(reader.poll().unwrap(), Changes::default());
    fs::write(&path, prompt("a larger replacement")).unwrap();
    let change = reader.poll().unwrap();
    assert!(change.reset);
    assert_eq!(change.added.len(), 1);
    assert_eq!(reader.items().len(), 1);
    assert!(
        matches!(&reader.items().next().unwrap().1.kind, ItemKind::Prompt { text, .. } if text == "a larger replacement")
    );
}

#[test]
fn ignores_large_blobs_with_type_before_or_after_the_payload() {
    let blob = "A".repeat(3 * 1024 * 1024);
    let first = format!(r#"{{"type":"api-request-blob","message":{{"content":"{blob}"}}}}"#);
    let last = format!(r#"{{"message":{{"type":"user","content":"{blob}"}},"type":"api-request-blob"}}"#);
    assert!(line_kind(first.as_bytes()).is_none());
    assert!(line_kind(last.as_bytes()).is_none());
    let folder = Folder::new();
    let path = folder.session();
    fs::write(&path, first + "\n" + &last + "\n" + &prompt("visible")).unwrap();
    let reader = Reader::open(&path, 8 * 1024 * 1024).unwrap();
    assert_eq!(reader.items().len(), 1);
}

#[test]
#[ignore = "manual transcript timing on generated data"]
fn timing_generated_20_mb() {
    let folder = Folder::new();
    let path = folder.session();
    let mut file = File::create(&path).unwrap();
    let user = prompt(&"a".repeat(1024));
    let assistant = record(json!({"type":"assistant","message":{"content":"**markdown**"}}));
    let picture = record(json!({"type":"user","message":{"content":[image(&"A".repeat(128 * 1024))]}}));
    let ignored = format!("{{\"type\":\"api-request-blob\",\"payload\":\"{}\"}}\n", "A".repeat(256 * 1024));
    let mut written = 0;
    let mut i = 0;
    while written < 20_000_000 {
        let line = match i % 128 {
            0 => &ignored,
            1 => &picture,
            n if n % 2 == 0 => &user,
            _ => &assistant,
        };
        file.write_all(line.as_bytes()).unwrap();
        written += line.len();
        i += 1;
    }
    drop(file);
    let start = Instant::now();
    let mut reader = Reader::open(&path, 2 * 1024 * 1024).unwrap();
    let open = start.elapsed();
    assert!(reader.has_earlier());
    let items = reader.items().len();
    assert!(items > 0);
    let start = Instant::now();
    let added = reader.earlier().unwrap().added.len();
    let earlier = start.elapsed();
    assert!(added > 0);
    append(&path, prompt("after timing"));
    let start = Instant::now();
    assert_eq!(reader.poll().unwrap().added.len(), 1);
    eprintln!(
        "{written} bytes: open {open:?} ({items} items), earlier {earlier:?} ({added} items), poll {:?}",
        start.elapsed()
    );
}
