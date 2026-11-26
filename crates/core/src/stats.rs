//! Counts and token usage for one transcript. No application state or UI.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::fulltext::{blocks, prompts, records, text};
use crate::transcript::LineKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub count: u64,
    pub about: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub turns: usize,
    pub tools: usize,
    pub pictures: usize,
    pub subagents: usize,
    pub context: Tokens,
    /// The conversation was compacted after its last answer: `context` is the size from before.
    pub compacted: bool,
    pub output: Tokens,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: SystemTime,
    agents: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cached {
    stamp: Stamp,
    pub stats: Stats,
}

impl Stamp {
    fn at(file: &Metadata, agents: &Path) -> io::Result<Self> {
        Ok(Self {
            len: file.len(),
            modified: file.modified()?,
            agents: fs::metadata(agents).ok().and_then(|m| m.modified().ok()),
        })
    }
}

/// Check the file length and modification time, and the subagent directory's time.
/// Do not cache a read that raced an append/replacement or was cancelled.
pub fn read(path: &Path, cached: Option<Cached>, cancelled: impl Fn() -> bool) -> io::Result<Option<Cached>> {
    if cancelled() {
        return Ok(None);
    }
    let agents = path.with_extension("").join("subagents");
    let file = File::open(path)?;
    let stamp = Stamp::at(&file.metadata()?, &agents)?;
    if cached.is_some_and(|c| c.stamp == stamp) {
        return Ok(cached);
    }
    let mut stats = collect(file.take(stamp.len), &cancelled)?;
    if cancelled() || Stamp::at(&fs::metadata(path)?, &agents)? != stamp {
        return Ok(None);
    }
    let entries = match fs::read_dir(&agents) {
        Ok(entries) => Some(entries),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    for entry in entries.into_iter().flatten() {
        if cancelled() {
            return Ok(None);
        }
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.file_name().to_str().is_some_and(|n| {
                n.strip_prefix("agent-").and_then(|n| n.strip_suffix(".jsonl")).is_some_and(|id| !id.is_empty())
            })
        {
            stats.subagents += 1;
        }
    }
    Ok((Stamp::at(&fs::metadata(path)?, &agents)? == stamp).then_some(Cached { stamp, stats }))
}

#[derive(Default)]
struct Output {
    exact: Option<u64>,
    chars: u64,
    seen: HashSet<u64>,
}

impl Output {
    fn add(&mut self, message: &Value) {
        if let Some(count) = message["usage"]["output_tokens"].as_u64() {
            self.exact = Some(count);
        }
        for part in text(message) {
            let mut hash = DefaultHasher::new();
            part.hash(&mut hash);
            if self.seen.len() < 128 && self.seen.insert(hash.finish()) {
                self.chars += part.chars().count() as u64;
            }
        }
    }
}

/// One turn per user prompt (including picture-only prompts), not per tool response.
/// Use the latest assistant record for context and the latest reported output per message ID.
/// Missing output is estimated for those messages only; context falls back to conversation text.
pub fn collect(input: impl io::Read, cancelled: impl Fn() -> bool) -> io::Result<Stats> {
    let mut stats = Stats::default();
    let (mut user_chars, mut assistant_chars, mut output, mut missing) = (0u64, 0u64, 0u64, 0u64);
    let mut context = None;
    let mut estimated_output = false;
    let mut assistant_seen = false;
    const MAX_IDS: usize = 16_384;
    let (mut messages, mut tools) = (HashMap::<u64, Output>::new(), HashSet::<u64>::new());
    let mut exceeded = false;
    records(input, cancelled, |kind, message| {
        if exceeded {
            return;
        }
        if kind == LineKind::System {
            stats.compacted = true;
            return;
        }
        let mut pictures = 0;
        let mut prompt = false;
        for block in blocks(message) {
            match block["type"].as_str() {
                Some("image") => {
                    pictures += 1;
                    prompt = true;
                }
                Some("tool_result") => {
                    stats.pictures +=
                        block["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "image").count();
                }
                Some("tool_use" | "server_tool_use")
                    if kind == LineKind::Assistant && block["id"].as_str().is_none_or(|id| tools.insert(hash(id))) =>
                {
                    stats.tools += 1;
                    exceeded |= tools.len() > MAX_IDS;
                }
                _ => {}
            }
        }
        stats.pictures += pictures;
        if kind == LineKind::User {
            let chars = prompts(message).map(|s| s.chars().count() as u64).sum::<u64>();
            user_chars += chars;
            stats.turns += usize::from(prompt || chars != 0);
        } else {
            assistant_seen = true;
            stats.compacted = false;
            let usage = &message["usage"];
            context = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
                .into_iter()
                .filter_map(|field| usage[field].as_u64())
                .reduce(u64::saturating_add);
            if let Some(id) = message["id"].as_str() {
                let entry = messages.entry(hash(id)).or_default();
                entry.add(message);
                exceeded |= entry.seen.len() >= 128 || messages.len() > MAX_IDS;
            } else {
                let mut entry = Output::default();
                entry.add(message);
                assistant_chars += entry.chars;
                if let Some(count) = entry.exact {
                    output = output.saturating_add(count);
                } else {
                    missing += entry.chars;
                    estimated_output = true;
                }
            }
        }
    })?;
    if exceeded {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Conversation exceeds the statistics deduplication budget",
        ));
    }
    for entry in messages.into_values() {
        assistant_chars += entry.chars;
        if let Some(count) = entry.exact {
            output = output.saturating_add(count);
        } else {
            missing += entry.chars;
            estimated_output = true;
        }
    }
    stats.context = context.map_or(Tokens { count: (user_chars + assistant_chars) / 4, about: true }, |count| Tokens {
        count,
        about: false,
    });
    stats.output = Tokens { count: output.saturating_add(missing / 4), about: estimated_output || !assistant_seen };
    Ok(stats)
}

fn hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests;
