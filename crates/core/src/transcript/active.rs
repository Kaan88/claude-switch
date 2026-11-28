//! Which records are on the conversation's active branch.
//!
//! Claude Code links each message record to the one before it (`parentUuid`; a compaction boundary,
//! which starts a fresh chain, names the one before it in `logicalParentUuid`). A rewind appends
//! nothing; the next prompt names an earlier message as its parent, so the turns it replaced stay in
//! the file on a branch of their own. The active branch is the chain from the newest record back.
//!
//! Only what the loaded records prove is hidden: a record is off the branch when, following its own
//! parents, it reaches a record on the chain through a typed prompt that was written before the
//! chain's own next record there, and that next record is a typed prompt too (the prompt a rewind
//! replaced, and the one typed after the rewind). A prompt is never hidden because a later notice
//! or recap parented to an earlier record became the newest record. Records whose parents lead out of
//! the loaded slice, records without a `uuid`, sidechains, and records hanging off the chain that do
//! not start with a prompt (progress, tool results, notices) are never hidden.

use serde::Deserialize;
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};

struct Node {
    offset: u64,
    parent: Option<String>,
    /// A user record with text or a picture: a prompt a rewind can replace.
    prompt: bool,
}

#[derive(Default)]
pub(super) struct Active {
    nodes: HashMap<String, Node>,
    dirty: bool,
    /// The newest linked record's offset.
    newest: Option<u64>,
    /// Offsets of the records off the active branch.
    hidden: BTreeSet<u64>,
}

#[derive(Deserialize)]
struct Link<'a> {
    #[serde(rename = "type", borrow, default)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    uuid: Option<Cow<'a, str>>,
    #[serde(rename = "parentUuid", borrow, default)]
    parent: Option<Cow<'a, str>>,
    #[serde(rename = "logicalParentUuid", borrow, default)]
    logical: Option<Cow<'a, str>>,
    #[serde(rename = "isSidechain", default)]
    sidechain: Option<serde_json::Value>,
    #[serde(borrow, default)]
    message: Option<Message<'a>>,
}

#[derive(Deserialize)]
struct Message<'a> {
    #[serde(borrow, default)]
    content: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Block<'a> {
    #[serde(rename = "type", borrow, default)]
    kind: Option<Cow<'a, str>>,
}

/// Text or a picture: anything but tool results only.
fn is_prompt(content: &RawValue) -> bool {
    let raw = content.get().trim_start();
    if raw.starts_with('"') {
        return true;
    }
    serde_json::from_str::<Vec<Block>>(raw)
        .is_ok_and(|blocks| blocks.iter().any(|b| b.kind.as_deref().is_some_and(|k| k != "tool_result")))
}

#[derive(Clone, Copy)]
struct Next {
    offset: u64,
    prompt: bool,
}

enum Verdict {
    Keep,
    Hide,
}

impl Active {
    /// Notes the record at `offset` from its line, for a line the reader parses no further (a
    /// context attachment, a record of another type): its links are read without building a tree.
    /// Records come in any order (history pages in after newer records).
    pub(super) fn line(&mut self, offset: u64, line: &[u8]) {
        let Ok(link) = serde_json::from_slice::<Link>(line) else { return };
        let prompt =
            link.kind.as_deref() == Some("user") && link.message.and_then(|m| m.content).is_some_and(is_prompt);
        let sidechain = link.sidechain.as_ref().is_some_and(|s| s != &serde_json::Value::Bool(false));
        self.note(offset, link.uuid.as_deref(), link.parent.as_deref(), link.logical.as_deref(), sidechain, prompt);
    }

    /// Notes the record at `offset` from the tree the reader already parsed.
    pub(super) fn value(&mut self, offset: u64, value: &serde_json::Value) {
        let text = |key: &str| value[key].as_str();
        let content = &value["message"]["content"];
        let prompt = value["type"] == "user"
            && (content.is_string()
                || content.as_array().is_some_and(|blocks| {
                    blocks.iter().any(|b| b["type"].as_str().is_some_and(|k| k != "tool_result"))
                }));
        let sidechain = value.get("isSidechain").is_some_and(|s| s != &serde_json::Value::Bool(false));
        self.note(offset, text("uuid"), text("parentUuid"), text("logicalParentUuid"), sidechain, prompt);
    }

