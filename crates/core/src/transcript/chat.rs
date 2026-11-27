//! Bounded Chat metadata taken from decoded transcript records, not the terminal screen.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Active,
    EndTurn,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatInfo {
    pub generation: u64,
    pub latest: Option<(u64, Phase)>,
    pub prompt: Option<u64>,
    pub pending: Option<u64>,
    pub model: Option<String>,
    pub context: Option<u64>,
    /// The conversation was compacted after the last usage: `context` is the size from before,
    /// and the real size comes with the next answer.
    pub compacted: bool,
    /// While `compacted`: the size the compaction left (`compactMetadata.postTokens`), if known.
    pub compacted_size: Option<u64>,
    /// The newest "Set effort level to …" a local command printed, whoever typed it, with its
    /// offset.
    pub effort_report: Option<(u64, String)>,
    /// The newest "Set model to …" a local command printed, with its offset.
    pub model_report: Option<(u64, String)>,
    usage_at: Option<u64>,
    compacted_at: Option<u64>,
    /// A slash command typed while the CLI was at rest: its record's offset, until its output.
    command_at: Option<u64>,
}

impl ChatInfo {
    pub fn usage_offset(&self) -> Option<u64> {
        self.usage_at
    }

    pub(super) fn observe(&mut self, at: u64, phase: Phase) {
        if self.latest.is_none_or(|(old, _)| at >= old) {
            self.latest = Some((at, phase));
        }
    }

    /// A slash command's record (`<command-name>`), seen before it counts as a prompt. Typed at
    /// rest, it may run in the CLI alone and only print (`/model`, `/effort`): its output ends it
    /// ([`command_done`](Self::command_done)). Typed during a turn it changes nothing.
    pub(super) fn command_started(&mut self, at: u64) {
        let at_rest = self.latest.is_none_or(|(_, phase)| phase == Phase::EndTurn);
        self.command_at = at_rest.then_some(at);
    }

    /// A local command's output (`<local-command-stdout>`): a command typed at rest that nothing
    /// followed but this is over, and the CLI is at rest again. Without it the command's record
    /// stayed the newest prompt, so the turn read as running until the next answer, and Chat
    /// refused every slash command after `/model`.
    pub(super) fn command_done(&mut self, at: u64) {
        if let Some(start) = self.command_at.take()
            && self.latest == Some((start, Phase::Active))
        {
            self.observe(at, Phase::EndTurn);
        }
    }

    pub(super) fn usage(&mut self, at: u64, message: &Value) {
        if self.usage_at.is_some_and(|old| old >= at) {
            return;
        }
        let values = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
            .into_iter()
            .filter_map(|key| message["usage"][key].as_u64());
        let context = values.reduce(u64::saturating_add);
        let model = message["model"].as_str().filter(|s| !s.is_empty());
        if model.is_some() || context.is_some() {
            self.usage_at = Some(at);
            if let Some(model) = model {
                self.model = Some(model.chars().take(100).collect());
            }
            self.context = context;
            self.compacted = self.compacted_at.is_some_and(|c| c > at);
            if !self.compacted {
                self.compacted_size = None;
            }
        }
    }

    /// A compaction record: the last usage no longer tells the conversation's size. `boundary`:
    /// the `compact_boundary` record with its `compactMetadata`; else the summary that follows it,
    /// which keeps the boundary's size.
    pub(super) fn compaction(&mut self, at: u64, boundary: Option<&Value>) {
        if self.compacted_at.is_none_or(|old| at > old) {
            let after_boundary = self.compacted;
            self.compacted_at = Some(at);
            self.compacted = self.usage_at.is_none_or(|usage| at > usage);
            self.compacted_size = match boundary {
                Some(meta) => meta["postTokens"].as_u64(),
                None if after_boundary => self.compacted_size,
                None => None,
            }
            .filter(|_| self.compacted);
        }
    }

    /// A record's text: keeps the newest effort and model reports of its `<local-command-stdout>`.
    pub(super) fn local_output(&mut self, at: u64, text: &str) {
        for output in local_stdout(text) {
            for line in strip_escapes(output).lines().map(str::trim) {
                let slot = if line.starts_with(EFFORT_SET) {
                    &mut self.effort_report
                } else if line.starts_with(MODEL_SET) {
                    &mut self.model_report
                } else {
                    continue;
                };
                if slot.as_ref().is_none_or(|(old, _)| at >= *old) && line.len() <= 200 {
                    *slot = Some((at, line.to_owned()));
                }
            }
        }
    }
}

