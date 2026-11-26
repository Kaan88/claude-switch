//! Transcript-named deliverables. Result text is never interpreted as code or a local file path.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use claude_switch_core::transcript::{Item, ItemId, ItemKind, ResultDetails, ToolState, timestamp_ms};

use super::links::{self, Files, NamedFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WebKind {
    Artifact,
    PullRequest,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Destination {
    Web(String, WebKind),
    File(NamedFile),
    Picture(ItemId, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Output {
    pub title: String,
    pub time: String,
    pub to: Destination,
    pub has_markdown_chip: bool,
    at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Identity {
    Url(String),
    Path(PathBuf),
    Picture(ItemId, usize),
}

impl Output {
    pub fn key(&self) -> Identity {
        match &self.to {
            Destination::Web(url, _) => Identity::Url(url.clone()),
            Destination::File(file) => Identity::Path(if cfg!(windows) {
                PathBuf::from(file.path().to_string_lossy().to_ascii_lowercase())
            } else {
                file.path().into()
            }),
            Destination::Picture(id, i) => Identity::Picture(*id, *i),
        }
    }

    pub fn kind(&self) -> &'static str {
        match &self.to {
            Destination::Web(_, WebKind::Artifact) => "Artifact",
            Destination::Web(_, WebKind::PullRequest) => "Pull request",
            Destination::Web(_, WebKind::Link) => "Link",
            Destination::Picture(..) => "Picture",
            Destination::File(file) if file.picture() => "Picture",
            Destination::File(_) => "File",
        }
    }
}

pub(super) fn collect(id: ItemId, item: &Item, result: Option<&ResultDetails>, files: Option<&Files>) -> Vec<Output> {
    let from_result = result.and_then(|r| r.timestamp.as_deref());
    let time = from_result.or(item.timestamp.as_deref()).and_then(|s| s.get(11..19)).unwrap_or("");
    let at = from_result.or(item.timestamp.as_deref()).and_then(timestamp_ms);
    let mut out = match &item.kind {
        ItemKind::Assistant { markdown } => urls(markdown, time),
        ItemKind::Tool { .. } => result.map_or_else(Vec::new, |r| urls(&r.text, time)),
        _ => Vec::new(),
    };
    if let ItemKind::Tool { name, file_path, state, pictures, .. } = &item.kind {
        if *state == ToolState::Ok
            && let Some(files) = files
        {
            let declared = file_path.as_deref().filter(|_| matches!(name.as_str(), "Write" | "Edit" | "NotebookEdit"));
            let reported = result.into_iter().flat_map(ResultDetails::saved_paths);
            let mut seen = BTreeSet::new();
            for path in declared.into_iter().chain(reported) {
                let Some(file) = files.get(path).filter(|file| seen.insert(file.path())) else { continue };
                let title = file.path().file_name().and_then(|n| n.to_str()).unwrap_or("Named file");
                let has_markdown_chip =
                    file.markdown() && matches!(name.as_str(), "Write" | "Edit") && Some(path) == declared;
                out.push(Output {
                    title: short(title, 150),
                    time: time.into(),
                    to: Destination::File(file.clone()),
                    at,
                    has_markdown_chip,
                });
            }
        }
        for i in 0..pictures.len().min(24) {
            out.push(Output {
                title: format!("Tool result picture {}", i + 1),
                time: time.into(),
                to: Destination::Picture(id, i),
                has_markdown_chip: false,
                at,
            });
        }
    }
    for output in &mut out {
        output.at = at;
    }
    out
}

pub(super) fn latest(items: &BTreeMap<ItemId, Vec<Output>>) -> Vec<Output> {
    let mut seen = BTreeSet::new();
    let mut sorted: Vec<_> = items.values().rev().flatten().collect();
    sorted.sort_by_key(|o| std::cmp::Reverse(o.at));
    sorted.into_iter().filter(|o| seen.insert(o.key())).cloned().collect()
}

fn urls(raw: &str, time: &str) -> Vec<Output> {
    let raw = &raw[..raw.floor_char_boundary(raw.len().min(128 * 1024))];
    if ![b"https://".as_slice(), b"http://"]
        .into_iter()
        .any(|scheme| raw.as_bytes().windows(scheme.len()).any(|s| s.eq_ignore_ascii_case(scheme)))
    {
        return Vec::new();
    }
    let doc = mdview_core::parse(raw);
    let explicit = doc.spans.iter().filter_map(|span| {
        let mdview_core::model::SpanKind::Link(index) = span.kind else { return None };
        let link = doc.links.get(index as usize).filter(|l| !l.wiki && !l.missing)?;
        let label = doc.text.get(span.start as usize..span.end as usize).unwrap_or("");
        Some((doc.str(link.dest).to_owned(), short(label, 100)))
    });
    let plain =
        links::detect(raw).into_iter().filter(|(_, word)| links::url(word)).map(|(_, word)| (word, String::new()));
    let mut seen = BTreeSet::new();
    explicit
        .chain(plain)
        .filter_map(|(url, label)| {
            let web = links::web_url(&url)?;
            let base = web.suffix.split(['?', '#']).next().unwrap_or("");
            let host = web.host;
            let segments = base.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>();
            let kind = if host.eq_ignore_ascii_case("claude.ai") && segments.first() == Some(&"artifacts") {
                WebKind::Artifact
            } else if (host.eq_ignore_ascii_case("github.com") && pr(&segments, "pull"))
                || (host.eq_ignore_ascii_case("bitbucket.org") && pr(&segments, "pull-requests"))
                || segments.windows(3).any(|parts| parts[0] == "-" && parts[1] == "merge_requests" && number(parts[2]))
                || (host.eq_ignore_ascii_case("dev.azure.com") && pr(&segments, "pullrequest"))
            {
                WebKind::PullRequest
            } else {
                WebKind::Link
            };
            let title = if !label.is_empty() && !links::url(&label) {
                label
            } else {
                short(&format!("{}{base}", web.authority), 100)
            };
            // Case-fold the scheme and host, preserving case-sensitive path/query/fragment.
            let normalized = format!("{}{}{}", web.scheme, web.authority.to_ascii_lowercase(), web.suffix);
            seen.insert(normalized.clone()).then(|| Output {
                title,
                time: time.into(),
                to: Destination::Web(normalized, kind),
                has_markdown_chip: false,
                at: None,
            })
        })
        .take(40)
        .collect()
}

fn pr(parts: &[&str], marker: &str) -> bool {
    parts.windows(2).any(|parts| parts[0] == marker && number(parts[1]))
}

fn number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn short(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_pull_request_and_http_or_https_links_are_classified_and_deduplicated() {
        let text = "[Preview](https://claude.ai/artifacts/abc) https://CLAUDE.ai/artifacts/abc\n\
            https://github.com/org/repo/pull/42 https://example.test/p?q=1#x HTTP://EXAMPLE.test/p?q=1#x \
            https://claude.ai.evil.test/artifacts/abc https://user@evil.test/ http://a:b@evil.test/";
        let links = urls(text, "11:22:33");
        assert_eq!(
            links.iter().map(Output::kind).collect::<Vec<_>>(),
            ["Artifact", "Pull request", "Link", "Link", "Link"]
        );
        assert_eq!(links[0].title, "Preview");
        assert_eq!(links[0].time, "11:22:33");
        assert!(matches!(&links[2].to, Destination::Web(url, _) if url.ends_with("?q=1#x")));
        assert!(matches!(&links[3].to, Destination::Web(url, _) if url == "http://example.test/p?q=1#x"));
    }

    #[test]
    fn latest_uses_result_time_and_only_keeps_the_latest_copy_of_a_destination() {
        let id = |offset| ItemId { offset, block: 0 };
        let item = |at: &str, markdown: &str| Item {
            timestamp: Some(format!("2026-11-26T{at}Z")),
            kind: ItemKind::Assistant { markdown: markdown.into() },
        };
        let older = item("12:00:05", "[Old title](https://example.test/output)");
        let other = item("12:00:07", "https://example.test/other");
        let latest = item("12:01:20", "[New title](https://example.test/output)");
        let map = BTreeMap::from([
            (id(0), collect(id(0), &latest, None, None)),
            (id(1), collect(id(1), &other, None, None)),
            (id(2), collect(id(2), &older, None, None)),
        ]);
        let rows = super::latest(&map);
        assert_eq!(rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(), ["New title", "example.test/other"]);

        let bash = Item {
            timestamp: Some("2026-11-26T11:55:00Z".into()),
            kind: ItemKind::Tool {
                id: "bash".into(),
                name: "Bash".into(),
                summary: String::new(),
                file_path: None,
                state: ToolState::Ok,
                duration_ms: None,
                pictures: Vec::new(),
                subagent: None,
                card: None,
            },
        };
        let result = ResultDetails {
            timestamp: Some("2026-11-26T12:05:00Z".into()),
            text: "https://example.test/result".into(),
        };
        assert_eq!(collect(id(3), &bash, Some(&result), None)[0].time, "12:05:00");
    }
}
