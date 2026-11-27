//! Undo and redo for the Chat draft, the way a Windows text box does them: typing is one step per
//! word or pause, a run of Backspace or Delete is one step, and every other change (paste, cut,
//! a completion, a recalled prompt, a filled chip) is a step of its own. A step brings back the
//! text with its caret and selection as they were. The history is bounded and starts afresh when
//! a message is sent.
use std::time::{Duration, Instant};

/// The draft as a step restores it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Typed text: runs on while the user keeps typing the same word.
    Typing,
    /// Backspace or Delete: runs on while the user keeps deleting.
    Deleting,
    /// Anything else: always a step of its own.
    Other,
}

/// The most steps kept; the oldest go first.
pub(super) const LIMIT: usize = 100;
/// A pause this long while typing or deleting starts a new step.
pub(super) const PAUSE: Duration = Duration::from_millis(1000);

#[derive(Default)]
pub(super) struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    open: Option<Open>,
    /// The time tests give instead of the clock.
    #[cfg(test)]
    pub clock: Option<Instant>,
}

/// The step typing or deleting is adding to.
struct Open {
    kind: Kind,
    /// Where the caret was after the step's last edit: an edit elsewhere starts a new step.
    caret: usize,
    at: Instant,
    /// The last typed character was white space, so a word typed next is a new step.
    space: bool,
}

impl History {
    fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(at) = self.clock {
            return at;
        }
        Instant::now()
    }

    /// An edit of `kind` (`typed` is the text typed, for word steps) turned `before` into `after`.
    pub fn record(&mut self, before: Snapshot, after: &Snapshot, kind: Kind, typed: &str) {
        if before.text == after.text {
            return;
        }
        let now = self.now();
        let continues = self.open.as_ref().is_some_and(|open| {
            open.kind == kind
                && open.caret == before.caret
                && before.caret == before.anchor
                && now.saturating_duration_since(open.at) < PAUSE
                && !(kind == Kind::Typing && open.space && !typed.starts_with(char::is_whitespace))
        });
        if !continues {
            self.undo.push(before);
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        let space = typed.chars().next_back().is_some_and(char::is_whitespace);
        self.open = (kind != Kind::Other).then_some(Open { kind, caret: after.caret, at: now, space });
    }

    /// The draft before the last step, given the draft now (which redo brings back).
    pub fn undo(&mut self, now: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo.push(now);
        self.open = None;
        Some(previous)
    }

    /// The draft after the step undone last, given the draft now.
    pub fn redo(&mut self, now: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(now);
        self.open = None;
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Typing after a caret move or a click is a new step even at the same place.
    pub fn close_step(&mut self) {
        self.open = None;
    }

    pub fn clear(&mut self) {
        *self = Self {
            #[cfg(test)]
            clock: self.clock,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, caret: usize) -> Snapshot {
        Snapshot { text: text.into(), caret, anchor: caret }
    }

    /// Types `text` one character at a time from the end of `start`.
    fn typing(h: &mut History, start: &str, text: &str) -> String {
        let mut draft = start.to_owned();
        for c in text.chars() {
            let before = at(&draft, draft.len());
            draft.push(c);
            h.record(before, &at(&draft, draft.len()), Kind::Typing, c.encode_utf8(&mut [0; 4]));
        }
        draft
    }

    #[test]
    fn typing_is_one_step_per_word_and_per_pause() {
        let start = Instant::now();
        let mut h = History { clock: Some(start), ..History::default() };
        let draft = typing(&mut h, "", "hello world");
        assert_eq!(draft, "hello world");
        let back = h.undo(at(&draft, 11)).unwrap();
        assert_eq!(back, at("hello ", 6), "the last word goes first");
        assert_eq!(h.undo(back).unwrap(), at("", 0));
        assert!(!h.can_undo());

        let mut h = History { clock: Some(start), ..History::default() };
        let draft = typing(&mut h, "", "ab");
        h.clock = Some(start + PAUSE);
        let draft = typing(&mut h, &draft, "cd");
        assert_eq!(h.undo(at(&draft, 4)).unwrap(), at("ab", 2), "a pause starts a new step");
    }

    #[test]
    fn deleting_runs_on_and_other_edits_and_caret_moves_are_their_own_steps() {
        let mut h = History { clock: Some(Instant::now()), ..History::default() };
        // Two Backspaces at the end, one step.
        h.record(at("abcd", 4), &at("abc", 3), Kind::Deleting, "");
        h.record(at("abc", 3), &at("ab", 2), Kind::Deleting, "");
        // A paste right after is its own step, and so is typing straight after it.
        h.record(at("ab", 2), &at("abXY", 4), Kind::Other, "");
        h.record(at("abXY", 4), &at("abXYz", 5), Kind::Typing, "z");
        // Typing elsewhere (the caret moved) is a new step.
        h.record(at("abXYz", 0), &at("_abXYz", 1), Kind::Typing, "_");
        // Typing over a selection is a new step and keeps the selection to restore.
        let selected = Snapshot { text: "_abXYz".into(), caret: 6, anchor: 3 };
        h.record(selected.clone(), &at("_abq", 4), Kind::Typing, "q");
        let mut now = at("_abq", 4);
        let mut seen = Vec::new();
        while let Some(previous) = h.undo(now.clone()) {
            seen.push(previous.text.clone());
            now = previous;
        }
        assert_eq!(seen, ["_abXYz", "abXYz", "abXY", "ab", "abcd"]);
        // Redo walks forward again, the selection included.
        let mut forward = Vec::new();
        while let Some(next) = h.redo(now.clone()) {
            forward.push(next.text.clone());
            now = next;
        }
        assert_eq!(forward, ["ab", "abXY", "abXYz", "_abXYz", "_abq"]);
        let undone = h.undo(now).unwrap();
        assert_eq!(undone, selected, "the selection typed over comes back selected");
    }

    #[test]
    fn a_new_edit_drops_redo_and_the_history_is_bounded() {
        let mut h = History { clock: Some(Instant::now()), ..History::default() };
        h.record(at("", 0), &at("a", 1), Kind::Other, "");
        let back = h.undo(at("a", 1)).unwrap();
        assert!(h.can_redo());
        h.record(back, &at("b", 1), Kind::Other, "");
        assert!(!h.can_redo(), "a new edit drops what was undone");
        for i in 0..LIMIT + 20 {
            h.record(at(&i.to_string(), 0), &at(&(i + 1).to_string(), 0), Kind::Other, "");
        }
        let mut steps = 0;
        let mut now = at("", 0);
        while let Some(previous) = h.undo(now) {
            steps += 1;
            now = previous;
        }
        assert_eq!(steps, LIMIT);
        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
    }
}