/// How Claude Code confirms `/effort` and `/model` (to confirm in a live session).
pub const EFFORT_SET: &str = "Set effort level to ";
pub const MODEL_SET: &str = "Set model to ";

fn local_stdout(text: &str) -> impl Iterator<Item = &str> {
    const OPEN: &str = "<local-command-stdout>";
    const CLOSE: &str = "</local-command-stdout>";
    text.match_indices(OPEN).filter_map(move |(at, _)| {
        let rest = &text[at + OPEN.len()..];
        rest.find(CLOSE).map(|end| &rest[..end])
    })
}

/// Text without terminal escape sequences (the CLI colors its command output).
pub(super) fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then one final character from @ to ~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || c == '\u{1b}' && chars.next_if_eq(&'\\').is_some() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolCard {
    Edit {
        path: String,
        added: usize,
        removed: usize,
        lines: Vec<String>,
        /// The edit as Claude Code recorded it with the call's result: line numbers and the
        /// lines around the change. None until the result is read, and when it has none.
        /// Boxed: most cards have none, and an item stays small.
        patch: Option<Box<Patch>>,
    },
    Shell {
        command: String,
        result: Vec<String>,
    },
    /// The questions of an `AskUserQuestion` call, exactly as Claude asked them.
    Questions(Vec<Question>),
    /// The Markdown plan of an `ExitPlanMode` call.
    Plan(String),
    /// What a slash command or a shell-mode command printed, as mono text.
    Output(String),
}

/// An edit's recorded patch (`toolUseResult.structuredPatch`), bounded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    pub hunks: Vec<Hunk>,
}

impl ToolCard {
    /// The hunks of an edit's recorded patch; none for any other card and before the result.
    pub fn hunks(&self) -> &[Hunk] {
        match self {
            Self::Edit { patch: Some(patch), .. } => &patch.hunks,
            _ => &[],
        }
    }
}

/// One hunk of an edit's recorded patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The first line's number in the file before the edit and after it.
    pub old_start: usize,
    pub new_start: usize,
    /// The hunk's lines, each starting with a space (unchanged), `-` or `+`.
    pub lines: Vec<String>,
    /// How many further lines of the patch were not kept.
    pub more: usize,
}

/// The most lines of a patch that are kept.
const MAX_PATCH_LINES: usize = 60;

/// The patch of an `Edit` or `Write` result, bounded; empty when the record has none or it is not
/// in the shape Claude Code 2.1 writes.
pub(super) fn patch(structured: &Value) -> Vec<Hunk> {
    let Some(hunks) = structured["structuredPatch"].as_array() else { return Vec::new() };
    let mut out: Vec<Hunk> = Vec::new();
    let mut kept = 0;
    for hunk in hunks {
        let (Some(old_start), Some(new_start), Some(lines)) =
            (hunk["oldStart"].as_u64(), hunk["newStart"].as_u64(), hunk["lines"].as_array())
        else {
            return Vec::new();
        };
        let mut texts = Vec::new();
        for line in lines {
            let Some(line) = line.as_str().filter(|l| l.starts_with([' ', '-', '+'])) else { return Vec::new() };
            texts.push(line);
        }
        let room = MAX_PATCH_LINES - kept;
        if room == 0 {
            if let Some(last) = out.last_mut() {
                last.more += texts.len();
            }
            continue;
        }
        let more = texts.len().saturating_sub(room);
        kept += texts.len().min(room);
        out.push(Hunk {
            old_start: old_start as usize,
            new_start: new_start as usize,
            lines: texts.into_iter().take(room).map(|line| short(line, 240)).collect(),
            more,
        });
    }
    out
}

/// One question of an `AskUserQuestion` call. Its texts are whole: the window matches them
/// against the terminal before it answers, so a shortened text would never match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

const MAX_QUESTIONS: usize = 4;
/// A plan longer than this is cut for the conversation; the terminal shows it whole.
const MAX_PLAN: usize = 20_000;
const MAX_OPTIONS: usize = 8;
const MAX_QUESTION_TEXT: usize = 2000;

