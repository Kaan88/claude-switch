//! Records the CLI writes for what the user ran locally: a slash command's output
//! (`<local-command-stdout>`) and shell mode (`<bash-input>`, then `<bash-stdout>` and
//! `<bash-stderr>`). Each becomes a finished tool row with a mono `ToolCard::Output` card, under
//! the command's own prompt bubble. Tag names are to confirm against a live CLI.
use super::{ItemKind, ToolCard, ToolState};

const STDOUT: &str = "local-command-stdout";
const STDERR: &str = "local-command-stderr";
const BASH_INPUT: &str = "bash-input";
const BASH_STDOUT: &str = "bash-stdout";
const BASH_STDERR: &str = "bash-stderr";
/// Output beyond this is cut for the conversation; the terminal keeps it whole.
const MAX_TEXT: usize = 6000;
const MAX_LINES: usize = 60;
/// The most bytes kept of an output with its escape sequences: colour codes take many bytes per
/// character (`/context` colours each cell of its grid), so this is well above [`MAX_TEXT`].
const MAX_ANSI: usize = 64 * 1024;

/// What one record holds.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Local {
    /// A command's output (stdout and stderr together), and the same with its escape sequences
    /// when it has any.
    Output { text: String, ansi: Option<String>, failed: bool },
    /// A shell-mode command as typed.
    Shell(String),
    /// A shell-mode command's output.
    ShellOutput { text: String, ansi: Option<String>, failed: bool },
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find(&close)?;
    Some(&text[start..end])
}

fn bounded(text: &str) -> String {
    let clean = super::chat::strip_escapes(text);
    let mut out: String = clean.trim_end().lines().take(MAX_LINES).collect::<Vec<_>>().join("\n");
    if out.chars().count() > MAX_TEXT {
        out = out.chars().take(MAX_TEXT).collect();
        out.push('…');
    } else if clean.trim_end().lines().count() > MAX_LINES {
        out.push_str("\n…");
    }
    out
}

/// The byte length of the escape sequence `text` starts with (it starts with ESC): a CSI to its
/// final character and an OSC to BEL or ESC \, as [`strip_escapes`](super::chat::strip_escapes)
/// reads them, one with intermediate bytes (`ESC ( 0`) to its final byte, anything else as ESC
/// and the character after it. An unfinished one runs to the end of `text`.
pub fn escape_len(text: &str) -> usize {
    let mut chars = text.char_indices().skip(1);
    match chars.next() {
        Some((_, '[')) => chars.find(|(_, c)| ('@'..='~').contains(c)).map_or(text.len(), |(i, c)| i + c.len_utf8()),
        Some((_, ']')) => {
            let mut after_esc = false;
            for (i, c) in chars {
                if c == '\u{7}' || after_esc && c == '\\' {
                    return i + c.len_utf8();
                }
                after_esc = c == '\u{1b}';
            }
            text.len()
        }
        // ESC, intermediate bytes (`(` in `ESC ( 0`), and a final one.
        Some((_, c)) if (' '..='/').contains(&c) => {
            chars.find(|(_, c)| !(' '..='/').contains(c)).map_or(text.len(), |(i, c)| i + c.len_utf8())
        }
        Some((i, c)) => i + c.len_utf8(),
        None => text.len(),
    }
}

/// The output as [`bounded`] cuts it with its escape sequences kept, or `None` when it has none:
/// the same lines, cut after as many characters, or before [`MAX_ANSI`] bytes; never inside a
/// sequence.
fn bounded_ansi(text: &str) -> Option<String> {
    if !text.contains('\u{1b}') {
        return None;
    }
    let all = text.trim_end();
    let mut out = String::new();
    let mut shown = 0;
    let mut cut = false;
    'lines: for (n, line) in all.lines().take(MAX_LINES).enumerate() {
        if n > 0 {
            out.push('\n');
            shown += 1;
        }
        let mut rest = line;
        while let Some(c) = rest.chars().next() {
            let escape = c == '\u{1b}';
            let len = if escape { escape_len(rest) } else { c.len_utf8() };
            if !escape && shown >= MAX_TEXT || out.len() + len > MAX_ANSI {
                cut = true;
                break 'lines;
            }
            shown += usize::from(!escape);
            out.push_str(&rest[..len]);
            rest = &rest[len..];
        }
    }
    if cut {
        out.push('…');
    } else if all.lines().count() > MAX_LINES {
        out.push_str("\n…");
    }
    Some(out)
}

/// Without the blank lines around it; the first line keeps its indent, which is part of the output
/// (`git status --short`).
fn blank_lines_trimmed(text: &str) -> &str {
    let text = text.trim_end();
    let first = text.len() - text.trim_start().len();
    &text[text[..first].rfind('\n').map_or(0, |i| i + 1)..]
}

/// The record's text, if it is one of these records.
pub(super) fn parse(text: &str) -> Option<Local> {
    let text = text.trim_start();
    if !text.starts_with('<') {
        return None;
    }
    if text.starts_with(&format!("<{BASH_INPUT}>")) {
        let input = tag(text, BASH_INPUT)?.trim();
        return (!input.is_empty()).then(|| Local::Shell(bounded(input)));
    }
    let both = |out: &str, err: &str| {
        let (stdout, stderr) = (tag(text, out), tag(text, err));
        (stdout.is_some() || stderr.is_some()).then(|| {
            let (stdout, stderr) =
                (blank_lines_trimmed(stdout.unwrap_or("")), blank_lines_trimmed(stderr.unwrap_or("")));
            let joined = [stdout, stderr].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
            (bounded(&joined), bounded_ansi(&joined), stdout.is_empty() && !stderr.is_empty())
        })
    };
    if text.starts_with(&format!("<{BASH_STDOUT}>")) || text.starts_with(&format!("<{BASH_STDERR}>")) {
        let (text, ansi, failed) = both(BASH_STDOUT, BASH_STDERR)?;
        return Some(Local::ShellOutput { text, ansi, failed });
    }
    if text.starts_with(&format!("<{STDOUT}>")) || text.starts_with(&format!("<{STDERR}>")) {
        let (text, ansi, failed) = both(STDOUT, STDERR)?;
        return (!text.is_empty()).then_some(Local::Output { text, ansi, failed });
    }
    None
}

