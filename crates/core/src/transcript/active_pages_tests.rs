//! The Details pages (tasks, agents, shells and MCP tasks) over the active branch: a rewind, a
//! fork's copy, compaction and the bounded slice. Files and outputs come from the shown items.

use super::*;
use serde_json::json;
use std::io::Write;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("cs-pages-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn line(value: Value) -> String {
    format!("{value}\n")
}

fn link(mut value: Value, uuid: &str, parent: Option<&str>) -> String {
    value["uuid"] = json!(uuid);
    value["parentUuid"] = json!(parent);
    line(value)
}

fn prompt(uuid: &str, parent: Option<&str>, text: &str) -> String {
    link(json!({"type":"user","message":{"role":"user","content":text}}), uuid, parent)
}

fn answer(uuid: &str, parent: &str, text: &str) -> String {
    link(
        json!({"type":"assistant","message":{"content":[{"type":"text","text":text}],"stop_reason":"end_turn"}}),
        uuid,
        Some(parent),
    )
}

fn tool(uuid: &str, parent: &str, id: &str, name: &str, input: Value) -> String {
    link(
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}],
            "stop_reason":"tool_use"}}),
        uuid,
        Some(parent),
    )
}

fn result(uuid: &str, parent: &str, id: &str, text: &str) -> String {
    link(
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":text}]}}),
        uuid,
        Some(parent),
    )
}

fn plan(uuid: &str, parent: &str, id: &str, content: &str) -> String {
    tool(uuid, parent, id, "TodoWrite", json!({"todos":[{"content":content,"status":"pending","activeForm":content}]}))
        + &result(&format!("{uuid}-r"), uuid, id, "ok")
}

/// A background shell started by call `id` (task `task`): the call and its result.
fn shell(uuid: &str, parent: &str, id: &str, task: &str) -> String {
    tool(uuid, parent, id, "Bash", json!({"command":"watch","run_in_background":true}))
        + &result(
            &format!("{uuid}-r"),
            uuid,
            id,
            &format!(
                "Command running in background with ID: {task}. Output is being written to: C:\\tmp\\{task}.output"
            ),
        )
}

fn ended(uuid: &str, parent: &str, task: &str) -> String {
    link(
        json!({"type":"user","isMeta":true,"message":{"content":format!(
            "<task-notification><task-id>{task}</task-id><status>completed</status><summary>Command ended (exit code 0)</summary></task-notification>")}}),
        uuid,
        Some(parent),
    )
}

