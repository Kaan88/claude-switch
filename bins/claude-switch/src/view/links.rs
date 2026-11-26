//! Transcript references are resolved by the reader worker. Paint never probes a path.

use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use claude_switch_core::grouping::open_local_file;
use mdshared_ui::platform;

pub type Files = BTreeMap<String, NamedFile>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedFile(PathBuf);

pub const MAX_MARKDOWN: u64 = 1024 * 1024;

impl NamedFile {
    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn markdown(&self) -> bool {
        is_markdown(&self.0)
    }

    pub fn read_markdown(&self) -> std::io::Result<String> {
        use std::io::{Error, ErrorKind};
        if !self.markdown() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let file = open_local_file(&self.0, true)
            .ok_or_else(|| Error::other("The named file is no longer available as a local file."))?;
        let mut bytes = Vec::new();
        file.take(MAX_MARKDOWN + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MARKDOWN {
            return Err(Error::other("This file exceeds the 1 MiB preview limit. Use Open in editor."));
        }
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Error::other("This file is not UTF-8. Use Open in editor."))
    }

    pub fn open(&self, reveal: bool) -> bool {
        let Some(_checked) = open_local_file(&self.0, false) else { return false };
        if reveal { crate::run::reveal(&self.0).is_ok() } else { platform::open_file(&self.0, DOCUMENT_TYPES) }
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("md") || s.eq_ignore_ascii_case("markdown"))
}

/// Decode only explicit Markdown-link destinations, never a literal filename in prose or a tool.
pub fn markdown_destination(s: &str) -> Option<String> {
    let path = s.split('#').next()?;
    let mut bytes = Vec::with_capacity(path.len());
    let mut input = path.bytes();
    let digit = |b: u8| char::from(b).to_digit(16).map(|n| n as u8);
    while let Some(b) = input.next() {
        bytes.push(if b == b'%' { digit(input.next()?)? * 16 + digit(input.next()?)? } else { b });
    }
    let path = String::from_utf8(bytes).ok()?;
    (local(&path) && is_markdown(Path::new(&path))).then_some(path)
}

pub fn resolve(s: &str, cwd: &Path) -> Option<NamedFile> {
    if !local(s) || s.len() > 4096 {
        return None;
    }
    let path = Path::new(s);
    let file = if path.is_absolute() {
        path.to_owned()
    } else {
        if !cwd.is_absolute() {
            return None;
        }
        let mut depth = 0;
        for component in path.components() {
            match component {
                Component::Normal(_) => depth += 1,
                Component::ParentDir if depth > 0 => depth -= 1,
                Component::CurDir => {}
                _ => return None,
            }
        }
        cwd.join(path)
    };
    // Keep the exact normalized name that is checked; the shell must not reinterpret a
    // discarded `..` component through a different filesystem path.
    let mut normalized = PathBuf::new();
    for component in file.components() {
        if component == Component::ParentDir {
            if !normalized.pop() {
                return None;
            }
        } else {
            normalized.push(component.as_os_str());
        }
    }
    open_local_file(&normalized, false)?;
    Some(NamedFile(normalized))
}

pub fn url(s: &str) -> bool {
    !s.chars().any(char::is_control)
        && ["https://", "http://"]
            .iter()
            .any(|p| s.get(..p.len()).is_some_and(|start| start.eq_ignore_ascii_case(p)) && s.len() > p.len())
}

fn local(s: &str) -> bool {
    let b = s.as_bytes();
    if s.is_empty()
        || s.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
        || s.starts_with('\\')
        || s.starts_with("//")
        || cfg!(windows) && s.starts_with('/')
        || s.contains("://")
    {
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
            '`' | '"' | '\'' => tail[1..].find(first).map(|i| i + 1),
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

pub fn existing(text: &str, files: &Files) -> Vec<(Range<usize>, String)> {
    if files.contains_key(text) {
        return vec![(0..text.len(), text.to_owned())];
    }
    detect(text).into_iter().filter(|(_, name)| url(name) || files.contains_key(name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn checks_named_regular_files_and_does_not_highlight_command_words() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let file = resolve("field-notes.md", cwd).unwrap();
        assert_eq!(file.path(), cwd.join("field-notes.md"));
        assert_eq!(resolve(file.path().to_str().unwrap(), cwd), Some(file.clone()));
        assert!(resolve("../field-notes.md", cwd).is_none());
        assert!(resolve("not-created.md", cwd).is_none());
        std::fs::create_dir(cwd.join("directory.md")).unwrap();
        assert!(resolve("directory.md", cwd).is_none());
        let files = Files::from([("field-notes.md".into(), file)]);
        let raw = "echo enter/leave then field-notes.md";
        let hits = existing(raw, &files);
        assert_eq!(hits.iter().map(|(r, _)| &raw[r.clone()]).collect::<Vec<_>>(), ["field-notes.md"]);
    }

    #[test]
    fn editor_preview_is_bounded_and_reads_only_the_named_file() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let file = resolve("field-notes.md", cwd).unwrap();
        assert!(file.read_markdown().unwrap().starts_with("# North ridge"));
        std::fs::write(file.path(), vec![b'x'; MAX_MARKDOWN as usize + 1]).unwrap();
        assert!(file.read_markdown().unwrap_err().to_string().contains("limit"));
        std::fs::remove_file(file.path()).unwrap();
        assert!(file.read_markdown().is_err());
    }

    #[test]
    fn markdown_destinations_decode_spaces_but_reject_urls_and_device_names() {
        assert_eq!(markdown_destination("docs/Read%20me.MD#Build"), Some("docs/Read me.MD".into()));
        for s in ["https://example.test/Read.md", "%5c%5cfoo/bar.md", "a%00.md", "foo.txt", "%oops.md"] {
            assert!(markdown_destination(s).is_none(), "{s}");
        }
    }
}
