//! What Chat read when it could not send, kept so that a screen it misreads can become a test.
//!
//! The screen goes into a `.ansi` file in the form `tools/screen-recorder` writes and the
//! fixtures and `probe_recordings` read, and a `.txt` beside it says why Chat stopped and what it
//! made of the screen and the transcript. Both stay on this computer, in
//! `%APPDATA%\claude-switch\screen-reports`: the screen holds the conversation's text, so nothing
//! is sent anywhere. At most one report a minute per tab; the newest [`KEEP`] are kept.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mdshared_term::{Attrs, Color, Screen};

use super::{Reason, submission};
use crate::tabs::Tab;

/// How many reports are kept.
const KEEP: usize = 20;
/// The least time between two reports of one tab.
const GAP: Duration = Duration::from_secs(60);
const ESC: char = '\u{1b}';

/// A refusal that says Chat could not read the screen as it expected: worth a report. A running
/// turn, a card or a missing transcript are not: those are read correctly.
pub(super) fn worth(notice: &str) -> bool {
    [
        submission::NOT_EMPTY,
        submission::INPUT_HAS_TEXT,
        submission::NO_QUIET_PROMPT,
        submission::MENU_OPEN,
        Reason::Tool.label(),
    ]
    .contains(&notice)
}

/// Save what `tab` shows and why Chat stopped (`why`), unless a report of this tab is newer
/// than [`GAP`]. Never fails the caller: a report that cannot be written is skipped.
pub(super) fn save(tab: &mut Tab, now: Instant, why: &str) {
    let Some(pane) = tab.pane.as_mut() else { return };
    if pane.chat.demo || pane.chat.last_report.is_some_and(|at| now.saturating_duration_since(at) < GAP) {
        return;
    }
    pane.chat.last_report = Some(now);
    let Some(dir) = folder() else { return };
    let note = note(tab, now, why);
    let _ = write(&dir, &tab.term.screen, &note);
}

fn folder() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let roots = claude_switch_core::Roots::from_env()?;
    Some(roots.config_file().parent()?.join("screen-reports"))
}

fn note(tab: &Tab, now: Instant, why: &str) -> String {
    let pane = tab.pane.as_ref().unwrap();
    let chat = &pane.chat;
    let info = &pane.model.chat;
    let screen = &tab.term.screen;
    let (gate, reason) = chat.decision(info, &tab.term, &tab.program_title, now);
    let mut s = String::new();
    let _ = writeln!(s, "Chat stopped: {why}");
    let _ = writeln!(s, "gate {gate:?}, reason {reason:?}, last shown {:?}", chat.reason);
    let _ = writeln!(s, "screen: prompt {:?}", submission::screen::prompt(screen));
    let _ = writeln!(
        s,
        "screen: idle {}, resting {}, working {}, menu {}",
        chat.screen_idle(&tab.term, now),
        chat.screen_resting(&tab.term, now),
        submission::screen::working(screen),
        submission::screen::menu(screen),
    );
    let quiet_for = chat.last_output.map(|at| now.saturating_duration_since(at));
    let _ = writeln!(s, "quiet {} (for {quiet_for:?})", chat.quiet(now));
    let _ = writeln!(s, "transcript: latest {:?}, prompt {:?}, pending {:?}", info.latest, info.prompt, info.pending);
    let _ = writeln!(s, "queued {}, title {:?}", chat.queue.is_some(), tab.program_title);
    s
}

fn write(dir: &Path, screen: &Screen, note: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let name = format!("screen-{stamp}");
    std::fs::write(dir.join(format!("{name}.ansi")), ansi(screen))?;
    std::fs::write(dir.join(format!("{name}.txt")), note)?;
    prune(dir)
}

/// Keep the newest [`KEEP`] reports (their names sort by time).
fn prune(dir: &Path) -> std::io::Result<()> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("screen-") && (n.ends_with(".ansi") || n.ends_with(".txt")))
        .collect();
    let mut stems: Vec<String> = names.iter().filter_map(|n| n.strip_suffix(".ansi")).map(str::to_owned).collect();
    stems.sort_by_key(|s| s.trim_start_matches("screen-").parse::<u128>().unwrap_or(0));
    let old = stems.len().saturating_sub(KEEP);
    names.retain(|n| stems[..old].iter().any(|s| n.strip_suffix(".ansi").or(n.strip_suffix(".txt")) == Some(s)));
    for name in names {
        let _ = std::fs::remove_file(dir.join(name));
    }
    Ok(())
}

