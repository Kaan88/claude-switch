use super::*;
use std::fs::FileTimes;
use std::time::{Duration, UNIX_EPOCH};

fn measure(transcript: &str) -> Stats {
    collect(transcript.as_bytes(), || false).unwrap()
}

#[test]
fn optional_usage_fields_latest_context_and_unique_message_output() {
    let stats = measure(concat!(
        r#"{"type":"user","message":{"content":"hello"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"a","usage":{"input_tokens":12,"output_tokens":14},"content":[{"type":"tool_use","id":"t"},{"type":"text","text":"first"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"a","usage":{"input_tokens":12,"output_tokens":17},"content":[{"type":"tool_use","id":"t"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"a","content":[{"type":"tool_use","id":"other"}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"image"},{"type":"text","text":"not a prompt"}]}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"image"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"b","usage":{"input_tokens":11,"output_tokens":0,"cache_read_input_tokens":600,"cache_creation_input_tokens":99}}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"c","usage":{"cache_creation_input_tokens":3,"cache_read_input_tokens":100,"output_tokens":20}}}"#,
        "\n",
        r#"{"type":"assistant","isSidechain":true,"message":{"usage":{"input_tokens":500,"output_tokens":500}}}"#,
        "\n",
    ));
    assert_eq!((stats.turns, stats.tools, stats.pictures), (2, 2, 2));
    assert_eq!(stats.context, Tokens { count: 103, about: false });
    assert_eq!(stats.output, Tokens { count: 37, about: false });
}

#[test]
fn missing_usage_estimates_characters_not_bytes_and_leaves_real_output_intact() {
    let stats = measure(concat!(
        r#"{"type":"user","message":{"content":"12好4"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"known","content":"ahoy","usage":{"input_tokens":42,"output_tokens":11}}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"unknown","content":[{"type":"text","text":"猫猫猫猫"},{"type":"thinking","text":"hidden"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"unknown","content":"猫猫猫猫"}}"#,
        "\n",
    ));
    assert_eq!(stats.context, Tokens { count: 3, about: true });
    assert_eq!(stats.output, Tokens { count: 12, about: true });
    let zero = measure("{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":0}}}\n");
    assert_eq!(zero.context, Tokens { count: 0, about: false });
    assert_eq!(zero.output, Tokens { count: 0, about: true });
}

#[test]
fn a_stamp_checks_both_size_and_time_and_counts_subagent_files() {
    use crate::fulltext::tests::Folder;
    let temp = Folder::new();
    let path = temp.0.join("session.jsonl");
    let agents = temp.0.join("session").join("subagents");
    fs::create_dir_all(&agents).unwrap();
    for file in ["agent-one.jsonl", "agent-two.jsonl", "agent-two.meta.json", "agent-.jsonl", "random.jsonl"] {
        fs::write(agents.join(file), "").unwrap();
    }
    fs::create_dir(agents.join("agent-directory.jsonl")).unwrap();
    let timestamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let write = |src: &str, modified| {
        fs::write(&path, src).unwrap();
        File::options().write(true).open(&path).unwrap().set_times(FileTimes::new().set_modified(modified)).unwrap();
    };
    let one = "{\"type\":\"user\",\"message\":{\"content\":\"abc\"}}\n";
    let two = format!("{one}{one}");
    write(one, timestamp);
    let first = read(&path, None, || false).unwrap().unwrap();
    assert_eq!((first.stats.turns, first.stats.subagents), (1, 2));
    assert_eq!(read(&path, Some(first), || false).unwrap(), Some(first));
    assert_eq!(read(&path, Some(first), || true).unwrap(), None);
    write(&two, timestamp);
    let second = read(&path, Some(first), || false).unwrap().unwrap();
    assert_eq!(second.stats.turns, 2);
    write(&" ".repeat(two.len()), timestamp + Duration::from_secs(2));
    assert_eq!(read(&path, Some(second), || false).unwrap().unwrap().stats.turns, 0);
}

#[test]
fn a_compaction_after_the_last_answer_makes_the_context_size_stale() {
    let before = concat!(
        r#"{"type":"user","message":{"content":"hello"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"a","usage":{"input_tokens":275000,"output_tokens":10}}}"#,
        "\n",
        r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted","compactMetadata":{"trigger":"manual","preTokens":275000}}"#,
        "\n",
        r#"{"type":"user","isCompactSummary":true,"message":{"content":"Summary of the conversation"}}"#,
        "\n",
    );
    let stats = measure(before);
    assert!(stats.compacted);
    assert_eq!(stats.turns, 1, "the summary is not a turn");
    let after = format!(
        "{before}{}\n",
        r#"{"type":"assistant","message":{"id":"b","usage":{"input_tokens":9000,"output_tokens":10}}}"#
    );
    let stats = measure(&after);
    assert!(!stats.compacted);
    assert_eq!(stats.context, Tokens { count: 9000, about: false });
}
