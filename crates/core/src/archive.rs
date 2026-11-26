//! Local archive rules and offline Markdown export. No operation here changes a transcript.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::{Refusal, sessions::folder_key};

pub const WEEKS: [u64; 4] = [2, 4, 8, 12];
const WEEK_MS: u64 = 7 * 86_400_000;

pub fn older_than(now: u64, weeks: u64, last_active: u64) -> bool {
    last_active != 0 && last_active < now.saturating_sub(weeks.saturating_mul(WEEK_MS))
}

pub fn eligible(pinned: bool, running: bool, archived: bool) -> bool {
    !pinned && !running && !archived
}

/// Local archives override hiding only for explicit archive views and active search.
pub fn visible(archived: bool, archive_view: bool, searching: bool, normally_visible: bool) -> bool {
    if archive_view {
        archived
    } else if archived {
        searching
    } else {
        normally_visible
    }
}

pub struct Row {
    pub name: String,
    pub cwd: PathBuf,
    pub transcript: PathBuf,
    pub first: Option<u64>,
    pub last: u64,
    pub bytes: u64,
    pub first_prompt: String,
    pub last_prompt: String,
    pub handover: Option<String>,
    pub stats: Option<crate::stats::Stats>,
}

/// Render a snapshot with no I/O or clock reads. `when` is the caller's time display convention.
pub fn markdown(rows: &[Row], date: &str, when: impl Fn(u64) -> String) -> String {
    let mut groups = BTreeMap::<String, Vec<&Row>>::new();
    let mut locations = BTreeSet::new();
    for row in rows {
        groups.entry(folder_key(&row.cwd)).or_default().push(row);
        if let Some(parent) = row.transcript.parent().filter(|p| !p.as_os_str().is_empty()) {
            locations.insert(parent);
        }
    }
    let first = rows.iter().filter_map(|r| r.first).filter(|&n| n != 0).min();
    let last = rows.iter().map(|r| r.last).filter(|&n| n != 0).max();
    let time = |at: Option<u64>| at.filter(|&a| a != 0).map_or_else(|| "—".into(), &when);
    let bytes = rows.iter().map(|r| u128::from(r.bytes)).sum::<u128>();
    let mut out = format!(
        "# Archive summary — {date}\n\n**Sessions:** {} · **Folders:** {} · **Size on disk:** {bytes} bytes\n\n\
         **Time span:** {} → {}\n",
        rows.len(),
        groups.len(),
        time(first),
        time(last),
    );
    for (_, mut group) in groups {
        group.sort_by_key(|r| (std::cmp::Reverse(r.last), &r.transcript));
        let label = group[0].cwd.to_string_lossy();
        let label = if label.is_empty() { "(No folder recorded)" } else { &label };
        let _ = writeln!(out, "\n## {}\n", literal(label, usize::MAX));
        out.push_str(
            "| Name | First active | Last active | Size (bytes) | First prompt | Last prompt | Handover | Turns | Tokens (context / output) |\n\
             | --- | --- | --- | ---: | --- | --- | --- | ---: | --- |\n",
        );
        for r in group {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                literal(&r.name, usize::MAX),
                time(r.first),
                time(Some(r.last)),
                r.bytes,
                literal(&r.first_prompt, 160),
                literal(&r.last_prompt, 160),
                literal(r.handover.as_deref().unwrap_or(""), usize::MAX),
                r.stats.map_or_else(|| "—".into(), |s| s.turns.to_string()),
                r.stats.map_or_else(
                    || "—".into(),
                    |s| {
                        let show = |t: crate::stats::Tokens| format!("{}{}", if t.about { "~" } else { "" }, t.count);
                        format!("{} / {}", show(s.context), show(s.output))
                    },
                ),
            );
        }
    }
    out.push_str("\nNothing was deleted or moved. ");
    if locations.is_empty() {
        out.push_str("No transcript locations are recorded.\n");
    } else {
        out.push_str("Transcripts remain in their original directories:\n\n");
        for dir in locations {
            let _ = writeln!(out, "- {}", literal(&dir.to_string_lossy(), usize::MAX));
        }
    }
    out
}

/// Escape syntax and line breaks before putting untrusted text in a Markdown cell or heading.
fn literal(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    for _ in 0..limit {
        match chars.next() {
            Some('\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('↵');
            }
            Some('\n' | '\u{2028}' | '\u{2029}') => out.push('↵'),
            Some(c) if c.is_control() => out.push(' '),
            Some(c) => {
                if "\\|`*_[]<>!&~#".contains(c) {
                    out.push('\\');
                }
                out.push(c);
            }
            None => break,
        }
    }
    out
}

