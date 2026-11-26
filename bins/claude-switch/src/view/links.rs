//! Transcript references are resolved by the reader worker. Paint never probes a path.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use claude_switch_core::grouping::open_local_file;
use mdshared_ui::platform;

mod shell_logs;
pub(super) use shell_logs::ShellLogs;

pub type Files = BTreeMap<String, NamedFile>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedFile {
    path: PathBuf,
    view_only: bool,
    /// For a bare name that also named a file the transcript wrote elsewhere: the other
    /// candidate, which the viewer offers beside this one.
    other: Option<PathBuf>,
}

pub const MAX_MARKDOWN: u64 = 1024 * 1024;

impl NamedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The other file a bare name could mean, when there were two.
    pub fn other(&self) -> Option<NamedFile> {
        let path = self.other.clone()?;
        Some(NamedFile { other: Some(self.path.clone()), path, view_only: self.view_only })
    }

    pub fn markdown(&self) -> bool {
        is_markdown(&self.path)
    }

    pub fn picture(&self) -> bool {
        self.path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff"].iter().any(|ext| s.eq_ignore_ascii_case(ext))
        })
    }

    pub fn supports_default_app(&self) -> bool {
        !self.view_only
            && self
                .path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|extension| DOCUMENT_TYPES.iter().any(|allowed| extension.eq_ignore_ascii_case(allowed)))
    }

    pub fn available(&self) -> bool {
        open_local_file(&self.path, false).is_some()
    }

    pub fn read_markdown(&self) -> std::io::Result<String> {
        use std::io::{Error, ErrorKind};
        if !self.markdown() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let file = open_local_file(&self.path, true)
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
        let Some(path) = self.external_path() else { return false };
        let Some(_checked) = open_local_file(path, false) else { return false };
        if reveal { crate::run::reveal(path).is_ok() } else { platform::open_file(path, DOCUMENT_TYPES) }
    }

    fn external_path(&self) -> Option<&Path> {
        (!self.view_only).then_some(self.path())
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

#[cfg(test)]
pub fn resolve(s: &str, cwd: &Path, touched: &BTreeSet<PathBuf>) -> Option<NamedFile> {
    resolve_recent(s, cwd, touched, &[])
}

/// As [`resolve`], with the transcript's own outside writes newest first: a bare file name
/// (no folder) that is also the name of one of them means that file, the newest, and the
/// session folder's file of the same name, if there is one, is kept as the other candidate. A
/// relative path with a folder in it still means the session folder's file only.
pub fn resolve_recent(s: &str, cwd: &Path, touched: &BTreeSet<PathBuf>, recent: &[PathBuf]) -> Option<NamedFile> {
    let normalized = transcript_path(s, cwd)?;
    if !inside(&normalized, cwd) && !touched.contains(&normalized) {
        return None;
    }
    let here = open_local_file(&normalized, false).map(|_| normalized.clone());
    let bare = !s.contains(['/', '\\']) && !Path::new(s).is_absolute();
    let elsewhere = bare
        .then(|| {
            recent.iter().find(|p| {
                **p != normalized
                    && touched.contains(*p)
                    && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| same_name(n, s))
                    && open_local_file(p, false).is_some()
            })
        })
        .flatten();
    match (elsewhere, here) {
        (Some(path), other) => Some(NamedFile { path: path.clone(), view_only: false, other }),
        (None, Some(path)) => Some(NamedFile { path, view_only: false, other: None }),
        (None, None) => None,
    }
}

/// File names compare as Windows compares them: without case.
fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.to_lowercase() == b.to_lowercase() } else { a == b }
}

pub(super) fn inside(path: &Path, cwd: &Path) -> bool {
    absolute(cwd).is_some_and(|root| path.starts_with(root))
}