    fn note(
        &mut self,
        offset: u64,
        uuid: Option<&str>,
        parent: Option<&str>,
        logical: Option<&str>,
        sidechain: bool,
        prompt: bool,
    ) {
        let Some(uuid) = uuid.filter(|u| !u.is_empty()) else { return };
        if sidechain {
            return;
        }
        let parent = parent.filter(|p| !p.is_empty()).or(logical.filter(|p| !p.is_empty()));
        self.nodes.insert(uuid.to_owned(), Node { offset, parent: parent.map(str::to_owned), prompt });
        self.newest = self.newest.max(Some(offset));
        self.dirty = true;
    }

    /// The newest linked record's offset (a message, notice or command record).
    pub(super) fn newest(&self) -> Option<u64> {
        self.newest
    }

    pub(super) fn hides(&self, offset: u64) -> bool {
        self.hidden.contains(&offset)
    }

    /// Works out the branch again after new records; true when what is hidden changed.
    pub(super) fn finish(&mut self) -> bool {
        if !std::mem::take(&mut self.dirty) {
            return false;
        }
        let hidden = self.compute();
        let changed = hidden != self.hidden;
        self.hidden = hidden;
        changed
    }

    fn compute(&self) -> BTreeSet<u64> {
        let Some((leaf, _)) = self.nodes.iter().max_by_key(|(_, node)| node.offset) else {
            return BTreeSet::new();
        };
        // The chain, each record with the chain's record after it: its offset, and whether it is a
        // typed prompt (only a prompt can have replaced another: a rewind's next prompt).
        let mut chain: HashMap<&str, Next> = HashMap::new();
        let mut at = leaf.as_str();
        let mut next = Next { offset: u64::MAX, prompt: false };
        // The chain's first record when the chain is whole back to its start.
        let mut start = None;
        loop {
            let node = &self.nodes[at];
            if chain.insert(at, next).is_some() {
                break;
            }
            next = Next { offset: node.offset, prompt: node.prompt };
            match node.parent.as_deref() {
                None => {
                    start = Some(next);
                    break;
                }
                Some(parent) if self.nodes.contains_key(parent) => at = parent,
                Some(_) => break,
            }
        }
        let mut verdicts: HashMap<&str, bool> = HashMap::new();
        let mut hidden = BTreeSet::new();
        for (uuid, node) in &self.nodes {
            if chain.contains_key(uuid.as_str()) {
                continue;
            }
            // Up through the parents to the chain, the start or the slice's edge.
            let mut path = vec![uuid.as_str()];
            let verdict = loop {
                let here = path[path.len() - 1];
                if let Some(&known) = verdicts.get(here) {
                    break if known { Verdict::Hide } else { Verdict::Keep };
                }
                let current = &self.nodes[here];
                // A prompt written before the chain's own next prompt at the same place: replaced
                // by a rewind. A notice or recap parented to an earlier record replaces nothing.
                let replaced = |next: Next| current.prompt && next.prompt && current.offset < next.offset;
                match current.parent.as_deref() {
                    Some(parent) if chain.contains_key(parent) => {
                        break if replaced(chain[parent]) { Verdict::Hide } else { Verdict::Keep };
                    }
                    Some(parent) if self.nodes.contains_key(parent) && !path.contains(&parent) => path.push(parent),
                    Some(_) => break Verdict::Keep,
                    // A first prompt replaced by a rewind to the start of the conversation.
                    None => break if start.is_some_and(replaced) { Verdict::Hide } else { Verdict::Keep },
                }
            };
            let hide = matches!(verdict, Verdict::Hide);
            for uuid in path {
                verdicts.entry(uuid).or_insert(hide);
            }
            if hide {
                hidden.insert(node.offset);
            }
        }
        hidden
    }
}
