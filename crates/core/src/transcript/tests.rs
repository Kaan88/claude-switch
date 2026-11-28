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
    assert_eq!(items.len(), 6, "an ordinary system record is no notice");
    assert!(matches!(&items[0].kind, ItemKind::Prompt { text, .. } if text == "/x"));
    assert_eq!(items[1].kind, ItemKind::Notice { kind: NoticeKind::Thought, text: "hide".into() });
    assert_eq!(items[2].timestamp.as_deref(), Some("2026-11-26T01:00:00Z"));
    assert!(matches!(&items[2].kind, ItemKind::Assistant { markdown } if markdown == "**hi**\n  there"));
    assert!(matches!(&items[3].kind, ItemKind::Prompt { text, .. } if text == "works"));
    assert_eq!(items[4].kind, ItemKind::Notice { kind: NoticeKind::Compaction, text: "one\ntwo".into() });
    assert_eq!(items[5].kind, ItemKind::Notice { kind: NoticeKind::ApiError, text: "unavailable".into() });
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
    assert_eq!(reader.items().count(), 1);
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
        assert_eq!(reader.items().count(), 0);
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
    assert_eq!(reader.items().count(), 0);
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
    assert_eq!(info.end, None);
    assert_eq!(info.started_at.as_deref(), Some("2026-11-26T01:00:00Z"));
    assert_eq!(info.tool_calls, 0);
    let mut child = reader.open_subagent("abc", 4096).unwrap();
    assert_eq!(child.items().count(), 1);
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
    assert_eq!(info.end, Some((AgentEnd::Finished, EndSource::Sidecar)));
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

