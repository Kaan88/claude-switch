//! Handovers written in the background. **Compact and open…** closes its dialog when the model
//! starts writing; the dialog itself, with every choice made in it, waits here while its worker
//! runs, and opens again as the Review handover window by itself when the handover is written, as
//! soon as nothing modal is open. The Sessions column's strip shows the ones being written, the
//! failed ones and the ones whose review was closed without acting on it.
use crate::compact_dialog::Compact;
use crate::dialogs::Dialog;
use crate::state::{AppState, Target};
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::infobar::{InfoBar, Severity};
use std::time::{Duration, Instant};

/// A review that opened by itself ignores keys, and clicks on what acts, for this long, so a key
/// typed into the composer or a terminal just before it opened cannot press one of its buttons.
pub const SETTLE: Duration = Duration::from_millis(300);
/// A written handover's review waits to open by itself while the keyboard was used anywhere in
/// the app this recently: someone typing a sentence into the composer or a terminal goes on
/// typing there.
pub const TYPING_HOLD: Duration = Duration::from_millis(1500);
/// [`Target::Info`] ids at and above this retry the background handover `id - INFO_RETRY`.
pub const INFO_RETRY: u32 = 0x4800_0000;
/// One line of the strip.
pub const ROW_H: f32 = 30.0;
/// A failed handover's line has the reason under it.
pub const FAILED_H: f32 = 46.0;
/// More handovers than this fold into one line that opens the list.
const FOLD_ABOVE: usize = 2;

pub struct Background {
    pub id: u64,
    pub(crate) dialog: Box<Compact>,
    pub started: Instant,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Writing,
    /// Written: its review opens by itself when nothing modal is open.
    Finished,
    /// Its review was closed without acting on it: Review opens it again.
    Ready,
    Failed(String),
}

/// What the strip's buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Review,
    Cancel,
    Dismiss,
    Retry,
    /// Opens or closes the folded list (the id is unused).
    Fold,
}

#[derive(Default)]
pub struct Jobs {
    pub list: Vec<Background>,
    next: u64,
    /// The folded line is open.
    pub open: bool,
    /// What finished since the window last flashed for it.
    pub(crate) flash: Option<String>,
    /// When a key was last pressed or text typed anywhere in the app.
    pub(crate) typed: Option<Instant>,
}

impl Background {
    pub fn title(&self) -> &str {
        self.dialog.title()
    }

    pub fn session_id(&self) -> &str {
        self.dialog.session_id()
    }
}