/// Publish a new Markdown file without replacing anything, including a symlink or hard link.
pub fn save_report(path: &Path, report: &str) -> Result<(), Refusal> {
    if !path.is_absolute()
        || !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
        || path.components().any(
            |part| matches!(part, std::path::Component::Normal(name) if name.to_string_lossy().contains([':', '\0'])),
        )
    {
        return Err(Refusal::Io("Choose a normal absolute file path ending in .md.".into()));
    }
    crate::util::write_new_atomic(path, report.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{marks::Marks, util::testing::TempDir};

    #[test]
    fn age_boundary_and_protections() {
        let now = 16 * WEEK_MS;
        for w in WEEKS {
            assert!(older_than(now, w, now - w * WEEK_MS - 1));
            assert!(!older_than(now, w, now - w * WEEK_MS));
        }
        assert!(!older_than(now, 2, 0));
        assert!(eligible(false, false, false));
        assert!(!eligible(true, false, false));
        assert!(!eligible(false, true, false));
        assert!(!eligible(false, false, true));
    }

    #[test]
    fn archive_overrides_ordinary_visibility_only_when_explicit() {
        assert!(!visible(true, false, false, true));
        assert!(visible(true, false, true, false));
        assert!(visible(true, true, false, false));
        assert!(!visible(false, true, true, true));
        assert!(!visible(false, false, true, false));
    }

    #[test]
    fn archive_marks_do_not_overwrite_hide_or_transcripts() {
        let t = TempDir::new("archive-marks");
        let roots = t.roots();
        let file = t.0.join("session.jsonl");
        std::fs::write(&file, "untouched").unwrap();
        let mut marks = Marks::default();
        marks.hide("a", &file, true);
        marks.pin("b", &file, true);
        assert!(marks.archive("a", &file, Some(42)));
        assert!(!marks.archive("a", &file, Some(43)));
        assert!(!marks.archive("b", &file, Some(42)));
        marks.save(&roots).unwrap();
        let mut marks = Marks::load(&roots).unwrap();
        assert_eq!(marks.archived_at("a"), Some(42));
        assert!(marks.archive("a", &file, None) && marks.hidden("a"));
        assert!(marks.archive("orphan", &file, Some(45)));
        assert_eq!(marks.clear_archive(), 1);
        assert_eq!(marks.archived_at("orphan"), None);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "untouched");
    }

    #[test]
    fn report_totals_grouping_order_and_literal_unicode_cells() {
        let row = |name: &str, folder: &str, last| Row {
            name: name.into(),
            cwd: folder.into(),
            transcript: Path::new("projects").join(folder).join("s.jsonl"),
            first: (last != 9).then_some(last - 1),
            last,
            bytes: u64::MAX,
            first_prompt: "a|\r\nb\n<script>".into(),
            last_prompt: format!("{}TRUNCATE", "ş".repeat(160)),
            handover: Some("session-1.md".into()),
            stats: Some(crate::stats::Stats {
                turns: 2,
                context: crate::stats::Tokens { count: 400, about: false },
                output: crate::stats::Tokens { count: 50, about: true },
                ..Default::default()
            }),
        };
        let out = markdown(&[row("earlier", "A", 3), row("newer", "a", 8), row("other", "b", 9)], "today", |n| {
            format!("t{n}")
        });
        assert!(out.contains("**Sessions:** 3 · **Folders:** 2 · **Size on disk:** 55340232221128654845 bytes"));
        assert!(out.contains("**Time span:** t2 → t9"));
        assert!(out.find("| newer").unwrap() < out.find("| earlier").unwrap());
        assert!(out.contains(r"a\|↵b↵\<script\>") && !out.contains("TRUNCATE"));
        assert!(out.contains("| — | t9 |") && out.contains("| session-1.md | 2 | 400 / ~50 |"));
        assert!(out.contains("Nothing was deleted or moved.") && out.contains("projects"));
    }

    #[test]
    fn export_never_replaces_even_a_markdown_file() {
        let t = TempDir::new("archive-report");
        let chosen = t.0.join("report.md");
        save_report(&chosen, "first").unwrap();
        assert!(save_report(&chosen, "second").is_err());
        assert_eq!(std::fs::read_to_string(chosen).unwrap(), "first");
        assert!(save_report(&t.0.join("transcript.jsonl"), "no").is_err());
        assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 1);
    }
}
