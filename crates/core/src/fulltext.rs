//! Read-only conversation search, with bounded lines and result excerpts.

use std::io::{self, BufRead, BufReader};
use std::ops::Range;

use serde_json::Value;

use crate::transcript::{LineKind, MAX_LINE, line_kind};

pub const SHOWN: usize = 3;
const CHUNK: usize = 64 * 1024;

/// A scan can be interrupted between chunks, even inside an oversized record.
/// Like the live reader, ignore unterminated lines (the writer may still be appending).
pub(crate) fn records(
    input: impl io::Read,
    cancelled: impl Fn() -> bool,
    mut visit: impl FnMut(LineKind, &Value),
) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(CHUNK, input);
    let mut line = Vec::new();
    let mut over = false;
    while !cancelled() {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let mut rest = chunk;
        while !rest.is_empty() && !cancelled() {
            let end = rest.iter().position(|&b| b == b'\n');
            let len = end.unwrap_or(rest.len());
            let part = &rest[..len];
            over |= len + usize::from(end.is_some()) > MAX_LINE - line.len();
            if end.is_some() {
                if !over {
                    if line.is_empty() {
                        record(part, &mut visit)?;
                    } else {
                        line.extend_from_slice(part);
                        record(&line, &mut visit)?;
                    }
                }
                line.clear();
                over = false;
                rest = &rest[len + 1..];
            } else {
                if !over {
                    line.extend_from_slice(part);
                }
                break;
            }
        }
        let len = chunk.len();
        reader.consume(len);
    }
    Ok(())
}

fn record(bytes: &[u8], visit: &mut impl FnMut(LineKind, &Value)) -> io::Result<()> {
    let Some(kind @ (LineKind::User | LineKind::Assistant)) = line_kind(bytes) else { return Ok(()) };
    if !bounded_structure(bytes, 65_536) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Conversation record exceeds the JSON structure budget",
        ));
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else { return Ok(()) };
    let expected = if kind == LineKind::User { "user" } else { "assistant" };
    let message = &value["message"];
    if value["type"] == expected
        && (kind != LineKind::User || crate::prompt_clean::main_user(&value))
        && ["isMeta", "isSidechain", "isCompactSummary", "isApiErrorMessage"].into_iter().all(|k| value[k] != true)
        && value["parent_tool_use_id"].is_null()
        && message.is_object()
    {
        visit(kind, message);
    }
    Ok(())
}

fn bounded_structure(bytes: &[u8], mut remaining: usize) -> bool {
    let (mut quoted, mut escaped) = (false, false);
    for &b in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
        } else if b == b'"' {
            quoted = true;
        } else if matches!(b, b'{' | b'[' | b',' | b':') {
            let Some(next) = remaining.checked_sub(1) else { return false };
            remaining = next;
        }
    }
    true
}

pub(crate) fn blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message["content"].as_array().into_iter().flatten()
}

pub(crate) fn text(message: &Value) -> impl Iterator<Item = &str> {
    message["content"]
        .as_str()
        .into_iter()
        .chain(blocks(message).filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str()))
}

pub(crate) fn prompts(message: &Value) -> impl Iterator<Item = String> {
    crate::prompt_clean::prompt_blocks(text(message))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub line: String,
    /// Byte ranges in `line`, sorted and disjoint.
    pub matches: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hits {
    pub shown: Vec<Hit>,
    pub more: usize,
}

#[derive(Debug)]
pub struct Query(Vec<String>);

impl Query {
    pub fn new(text: &str) -> Option<Self> {
        let mut words: Vec<_> = text.split_whitespace().map(fold).collect();
        words.sort_unstable();
        words.dedup();
        (!words.is_empty()).then_some(Self(words))
    }

    /// Publish each newly visible hit immediately, and publish the final overflow count once.
    pub fn scan(
        &self,
        input: impl io::Read,
        cancelled: impl Fn() -> bool,
        mut publish: impl FnMut(&Hits),
    ) -> io::Result<()> {
        let mut hits = Hits::default();
        let result = records(input, &cancelled, |kind, message| {
            let parts: Vec<_> = if kind == LineKind::User {
                prompts(message).map(std::borrow::Cow::Owned).collect()
            } else {
                text(message).map(std::borrow::Cow::Borrowed).collect()
            };
            for line in parts.iter().flat_map(|part| part.split(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'])) {
                if cancelled() {
                    break;
                }
                let folded = fold(line);
                if !self.0.iter().all(|part| folded.contains(part.as_str())) {
                    continue;
                }
                if hits.shown.len() < SHOWN {
                    hits.shown.push(self.excerpt(line, &folded));
                    publish(&hits);
                } else {
                    hits.more += 1;
                }
            }
        });
        if hits.more > 0 && !cancelled() {
            publish(&hits);
        }
        result
    }

    fn excerpt(&self, line: &str, folded: &str) -> Hit {
        let first = self.0.iter().filter_map(|word| folded.find(word)).min().unwrap_or_default();
        let first = source_range(line, first..first + 1).start;
        let from = line[..first].char_indices().rev().nth(31).map_or(0, |(i, _)| i);
        let tail = &line[from..];
        let to = from + tail.char_indices().nth(256).map_or(tail.len(), |(i, _)| i);
        let mut excerpt = String::new();
        if from > 0 {
            excerpt.push('…');
        }
        let prefix = excerpt.len();
        // Keep character-to-byte mapping in the rendered excerpt, including tabs/control text.
        let mut offsets = Vec::with_capacity(257);
        let mut folded_excerpt = String::new();
        for c in line[from..to].chars() {
            offsets.push((folded_excerpt.len(), excerpt.len()));
            folded_excerpt.extend(c.to_lowercase());
            excerpt.push(if c.is_control() { ' ' } else { c });
        }
        offsets.push((folded_excerpt.len(), excerpt.len()));
        if to < line.len() {
            excerpt.push('…');
        }
        let mut ranges = Vec::new();
        for word in &self.0 {
            for (start, part) in folded_excerpt.match_indices(word) {
                let a = offsets.partition_point(|&(pos, _)| pos <= start) - 1;
                let b = offsets.partition_point(|&(pos, _)| pos < start + part.len());
                ranges.push(offsets[a].1..offsets[b].1);
            }
        }
        // A query term can itself exceed the excerpt: still emphasize its visible start.
        if ranges.is_empty() {
            let at = line[from..first].chars().count();
            ranges.push(offsets[at].1.max(prefix)..offsets.last().map_or(prefix, |p| p.1));
        }
        ranges.sort_unstable_by_key(|r| r.start);
        let mut matches: Vec<Range<usize>> = Vec::new();
        for r in ranges {
            if let Some(last) = matches.last_mut().filter(|last| last.end >= r.start) {
                last.end = last.end.max(r.end);
            } else {
                matches.push(r);
            }
        }
        Hit { line: excerpt, matches }
    }
}

fn fold(text: &str) -> String {
    if text.is_ascii() { text.to_ascii_lowercase() } else { text.chars().flat_map(char::to_lowercase).collect() }
}

fn source_range(text: &str, range: Range<usize>) -> Range<usize> {
    if text.is_ascii() {
        return range;
    }
    let mut offset = 0;
    let mut start = None;
    for (i, c) in text.char_indices() {
        offset += c.to_lowercase().map(char::len_utf8).sum::<usize>();
        if offset > range.start {
            start.get_or_insert(i);
        }
        if offset >= range.end {
            return start.unwrap_or(i)..i + c.len_utf8();
        }
    }
    text.len()..text.len()
}

#[cfg(test)]
pub(crate) mod tests;