/// Lexical identity only. A caller must never treat this as evidence that a file can be opened.
pub fn transcript_path(s: &str, cwd: &Path) -> Option<PathBuf> {
    if !local(s) || s.len() > 4096 {
        return None;
    }
    let path = Path::new(s);
    if path.is_absolute() {
        return absolute(path);
    }
    // Never drop a parent hop: `alias/../x` can resolve differently through a junction.
    let mut file = absolute(cwd)?;
    for part in path.components() {
        match part {
            Component::Normal(name) => file.push(name),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(file)
}

fn absolute(path: &Path) -> Option<PathBuf> {
    (path.is_absolute() && !path.components().any(|part| part == Component::ParentDir))
        .then(|| path.components().collect())
}

pub fn url(s: &str) -> bool {
    web_url(s).is_some()
}

pub(super) struct WebUrl<'a> {
    pub scheme: &'static str,
    pub authority: &'a str,
    pub host: &'a str,
    pub suffix: &'a str,
}

/// Both discovery and activation use this check. Do not let the browser reinterpret
/// credential-bearing, backslash-delimited, or percent-encoded authorities.
pub(super) fn web_url(s: &str) -> Option<WebUrl<'_>> {
    let (scheme, rest) = ["https://", "http://"].into_iter().find_map(|scheme| {
        let (prefix, rest) = s.split_at_checked(scheme.len())?;
        prefix.eq_ignore_ascii_case(scheme).then_some((scheme, rest))
    })?;
    if s.len() > 4096 || s.chars().any(|c| c.is_whitespace() || c.is_control() || "\\<>\"`".contains(c)) {
        return None;
    }
    let (authority, suffix) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let (host, port) = if let Some(ip) = authority.strip_prefix('[') {
        let (ip, extra) = ip.split_once(']')?;
        ip.parse::<std::net::Ipv6Addr>().ok()?;
        (ip, if extra.is_empty() { None } else { Some(extra.strip_prefix(':')?) })
    } else {
        let (host, port) = authority.split_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.len() > 253
            || !host.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return None;
        }
        (host, port)
    };
    if port.is_some_and(|p| !p.bytes().all(|b| b.is_ascii_digit()) || p.parse::<u16>().is_err()) {
        return None;
    }
    Some(WebUrl { scheme, authority, host, suffix })
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
        if first.is_whitespace() || matches!(first, '(' | '[' | '{' | '<' | '"' | '\'' | '`') {
            at += first.len_utf8();
            continue;
        }
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).unwrap_or(tail.len());
        let mut word = tail[..end].trim_end_matches([',', '.', ';', '!', '}', '\'', ':']);
        if let Some(trimmed) = word.strip_suffix(']')
            && !web_url(word).is_some_and(|web| web.suffix.is_empty() && web.authority.starts_with('['))
        {
            word = trimmed;
        }
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
    fn web_links_allow_both_schemes_with_only_valid_hosts_and_no_userinfo() {
        for ok in ["http://localhost:8080/", "HTTPS://example.test/a?user=x@y", "https://[::1]:443/"] {
            assert!(url(ok), "{ok}");
        }
        for bad in [
            "http://user:pass@example.test/",
            "https://user@example.test/",
            "http://%75ser%40example.test/",
            "https://example.test\\other.test",
            "http:///example.test",
            "http://:80",
            "https://example..test",
            "http://example.test:bad",
            "https://example.test:65536",
            "http://host:80:90",
            "https://[::1]@host/",
            "ftp://example.test/",
        ] {
            assert!(!url(bad), "{bad}");
            assert!(existing(bad, &Files::new()).is_empty(), "highlighted {bad}");
        }
        assert!(!url("https://example.test/a b"));
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

    #[test]
    fn checks_named_regular_files_and_does_not_highlight_command_words() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let resolve = |name| super::resolve(name, cwd, &BTreeSet::new());
        let file = resolve("field-notes.md").unwrap();
        assert_eq!(file.path(), cwd.join("field-notes.md"));
        assert_eq!(resolve(file.path().to_str().unwrap()), Some(file.clone()));
        assert!(resolve("../field-notes.md").is_none());
        assert!(resolve("not-created.md").is_none());
        std::fs::create_dir(cwd.join("directory.md")).unwrap();
        assert!(resolve("directory.md").is_none());
        let files = Files::from([("field-notes.md".into(), file)]);
        let raw = "echo enter/leave then field-notes.md";
        let hits = existing(raw, &files);
        assert_eq!(hits.iter().map(|(r, _)| &raw[r.clone()]).collect::<Vec<_>>(), ["field-notes.md"]);
    }

    #[test]
    fn editor_preview_is_bounded_and_reads_only_the_named_file() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let file = resolve("field-notes.md", cwd, &BTreeSet::new()).unwrap();
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

    #[test]
    fn plain_paths_stay_inside_the_working_folder_and_never_walk_parent_hops() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd().join("project");
        let sibling = demo.cwd().join("project-other");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let inside = cwd.join("notes.md");
        let outside = sibling.join("notes.md");
        for file in [&inside, &outside] {
            std::fs::write(file, "# Fixture").unwrap();
        }
        let resolve = |name: &str| resolve(name, &cwd, &BTreeSet::new());
        assert!(resolve("notes.md").is_some());
        assert!(resolve(inside.to_str().unwrap()).is_some());
        assert!(resolve(outside.to_str().unwrap()).is_none());
        assert!(resolve("../project-other/notes.md").is_none());
        assert!(resolve(cwd.join("../project-other/notes.md").to_str().unwrap()).is_none());
        assert!(resolve("./notes.md").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn even_a_touched_path_is_refused_through_a_junction() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd().join("project");
        let target = demo.cwd().join("outside");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("note.md"), "# Outside").unwrap();
        let junction = cwd.join("alias");
        let shell =
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let status = std::process::Command::new(shell)
            .args([
                "-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:CS_JUNCTION -Target $env:CS_TARGET | Out-Null",
            ])
            .env("CS_JUNCTION", &junction)
            .env("CS_TARGET", &target)
            .output()
            .unwrap();
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        let pointed = junction.join("note.md");
        assert!(pointed.is_file(), "the fixture really leads to a file outside the session");
        let ordinary = resolve("alias/note.md", &cwd, &BTreeSet::new());
        let touched = resolve(pointed.to_str().unwrap(), &cwd, &BTreeSet::from([pointed.clone()]));
        std::fs::remove_dir(junction).unwrap();
        assert!(ordinary.is_none());
        assert!(touched.is_none());
    }
}