/// "0:42", "12:05", "1:02:03".
pub fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl AppState {
    /// A handover dialog was closed (its button returned false). A handover now being written
    /// goes on in the background; a written one closed without acting on it waits in the strip;
    /// anything else is dropped, its worker cancelled.
    pub(crate) fn compact_closed(&mut self, mut dialog: Box<Compact>, now: Instant) {
        if dialog.generating() {
            let id = self.compact.jobs.next_id();
            self.compact.jobs.list.push(Background { id, dialog, started: now, state: State::Writing });
            return;
        }
        if let Some(job) = dialog.take_work() {
            self.compact.retire(job);
        }
        if dialog.parkable() {
            let id = self.compact.jobs.next_id();
            self.compact.jobs.list.push(Background { id, dialog, started: now, state: State::Ready });
        }
    }

    /// Moves an open review into the strip (before something replaces the dialog).
    pub(crate) fn park_open_review(&mut self, now: Instant) {
        if let Some(Dialog::Compact(dialog)) = self.dialog.take_if(|d| matches!(d, Dialog::Compact(c) if c.parkable()))
        {
            self.compact_closed(dialog, now);
        }
    }

    /// Takes what background workers finished. True when something changed.
    pub fn take_background_handovers(&mut self) -> bool {
        let mut list = std::mem::take(&mut self.compact.jobs.list);
        let mut changed = false;
        for job in &mut list {
            if job.state != State::Writing {
                continue;
            }
            match job.dialog.poll_generation(self) {
                None => {}
                Some(Ok(())) => {
                    job.state = State::Finished;
                    self.compact.jobs.flash = Some(format!("Handover ready · {}", job.title()));
                    changed = true;
                }
                Some(Err(reason)) => {
                    if self.terminal_shown() {
                        let text = format!("“{}”: {reason}", job.title());
                        self.info = Some(
                            InfoBar::new(Severity::Error, "Handover failed", text)
                                .closable(true)
                                .action(INFO_RETRY + job.id as u32, "Retry"),
                        );
                    }
                    self.compact.jobs.flash = Some(format!("Handover failed · {}", job.title()));
                    job.state = State::Failed(reason);
                    changed = true;
                }
            }
        }
        list.append(&mut self.compact.jobs.list);
        self.compact.jobs.list = list;
        changed
    }

    /// Opens the review of the first written handover when nothing modal is open (a dialog,
    /// Settings, a menu, the palette, the accounts list or the usage pop-up) and the keyboard has
    /// rested for [`TYPING_HOLD`]. True when it did.
    pub fn open_finished_handover(&mut self, now: Instant) -> bool {
        if !self.nothing_over() || self.usage.popup.open || self.quit || self.handover_held(now).is_some() {
            return false;
        }
        let Some(i) = self.compact.jobs.list.iter().position(|j| j.state == State::Finished) else { return false };
        let job = self.compact.jobs.list.remove(i);
        self.show_review(job.dialog, Some(now));
        true
    }

    /// Until when a written handover waits for the keyboard to rest, while it does.
    pub(crate) fn handover_held(&self, now: Instant) -> Option<Instant> {
        let until = self.compact.jobs.typed? + TYPING_HOLD;
        let waiting = self.compact.jobs.list.iter().any(|j| j.state == State::Finished);
        (waiting && until > now).then_some(until)
    }

    fn show_review(&mut self, mut dialog: Box<Compact>, settle_from: Option<Instant>) {
        dialog.reopened(self, settle_from);
        self.dialog = Some(Dialog::Compact(dialog));
    }

    /// The background handover of `session`, when one is being written, is written or failed:
    /// true when it was dealt with (its review shown, or a notice that it is being written).
    pub(crate) fn background_handover_of(&mut self, session: &str, now: Instant) -> bool {
        let Some(i) = self.compact.jobs.list.iter().position(|j| j.session_id() == session) else { return false };
        if self.compact.jobs.list[i].state == State::Writing {
            let title = self.compact.jobs.list[i].title().to_owned();
            self.toast(format!("A handover of “{title}” is being written. It opens when it is ready."), now);
            return true;
        }
        let job = self.compact.jobs.list.remove(i);
        if let State::Failed(reason) = &job.state {
            let mut dialog = job.dialog;
            dialog.failed(reason.clone());
            self.show_review(dialog, None);
        } else {
            self.show_review(job.dialog, None);
        }
        true
    }

    pub fn handover_act(&mut self, id: u64, act: Act, now: Instant) {
        if act == Act::Fold {
            self.compact.jobs.open = !self.compact.jobs.open;
            return;
        }
        let Some(i) = self.compact.jobs.list.iter().position(|j| j.id == id) else { return };
        match act {
            Act::Review if self.dialog.is_none() => {
                let job = self.compact.jobs.list.remove(i);
                self.show_review(job.dialog, None);
            }
            Act::Cancel | Act::Dismiss => {
                let mut job = self.compact.jobs.list.remove(i);
                if let Some(work) = job.dialog.take_work() {
                    self.compact.retire(work);
                }
                if act == Act::Cancel {
                    self.toast(format!("Handover of “{}” canceled.", job.title()), now);
                }
            }
            Act::Retry => {
                let mut job = self.compact.jobs.list.remove(i);
                if job.dialog.retry(self) {
                    job.state = State::Writing;
                    job.started = now;
                    self.compact.jobs.list.insert(i, job);
                } else if self.dialog.is_none() {
                    // It needs an answer first (consent again, or the reason it cannot start).
                    self.show_review(job.dialog, None);
                } else {
                    self.compact.jobs.list.insert(i, job);
                }
            }
            _ => {}
        }
    }

    /// What closing the window does to handovers, for its confirmation; None when nothing.
    pub fn handover_close_note(&self) -> Option<String> {
        let jobs = &self.compact.jobs.list;
        let writing = jobs.iter().filter(|j| j.state == State::Writing).count();
        let unread = jobs.iter().filter(|j| matches!(j.state, State::Finished | State::Ready)).count()
            + usize::from(matches!(&self.dialog, Some(Dialog::Compact(c)) if c.parkable()));
        let mut parts = Vec::new();
        match writing {
            0 => {}
            1 => parts.push("A handover is still being written. It is canceled.".to_owned()),
            n => parts.push(format!("{n} handovers are still being written. They are canceled.")),
        }
        match unread {
            0 => {}
            1 => parts.push("A written handover has not been opened. It is not saved and is lost.".to_owned()),
            n => parts.push(format!("{n} written handovers have not been opened. They are not saved and are lost.")),
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// Cancels every background worker for the window's exit. True while any still runs.
    pub(crate) fn cancel_background_handovers(&mut self) {
        for job in &mut self.compact.jobs.list {
            if let Some(work) = job.dialog.take_work() {
                work.cancel();
                self.compact.retiring.push(work);
            }
        }
    }
}

impl Jobs {
    fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn of(&self, session: &str) -> Option<&Background> {
        self.list.iter().find(|j| j.session_id() == session)
    }

    /// The strip's height: nothing without handovers, one line when they are folded.
    pub fn height(&self) -> f32 {
        let rows = || self.list.iter().map(|j| row_height(&j.state)).sum::<f32>();
        match self.list.len() {
            0 => 0.0,
            n if n > FOLD_ABOVE && !self.open => ROW_H + 8.0,
            n if n > FOLD_ABOVE => ROW_H + rows() + 8.0,
            _ => rows() + 8.0,
        }
    }
}

impl AppState {
    /// A handover finished (written or failed) while the window was not in front: its taskbar
    /// button flashes, and when it is hidden in the notification area a notice says so. It never
    /// takes the focus from another program.
    pub fn handover_attention(&mut self, cx: &mut mdshared_ui::host::Cx<'_, crate::state::Msg>) {
        let Some(text) = self.compact.jobs.flash.take() else { return };
        if cx.focused() && !self.notifications.hidden {
            return;
        }
        if self.notifications.hidden {
            self.notifications.tray.notice("Claude Switch", &text, None);
        }
        crate::tray::flash(cx.window());
    }
}

/// The strip under the Sessions column's filter chips, `r.w` wide from `r`'s top; nothing (and
/// no room: [`Jobs::height`] is 0) when there is nothing to show. `anim` turns the spinners.
/// Returns the rows to keep redrawn while a handover is being written.
pub fn draw_strip(
    st: &mut AppState,
    p: &mut Painter,
    r: Rect,
    now: Instant,
    mut anim: Option<&mut mdshared_ui::anim::Animator>,
) -> Option<Rect> {
    let height = st.compact.jobs.height();
    if height == 0.0 {
        return None;
    }
    let pal = *st.pal();
    let strip = Rect::new(r.x, r.y, r.w, height);
    p.fill(strip, pal.bar);
    p.fill(Rect::new(strip.x, strip.bottom() - p.px(), strip.w, p.px()), pal.line);
    let mut y = strip.y + 4.0;
    let mut moving: Option<Rect> = None;
    let jobs = &st.compact.jobs;
    let count = jobs.list.len();
    if count > FOLD_ABOVE {
        let row = Rect::new(strip.x + 12.0, y, strip.w - 24.0, ROW_H);
        let writing = jobs.list.iter().filter(|j| j.state == State::Writing).count();
        let ready = jobs.list.iter().filter(|j| matches!(j.state, State::Finished | State::Ready)).count();
        let failed = count - writing - ready;
        let mut text = format!("{count} handovers");
        for (n, what) in [(writing, "being written"), (ready, "ready"), (failed, "failed")] {
            if n > 0 {
                text.push_str(&format!(" · {n} {what}"));
            }
        }
        let open = jobs.open;
        let button =
            small_button(st, p, row, &[(if open { "Hide" } else { "Show" }, Target::HandoverJob(0, Act::Fold))]);
        icon(
            p,
            &pal,
            row,
            if writing > 0 {
                Icon::Spin
            } else if failed > 0 {
                Icon::Failed
            } else {
                Icon::Ready
            },
            anim.as_deref_mut(),
            0,
        );
        p.text(Face::Semibold, 12.0, row.x + 22.0, row.cy(), &text, pal.text, button - row.x - 30.0);
        if writing > 0 {
            moving = Some(row);
        }
        y += ROW_H;
        if !open {
            return moving;
        }
    }
    let rows: Vec<_> = st
        .compact
        .jobs
        .list
        .iter()
        .map(|j| (j.id, j.state.clone(), j.title().to_owned(), now.saturating_duration_since(j.started)))
        .collect();
    for (id, state, title, took) in rows {
        // Buttons and the first line keep to the top ROW_H; a failure's reason goes below.
        let row = Rect::new(strip.x + 12.0, y, strip.w - 24.0, ROW_H);
        let (kind, head, rest, buttons): (Icon, &str, String, Vec<(&str, Target)>) = match &state {
            State::Writing => (
                Icon::Spin,
                "Writing handover",
                // The time first, so a long title cannot push it out of sight.
                format!(" · {} · {title}", elapsed(took)),
                vec![("Cancel", Target::HandoverJob(id, Act::Cancel))],
            ),
            State::Finished | State::Ready => (
                Icon::Ready,
                "Handover ready",
                format!(" · {title}"),
                vec![("Review", Target::HandoverJob(id, Act::Review)), ("✕", Target::HandoverJob(id, Act::Dismiss))],
            ),
            State::Failed(_) => (
                Icon::Failed,
                "Handover failed",
                format!(" · {title}"),
                vec![("Retry", Target::HandoverJob(id, Act::Retry)), ("✕", Target::HandoverJob(id, Act::Dismiss))],
            ),
        };
        let left = small_button(st, p, row, &buttons);
        icon(p, &pal, row, kind, anim.as_deref_mut(), id);
        let room = (left - row.x - 30.0).max(0.0);
        let x = row.x + 22.0;
        let head_w = p.measure(Face::Semibold, 12.0, head).min(room);
        let colour = if matches!(state, State::Failed(_)) { pal.danger } else { pal.text };
        p.text(Face::Semibold, 12.0, x, row.cy(), head, colour, room);
        p.text(Face::Ui, 12.0, x + head_w, row.cy(), &rest, pal.muted, (room - head_w).max(0.0));
        if let State::Failed(reason) = &state {
            let width = (strip.right() - 12.0 - x).max(0.0);
            p.text(Face::Ui, 11.5, x, row.bottom() + 3.0, reason, pal.muted, width);
        }
        if state == State::Writing {
            moving = Some(moving.map_or(row, |m| Rect::new(m.x, m.y, m.w, row.bottom() - m.y)));
        }
        y += row_height(&state);
    }
    moving
}

fn row_height(state: &State) -> f32 {
    if matches!(state, State::Failed(_)) { FAILED_H } else { ROW_H }
}

#[derive(Clone, Copy)]
enum Icon {
    Spin,
    Ready,
    Failed,
}

fn icon(
    p: &mut Painter,
    pal: &mdshared_ui::theme::Palette,
    row: Rect,
    kind: Icon,
    anim: Option<&mut mdshared_ui::anim::Animator>,
    id: u64,
) {
    let (cx, cy) = (row.x + 7.0, row.cy());
    match kind {
        Icon::Spin => match anim {
            Some(anim) => mdshared_ui::slider::draw_spinner(p, pal, anim, ("handover-job", id), cx, cy, 5.5),
            None => mdshared_ui::slider::draw_spinner_at(p, pal, cx, cy, 5.5, Some((0.1, 0.4))),
        },
        Icon::Ready => p.text_center(Face::Semibold, 13.0, Rect::new(cx - 7.0, cy - 8.0, 14.0, 16.0), "✓", pal.accent),
        Icon::Failed => p.rbox(Rect::new(cx - 4.0, cy - 4.0, 8.0, 8.0), 4.0, pal.danger),
    }
}

/// Small buttons at the right of `row`, the first leftmost. Returns where the first starts.
fn small_button(st: &mut AppState, p: &mut Painter, row: Rect, buttons: &[(&str, Target)]) -> f32 {
    let pal = *st.pal();
    let widths: Vec<f32> =
        buttons.iter().map(|(label, _)| (p.measure(Face::Ui, 11.5, label) + 18.0).max(26.0)).collect();
    let mut x = row.right() - widths.iter().sum::<f32>() - 4.0 * buttons.len().saturating_sub(1) as f32;
    let left = x;
    for ((label, target), w) in buttons.iter().zip(widths) {
        let r = Rect::new(x, row.cy() - 11.0, w, 22.0);
        let hot = st.hover == Some(*target);
        let fill = if st.pressed == Some(*target) && hot {
            pal.press
        } else if hot {
            pal.hover
        } else {
            pal.bar
        };
        p.frame(r, 5.0, fill, pal.line);
        p.text_center(Face::Ui, 11.5, r, label, pal.text);
        st.hits.push((r, *target));
        x += w + 4.0;
    }
    left
}

/// The handover's line in the details pane of its session, beside “Handovers…”, at `x`, `y`
/// (the button's middle) and at most `w` wide.
pub fn draw_details_line(st: &mut AppState, p: &mut Painter, id: &str, x: f32, y: f32, w: f32, now: Instant) {
    let Some(job) = st.compact.jobs.of(id) else { return };
    let pal = *st.pal();
    let (text, colour, button) = match &job.state {
        State::Writing => (
            format!("Writing handover · {}", elapsed(now.saturating_duration_since(job.started))),
            pal.muted,
            ("Cancel", Target::HandoverJob(job.id, Act::Cancel)),
        ),
        State::Finished | State::Ready => {
            ("Handover ready".to_owned(), pal.text, ("Review", Target::HandoverJob(job.id, Act::Review)))
        }
        State::Failed(_) => {
            ("Handover failed".to_owned(), pal.danger, ("Retry", Target::HandoverJob(job.id, Act::Retry)))
        }
    };
    let row = Rect::new(x, y - ROW_H / 2.0, w, ROW_H);
    let left = small_button(st, p, row, &[button]);
    p.text(Face::Ui, 12.0, x, y, &text, colour, (left - x - 8.0).max(0.0));
}

#[cfg(test)]
mod tests;