/// Each shape an agent's end comes in (Claude Code 2.1.332): the sidecar's marks (only ever
/// `true`), a background agent's `<task-notification>` (a user record, or a `queued_command`
/// attachment while a turn ran), a foreground call's result, and the agent's own transcript.
#[test]
fn agent_ends_are_read_from_each_record_that_tells() {
    let folder = Folder::new();
    let path = folder.session();
    let dir = path.with_extension("").join("subagents");
    fs::create_dir_all(&dir).unwrap();
    let agent = |id: &str, meta: Value, last: Value| {
        let mut lines = record(json!({"type":"user","isSidechain":true,"message":{"content":"task"}}));
        lines.push_str(&record(last));
        fs::write(dir.join(format!("agent-{id}.jsonl")), lines).unwrap();
        fs::write(dir.join(format!("agent-{id}.meta.json")), meta.to_string()).unwrap();
    };
    let call =
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{}}]}});
    let answer = |stop: Value| json!({"type":"assistant","message":{"content":[{"type":"text","text":"done"}],"stop_reason":stop}});
    let esc = json!({"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}});
    agent("running", json!({"agentType":"general-purpose","toolUseId":"bg1"}), call.clone());
    agent("notified", json!({"toolUseId":"bg2"}), call.clone());
    agent("queued", json!({"toolUseId":"bg3"}), call.clone());
    agent("foreground", json!({"toolUseId":"fg1"}), call.clone());
    agent("escaped", json!({"toolUseId":"fg2"}), call.clone());
    agent("stopped", json!({"finished":true,"stoppedByUser":true}), call.clone());
    agent("sidecar", json!({"finished":true}), call.clone());
    agent("answered", json!({}), answer(json!("end_turn")));
    agent("streaming", json!({}), answer(Value::Null));
    agent("interrupted", json!({}), esc);
    let background = |id| tool(id, "Agent", json!({"description":"d","prompt":"p","run_in_background":true}));
    let launched = |id: &str, agent: &str| {
        record(json!({"type":"user","toolUseResult":{"status":"async_launched","agentId":agent},
            "message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"Async agent launched"}]}}))
    };
    let mut session =
        [background("bg1"), launched("bg1", "running"), background("bg2"), launched("bg2", "notified")].concat();
    session.push_str(&tool("fg1", "Agent", json!({"description":"d","prompt":"p"})));
    session.push_str(&record(json!({"type":"user","toolUseResult":{"status":"completed"},
        "message":{"content":[{"type":"tool_result","tool_use_id":"fg1","content":"report"}]}})));
    session.push_str(&tool("fg2", "Agent", json!({"description":"d","prompt":"p"})));
    session.push_str(&record(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fg2",
        "is_error":true,"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}]}})));
    session.push_str(&record(json!({"type":"user","message":{"content":
        "<task-notification>\n<task-id>notified</task-id>\n<tool-use-id>bg2</tool-use-id>\n<status>killed</status>\n<summary>Agent \"d\" was stopped</summary>\n</task-notification>"}})));
    session.push_str(&record(json!({"type":"attachment","attachment":{"type":"queued_command","commandMode":"task-notification",
        "prompt":"<task-notification>\n<task-id>queued</task-id>\n<status>completed</status>\n<summary>Agent \"d\" completed</summary>\n</task-notification>"}})));
    fs::write(&path, session).unwrap();
    let reader = Reader::open(&path, 1024 * 1024).unwrap();
    let end = |id: &str| reader.subagents().find(|a| a.id == id).unwrap().end;
    assert_eq!(end("running"), None, "launched in the background, nothing ended it");
    assert_eq!(end("notified"), Some((AgentEnd::Stopped, EndSource::Notification)));
    assert_eq!(end("queued"), Some((AgentEnd::Finished, EndSource::Notification)));
    assert_eq!(end("foreground"), Some((AgentEnd::Finished, EndSource::Result)));
    assert_eq!(end("escaped"), Some((AgentEnd::Interrupted, EndSource::Result)));
    assert_eq!(end("stopped"), Some((AgentEnd::Stopped, EndSource::Sidecar)));
    assert_eq!(end("sidecar"), Some((AgentEnd::Finished, EndSource::Sidecar)));
    assert_eq!(end("answered"), Some((AgentEnd::Finished, EndSource::Transcript)));
    assert_eq!(end("streaming"), None, "an answer without its stop reason may be followed by a call");
    assert_eq!(end("interrupted"), Some((AgentEnd::Interrupted, EndSource::Transcript)));
}

/// A notification paged in with earlier history settles an agent the newest slice left unknown.
#[test]
fn an_agent_end_in_earlier_history_is_found_when_it_loads() {
    let folder = Folder::new();
    let path = folder.session();
    let dir = path.with_extension("").join("subagents");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("agent-old.jsonl"), record(json!({"type":"user","message":{"content":"task"}}))).unwrap();
    let mut session = record(json!({"type":"user","message":{"content":
        "<task-notification><task-id>old</task-id><status>completed</status></task-notification>"}}));
    for i in 0..40 {
        session.push_str(&prompt(&format!("later prompt {i} with some words to fill the slice")));
    }
    fs::write(&path, session).unwrap();
    let mut reader = Reader::open(&path, 512).unwrap();
    assert!(reader.has_earlier());
    assert_eq!(reader.subagents().next().unwrap().end, None);
    let mut changed = false;
    while reader.has_earlier() {
        changed |= reader.earlier().unwrap().subagents_changed;
    }
    assert!(changed);
    assert_eq!(reader.subagents().next().unwrap().end, Some((AgentEnd::Finished, EndSource::Notification)));
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
    assert_eq!(reader.items().count(), 1);
    assert!(reader.load_picture(&stale_picture).is_err());
    fs::rename(&path, folder.0.join("old.jsonl")).unwrap();
    assert_eq!(reader.poll().unwrap(), Changes::default());
    fs::write(&path, prompt("a larger replacement")).unwrap();
    let change = reader.poll().unwrap();
    assert!(change.reset);
    assert_eq!(change.added.len(), 1);
    assert_eq!(reader.items().count(), 1);
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
    assert_eq!(reader.items().count(), 1);
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
    let items = reader.items().count();
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

#[test]
fn native_tasks_follow_confirmed_creates_and_updates_and_the_latest_list_wins() {
    let folder = Folder::new();
    let path = folder.session();
    let result = |id: &str, text: &str, error: bool| {
        record(json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":id,"content":text,"is_error":error}]}}))
    };
    let todo = tool("t", "TodoWrite", json!({"todos":[{"content":"old plan","status":"pending"}]}));
    fs::write(&path, todo).unwrap();
    let mut reader = Reader::open(&path, 1024 * 1024).unwrap();
    assert_eq!(reader.tasks()[0].content, "old plan");

    append(&path, tool("c1", "TaskCreate", json!({"subject":"Write the parser","description":"d"})));
    assert!(!reader.poll().unwrap().tasks_changed, "a call alone is not a task");
    append(&path, result("c1", "Task #1 created successfully: Write the parser", false));
    assert!(reader.poll().unwrap().tasks_changed);
    assert_eq!(reader.tasks(), [Task { content: "Write the parser".into(), status: TaskStatus::Pending }]);

    append(&path, tool("c2", "TaskCreate", json!({"subject":"Refused"})));
    append(&path, result("c2", "Task #2 created successfully: Refused", true));
    append(&path, tool("u1", "TaskUpdate", json!({"taskId":"1","status":"in_progress","subject":"Write the reader"})));
    append(&path, result("u1", "Updated task #1 status", false));
    reader.poll().unwrap();
    assert_eq!(reader.tasks(), [Task { content: "Write the reader".into(), status: TaskStatus::InProgress }]);

    append(&path, tool("t2", "TodoWrite", json!({"todos":[{"content":"new plan","status":"completed"}]})));
    reader.poll().unwrap();
    assert_eq!(reader.tasks()[0].content, "new plan", "a later TodoWrite wins again");
    append(&path, tool("u2", "TaskUpdate", json!({"taskId":"1","status":"deleted"})));
    append(&path, result("u2", "Updated task #1", false));
    reader.poll().unwrap();
    assert!(reader.tasks().is_empty(), "the native list changed last, and is now empty");
}

#[test]
fn native_tasks_survive_history_paged_in_after_their_results() {
    let folder = Folder::new();
    let path = folder.session();
    let call = tool("c1", "TaskCreate", json!({"subject":"Early task"}));
    let filler = prompt(&"x".repeat(4000));
    let done = record(json!({"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"c1","content":"Task #7 created successfully: Early task"}]}}));
    fs::write(&path, call + &filler + &done).unwrap();
    let mut reader = Reader::open(&path, done.len() + 10).unwrap();
    assert!(reader.tasks().is_empty());
    while reader.has_earlier() {
        reader.earlier().unwrap();
    }
    assert_eq!(reader.tasks(), [Task { content: "Early task".into(), status: TaskStatus::Pending }]);
}

#[test]
fn shown_attachments_become_notices_and_context_attachments_are_skipped() {
    let folder = Folder::new();
    let path = folder.session();
    let lines = [
        record(json!({"type":"attachment","attachment":{"type":"prompt_snapshot","systemPrompt":["long"]}})),
        record(json!({"type":"attachment","attachment":{"type":"nested_memory","displayPath":"docs/CLAUDE.md"}})),
        record(json!({"type":"attachment","attachment":{"type":"hook_non_blocking_error","hookName":"PreToolUse:Bash",
            "hookEvent":"PreToolUse","stderr":"refused","exitCode":1}})),
        record(json!({"type":"system","subtype":"away_summary","content":"Shipped the fix.","isMeta":false})),
        record(json!({"type":"system","subtype":"turn_duration","durationMs":5})),
    ];
    fs::write(&path, lines.concat()).unwrap();
    let reader = Reader::open(&path, 1024 * 1024).unwrap();
    let kinds: Vec<_> = reader
        .items()
        .map(|(_, item)| match &item.kind {
            ItemKind::Notice { kind, .. } => *kind,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kinds, [NoticeKind::Memory, NoticeKind::HookError, NoticeKind::Recap]);
}

#[test]
fn a_message_sent_while_a_turn_ran_is_a_prompt_and_other_queued_records_are_not() {
    let folder = Folder::new();
    let path = folder.session();
    let queued = |mode: &str, origin: Value, prompt: Value| {
        record(json!({"type":"attachment","uuid":"u","timestamp":"2026-11-28T01:42:27Z","attachment":{
            "type":"queued_command","prompt":prompt,"commandMode":mode,"origin":origin,"humanTurn":true}}))
    };
    let lines = [
        prompt("first"),
        queued("prompt", json!({"kind":"human"}), json!("does this make sense?")),
        queued(
            "task-notification",
            json!({"kind":"task-notification"}),
            json!("<task-notification>x</task-notification>"),
        ),
        queued("prompt", json!({"kind":"peer"}), json!("from another session")),
        queued("prompt", json!({"kind":"human"}), json!([{"type":"text","text":"[Image #1] look"}, image("aGk=")])),
    ];
    fs::write(&path, lines.concat()).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    let items: Vec<_> = reader.items().map(|(_, item)| item.clone()).collect();
    let texts: Vec<_> = items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Prompt { text, synthetic: false, .. } => text.as_str(),
            other => panic!("only prompts expected, got {other:?}"),
        })
        .collect();
    assert_eq!(texts, ["first", "does this make sense?", "[Image #1] look"]);
    let queued: Vec<_> = items.iter().map(|item| matches!(item.kind, ItemKind::Prompt { queued: true, .. })).collect();
    assert_eq!(queued, [false, true, true]);
    // It starts no turn: the newest prompt is still the one typed at the idle input.
    let first = reader.items().next().map(|(id, _)| id.offset);
    assert_eq!(reader.chat().prompt, first);
    assert_eq!(items[1].timestamp.as_deref(), Some("2026-11-28T01:42:27Z"));
    let ItemKind::Prompt { pictures, .. } = &items[2].kind else { unreachable!() };
    assert_eq!(pictures.len(), 1);
    assert_eq!(reader.load_picture(&pictures[0]).unwrap(), b"hi");
}

#[test]
fn thinking_shown_as_updates_is_an_answer_and_other_thinking_stays_a_thought() {
    let folder = Folder::new();
    let path = folder.session();
    let thinking = |display: Option<&str>, text: &str| {
        let mut value = json!({"type":"assistant","message":{"role":"assistant","stop_reason":"tool_use",
            "content":[{"type":"thinking","thinking":text,"signature":"s"}]}});
        if let Some(display) = display {
            value["thinkingDisplay"] = json!(display);
        }
        record(value)
    };
    let lines = [
        thinking(Some("updates"), ""),
        thinking(Some("updates"), "Encrypting the database would not protect it."),
        thinking(None, "a private thought"),
    ];
    fs::write(&path, lines.concat()).unwrap();
    let reader = Reader::open(&path, 4096).unwrap();
    let kinds: Vec<_> = reader.items().map(|(_, item)| item.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            ItemKind::Assistant { markdown: "Encrypting the database would not protect it.".into() },
            ItemKind::Notice { kind: NoticeKind::Thought, text: "a private thought".into() },
        ]
    );
}

#[test]
fn claude_codes_record_of_an_esc_is_a_notice_and_no_prompt_of_the_users() {
    let folder = Folder::new();
    let path = folder.session();
    let user_text =
        |text: &str| record(json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}}));
    let lines = [
        prompt("run the ping"),
        tool("toolu_1", "Bash", json!({"command":"ping"})),
        record(json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1",
            "content":"The user doesn't want to proceed with this tool use.","is_error":true}]}})),
        user_text("[Request interrupted by user for tool use]"),
        user_text("[Request interrupted by user]"),
    ];
    fs::write(&path, lines.concat()).unwrap();
    let reader = Reader::open(&path, 4096).unwrap();
    let items: Vec<_> = reader.items().map(|(id, item)| (id, item.kind.clone())).collect();
    let prompts: Vec<_> =
        items.iter().filter(|(_, k)| matches!(k, ItemKind::Prompt { .. })).map(|(id, _)| id.offset).collect();
    assert_eq!(prompts, [0], "only the user's own prompt");
    let notices: Vec<_> = items
        .iter()
        .filter_map(|(_, k)| match k {
            ItemKind::Notice { kind: NoticeKind::Interrupted, text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(notices, ["Request interrupted by user for tool use", "Request interrupted by user"]);
    // No new turn: the newest prompt is still the one typed.
    assert_eq!(reader.chat().prompt, Some(0));
}

#[test]
fn a_long_paste_is_shown_as_its_text_without_claude_codes_tags() {
    let folder = Folder::new();
    let path = folder.session();
    let wrapped = "<pasted_content id=\"3589\">\nFinally reply with golf.\n</pasted_content id=\"3589\">";
    let lines = [
        prompt(&format!("look: {wrapped} please")),
        record(json!({"type":"attachment","attachment":{"type":"queued_command","prompt":wrapped,
            "commandMode":"prompt","origin":{"kind":"human"}}})),
        prompt("<command-name>/clear</command-name>"),
    ];
    fs::write(&path, lines.concat()).unwrap();
    let reader = Reader::open(&path, 4096).unwrap();
    let texts: Vec<_> = reader
        .items()
        .filter_map(|(_, item)| match &item.kind {
            ItemKind::Prompt { text, queued, .. } => Some((text.clone(), *queued)),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        [
            ("look: Finally reply with golf. please".to_owned(), false),
            ("Finally reply with golf.".to_owned(), true),
            ("/clear".to_owned(), false),
        ]
    );
    // A tag without its end stays as it is.
    assert_eq!(super::unwrap_pastes("a <pasted_content id=\"1\">b"), "a <pasted_content id=\"1\">b");
}

#[test]
fn the_session_a_fork_goes_on_in_is_read_from_the_record_it_writes_last() {
    // Claude Code 2.1.332's `/fork` ends the transcript it leaves with this record.
    const NEXT: &str = "7e6b2a9e-5649-42e7-8db5-01617d00d7cd";
    let folder = Folder::new();
    let path = folder.session();
    let message = |uuid: &str| record(json!({"type":"user","uuid":uuid,"message":{"content":"hi"}}));
    let continued = |id: &str| record(json!({"type":"continued-in","sessionId":"x","continuedInSessionId":id}));
    fs::write(&path, message("u1") + &record(json!({"type":"cost-state"}))).unwrap();
    let mut reader = Reader::open(&path, 4096).unwrap();
    assert_eq!(reader.chat().continued_in, None);
    append(&path, continued("not an id"));
    reader.poll().unwrap();
    assert_eq!(reader.chat().continued_in, None, "an invalid id is ignored");
    append(&path, continued(NEXT));
    reader.poll().unwrap();
    assert_eq!(reader.chat().continued_in.as_deref(), Some(NEXT));
    assert_eq!(reader.items().count(), 1, "the record is no item");
    // The session resumed here later: the conversation goes on in this transcript again.
    append(&path, message("u2"));
    reader.poll().unwrap();
    assert_eq!(reader.chat().continued_in, None);
    // A subagent's transcript names no continuation.
    let agents = folder.0.join("session").join("subagents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("agent-a.jsonl"), continued(NEXT)).unwrap();
    assert_eq!(Reader::open(agents.join("agent-a.jsonl"), 4096).unwrap().chat().continued_in, None);
}

/// Claude Code 2.1.332's `/fork`, recorded by `recorder compat` (script `fork`, trimmed: context
/// attachments are stubs that keep their links, account fields are left out): the terminal's own
/// transcript, and the background session the fork made from it.
const FORK_TAB: &str = include_str!("fixtures/fork-2.1.332/b251c8e6-184e-48ea-a1db-c43a9618612d.jsonl");
const FORK_COPY: &str = include_str!("fixtures/fork-2.1.332/d356e845-f0ad-412a-a14a-bd7e652ebc66.jsonl");

fn shown(reader: &Reader) -> Vec<String> {
    reader
        .items()
        .map(|(_, item)| match &item.kind {
            ItemKind::Prompt { text, .. } => format!("> {text}"),
            ItemKind::Assistant { markdown } => markdown.clone(),
            ItemKind::Tool { name, .. } => name.clone(),
            ItemKind::Notice { text, .. } => format!("! {text}"),
        })
        .collect()
}

#[test]
fn after_fork_the_tabs_transcript_goes_on_and_the_copy_shows_only_the_copied_turn() {
    let folder = Folder::new();
    let path = folder.session();
    // As the file grew: up to `/fork`, then its records, then the next turn.
    let fork = FORK_TAB.find("<command-name>/fork").unwrap();
    let fork = FORK_TAB[..fork].rfind('\n').unwrap() + 1;
    let bravo = FORK_TAB.find("Reply with just the word: bravo").unwrap();
    let bravo = FORK_TAB[..bravo].rfind('\n').unwrap() + 1;
    fs::write(&path, &FORK_TAB[..fork]).unwrap();
    let mut reader = Reader::open(&path, 256 * 1024).unwrap();
    assert_eq!(shown(&reader), ["> Reply with just the word: alpha", "alpha"]);
    append(&path, &FORK_TAB[fork..bravo]);
    let changes = reader.poll().unwrap();
    assert!(!changes.reset && !changes.branch_changed);
    // The command's row; Claude Code's note to the model about the fork is no prompt and no turn.
    assert_eq!(shown(&reader), ["> Reply with just the word: alpha", "alpha", "/fork"]);
    let chat = reader.chat();
    assert!(matches!(chat.latest, Some((_, ChatPhase::EndTurn))), "{chat:?}");
    assert_eq!(chat.continued_in, None, "2.1.332 leaves the terminal in its session");
    append(&path, &FORK_TAB[bravo..]);
    reader.poll().unwrap();
    let after = ["> Reply with just the word: alpha", "alpha", "/fork", "> Reply with just the word: bravo", "bravo"];
    assert_eq!(shown(&reader), after);
    assert!(matches!(reader.chat().latest, Some((_, ChatPhase::EndTurn))));

    // The background copy: its task notification about the fork is not a prompt of the user's.
    let copy = folder.0.join("copy.jsonl");
    fs::write(&copy, FORK_COPY).unwrap();
    let reader = Reader::open(&copy, 256 * 1024).unwrap();
    assert_eq!(shown(&reader), ["> Reply with just the word: alpha", "alpha"]);
    assert_eq!(reader.chat().continued_in, None);
}