/// A finished row for a local record.
pub(super) fn item(
    id: String,
    name: &str,
    summary: String,
    text: String,
    ansi: Option<String>,
    failed: bool,
) -> ItemKind {
    ItemKind::Tool {
        id,
        name: name.to_owned(),
        summary,
        file_path: None,
        state: if failed { ToolState::Failed } else { ToolState::Ok },
        duration_ms: None,
        pictures: Vec::new(),
        subagent: None,
        denial: None,
        card: Some(ToolCard::Output { text, ansi }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_records_are_read_by_their_tags() {
        assert_eq!(
            parse("<local-command-stdout>Set effort level to \u{1b}[1mhigh\u{1b}[22m</local-command-stdout>"),
            Some(Local::Output {
                text: "Set effort level to high".into(),
                ansi: Some("Set effort level to \u{1b}[1mhigh\u{1b}[22m".into()),
                failed: false
            })
        );
        assert_eq!(parse("<local-command-stdout></local-command-stdout>"), None, "nothing to show");
        assert_eq!(parse("<bash-input>git status</bash-input>"), Some(Local::Shell("git status".into())));
        assert_eq!(
            parse("<bash-stdout></bash-stdout><bash-stderr>fatal: not a repository</bash-stderr>"),
            Some(Local::ShellOutput { text: "fatal: not a repository".into(), ansi: None, failed: true })
        );
        assert_eq!(
            parse("<bash-stdout>\n  \n M docs/TODO.md\n?? notes.txt\n</bash-stdout><bash-stderr></bash-stderr>"),
            Some(Local::ShellOutput { text: " M docs/TODO.md\n?? notes.txt".into(), ansi: None, failed: false }),
            "blank lines go, the first line keeps its indent"
        );
        assert_eq!(
            parse("<bash-stdout>  \n</bash-stdout><bash-stderr></bash-stderr>"),
            Some(Local::ShellOutput { text: String::new(), ansi: None, failed: false }),
            "a command that printed nothing"
        );
        assert_eq!(parse("Plain prompt mentioning <bash-input>"), None);
        assert_eq!(parse("<command-name>/effort</command-name>"), None);
        let long = format!("<local-command-stdout>{}</local-command-stdout>", "row\n".repeat(100));
        let Some(Local::Output { text, .. }) = parse(&long) else { panic!() };
        assert_eq!(text.lines().count(), MAX_LINES + 1);
    }

    #[test]
    fn coloured_output_keeps_its_escapes_cut_where_the_plain_text_is() {
        let red = "\u{1b}[31m";
        let inner = format!("{red}a\u{1b}[0m\n\u{1b}]8;;x\u{7}b\u{1b}]8;;\u{7}");
        let record = format!("<local-command-stdout>{inner}</local-command-stdout>");
        let Some(Local::Output { text, ansi, .. }) = parse(&record) else { panic!() };
        assert_eq!(text, "a\nb");
        assert_eq!(ansi.as_deref(), Some(inner.as_str()));
        // As many lines as the plain text, then the same mark.
        let long = format!("<local-command-stdout>{}</local-command-stdout>", format!("{red}row\n").repeat(100));
        let Some(Local::Output { text, ansi: Some(ansi), .. }) = parse(&long) else { panic!() };
        assert_eq!(ansi.lines().count(), text.lines().count());
        assert!(ansi.ends_with("\n…"));
        // Many characters: cut after as many as the plain text keeps.
        let wide = format!("<local-command-stdout>{red}{}</local-command-stdout>", "x".repeat(MAX_TEXT * 2));
        let Some(Local::Output { text, ansi: Some(ansi), .. }) = parse(&wide) else { panic!() };
        assert_eq!(super::super::chat::strip_escapes(&ansi), text);
        // Escape codes that are most of the bytes: cut at the byte bound, never inside a sequence.
        let dense = format!(
            "<local-command-stdout>{}</local-command-stdout>",
            "\u{1b}[38;2;1;2;3m\u{1b}[48;2;4;5;6m⛁".repeat(5000)
        );
        let Some(Local::Output { ansi: Some(ansi), .. }) = parse(&dense) else { panic!() };
        assert!(ansi.len() <= MAX_ANSI + '…'.len_utf8());
        assert!(super::super::chat::strip_escapes(&ansi).ends_with("⛁…"), "a sequence cut in two would take the mark");
    }

    #[test]
    fn an_escape_sequence_is_measured_to_its_end() {
        assert_eq!(escape_len("\u{1b}[1;31mx"), 7);
        assert_eq!(escape_len("\u{1b}]8;;http://a\u{1b}\\x"), 15);
        assert_eq!(escape_len("\u{1b}]0;t\u{7}x"), 6);
        assert_eq!(escape_len("\u{1b}(0x"), 3);
        assert_eq!(escape_len("\u{1b}7x"), 2);
        assert_eq!(escape_len("\u{1b}[12"), 4, "unfinished: to the end");
        assert_eq!(escape_len("\u{1b}"), 1);
    }
}
