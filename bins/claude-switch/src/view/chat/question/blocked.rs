//! The calls auto mode blocked, reviewed from Chat through Claude Code's own list: the "Recently
//! blocked" tab of its /permissions dialog (Claude Code 2.1.332, the `blocked_*` fixtures). The
//! card shows that list as the screen has it: each call with its mark and the classifier's
//! reason, the highlighted call's command, and what Esc will do.
//!
//! What the CLI offers, and all the card offers: Enter marks the highlighted call to be approved
//! and Enter again unmarks it; `r` marks it to be retried and `r` again makes that an approval;
//! Esc closes the list and applies the marks ("Told Claude it may retry …. Auto mode checks it
//! again; this adds no rule."; a retry also starts the turn at once). Nothing is marked or
//! applied without a click, and never for a call whose command the card did not show: a row is
//! first chosen (arrow keys only), and the keys that mark or apply go only while the screen shows
//! exactly the list the card painted, half a second or more ago.
use super::screens::Mark;
use super::*;

pub(super) const LIST_CHANGED: &str = "The list changed in the terminal. Look at it again, then choose.";
pub(super) const NOT_OFFERED: &str = "Claude Code does not offer that for this call. Use the terminal.";
pub(super) const CUT: &str = "This call's command does not fit the card. Review it in the terminal.";
const EXPLAIN: &str = "A mark takes effect when the list closes. Auto mode checks a retried call again.";
/// The most rows of a call's command the card shows; a longer one is the terminal's.
const DETAIL_ROWS: usize = 8;
/// The most calls the card lists at once, around the highlighted one.
const ENTRIES: usize = 5;
const RETRY: &[u8] = b"r";

/// The list as last read from the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Shown {
    pub note: String,
    pub entries: Vec<String>,
    pub marks: Vec<Mark>,
    pub highlight: usize,
    pub pending: Option<String>,
    pub detail: Vec<String>,
    /// What the footer says Esc does: "close", "approve 1 blocked", …
    pub esc: String,
}

impl Shown {
    fn marked(&self) -> bool {
        self.pending.is_some()
    }

    /// The label of the button that writes Esc: the footer's own words.
    fn esc_label(&self) -> String {
        if self.esc == "close" {
            return "Close".to_owned();
        }
        let mut words = self.esc.chars();
        let first = words.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
        format!("{first}{}", words.as_str())
    }
}

pub(super) fn shown(screen: &Screen) -> Option<Shown> {
    let list = screens::blocked(screen)?;
    let highlight = list.menu.highlight()?;
    Some(Shown {
        note: list.note,
        entries: list.menu.texts(),
        marks: list.marks,
        highlight,
        pending: list.pending,
        detail: list.detail,
        esc: list.esc,
    })
}

/// The screen still shows a list of these calls with these marks: its rows, for the highlight.
pub(super) fn still_shown(entries: &[String], screen: &Screen) -> Option<screens::Menu> {
    let list = screens::blocked(screen)?;
    (list.menu.texts() == entries).then_some(list.menu)
}

/// What the card last painted, and since when.
#[derive(Clone, Debug, Default)]
pub(super) struct View {
    painted: Option<(Shown, Instant)>,
    /// A row of the highlighted call's command did not fit the card.
    cut: bool,
}

impl View {
    fn note(&mut self, shown: &Shown, now: Instant, cut: bool) {
        if self.painted.as_ref().map(|(painted, _)| painted) != Some(shown) {
            self.painted = Some((shown.clone(), now));
        }
        self.cut = cut;
    }

    /// The card has painted exactly this list, and for how long.
    fn painted_for(&self, shown: &Shown, now: Instant) -> Option<Duration> {
        let (painted, at) = self.painted.as_ref()?;
        (painted == shown).then(|| now.saturating_duration_since(*at))
    }
}

impl Card {
    /// The card shows Claude Code's list of blocked calls.
    pub(in crate::view::chat) fn blocked_shown(&self) -> bool {
        self.blocked.is_some()
    }
}

