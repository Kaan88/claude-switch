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

/// What one record holds.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Local {
    /// A command's output (stdout and stderr together).
    Output { text: String, failed: bool },
    /// A shell-mode command as typed.
    Shell(String),
    /// A shell-mode command's output.
    ShellOutput { text: String, failed: bool },
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
            let (stdout, stderr) = (stdout.unwrap_or("").trim(), stderr.unwrap_or("").trim());
            let joined = [stdout, stderr].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
            (bounded(&joined), stdout.is_empty() && !stderr.is_empty())
        })
    };
    if text.starts_with(&format!("<{BASH_STDOUT}>")) || text.starts_with(&format!("<{BASH_STDERR}>")) {
        let (text, failed) = both(BASH_STDOUT, BASH_STDERR)?;
        return Some(Local::ShellOutput { text, failed });
    }
    if text.starts_with(&format!("<{STDOUT}>")) || text.starts_with(&format!("<{STDERR}>")) {
        let (text, failed) = both(STDOUT, STDERR)?;
        return (!text.is_empty()).then_some(Local::Output { text, failed });
    }
    None
}

/// A finished row for a local record.
pub(super) fn item(id: String, name: &str, summary: String, text: String, failed: bool) -> ItemKind {
    ItemKind::Tool {
        id,
        name: name.to_owned(),
        summary,
        file_path: None,
        state: if failed { ToolState::Failed } else { ToolState::Ok },
        duration_ms: None,
        pictures: Vec::new(),
        subagent: None,
        card: Some(ToolCard::Output(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_records_are_read_by_their_tags() {
        assert_eq!(
            parse("<local-command-stdout>Set effort level to \u{1b}[1mhigh\u{1b}[22m</local-command-stdout>"),
            Some(Local::Output { text: "Set effort level to high".into(), failed: false })
        );
        assert_eq!(parse("<local-command-stdout></local-command-stdout>"), None, "nothing to show");
        assert_eq!(parse("<bash-input>git status</bash-input>"), Some(Local::Shell("git status".into())));
        assert_eq!(
            parse("<bash-stdout></bash-stdout><bash-stderr>fatal: not a repository</bash-stderr>"),
            Some(Local::ShellOutput { text: "fatal: not a repository".into(), failed: true })
        );
        assert_eq!(parse("Plain prompt mentioning <bash-input>"), None);
        assert_eq!(parse("<command-name>/effort</command-name>"), None);
        let long = format!("<local-command-stdout>{}</local-command-stdout>", "row\n".repeat(100));
        let Some(Local::Output { text, .. }) = parse(&long) else { panic!() };
        assert_eq!(text.lines().count(), MAX_LINES + 1);
    }
}