fn whole(value: &Value, limit: usize) -> Option<String> {
    let text = value.as_str()?.trim();
    (text.chars().count() <= limit && !text.chars().any(|c| c.is_control() && c != '\n')).then(|| text.to_owned())
}

impl Question {
    /// `None` for anything unexpected: the card is then not offered and the terminal answers.
    fn parse(value: &Value) -> Option<Self> {
        let options = value["options"].as_array()?;
        (!options.is_empty() && options.len() <= MAX_OPTIONS).then_some(())?;
        let options = options
            .iter()
            .map(|option| {
                let label = whole(&option["label"], 200).filter(|label| !label.is_empty() && !label.contains('\n'))?;
                let description = option.get("description").map_or(Some(String::new()), |d| whole(d, 1000))?;
                Some(QuestionOption { label, description })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            header: value.get("header").map_or(Some(String::new()), |h| whole(h, 100))?,
            question: whole(&value["question"], MAX_QUESTION_TEXT).filter(|q| !q.is_empty())?,
            options,
            multi_select: value.get("multiSelect").map_or(Some(false), Value::as_bool)?,
        })
    }
}

fn short(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

impl ToolCard {
    pub(super) fn from_input(name: &str, input: &Value) -> Option<Self> {
        match name {
            "Write" | "Edit" => {
                let path = input["file_path"].as_str()?;
                let (mut old, mut new) = if name == "Write" {
                    ("".lines(), input["content"].as_str()?.lines())
                } else {
                    (input["old_string"].as_str()?.lines(), input["new_string"].as_str()?.lines())
                };
                while let (Some(a), Some(b)) = (old.clone().next(), new.clone().next()) {
                    if a != b {
                        break;
                    }
                    old.next();
                    new.next();
                }
                while let (Some(a), Some(b)) = (old.clone().next_back(), new.clone().next_back()) {
                    if a != b {
                        break;
                    }
                    old.next_back();
                    new.next_back();
                }
                let (added, removed) = (new.clone().count(), old.clone().count());
                let lines = old
                    .take(10)
                    .map(|s| format!("− {}", short(s, 180)))
                    .chain(new.take(10).map(|s| format!("+ {}", short(s, 180))))
                    .collect();
                // Never turn a shortened display string into a different actionable filename.
                (path.len() <= 4096).then(|| Self::Edit { path: path.to_owned(), added, removed, lines, patch: None })
            }
            "Bash" | "PowerShell" => {
                Some(Self::Shell { command: short(input["command"].as_str()?, 500), result: Vec::new() })
            }
            "AskUserQuestion" => {
                let questions = input["questions"].as_array()?;
                (!questions.is_empty() && questions.len() <= MAX_QUESTIONS).then_some(())?;
                Some(Self::Questions(questions.iter().map(Question::parse).collect::<Option<_>>()?))
            }
            "ExitPlanMode" => {
                let plan = input["plan"].as_str().filter(|p| !p.trim().is_empty())?;
                Some(Self::Plan(short(plan, MAX_PLAN)))
            }
            _ => None,
        }
    }
}

pub(super) fn result_lines(content: &Value) -> Vec<String> {
    result_text(content).flat_map(str::lines).take(10).map(|line| short(line, 180)).collect()
}

pub(super) fn result_text(content: &Value) -> impl Iterator<Item = &str> {
    content.as_str().into_iter().chain(
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten()),
    )
}

