//! Archive report export: informed Save As, a local worker, and an explicit offer to open it.

use std::path::{Path, PathBuf};

use claude_switch_core::{Refusal, Roots, Session, archive, clock, compact, sessions, stats};
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::overlay::{DIALOG_PAD, dialog_card};
use mdshared_ui::platform;

use crate::dialogs::{CANCEL, Dialog, Draw, OK};
use crate::list::now_ms;
use crate::source::Source;
use crate::state::{AppState, Msg};

pub enum Report {
    Ask(Option<(usize, usize)>),
    Busy,
    Saved(PathBuf),
}

pub type ReportResult = Option<Result<PathBuf, Refusal>>;

impl AppState {
    pub fn ask_archive_report(&mut self, archived: Option<(usize, usize)>) {
        if self.sessions.iter().any(|s| self.is_archived(s)) {
            self.dialog = Some(Dialog::ArchiveReport(Report::Ask(archived)));
        }
    }

    fn choose_report_path(&mut self) {
        let Some(proxy) = self.proxy.clone() else { return };
        let rows: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| self.is_archived(s))
            .map(|s| (s.clone(), self.title_of(s).to_owned()))
            .collect();
        let source = match &self.source {
            Source::Real(roots) => Some(roots.clone()),
            Source::Demo => None,
        };
        let offset = clock::local_offset_minutes();
        let date = mdshared_app::fmt::date((now_ms() / 1000) as i64, offset);
        let preferred =
            source.as_ref().map_or_else(std::env::temp_dir, |roots| documents(&roots.home).join("Claude Switch"));
        let chosen_dir = if source.is_some() && std::fs::create_dir_all(&preferred).is_err() {
            preferred.parent().unwrap_or(&preferred).to_owned()
        } else {
            preferred
        };
        let suggested = chosen_dir.join(format!("archive-{date}.md"));
        let started = platform::pick_save_async(&suggested, move |chosen| {
            let result = chosen.map(|mut path| {
                if path.extension().is_none() {
                    path.set_extension("md");
                }
                let rows: Vec<_> = rows.into_iter().map(|(s, name)| report_row(source.as_ref(), s, name)).collect();
                let text = archive::markdown(&rows, &date, |at| crate::details::local_date_time(at, offset));
                archive::save_report(&path, &text).map(|()| path)
            });
            if let Ok(p) = proxy.lock() {
                p.send(Msg::ArchiveReportSaved(result));
            }
        });
        if started {
            self.dialog = Some(Dialog::ArchiveReport(Report::Busy));
        } else {
            self.warn("Could not open Save As", "Close any other file picker and try again.".into());
            self.dialog = Some(Dialog::ArchiveReport(Report::Ask(None)));
        }
    }

    pub fn report_saved(&mut self, result: ReportResult) {
        if matches!(self.dialog, Some(Dialog::ArchiveReport(Report::Busy))) {
            self.dialog = None;
        }
        match result {
            Some(Ok(path)) => self.dialog = Some(Dialog::ArchiveReport(Report::Saved(path))),
            Some(Err(error)) => {
                self.error("Could not save archive report", error.to_string());
            }
            None => {}
        }
    }
}

fn report_row(roots: Option<&Roots>, s: Session, name: String) -> archive::Row {
    let (first, first_prompt, last_prompt, handover, stats) = match roots {
        Some(roots) => {
            let excerpts = sessions::archive_excerpts(&s.file);
            let handover = compact::list(roots, &s.id)
                .ok()
                .and_then(|files| files.into_iter().next())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            let stats = stats::read(&s.file, None, || false).ok().flatten().map(|c| c.stats);
            (
                excerpts.first_active,
                excerpts.first_prompt.unwrap_or_default(),
                excerpts.last_prompt.unwrap_or(s.last_prompt),
                handover,
                stats,
            )
        }
        None => (
            Some(s.last_active_ms.saturating_sub(3_600_000)),
            format!("{}. Start with the part that fails.", s.title),
            s.last_prompt,
            Some(format!("{}-demo.md", s.id)),
            Some(stats::Stats {
                turns: 3,
                context: stats::Tokens { count: 2800, about: true },
                output: stats::Tokens { count: 640, about: true },
                ..Default::default()
            }),
        ),
    };
    archive::Row {
        name,
        cwd: s.cwd,
        bytes: s.bytes,
        transcript: s.file,
        first,
        last: s.last_active_ms,
        first_prompt,
        last_prompt,
        handover,
        stats,
    }
}

#[cfg(windows)]
fn known_documents() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut raw = std::ptr::null_mut();
    // SAFETY: the known-folder GUID is static, the token is null for the current user, and `raw` is writable.
    let result = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut raw) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: if successful, Windows allocated a NUL-terminated UTF-16 string; free it exactly once
    // with the COM allocator on either path. The owned OsString does not borrow the allocation.
    unsafe {
        let path = (result >= 0).then(|| {
            let len = (0..).find(|&n| *raw.add(n) == 0).unwrap();
            PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw, len)))
        });
        CoTaskMemFree(raw.cast());
        path
    }
}

fn documents(home: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = known_documents() {
        return path;
    }
    home.join("Documents")
}

impl Report {
    pub fn draw(&self, d: &mut Draw<'_>, p: &mut Painter, window: Rect) {
        let card = dialog_card(490.0, 205.0, window.w, window.h);
        let footer = d.card(p, card);
        let (title, text) = match self {
            Self::Ask(archived) => {
                let result = archived.map_or(String::new(), |(n, skipped)| {
                    let rest =
                        if skipped == 0 { String::new() } else { format!(" Skipped {skipped} no longer eligible.") };
                    format!("Archived {n} session(s).{rest}\n\n")
                });
                (
                    "Save summary report",
                    format!(
                        "{result}The Markdown report covers all archived sessions and contains text from the conversations. It is created locally; nothing is sent anywhere."
                    ),
                )
            }
            Self::Busy => (
                "Saving summary report",
                "The report is written locally after you choose a file name in Save As.".into(),
            ),
            Self::Saved(path) => ("Summary report saved", path.display().to_string()),
        };
        d.title(p, card, title);
        p.paragraph_wrapped(
            Face::Ui,
            13.0,
            card.x + DIALOG_PAD,
            card.y + 57.0,
            card.w - 2.0 * DIALOG_PAD,
            &text,
            d.pal.text,
            if matches!(self, Self::Saved(_)) { Wrap::Anywhere } else { Wrap::Words },
        );
        match self {
            Self::Ask(_) => d.footer(p, footer, &[("Summary report…", true, OK), ("Close", false, CANCEL)]),
            Self::Saved(_) => d.footer(p, footer, &[("Open report", false, OK), ("Close", true, CANCEL)]),
            Self::Busy => {}
        }
    }

    pub fn click(self, st: &mut AppState, id: u32) {
        match (self, id) {
            (Self::Ask(_), OK) => st.choose_report_path(),
            (Self::Saved(path), OK) if st.source.is_demo() => {
                st.toast(format!("Demo: report saved, not opened ({})", path.display()), std::time::Instant::now());
            }
            (Self::Saved(path), OK) => {
                if !platform::open_file(&path, &["md"]) {
                    st.warn("Could not open report", path.display().to_string());
                }
            }
            (report @ Self::Busy, _) => st.dialog = Some(Dialog::ArchiveReport(report)),
            _ => {}
        }
    }
}
