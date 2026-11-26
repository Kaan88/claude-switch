//! Shared classification of main user records and removal of runtime reminder spans.
use serde_json::Value;

pub(crate) fn main_user(record: &Value) -> bool {
    record["type"] == "user"
        && record["message"]["role"].as_str().is_none_or(|role| role == "user")
        && ["isMeta", "isSidechain", "isCompactSummary", "isApiErrorMessage"].iter().all(|key| record[*key] != true)
        && record["parent_tool_use_id"].is_null()
        && record["parentToolUseId"].is_null()
}

pub(crate) fn strip_blocks<'a>(blocks: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut text = String::new();
    let ends: Vec<_> = blocks
        .into_iter()
        .map(|block| {
            text.push_str(block);
            text.len()
        })
        .collect();
    let mut result = vec![String::new(); ends.len()];
    let mut offset = 0;
    let mut block = 0;
    let mut keep = |mut start: usize, end: usize| {
        while start < end {
            while ends[block] <= start {
                block += 1;
            }
            let stop = end.min(ends[block]);
            result[block].push_str(&text[start..stop]);
            start = stop;
        }
    };
    while let Some(open) = text[offset..].find("<system-reminder") {
        let open = offset + open;
        keep(offset, open);
        let Some(close) = text[open..].find("</system-reminder>") else { return result };
        offset = open + close + "</system-reminder>".len();
    }
    keep(offset, text.len());
    result
}

pub(crate) fn user_blocks<'a>(blocks: impl IntoIterator<Item = &'a str>) -> impl Iterator<Item = String> {
    strip_blocks(blocks).into_iter().filter(|text| {
        let text = text.trim_start();
        !text.is_empty()
            && ![
                "<local-command-",
                "<command/>",
                "<command>",
                "<command-name>",
                "<command-message>",
                "<command-args>",
                "<task-notification>",
                "<teammate-message",
                "<tool-result",
                "<tool_result",
                "<ide_opened_file",
                "<ide_selection",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
    })
}

/// Index, naming, search and stats do not treat other blocks of an internal record as prompts.
pub(crate) fn prompt_blocks<'a>(blocks: impl IntoIterator<Item = &'a str>) -> impl Iterator<Item = String> {
    let mut cleaned = strip_blocks(blocks);
    if cleaned.iter().any(|block| block.trim_start().starts_with('<')) {
        cleaned.clear();
    }
    cleaned.into_iter().filter(|block| !block.trim().is_empty())
}