fn append(path: &Path, text: &str) {
    fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

fn tasks(reader: &Reader) -> Vec<String> {
    reader.tasks().iter().map(|t| t.content.clone()).collect()
}

fn shells(reader: &Reader) -> Vec<String> {
    reader.background_shells().into_iter().map(|s| s.id).collect()
}

fn agents(reader: &Reader) -> Vec<String> {
    let mut ids: Vec<String> = reader.subagents().map(|a| a.id.clone()).collect();
    ids.sort();
    ids
}

/// The first turn makes a plan; the second replaces it, starts two background shells (one ends)
/// and two agents (one finished, one still running).
fn session(folder: &Folder) -> PathBuf {
    let path = folder.0.join("s.jsonl");
    let text = prompt("u1", None, "one")
        + &plan("p1", "u1", "todo1", "first plan")
        + &answer("a1", "p1-r", "first")
        + &prompt("u2", Some("a1"), "two")
        + &plan("p2", "u2", "todo2", "replaced plan")
        + &shell("s1", "p2-r", "bash1", "done1")
        + &shell("s2", "s1-r", "bash2", "run2")
        + &tool("g1", "s2-r", "spawn1", "Agent", json!({"description":"gone","prompt":"x"}))
        + &result("g1-r", "g1", "spawn1", "done")
        + &tool("g2", "g1-r", "spawn2", "Agent", json!({"description":"live","prompt":"x","run_in_background":true}))
        + &result("g2-r", "g2", "spawn2", "started")
        + &ended("e1", "g2-r", "done1")
        + &answer("a2", "e1", "second");
    fs::write(&path, text).unwrap();
    let agents = folder.0.join("s").join("subagents");
    fs::create_dir_all(&agents).unwrap();
    for (id, tool, finished) in [("gone", "spawn1", true), ("live", "spawn2", false)] {
        let record = json!({"type":"user","isSidechain":true,"message":{"content":"task"}});
        fs::write(agents.join(format!("agent-{id}.jsonl")), line(record)).unwrap();
        let meta = json!({"agentType":"general-purpose","toolUseId":tool,"finished":finished});
        fs::write(agents.join(format!("agent-{id}.meta.json")), meta.to_string()).unwrap();
    }
    path
}

#[test]
fn after_a_rewind_the_pages_drop_what_the_replaced_turn_did_unless_it_still_runs() {
    let folder = Folder::new("rewind");
    let path = session(&folder);
    let mut reader = Reader::open(&path, 1 << 20).unwrap();
    // A rewind alone writes nothing: everything is there.
    assert_eq!(tasks(&reader), ["replaced plan"]);
    assert_eq!(shells(&reader), ["run2", "done1"]);
    assert_eq!(agents(&reader), ["gone", "live"]);

    append(&path, &prompt("u3", Some("a1"), "two again"));
    let changes = reader.poll().unwrap();
    assert!(changes.branch_changed && changes.tasks_changed && changes.subagents_changed && changes.background_changed);
    assert_eq!(tasks(&reader), ["first plan"], "the plan the active branch made");
    // A rewind stops no process: the running shell and agent stay, the ended ones go.
    assert_eq!(shells(&reader), ["run2"]);
    assert_eq!(agents(&reader), ["live"]);

    // A new plan on the branch is the plan.
    append(&path, &plan("p3", "u3", "todo3", "new plan"));
    reader.poll().unwrap();
    assert_eq!(tasks(&reader), ["new plan"]);
    // Read from the start, the same.
    let again = Reader::open(&path, 1 << 20).unwrap();
    assert_eq!((tasks(&again), shells(&again), agents(&again)), (tasks(&reader), shells(&reader), agents(&reader)));
}

#[test]
fn an_agent_of_a_replaced_turn_goes_once_its_end_is_known() {
    let folder = Folder::new("rewind-agent-end");
    let path = session(&folder);
    let mut reader = Reader::open(&path, 1 << 20).unwrap();
    append(&path, &prompt("u3", Some("a1"), "two again"));
    reader.poll().unwrap();
    // Nothing says the background agent ended (Claude Code never writes `finished: false`): it may
    // still run, so it stays.
    assert_eq!(agents(&reader), ["live"]);
    assert_eq!(reader.subagents().next().unwrap().end, None);
    // Its notification comes on the new branch: ended, and from a replaced turn, so it goes.
    append(&path, &ended("n3", "u3", "live"));
    let changes = reader.poll().unwrap();
    assert!(changes.subagents_changed);
    assert!(agents(&reader).is_empty());
    let again = Reader::open(&path, 1 << 20).unwrap();
    assert!(agents(&again).is_empty(), "the same read from the start");
}

#[test]
fn a_forks_copy_keeps_everything_it_copied() {
    // `/fork` and `/branch` copy the active chain, each record linked to the one before it.
    let folder = Folder::new("fork");
    let path = session(&folder);
    let original = fs::read_to_string(&path).unwrap();
    let mut copied = String::new();
    let mut parent: Option<String> = None;
    for text in original.lines() {
        let mut value: Value = serde_json::from_str(text).unwrap();
        let uuid = value["uuid"].as_str().unwrap().to_owned();
        value["parentUuid"] = json!(parent);
        value["forkedFrom"] = json!({"sessionId":"old","messageUuid":uuid});
        copied.push_str(&line(value));
        parent = Some(uuid);
    }
    fs::write(&path, copied).unwrap();
    let reader = Reader::open(&path, 1 << 20).unwrap();
    assert_eq!(tasks(&reader), ["replaced plan"]);
    assert_eq!(shells(&reader), ["run2", "done1"]);
    assert_eq!(agents(&reader), ["gone", "live"]);
}

#[test]
fn a_compaction_keeps_the_plan_made_before_it() {
    let folder = Folder::new("compact");
    let path = folder.0.join("s.jsonl");
    let boundary = line(json!({"type":"system","subtype":"compact_boundary","uuid":"b1","parentUuid":null,
        "logicalParentUuid":"a1","content":"Conversation compacted"}));
    let summary =
        link(json!({"type":"user","isCompactSummary":true,"message":{"content":"Summary"}}), "c1", Some("b1"));
    let text = prompt("u1", None, "one")
        + &plan("p1", "u1", "todo1", "plan before")
        + &answer("a1", "p1-r", "first")
        + &boundary
        + &summary
        + &prompt("u2", Some("c1"), "two")
        + &plan("p2", "u2", "todo2", "plan after")
        + &answer("a2", "p2-r", "second");
    fs::write(&path, &text).unwrap();
    assert_eq!(tasks(&Reader::open(&path, 1 << 20).unwrap()), ["plan after"]);
    // A rewind to the first prompt after the compaction: the plan from before it is the plan again.
    append(&path, &prompt("u3", Some("c1"), "two again"));
    assert_eq!(tasks(&Reader::open(&path, 1 << 20).unwrap()), ["plan before"]);
}

#[test]
fn the_bounded_slice_drops_a_replaced_turns_plan_once_history_proves_it() {
    let folder = Folder::new("slice");
    let path = folder.0.join("s.jsonl");
    let head = prompt("u1", None, &"long ".repeat(20_000)) + &answer("a1", "u1", "first");
    let replaced = prompt("u2", Some("a1"), "two");
    let tail = plan("p2", "u2", "todo2", "replaced plan") + &prompt("u3", Some("a1"), "two again");
    fs::write(&path, head + &replaced + &tail).unwrap();
    let mut reader = Reader::open(&path, tail.len() + 5).unwrap();
    // The replaced prompt is not loaded: the plan cannot be placed and stays.
    assert_eq!(tasks(&reader), ["replaced plan"]);
    let changes = reader.earlier().unwrap();
    assert!(changes.branch_changed && changes.tasks_changed);
    assert_eq!(tasks(&reader), Vec::<String>::new());
}

#[test]
fn a_replaced_turns_writes_are_kept_apart_for_the_files_page() {
    let folder = Folder::new("writes");
    let path = folder.0.join("s.jsonl");
    let write = |uuid: &str, parent: &str, id: &str, file: &str| {
        tool(uuid, parent, id, "Write", json!({"file_path":file,"content":"x"}))
            + &result(&format!("{uuid}-r"), uuid, id, "ok")
    };
    let failed = tool(
        "f2",
        "w2-r",
        "fail2",
        "Edit",
        json!({"file_path":"failed.md","old_string":"a","new_string":"b"}),
    ) + &link(
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fail2","is_error":true,"content":"no"}]}}),
        "f2-r",
        Some("f2"),
    );
    let text = prompt("u1", None, "one")
        + &write("w1", "u1", "write1", "kept.md")
        + &answer("a1", "w1-r", "first")
        + &prompt("u2", Some("a1"), "two")
        + &write("w2", "u2", "write2", "replaced.md")
        + &failed
        + &answer("a2", "f2-r", "second");
    fs::write(&path, text).unwrap();
    let mut reader = Reader::open(&path, 1 << 20).unwrap();
    assert!(reader.replaced_writes().is_empty(), "nothing is replaced before the next prompt");
    append(&path, &prompt("u3", Some("a1"), "two again"));
    reader.poll().unwrap();
    let replaced: Vec<_> = reader
        .replaced_writes()
        .into_iter()
        .map(|(id, item)| {
            assert!(reader.item(id).is_none(), "not a shown item");
            match item.kind {
                ItemKind::Tool { file_path, .. } => file_path.unwrap(),
                _ => unreachable!(),
            }
        })
        .collect();
    assert_eq!(replaced, ["replaced.md"], "a failed edit changed nothing");
}
