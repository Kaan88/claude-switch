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
    /// The newest "Set effort level to …" a local command printed, whoever typed it, with its
    /// offset.
    pub effort_report: Option<(u64, String)>,
    /// The newest "Set model to …" a local command printed, with its offset.
    pub model_report: Option<(u64, String)>,
    usage_at: Option<u64>,
    compacted_at: Option<u64>,
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
        }
    }

    /// A compaction record: the last usage no longer tells the conversation's size.
    pub(super) fn compaction(&mut self, at: u64) {
        if self.compacted_at.is_none_or(|old| at > old) {
            self.compacted_at = Some(at);
            self.compacted = self.usage_at.is_none_or(|usage| at > usage);
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
fn strip_escapes(text: &str) -> String {
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
    },
    Shell {
        command: String,
        result: Vec<String>,
    },
    /// The questions of an `AskUserQuestion` call, exactly as Claude asked them.
    Questions(Vec<Question>),
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
                (path.len() <= 4096).then(|| Self::Edit { path: path.to_owned(), added, removed, lines })
            }
            "Bash" | "PowerShell" => {
                Some(Self::Shell { command: short(input["command"].as_str()?, 500), result: Vec::new() })
            }
            "AskUserQuestion" => {
                let questions = input["questions"].as_array()?;
                (!questions.is_empty() && questions.len() <= MAX_QUESTIONS).then_some(())?;
                Some(Self::Questions(questions.iter().map(Question::parse).collect::<Option<_>>()?))
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
                lines: ["− old", "+ new", "+ more"].map(str::to_owned).into()
            })
        );
        let input = json!({"file_path":"a","content":"hello\n".repeat(100)});
        assert!(matches!(ToolCard::from_input("Write", &input),
            Some(ToolCard::Edit { added: 100, removed: 0, lines, .. }) if lines.len() == 10));
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
            json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted","compactMetadata":{"preTokens":275000}}),
        ] {
            writeln!(file, "{row}").unwrap();
        }
        let mut reader = Reader::open(&path, 8192).unwrap();
        let info = reader.chat();
        assert!(info.compacted);
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
        assert_eq!(info.context, Some(9000));
    }
}
