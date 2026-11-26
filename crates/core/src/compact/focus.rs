//! The focus a handover is written with when the user gives none: one of three texts, each with a
//! built-in version that Settings can replace and reset.

/// Which built-in focus a session gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusKind {
    #[default]
    Default,
    /// Most tool calls hand work to workers or agents; few edits of its own.
    Coordination,
    /// No code files edited.
    NonCode,
}

/// The longest focus text kept, in characters (the summarizer request reads as much).
pub const FOCUS_MAX: usize = 4_000;

impl FocusKind {
    pub const ALL: [Self; 3] = [Self::Default, Self::Coordination, Self::NonCode];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Coordination => "coordination",
            Self::NonCode => "nonCode",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Coordination => "Coordination",
            Self::NonCode => "Non-code work",
        }
    }

    pub fn built_in(self) -> &'static str {
        match self {
            Self::Default => DEFAULT,
            Self::Coordination => COORDINATION,
            Self::NonCode => NON_CODE,
        }
    }
}

const DEFAULT: &str = "The reader continues this work unattended and remembers nothing. Put the newest first, so anything cut off is the oldest; leave out finished history. stands: each background job, worker or review still running or unread, with its job/session id, worktree, branch and last commit; uncommitted or unpushed work; where a merge stopped. decisions: who decided (user or assistant) and why; mark them settled. instructions: only rules still in force, in their latest form; no next steps; never assume a permission was given. open: promised follow-ups with their trigger; recorded requests not yet done, by number. Do not restate this focus.";

const COORDINATION: &str = "This session coordinates workers and writes no code itself. stands: one line per worker or job: id, model, worktree, branch, brief file, state (running, finished unread, reviewed, merged) and what was promised when it reports; the merge order and where it stopped. instructions: which model does which kind of work, as last set, and other rules still in force. Leave out what the workers changed in the code. Newest first. Do not restate this focus.";

const NON_CODE: &str = "No code here. stands: the file or job in hand, how far it got (episode, line, part), what was applied and what was only proposed. decisions: terminology, style and guideline choices with their reason, so they stay consistent. instructions: rules still in force, such as what may be edited and what is read-only. open: checks and corrections still owed. Newest first. Do not restate this focus.";

/// A saved focus text: trimmed, without control characters other than line breaks and tabs,
/// at most [`FOCUS_MAX`] characters. `None` when nothing is left.
pub fn clean_focus(text: &str) -> Option<String> {
    let text: String =
        text.trim().chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(FOCUS_MAX).collect();
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_owned())
}
