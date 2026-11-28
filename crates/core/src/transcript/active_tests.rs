//! The active branch over made-up transcripts: a rewind, a fork's copy, sidechains and subagents,
//! compaction, and the bounded slice.

use super::*;
use serde_json::json;
use std::io::Write;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("cs-active-{name}-{}-{nonce}", std::process::id()));
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

fn prompt(uuid: &str, parent: Option<&str>, text: &str) -> String {
    line(
        json!({"type":"user","uuid":uuid,"parentUuid":parent,"isSidechain":false,"message":{"role":"user","content":text}}),
    )
}

fn answer(uuid: &str, parent: &str, text: &str) -> String {
    line(json!({"type":"assistant","uuid":uuid,"parentUuid":parent,"isSidechain":false,
        "message":{"content":[{"type":"text","text":text}],"stop_reason":"end_turn"}}))
}

fn call(uuid: &str, parent: &str, id: &str) -> String {
    line(json!({"type":"assistant","uuid":uuid,"parentUuid":parent,
        "message":{"content":[{"type":"tool_use","id":id,"name":"Bash","input":{"command":"echo"}}],"stop_reason":"tool_use"}}))
}

fn result(uuid: &str, parent: &str, id: &str) -> String {
    line(json!({"type":"user","uuid":uuid,"parentUuid":parent,
        "message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"ok"}]}}))
}

fn texts(reader: &Reader) -> Vec<String> {
    reader
        .items()
        .map(|(_, item)| match &item.kind {
            ItemKind::Prompt { text, .. } => format!("> {text}"),
            ItemKind::Assistant { markdown } => markdown.clone(),
            ItemKind::Tool { id, state, .. } => format!("{id} {state:?}"),
            ItemKind::Notice { text, .. } => format!("! {text}"),
        })
        .collect()
}

fn append(path: &Path, text: &str) {
    fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}

/// Two turns, the second rewound: Claude Code appends nothing until the next prompt.
fn two_turns() -> String {
    prompt("u1", None, "one")
        + &answer("a1", "u1", "first")
        + &prompt("u2", Some("a1"), "two")
        + &answer("a2", "u2", "second")
}

#[test]
fn the_turns_a_rewind_replaced_go_once_the_next_prompt_is_written() {
    let folder = Folder::new("rewind");
    let path = folder.0.join("s.jsonl");
    fs::write(&path, two_turns()).unwrap();
    let mut reader = Reader::open(&path, 1 << 20).unwrap();
    assert_eq!(texts(&reader), ["> one", "first", "> two", "second"], "a rewind alone writes nothing");
    append(&path, &prompt("u3", Some("a1"), "two again"));
    let changes = reader.poll().unwrap();
    assert!(changes.branch_changed, "the shown items are rebuilt");
    assert_eq!(texts(&reader), ["> one", "first", "> two again"]);
    assert!(changes.added.iter().all(|id| reader.item(*id).is_some()), "nothing hidden is reported as added");
    let gone = reader.items().all(|(_, item)| !matches!(&item.kind, ItemKind::Prompt { text, .. } if text == "two"));
    assert!(gone);
    append(&path, &answer("a3", "u3", "second again"));
    let changes = reader.poll().unwrap();
    assert!(!changes.branch_changed, "an answer on the branch changes nothing hidden");
    assert_eq!(texts(&reader), ["> one", "first", "> two again", "second again"]);
    // Read from the start, the same.
    assert_eq!(texts(&Reader::open(&path, 1 << 20).unwrap()), ["> one", "first", "> two again", "second again"]);
}

#[test]
fn a_rewind_to_the_first_prompt_hides_the_whole_earlier_conversation() {
    let folder = Folder::new("first");
    let path = folder.0.join("s.jsonl");
    fs::write(&path, two_turns() + &prompt("u3", None, "start over") + &answer("a3", "u3", "fresh")).unwrap();
    assert_eq!(texts(&Reader::open(&path, 1 << 20).unwrap()), ["> start over", "fresh"]);
}

#[test]
fn tool_calls_results_and_records_hanging_off_the_branch_stay() {
    let folder = Folder::new("dangling");
    let path = folder.0.join("s.jsonl");
    let text = prompt("u1", None, "run both")
        + &call("c1", "u1", "t1")
        + &call("c2", "c1", "t2")
        // Results recorded against each call's own message rather than in one line.
        + &result("r1", "c1", "t1")
        + &result("r2", "c2", "t2")
        // A progress record and a hook notice off the chain.
        + &line(json!({"type":"progress","uuid":"p1","parentUuid":"r2","data":{}}))
        + &line(json!({"type":"attachment","uuid":"h1","parentUuid":"r2","attachment":{"type":"hook_system_message","hookName":"Stop","content":"checked"}}))
        + &answer("a1", "r2", "both ran");
    fs::write(&path, &text).unwrap();
    let reader = Reader::open(&path, 1 << 20).unwrap();
    assert_eq!(texts(&reader), ["> run both", "t1 Ok", "t2 Ok", "! Stop: checked", "both ran"]);
}

#[test]
fn a_prompt_off_the_chain_written_after_the_chains_own_record_is_not_taken_for_a_rewind() {
    let folder = Folder::new("late");
    let path = folder.0.join("s.jsonl");
    let text = prompt("u1", None, "one")
        + &answer("a1", "u1", "first")
        + &prompt("u2", Some("a1"), "two")
        + &prompt("odd", Some("a1"), "recorded later")
        + &answer("a2", "u2", "second");
    fs::write(&path, &text).unwrap();
    assert_eq!(
        texts(&Reader::open(&path, 1 << 20).unwrap()),
        ["> one", "first", "> two", "> recorded later", "second"]
    );
}

#[test]
fn a_forks_copy_shows_the_conversation_it_copied_and_goes_on() {
    // `/fork` and `/branch` (Claude Code 2.1.332) write the active chain under the new id, each record
    // linked to the one before it and naming where it came from.
    let folder = Folder::new("fork");
    let path = folder.0.join("s.jsonl");
    let copied = |uuid: &str, parent: Option<&str>, text: &str, user: bool| {
        let mut value = if user {
            json!({"type":"user","message":{"role":"user","content":text}})
        } else {
            json!({"type":"assistant","message":{"content":[{"type":"text","text":text}],"stop_reason":"end_turn"}})
        };
        value["uuid"] = json!(uuid);
        value["parentUuid"] = json!(parent);
        value["sessionId"] = json!("new");
        value["forkedFrom"] = json!({"sessionId":"old","messageUuid":uuid});
        line(value)
    };
    let text = line(json!({"type":"custom-title","customTitle":"x","sessionId":"new"}))
        + &copied("u1", None, "one", true)
        + &copied("a1", Some("u1"), "first", false)
        + &copied("u3", Some("a1"), "two again", true)
        + &line(json!({"type":"derived-from","v":1,"sessionId":"new","sessions":[{"id":"old","via":"fork"}]}))
        + &prompt("u4", Some("u3"), "after the fork");
    fs::write(&path, &text).unwrap();
    assert_eq!(texts(&Reader::open(&path, 1 << 20).unwrap()), ["> one", "first", "> two again", "> after the fork"]);
}

#[test]
fn sidechains_and_subagents_neither_hide_nor_are_the_branch() {
    let folder = Folder::new("side");
    let path = folder.0.join("s.jsonl");
    let side = |uuid: &str, parent: &str, text: &str| {
        line(json!({"type":"user","uuid":uuid,"parentUuid":parent,"isSidechain":true,"message":{"content":text}}))
    };
    // A sidechain written last, branching off an earlier prompt, is not the newest leaf.
    let text = prompt("u1", None, "one")
        + &call("c1", "u1", "agent")
        + &side("s1", "c1", "subagent prompt")
        + &result("r1", "c1", "agent")
        + &answer("a1", "r1", "done")
        + &side("s2", "u1", "late sidechain");
    fs::write(&path, &text).unwrap();
    assert_eq!(texts(&Reader::open(&path, 1 << 20).unwrap()), ["> one", "agent Ok", "done"]);
    // A subagent's own transcript is all sidechain and never filtered.
    let agents = folder.0.join("s").join("subagents");
    fs::create_dir_all(&agents).unwrap();
    let agent = agents.join("agent-x.jsonl");
    fs::write(&agent, side("x1", "c1", "task") + &side("x2", "c1", "another root")).unwrap();
    assert_eq!(texts(&Reader::open(&agent, 1 << 20).unwrap()), ["> task", "> another root"]);
}

#[test]
fn a_compaction_keeps_the_conversation_before_it_on_the_branch() {
    let folder = Folder::new("compact");
    let path = folder.0.join("s.jsonl");
    let boundary = line(json!({"type":"system","subtype":"compact_boundary","uuid":"b1","parentUuid":null,
        "logicalParentUuid":"a2","content":"Conversation compacted","compactMetadata":{"trigger":"auto"}}));
    let summary = line(json!({"type":"user","uuid":"s1","parentUuid":"b1","isCompactSummary":true,
        "message":{"content":"Summary"}}));
    let text = two_turns() + &boundary + &summary + &prompt("u3", Some("s1"), "three") + &answer("a3", "u3", "third");
    fs::write(&path, &text).unwrap();
    let shown = texts(&Reader::open(&path, 1 << 20).unwrap());
    assert_eq!(shown, ["> one", "first", "> two", "second", "! Summary", "> three", "third"]);
    // A rewind after the compaction to its first prompt hides only the turn it replaced.
    append(&path, &prompt("u4", Some("s1"), "three again"));
    let shown = texts(&Reader::open(&path, 1 << 20).unwrap());
    assert_eq!(shown, ["> one", "first", "> two", "second", "! Summary", "> three again"]);
}

#[test]
fn the_bounded_slice_hides_only_what_its_records_prove_and_history_completes_it() {
    let folder = Folder::new("slice");
    let path = folder.0.join("s.jsonl");
    let head = prompt("u1", None, &"long ".repeat(20_000)) + &answer("a1", "u1", "first");
    let replaced = prompt("u2", Some("a1"), "two");
    let tail = answer("a2", "u2", "second") + &prompt("u3", Some("a1"), "two again") + &answer("a3", "u3", "again");
    fs::write(&path, head.clone() + &replaced + &tail).unwrap();
    let mut reader = Reader::open(&path, tail.len() + 5).unwrap();
    // The replaced prompt is not loaded: its answer cannot be placed and stays.
    assert_eq!(texts(&reader), ["second", "> two again", "again"]);
    let changes = reader.earlier().unwrap();
    assert!(changes.branch_changed);
    assert_eq!(texts(&reader), ["first", "> two again", "again"]);
    while reader.has_earlier() {
        reader.earlier().unwrap();
    }
    assert_eq!(reader.items().count(), 4);
    assert_eq!(texts(&reader)[1..], ["first", "> two again", "again"]);
}

#[test]
fn records_without_links_are_shown_as_before() {
    let folder = Folder::new("plain");
    let path = folder.0.join("s.jsonl");
    let plain = |text: &str| line(json!({"type":"user","message":{"content":text}}));
    fs::write(&path, plain("one") + &two_turns() + &plain("two")).unwrap();
    assert_eq!(texts(&Reader::open(&path, 1 << 20).unwrap()), ["> one", "> one", "first", "> two", "second", "> two"]);
}

#[test]
fn a_late_notice_parented_to_the_previous_turn_does_not_hide_the_prompt_just_typed() {
    // A recap or notice written after the new prompt but linked to the previous turn's last record
    // is the newest record: the prompt is not the one a rewind replaced, and stays with its turn.
    let folder = Folder::new("late-notice");
    let path = folder.0.join("s.jsonl");
    let recap = line(json!({"type":"system","subtype":"away_summary","uuid":"n1","parentUuid":"a1",
        "content":"While you were away"}));
    fs::write(&path, two_turns() + &prompt("u3", Some("a2"), "three") + &recap).unwrap();
    let mut reader = Reader::open(&path, 1 << 20).unwrap();
    let shown = ["> one", "first", "> two", "second", "> three"];
    assert_eq!(texts(&reader)[..5], shown, "{:?}", texts(&reader));
    append(&path, &answer("a3", "u3", "third"));
    reader.poll().unwrap();
    assert!(texts(&reader).contains(&"third".to_owned()));
    assert!(texts(&reader).contains(&"> three".to_owned()));
    // A rewind still hides: the next prompt at the same place replaces the earlier one.
    append(&path, &prompt("u4", Some("a2"), "three again"));
    reader.poll().unwrap();
    let after = texts(&reader);
    assert!(!after.contains(&"> three".to_owned()) && !after.contains(&"third".to_owned()), "{after:?}");
    assert!(after.contains(&"> three again".to_owned()));
}