pub(super) fn height(card: &Card) -> Option<f32> {
    let shown = card.blocked.as_ref()?;
    let entries = shown.entries.len().min(ENTRIES) as f32 * cards::CHOICE_ROW + 10.0;
    let more = if shown.entries.len() > ENTRIES { LINE } else { 0.0 };
    let pending = if shown.pending.is_some() { LINE } else { 0.0 };
    let detail = shown.detail.len().min(DETAIL_ROWS + 1) as f32 * LINE;
    Some(cards::HEAD + 10.0 + LINE + 8.0 + entries + more + pending + 6.0 + detail + 4.0 + LINE + 6.0 + cards::FOOT)
}

/// A click in the card.
pub(super) fn press(tab: &mut Tab, now: Instant, hit: Hit, shown: Shown) -> bool {
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    let at = shown.highlight;
    // What the click asks for: a row to move to, or a key on the highlighted row.
    let (target, then) = match (hit, shown.marks.get(at)) {
        (Hit::Option(row), _) if row == at => return true,
        (Hit::Option(row), _) if row < shown.entries.len() => (row, Then::Rest),
        // Only the changes recorded from the CLI: mark or unmark with Enter, `r` to retry, and `r`
        // again to make a retry an approval.
        (Hit::Approve, Some(Mark::None | Mark::Approve)) => (at, Then::Choose),
        (Hit::Retry, Some(Mark::None | Mark::Retry)) => (at, Then::Press(RETRY)),
        (Hit::Approve | Hit::Retry, _) => {
            chat.question.error = Some(NOT_OFFERED);
            return true;
        }
        (Hit::Escape, _) => (at, Then::Choose),
        _ => return false,
    };
    // Moving the highlight shows a call; marking it or applying the marks needs the card to have
    // painted exactly this list, for long enough to be read. Esc that only closes needs neither.
    let acts = then != Then::Rest && !(hit == Hit::Escape && !shown.marked());
    if acts {
        let view = &chat.question.blocked_view;
        let refused = match view.painted_for(&shown, now) {
            _ if view.cut && hit != Hit::Escape => Some(CUT),
            None => Some(LIST_CHANGED),
            Some(time) if time < prompt::SEEN => Some(prompt::NOT_SEEN),
            Some(_) => None,
        };
        if let Some(refused) = refused {
            chat.question.error = Some(refused);
            return true;
        }
    }
    if busy(chat) {
        return true;
    }
    let subject = Subject::Blocked { entries: shown.entries };
    if hit == Hit::Escape {
        return escape(tab, now, None, subject);
    }
    if !ready(tab) {
        tab.pane.as_mut().unwrap().chat.question_failed(BLOCKED);
        return true;
    }
    let from = read(&subject, &tab.term.screen, None).and_then(|(menu, ..)| menu.highlight());
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    // A key is for the row the card showed highlighted: not for one the highlight moved to since.
    let Some(from) = from.filter(|from| then == Then::Rest || *from == at) else {
        chat.question_failed(CHANGED);
        return true;
    };
    chat.question.error = None;
    let epoch = chat.output_epoch;
    let step = Step { item: None, subject, target, then, stage: Stage::Moving, due: now + STEP, epoch };
    start(tab, now, step, from)
}

