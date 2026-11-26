//! Cheap recognition of HTTP(S) and local file references. Never resolve or open during paint.

use std::ops::Range;
use std::path::{Path, PathBuf};

use mdshared_ui::platform;

pub fn url(s: &str) -> bool {
    !s.chars().any(char::is_control)
        && ["https://", "http://"]
            .iter()
            .any(|p| s.get(..p.len()).is_some_and(|start| start.eq_ignore_ascii_case(p)) && s.len() > p.len())
}

fn local(s: &str) -> bool {
    let b = s.as_bytes();
    if s.is_empty() || s.chars().any(char::is_control) || s.starts_with(['\\', '/']) || s.contains("://") {
        return false;
    }
    let drive = b.get(1..3).is_some_and(|cut| matches!(cut, b":\\" | b":/")) && b[0].is_ascii_alphabetic();
    if s.contains(':') {
        return drive && !s[2..].contains(':');
    }
    s.contains(['\\', '/'])
        || s.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty() && (1..=12).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

const DOCUMENT_TYPES: &[&str] = &[
    "txt", "md", "markdown", "rst", "adoc", "log", "csv", "tsv", "json", "jsonl", "yaml", "yml", "toml", "ini", "rs",
    "c", "h", "cpp", "hpp", "css", "ts", "tsx", "go", "kt", "swift", "sql", "pdf", "png", "jpg", "jpeg", "gif", "bmp",
    "webp", "ico", "tiff", "mp3", "wav", "ogg", "mp4", "webm", "docx", "xlsx", "pptx", "odt",
];

pub fn target(s: &str) -> bool {
    !s.chars().any(char::is_control) && (url(s) || local(s))
}

pub fn detect(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let tail = &text[at..];
        let first = tail.chars().next().expect("nonempty tail");
        if let Some(end) = match first {
            '`' | '"' => tail[1..].find(first).map(|i| i + 1),
            _ => None,
        } && target(&tail[1..end])
        {
            out.push((at + 1..at + end, tail[1..end].to_owned()));
            at += end + 1;
            continue;
        }
        if first.is_whitespace() || matches!(first, '(' | '[' | '<' | '"' | '\'' | '`') {
            at += first.len_utf8();
            continue;
        }
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).unwrap_or(tail.len());
        let mut word = tail[..end].trim_end_matches([',', '.', ';', '!', ']', '\'', ':']);
        if word.ends_with(')') {
            let excess = word.matches(')').count().saturating_sub(word.matches('(').count());
            let suffix = word.bytes().rev().take_while(|&b| b == b')').count();
            word = &word[..word.len() - suffix.min(excess)];
        }
        if target(word) {
            out.push((at..at + word.len(), word.to_owned()));
        }
        at += end.max(first.len_utf8());
    }
    out
}

pub fn open(s: &str, cwd: &Path, reveal: bool) {
    open_with(s, cwd, reveal, |file| {
        let _ = crate::run::reveal(file);
    });
}

fn open_with(s: &str, cwd: &Path, reveal: bool, reveal_file: impl FnOnce(&Path)) {
    if url(s) {
        platform::open_url(s);
    } else if local(s) {
        let path = Path::new(s);
        let file: PathBuf = if path.is_absolute() { path.to_owned() } else { cwd.join(path) };
        if !file.is_file() {
            return;
        }
        if reveal {
            reveal_file(&file);
        } else {
            platform::open_file(&file, DOCUMENT_TYPES);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_passes_an_existing_relative_file_to_the_app_launcher() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut revealed = None;
        open_with("Cargo.toml", cwd, true, |file| revealed = Some(file.to_owned()));
        assert_eq!(revealed, Some(cwd.join("Cargo.toml")));
        open_with("not-a-real-transcript.jsonl", cwd, true, |_| panic!("missing path must not launch"));
    }

    #[test]
    fn finds_url_and_local_files_with_source_byte_ranges() {
        let s = "Ç: (https://example.test/a_(b)), `src/lib.rs`, \"C:\\My files\\test.png\" then notes.md.";
        let links = detect(s);
        assert_eq!(
            links.iter().map(|(r, _)| &s[r.clone()]).collect::<Vec<_>>(),
            ["https://example.test/a_(b)", "src/lib.rs", "C:\\My files\\test.png", "notes.md"]
        );
        assert!(links.iter().all(|(r, target)| &s[r.clone()] == target));
    }

    #[test]
    fn refuses_network_files_special_schemes_and_control_characters() {
        for s in
            [r"\\server\share\x.md", "//server/x.md", "file:///C:/a", "javascript:alert(1)", "a.txt:stream", "a\n.rs"]
        {
            assert!(!target(s), "{s}");
        }
    }
}