pub(super) fn clock_ms(raw: &str) -> Option<i64> {
    let seconds = crate::usage::timestamp(raw)?;
    let fraction = raw.split_once(['.', ',']).map_or(0, |(_, tail)| {
        tail.bytes()
            .take_while(u8::is_ascii_digit)
            .chain(std::iter::repeat(b'0'))
            .take(3)
            .fold(0, |n, digit| n * 10 + i64::from(digit - b'0'))
    });
    Some(seconds * 1000 + fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn diff_omits_common_edges_and_keeps_counts_beyond_the_preview() {
        let input = json!({"file_path":"src/a.rs","old_string":"start\nold\nend","new_string":"start\nnew\nmore\nend"});
        assert_eq!(
            ToolCard::from_input("Edit", &input),
            Some(ToolCard::Edit {
                path: "src/a.rs".into(),
                added: 2,
                removed: 1,
                lines: ["− old", "+ new", "+ more"].map(str::to_owned).into(),
                patch: None,
            })
        );
        let input = json!({"file_path":"a","content":"hello\n".repeat(100)});
        assert!(matches!(ToolCard::from_input("Write", &input),
            Some(ToolCard::Edit { added: 100, removed: 0, lines, .. }) if lines.len() == 10));
    }

    #[test]
    fn a_recorded_patch_keeps_its_line_numbers_and_context_and_is_bounded() {
        // As Claude Code 2.1.332 writes an Edit's result.
        let result = json!({"filePath":"src/a.rs","oldString":"old","newString":"new\nmore","structuredPatch":[
            {"oldStart":86,"oldLines":3,"newStart":86,"newLines":4,"lines":[" start","-old","+new","+more"," end"]},
            {"oldStart":120,"oldLines":1,"newStart":121,"newLines":1,"lines":["-x","+y"]}
        ]});
        let hunks = patch(&result);
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].old_start, hunks[0].new_start, hunks[0].more), (86, 86, 0));
        assert_eq!(hunks[0].lines, [" start", "-old", "+new", "+more", " end"]);
        assert_eq!((hunks[1].old_start, hunks[1].new_start), (120, 121));
        // Not a patch, or not in the shape known: none, never a guess.
        assert!(patch(&json!({"stdout":""})).is_empty());
        assert!(patch(&json!("User rejected tool use")).is_empty());
        assert!(patch(&json!({"structuredPatch":[{"oldStart":1,"newStart":1,"lines":["no prefix"]}]})).is_empty());
        assert!(patch(&json!({"structuredPatch":[{"oldStart":1,"lines":[" a"]}]})).is_empty());
        assert!(patch(&json!({"structuredPatch":[]})).is_empty());
        let long: Vec<String> = (0..100).map(|i| format!("+line {i}")).collect();
        let big = patch(&json!({"structuredPatch":[
            {"oldStart":1,"newStart":1,"lines":long},
            {"oldStart":300,"newStart":400,"lines":[" tail","+one"]}
        ]}));
        assert_eq!((big.len(), big[0].lines.len(), big[0].more), (1, 60, 42));
    }

    #[test]
    fn an_edits_card_gets_the_patch_of_its_own_result_once_it_succeeded() {
        use super::super::{ItemKind, Reader};
        let dir = crate::util::testing::TempDir::new("chat-patch");
        let path = dir.0.join("chat.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let call = |id: &str| {
            json!({"type":"tool_use","id":id,"name":"Edit",
                "input":{"file_path":"src/a.rs","old_string":"old","new_string":"new"}})
        };
        let result =
            |id: &str, error: bool| json!({"type":"tool_result","tool_use_id":id,"content":"done","is_error":error});
        let patch = json!({"filePath":"src/a.rs","structuredPatch":[
            {"oldStart":7,"oldLines":3,"newStart":7,"newLines":3,"lines":[" a","-old","+new"," b"]}]});
        for row in [
            json!({"type":"assistant","message":{"role":"assistant","content":[call("e1"), call("e2"), call("e3")]}}),
            json!({"type":"user","message":{"role":"user","content":[result("e1", false)]},"toolUseResult":patch}),
            // A failed edit changed nothing: its record's patch, if any, is not shown as one.
            json!({"type":"user","message":{"role":"user","content":[result("e2", true)]},"toolUseResult":patch}),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        let reader = Reader::open(&path, 8192).unwrap();
        let patches: Vec<_> = reader
            .items()
            .filter_map(|(_, item)| match &item.kind {
                ItemKind::Tool { card: Some(card @ ToolCard::Edit { .. }), .. } => Some(card.hunks().to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(patches.len(), 3);
        assert_eq!(patches[0].len(), 1);
        assert_eq!((patches[0][0].old_start, patches[0][0].lines.len()), (7, 4));
        assert!(patches[1].is_empty() && patches[2].is_empty(), "a failed and a running edit have none");
    }

    #[test]
    fn shell_preview_reads_only_the_first_text_lines() {
        let input = json!({"command":"Get-ChildItem"});
        assert!(matches!(ToolCard::from_input("PowerShell", &input),
            Some(ToolCard::Shell { command, result }) if command == "Get-ChildItem" && result.is_empty()));
        assert_eq!(result_lines(&json!([{"type":"image"},{"type":"text","text":"one\ntwo"}])), ["one", "two"]);
        assert_eq!(result_lines(&json!("line\n".repeat(50))).len(), 10);
    }

    #[test]
    fn a_command_typed_at_rest_ends_with_its_output_and_one_typed_mid_turn_changes_nothing() {
        use super::super::Reader;
        let dir = crate::util::testing::TempDir::new("chat-command");
        let path = dir.0.join("chat.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut write = |rows: &[Value]| {
            for row in rows {
                writeln!(file, "{row}").unwrap();
            }
        };
        let phase = |reader: &Reader| reader.chat().latest.map(|(_, phase)| phase);
        // As Claude Code 2.1.332 writes `/model opus` typed after a finished turn.
        let command = [
            json!({"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>Caveat</local-command-caveat>"}}),
            json!({"type":"user","message":{"content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>opus</command-args>"}}),
        ];
        let output = json!({"type":"user","message":{"content":"<local-command-stdout>Set model to \u{1b}[1mOpus 5.5\u{1b}[22m and saved as your default for new sessions</local-command-stdout>"}});
        write(&[
            json!({"type":"user","message":{"content":"Hello"}}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Hi."}],"stop_reason":"end_turn"}}),
        ]);
        write(&command);
        let mut reader = Reader::open(&path, 8192).unwrap();
        assert_eq!(phase(&reader), Some(Phase::Active), "the command still runs");
        write(std::slice::from_ref(&output));
        reader.poll().unwrap();
        assert_eq!(phase(&reader), Some(Phase::EndTurn), "its output ends it");
        assert_eq!(
            reader.chat().model_report.map(|(_, line)| line).as_deref(),
            Some("Set model to Opus 5.5 and saved as your default for new sessions")
        );

        // `/effort` right after it: the same.
        write(&[
            json!({"type":"user","message":{"content":"<command-name>/effort</command-name>\n<command-args>high</command-args>"}}),
            json!({"type":"user","message":{"content":"<local-command-stdout>Set effort level to high</local-command-stdout>"}}),
        ]);
        reader.poll().unwrap();
        assert_eq!(phase(&reader), Some(Phase::EndTurn));

        // During a turn a command's output ends nothing.
        write(&[
            json!({"type":"user","message":{"content":"Run the tests"}}),
            json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"cargo test"}}]}}),
        ]);
        write(&command);
        write(&[output]);
        reader.poll().unwrap();
        assert_eq!(phase(&reader), Some(Phase::Active), "the turn goes on");

        // A command that becomes a prompt for the model (no output of its own) stays a turn.
        write(&[
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done."}],"stop_reason":"end_turn"}}),
            json!({"type":"user","message":{"content":"<command-name>/review</command-name>"}}),
            json!({"type":"user","isMeta":true,"message":{"content":"Review the open changes."}}),
        ]);
        reader.poll().unwrap();
        assert_eq!(phase(&reader), Some(Phase::Active));
        assert_eq!(reader.chat().pending, None);
    }

    #[test]
    fn reader_tracks_pending_results_and_explicit_completion_from_the_main_transcript() {
        use super::super::{ItemKind, Reader};
        let dir = crate::util::testing::TempDir::new("chat-reader");
        let path = dir.0.join("chat.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, r#"{{"type":"user","message":{{"content":"Please check the print layout."}}}}"#).unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"assistant","timestamp":"2026-11-26T23:59:59.800Z","message":{"model":"test-model","stop_reason":"tool_use",
            "usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30},
            "content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"preview --pocket"}}]}})
        )
        .unwrap();
        let mut reader = Reader::open(&path, 8192).unwrap();
        let info = reader.chat();
        assert_eq!(info.latest.map(|(_, phase)| phase), Some(Phase::Active));
        assert_eq!(info.latest.map(|(at, _)| at), info.pending);
        assert_eq!(info.context, Some(60));
        assert_eq!(info.model.as_deref(), Some("test-model"));

        assert!(reader.items().any(|(_, item)| matches!(item.kind, ItemKind::Tool { duration_ms: None, .. })));
        for row in [
            json!({"type":"user","timestamp":"2026-11-27T01:00:00.225+01:00","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"first\nsecond"}]}}),
            json!({"type":"assistant","message":{"content":"An incomplete streaming answer."}}),
            json!({"type":"assistant","isSidechain":true,"message":{"content":"Child","stop_reason":"end_turn"}}),
            json!({"type":"user","isMeta":true,"message":{"content":"synthetic prompt"}}),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        reader.poll().unwrap();
        let after = reader.chat();
        assert_eq!(after.pending, None);
        assert_eq!(after.prompt, info.prompt);
        assert_eq!(after.latest.map(|(_, phase)| phase), Some(Phase::Unknown));
        assert!(reader.items().any(|(_, item)| matches!(&item.kind,
            ItemKind::Tool { duration_ms: Some(425), card: Some(ToolCard::Shell { result, .. }), .. }
                if result == &["first", "second"])));
        writeln!(file, "{}", json!({"type":"assistant","message":{"content":"Done.","stop_reason":"end_turn"}}))
            .unwrap();
        reader.poll().unwrap();
        assert_eq!(reader.chat().latest.map(|(_, phase)| phase), Some(Phase::EndTurn));
    }

    #[test]
    fn a_tool_card_keeps_the_exact_path_or_refuses_it() {
        let path = "x".repeat(600) + ".md";
        assert!(matches!(
            ToolCard::from_input("Write", &json!({"file_path":path,"content":"hello"})),
            Some(ToolCard::Edit { path: actual, .. }) if actual == path
        ));
        assert!(ToolCard::from_input("Write", &json!({"file_path":"x".repeat(4097),"content":"hello"})).is_none());
    }

    #[test]
    fn questions_keep_whole_texts_and_refuse_anything_unexpected() {
        let input = json!({"questions":[{
            "question":"Who builds the parser?",
            "header":"Who builds",
            "multiSelect":false,
            "options":[{"label":"Me","description":"I write it"},{"label":"You"}]
        }]});
        let Some(ToolCard::Questions(questions)) = ToolCard::from_input("AskUserQuestion", &input) else {
            panic!("no card");
        };
        assert_eq!(questions.len(), 1);
        let q = &questions[0];
        assert_eq!(
            (q.header.as_str(), q.question.as_str(), q.multi_select),
            ("Who builds", "Who builds the parser?", false)
        );
        assert_eq!(q.options[0], QuestionOption { label: "Me".into(), description: "I write it".into() });
        assert_eq!(q.options[1].description, "");
        for bad in [
            json!({"questions":[]}),
            json!({"questions":[{"question":"?","options":[]}]}),
            json!({"questions":[{"question":"","options":[{"label":"A"}]}]}),
            json!({"questions":[{"question":"Q","options":[{"label":"A\u{1b}[2J"}]}]}),
            json!({"questions":[{"question":"Q","options":[{"label":"x".repeat(201)}]}]}),
            json!({"questions":[{"question":"Q","multiSelect":"yes","options":[{"label":"A"}]}]}),
        ] {
            assert_eq!(ToolCard::from_input("AskUserQuestion", &bad), None, "{bad}");
        }
    }

    #[test]
    fn compaction_and_local_command_reports_come_from_the_transcript() {
        use super::super::Reader;
        let dir = crate::util::testing::TempDir::new("chat-compact");
        let path = dir.0.join("chat.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for row in [
            json!({"type":"user","message":{"content":"hello"}}),
            json!({"type":"assistant","message":{"model":"m","usage":{"input_tokens":275000},"content":"Hi","stop_reason":"end_turn"}}),
            json!({"type":"user","message":{"content":"<command-name>/effort</command-name>"}}),
            json!({"type":"user","message":{"content":"<local-command-stdout>Set effort level to \u{1b}[1mmedium\u{1b}[22m (saved as your default for new sessions)</local-command-stdout>"}}),
            json!({"type":"system","subtype":"local_command","content":"<local-command-stdout>Set model to Opus 5.5 (default)</local-command-stdout>"}),
            json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted","level":"info","compactMetadata":{"trigger":"manual","preTokens":275000,"postTokens":18000}}),
            json!({"type":"user","isCompactSummary":true,"message":{"content":"Summary"}}),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        let mut reader = Reader::open(&path, 8192).unwrap();
        let info = reader.chat();
        assert!(info.compacted);
        assert_eq!(info.compacted_size, Some(18000), "the summary keeps the boundary's size");
        assert_eq!(
            info.effort_report.map(|(_, s)| s).as_deref(),
            Some("Set effort level to medium (saved as your default for new sessions)")
        );
        assert_eq!(info.model_report.map(|(_, s)| s).as_deref(), Some("Set model to Opus 5.5 (default)"));
        assert!(reader.items().all(|(_, item)| !matches!(&item.kind, super::super::ItemKind::Prompt { text, .. } if text.contains("Set effort"))));

        writeln!(file, "{}", json!({"type":"assistant","message":{"usage":{"input_tokens":9000},"content":"Next","stop_reason":"end_turn"}})).unwrap();
        reader.poll().unwrap();
        let info = reader.chat();
        assert!(!info.compacted);
        assert_eq!(info.compacted_size, None);
        assert_eq!(info.context, Some(9000));
    }

    #[test]
    fn an_exit_plan_mode_call_carries_its_plan() {
        let input = json!({"plan": "# Plan\n\n1. Parse"});
        assert_eq!(ToolCard::from_input("ExitPlanMode", &input), Some(ToolCard::Plan("# Plan\n\n1. Parse".into())));
        assert_eq!(ToolCard::from_input("ExitPlanMode", &json!({"plan": "  "})), None);
        let long = "x".repeat(MAX_PLAN + 5);
        let Some(ToolCard::Plan(cut)) = ToolCard::from_input("ExitPlanMode", &json!({ "plan": long })) else {
            panic!()
        };
        assert_eq!(cut.chars().count(), MAX_PLAN + 1);
    }

    #[test]
    fn command_output_and_shell_mode_become_finished_rows_under_their_command() {
        use super::super::{ItemKind, Reader};
        let dir = crate::util::testing::TempDir::new("chat-local");
        let path = dir.0.join("chat.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for row in [
            json!({"type":"user","message":{"content":"<command-name>/context</command-name><command-message>context</command-message><command-args></command-args>"}}),
            json!({"type":"user","message":{"content":"<local-command-stdout>Context usage 12k / 200k</local-command-stdout>"}}),
            json!({"type":"user","message":{"content":"<bash-input>git status</bash-input>"}}),
            json!({"type":"user","message":{"content":"<bash-stdout>On branch main</bash-stdout><bash-stderr></bash-stderr>"}}),
            json!({"type":"user","message":{"content":"<command-name>/model</command-name><command-args></command-args>"}}),
            json!({"type":"user","message":{"content":"<local-command-stdout></local-command-stdout>"}}),
            json!({"type":"system","subtype":"local_command","content":"<local-command-stdout>Reconnected to sprawl.</local-command-stdout>"}),
            // Shell mode as the CLI also writes it: the command and its output as two blocks of one record.
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"text","text":"<bash-input> git log -1 --oneline</bash-input>\n"},
                {"type":"text","text":"<bash-stdout>9981c63 SDK</bash-stdout><bash-stderr></bash-stderr>"}
            ]}}),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        let reader = Reader::open(&path, 8192).unwrap();
        let rows: Vec<_> = reader
            .items()
            .map(|(_, item)| match &item.kind {
                ItemKind::Prompt { text, .. } => format!("prompt {text}"),
                ItemKind::Tool { name, summary, state, card: Some(ToolCard::Output(text)), .. } => {
                    format!("{name}|{summary}|{state:?}|{text}")
                }
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            rows,
            [
                "prompt /context",
                "/context||Ok|Context usage 12k / 200k",
                "Shell|! git status|Ok|On branch main",
                "prompt /model",
                "Command output||Ok|Reconnected to sprawl.",
                "Shell|! git log -1 --oneline|Ok|9981c63 SDK",
            ],
            "an empty output closes its command, so /model does not label the next output"
        );
        assert_eq!(reader.chat().pending, None, "finished rows never wait");
    }
}
