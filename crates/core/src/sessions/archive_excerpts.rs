use super::{CHUNK, Lines, contains, flag, parse, prompt_text, split_lines, str_field};
use serde_json::Value;
use std::path::Path;

#[derive(Default)]
pub struct ArchiveExcerpts {
    pub first_active: Option<u64>,
    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
}

fn main_conversation(v: &Value) -> bool {
    !flag(v, "isSidechain") && !flag(v, "isMeta") && !flag(v, "isCompactSummary")
}

/// Read the same bounded head and tail as the index, retaining line breaks for Markdown escaping.
pub fn archive_excerpts(file: &Path) -> ArchiveExcerpts {
    let Some(lines) = Lines::read(file, CHUNK) else { return ArchiveExcerpts::default() };
    let mut result = ArchiveExcerpts::default();
    let head = if lines.head.len() > CHUNK as usize {
        let head = &lines.head[..CHUNK as usize];
        &head[..head.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)]
    } else {
        &lines.head
    };
    for v in split_lines(head)
        .into_iter()
        .filter(|l| contains(l, b"\"user\"") || contains(l, b"\"assistant\""))
        .filter_map(parse)
        .filter(main_conversation)
    {
        if result.first_active.is_none() && matches!(str_field(&v, "type"), Some("user" | "assistant")) {
            result.first_active = str_field(&v, "timestamp")
                .and_then(crate::usage::timestamp)
                .and_then(|secs| u64::try_from(secs).ok()?.checked_mul(1000));
        }
        if result.first_prompt.is_none() {
            result.first_prompt = prompt_text(&v).map(|s| limited(&s));
        }
        if result.first_active.is_some() && result.first_prompt.is_some() {
            break;
        }
    }
    result.last_prompt = lines
        .recent()
        .filter(|l| contains(l, b"\"user\"") || contains(l, b"\"last-prompt\""))
        .filter_map(parse)
        .filter(main_conversation)
        .find_map(|v| {
            if str_field(&v, "type") == Some("last-prompt") {
                str_field(&v, "lastPrompt")
                    .filter(|s| !s.trim().is_empty() && !s.trim_start().starts_with('<'))
                    .map(limited)
            } else {
                prompt_text(&v).map(|s| limited(&s))
            }
        });
    result
}

fn limited(text: &str) -> String {
    // The renderer counts CRLF as one display character. Keep enough source characters for that.
    text.trim().chars().take(320).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, put};

    #[test]
    fn indexed_head_and_tail_keep_literal_breaks_and_skip_internal_rows() {
        let t = TempDir::new("archive-excerpts");
        let file = t.0.join("transcript.jsonl");
        let head = r#"{"type":"user","isMeta":true,"message":{"content":"internal"},"timestamp":"2025-01-01T00:00:00Z"}
{"type":"assistant","timestamp":"1970-01-01T00:00:01Z","message":{"content":"Hello"}}
{"type":"user","message":{"content":[{"type":"text","text":"first|a\nb"}]}}
"#;
        let tail = r#"{"type":"last-prompt","lastPrompt":"last|one\r\ntwo"}
{"type":"user","isSidechain":true,"message":{"content":"not the last"}}
"#;
        let middle = " ".repeat(CHUNK as usize * 2);
        put(&file, &format!("{head}{middle}\n{tail}"));
        let excerpt = archive_excerpts(&file);
        assert_eq!(excerpt.first_active, Some(1000));
        assert_eq!(excerpt.first_prompt.as_deref(), Some("first|a\nb"));
        assert_eq!(excerpt.last_prompt.as_deref(), Some("last|one\r\ntwo"));
    }
}