impl Draw<'_> {
    /// Claude Code's list of the calls auto mode blocked.
    pub(super) fn blocked(&self, p: &mut Painter, chat: &mut Chat, inner: Rect, hits: &mut Vec<(Rect, Target)>) {
        let pal = self.pal;
        let Some(shown) = chat.question.blocked.clone() else { return };
        let notice = chat.question.error.or_else(|| chat.question_notice()).or_else(|| chat.question.typed_hint());
        let mut y = inner.y;
        let head = Rect::new(inner.x, y, inner.w, cards::HEAD);
        self.head(p, hits, head, "Blocked by auto mode", ("", ""), "Claude Code", false);
        y += cards::HEAD + 10.0;
        p.text(Face::Ui, 12.5, inner.x, y + LINE / 2.0, &shown.note, pal.text, inner.w);
        y += LINE + 8.0;
        // The calls around the highlighted one; the buttons below act on that one only, which is
        // marked as the CLI highlights it and takes no click.
        let first = shown.highlight.saturating_sub(ENTRIES / 2).min(shown.entries.len().saturating_sub(ENTRIES));
        let rows: Vec<cards::Choice> = shown
            .entries
            .iter()
            .enumerate()
            .skip(first)
            .take(ENTRIES)
            .map(|(i, entry)| cards::Choice {
                number: Some(i + 1),
                label: entry.clone(),
                note: String::new(),
                hit: Hit::Option(i),
                on: false,
            })
            .collect();
        let before = hits.len();
        let list_y = y;
        y += self.choices(p, hits, inner.x, y, inner.w, &rows, true, cards::Density::Plain) + 10.0;
        if let Some(at) = shown.highlight.checked_sub(first).filter(|at| *at < rows.len()) {
            // The call the buttons below act on, as the CLI highlights it.
            let top = list_y + at as f32 * cards::CHOICE_ROW;
            p.rbox(Rect::new(inner.x + 3.0, top + 7.0, 3.0, cards::CHOICE_ROW - 14.0), 1.5, pal.accent);
        }
        let highlighted = Target::Chat(Control::Question(Hit::Option(shown.highlight)));
        let drawn: Vec<_> = hits.drain(before..).filter(|(_, t)| *t != highlighted).collect();
        hits.extend(drawn);
        if shown.entries.len() > ENTRIES {
            let more = format!("{} blocked calls in the list", shown.entries.len());
            p.text(Face::Ui, 12.0, inner.x, y + LINE / 2.0, &more, pal.muted, inner.w);
            y += LINE;
        }
        if let Some(pending) = &shown.pending {
            p.text(Face::Semibold, 12.0, inner.x, y + LINE / 2.0, pending, pal.accent, inner.w);
            y += LINE;
        }
        y += 6.0;
        // The highlighted call's own rows: what would be approved.
        let mut cut = shown.detail.len() > DETAIL_ROWS;
        for row in shown.detail.iter().take(DETAIL_ROWS) {
            cut |= p.measure(Face::Mono, 11.5, row) > inner.w;
            p.text(Face::Mono, 11.5, inner.x, y + LINE / 2.0, row, pal.text, inner.w);
            y += LINE;
        }
        if shown.detail.len() > DETAIL_ROWS {
            let more = format!("{} more rows in the terminal", shown.detail.len() - DETAIL_ROWS);
            p.text(Face::Ui, 12.0, inner.x, y + LINE / 2.0, &more, pal.muted, inner.w);
            y += LINE;
        }
        y += 4.0;
        let status = notice.unwrap_or(if cut { CUT } else { EXPLAIN });
        let ink = if chat.question.error.is_some() { pal.warn } else { pal.muted };
        p.text(Face::Ui, 12.0, inner.x, y + LINE / 2.0, status, ink, inner.w);
        y += LINE + 6.0;
        chat.question.blocked_view.note(&shown, self.now, cut);
        let mut left: Vec<(&str, Hit)> = Vec::new();
        let label = shown.esc_label();
        let mut right = vec![("Open terminal", Hit::Open)];
        if chat.question.step.is_none() {
            // What the CLI offers for the highlighted call, by its mark.
            let offers: &[(&str, Hit)] = match shown.marks.get(shown.highlight) {
                _ if cut => &[],
                Some(Mark::None) => &[("Approve", Hit::Approve), ("Retry now", Hit::Retry)],
                Some(Mark::Approve) => &[("Unmark", Hit::Approve)],
                Some(Mark::Retry) => &[("Approve only", Hit::Retry)],
                None => &[],
            };
            left.extend_from_slice(offers);
            right.push((label.as_str(), Hit::Escape));
        }
        self.foot(p, hits, Rect::new(inner.x, y, inner.w, cards::FOOT), &left, "", &right);
    }
}

#[cfg(test)]
mod tests;
