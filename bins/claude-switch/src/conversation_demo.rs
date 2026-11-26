//! Synthetic JSONL shared by demo search and details; never touches the fake paths.

use std::path::Path;

use serde_json::json;

pub fn transcript(path: &Path, prompt: &str) -> String {
    let mut lines = vec![json!({"type":"user","message":{"content":prompt}})];
    let index = path.file_stem().and_then(|stem| stem.to_str());
    let first = index == Some("0");
    let reply = if first {
        "The splitter needs a regression test at minimum width.\nAt compact width, the splitter still keeps both panes visible.\nThe splitter width is stored independently of the window size.\nI'll add a splitter width case to the test suite.\nThe wider splitter width also works in dark mode."
    } else if index == Some("2") {
        "The width stays the same when the splitter is restored from saved settings."
    } else {
        "I reproduced the issue. I’ll write a regression test before changing the implementation."
    };
    lines.push(json!({"type":"assistant","message":{"id":"answer","content":[{"type":"text","text":reply}],
        "usage": if first { json!({"input_tokens":1216,"output_tokens":422,
            "cache_read_input_tokens":4608,"cache_creation_input_tokens":1024}) } else { json!(null) }}}));
    if first {
        lines.insert(1, json!({"type":"user","message":{"content":[{"type":"image"}]}}));
        lines.insert(
            2,
            json!({"type":"assistant","message":{"id":"tools","content":[
            {"type":"tool_use","id":"one","name":"Read"},{"type":"tool_use","id":"two","name":"Agent"}],
            "usage":{"input_tokens":1024,"output_tokens":50}}}),
        );
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}