/// The screen as the fixtures' `.ansi` form: cursor, size, wrapped rows, modes, then each row
/// with its SGR styles (as `tools/screen-recorder` writes it).
pub(super) fn ansi(screen: &Screen) -> String {
    let c = screen.cursor();
    let m = screen.modes();
    let mut s = format!("cursor {} {}\n@size {} {}\n", c.row, c.col, screen.cols(), screen.rows());
    let wrapped: Vec<String> =
        (0..screen.rows()).filter(|&r| screen.line(r).is_some_and(|l| l.wrapped())).map(|r| r.to_string()).collect();
    if !wrapped.is_empty() {
        let _ = writeln!(s, "@wrapped {}", wrapped.join(" "));
    }
    let paste = if m.bracketed_paste { " paste" } else { "" };
    let alt = if m.alt_screen { " alt" } else { "" };
    let _ = writeln!(s, "@modes{paste}{alt}{}", if c.visible { " cursor" } else { "" });
    let color = |c: Color, base: u8| match c {
        Color::Default => String::new(),
        Color::Indexed(n) => format!(";{base};5;{n}"),
        Color::Rgb(r, g, b) => format!(";{base};2;{r};{g};{b}"),
    };
    let bits = [
        (Attrs::BOLD, 1),
        (Attrs::DIM, 2),
        (Attrs::ITALIC, 3),
        (Attrs::UNDERLINE, 4),
        (Attrs::BLINK, 5),
        (Attrs::INVERSE, 7),
        (Attrs::HIDDEN, 8),
        (Attrs::STRIKE, 9),
    ];
    let plain = format!("{ESC}[0m");
    for r in 0..screen.rows() {
        let Some(line) = screen.line(r) else { continue };
        let cells = line.cells();
        let end = cells
            .iter()
            .rposition(|c| c.ch != ' ' || c.attrs != Attrs::NONE || c.bg != Color::Default)
            .map_or(0, |e| e + 1);
        let mut last = plain.clone();
        for cell in cells[..end].iter().filter(|c| !c.is_spacer()) {
            let mut sgr = format!("{ESC}[0");
            for (bit, code) in bits {
                if cell.attrs.contains(bit) {
                    let _ = write!(sgr, ";{code}");
                }
            }
            sgr.push_str(&color(cell.fg, 38));
            sgr.push_str(&color(cell.bg, 48));
            sgr.push('m');
            if sgr != last {
                s.push_str(&sgr);
                last = sgr;
            }
            s.push(cell.ch);
        }
        if last != plain {
            s.push_str(&plain);
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_back_as_the_same_screen_and_only_the_newest_are_kept() {
        let mut screen = Screen::new(40, 6, 0);
        screen.feed("\x1b[?2004h────\r\n\x1b[1m❯\x1b[0m \u{a0}\r\n────".as_bytes());
        screen.feed(b"\x1b[2;3H");
        let text = ansi(&screen);
        assert!(text.starts_with("cursor 1 2\n@size 40 6\n@modes paste cursor\n"), "{text}");
        assert_eq!(super::super::submission::screen::prompt(&screen).as_deref(), Some(""));

        let scratch = crate::testing::Scratch::new("cs-screen-report");
        let dir = scratch.to_path_buf();
        for i in 0..KEEP + 3 {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("screen-{}.ansi", 1000 + i)), "x").unwrap();
            std::fs::write(dir.join(format!("screen-{}.txt", 1000 + i)), "x").unwrap();
        }
        std::fs::write(dir.join("other.txt"), "kept").unwrap();
        prune(&dir).unwrap();
        let mut left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left.len(), KEEP * 2 + 1);
        assert!(!left.contains(&"screen-1002.ansi".to_owned()) && left.contains(&"screen-1003.ansi".to_owned()));
        assert!(left.contains(&"other.txt".to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
