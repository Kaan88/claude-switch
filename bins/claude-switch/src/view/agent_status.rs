//! What Chat says about a subagent: running, finished, or stopped (interrupted, failed), and `?`
//! only when nothing tells. The same everywhere agents show (Details › Background, the subagent
//! view's header, the Agents page).
//!
//! The files tell how an agent ended (`transcript::AgentEnd`: a notification, its call's result,
//! its sidecar, its own transcript's end). None of them says it still runs: Claude Code marks only
//! an end. While the session runs in a tab, the terminal's agent list under the input does: a row
//! whose status starts with how long it has run is running, "stopped …" stopped. That list shows
//! background agents only.

use claude_switch_core::transcript::{AgentEnd, EndSource, Subagent};

/// An agent's row in the terminal's agent list, as last read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    Running,
    Stopped,
}

/// The three plain states, and `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Finished,
    /// Stopped, interrupted or failed: it ended without its report.
    Stopped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// The word shown with the mark: "Running", "Finished", "Stopped", "Interrupted", "Failed",
    /// "Unknown".
    pub word: &'static str,
    /// Where the state was read, or for `Unknown` why nothing tells; for a tooltip.
    pub why: &'static str,
}

pub const UNKNOWN_WHY: &str = "Nothing tells: the loaded files record no end for it, its transcript stops \
     mid-step, and the terminal's agent list does not show it. It may still run, or it was cut off \
     without a record (a closed session).";

impl Status {
    pub fn of(agent: &Subagent, listed: Option<Listed>) -> Self {
        let status = |state, word, why| Status { state, word, why };
        match (listed, agent.end) {
            (Some(Listed::Running), _) => {
                status(State::Running, "Running", "The terminal's agent list shows it running.")
            }
            (Some(Listed::Stopped), _) => {
                status(State::Stopped, "Stopped", "The terminal's agent list shows it stopped.")
            }
            (None, Some((end, source))) => {
                let (state, word) = match end {
                    AgentEnd::Finished => (State::Finished, "Finished"),
                    AgentEnd::Stopped => (State::Stopped, "Stopped"),
                    AgentEnd::Interrupted => (State::Stopped, "Interrupted"),
                    AgentEnd::Failed => (State::Stopped, "Failed"),
                };
                let why = match source {
                    EndSource::Notification => "From the notification Claude Code wrote when it ended.",
                    EndSource::Result => "From the result of the call that ran it.",
                    EndSource::Sidecar => "From the end Claude Code recorded beside its transcript.",
                    EndSource::Transcript => "From the end of its own transcript.",
                };
                status(state, word, why)
            }
            (None, None) => status(State::Unknown, "Unknown", UNKNOWN_WHY),
        }
    }

    pub fn mark(self) -> &'static str {
        match self.state {
            State::Running => "◉",
            State::Finished => "✓",
            State::Stopped => "■",
            State::Unknown => "?",
        }
    }

    pub fn running(self) -> bool {
        self.state == State::Running
    }

    /// Ended one way or another: no message can reach it.
    pub fn ended(self) -> bool {
        matches!(self.state, State::Finished | State::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(end: Option<(AgentEnd, EndSource)>) -> Subagent {
        Subagent { id: "a".into(), end, ..Subagent::default() }
    }

    #[test]
    fn the_list_wins_then_the_files_and_unknown_only_without_either() {
        let finished = agent(Some((AgentEnd::Finished, EndSource::Sidecar)));
        assert_eq!(Status::of(&finished, None).word, "Finished");
        assert_eq!(Status::of(&finished, Some(Listed::Running)).state, State::Running);
        let stopped = agent(Some((AgentEnd::Interrupted, EndSource::Transcript)));
        assert_eq!(Status::of(&stopped, None).state, State::Stopped);
        assert_eq!(Status::of(&stopped, None).word, "Interrupted");
        assert_eq!(Status::of(&agent(None), Some(Listed::Stopped)).word, "Stopped");
        let unknown = Status::of(&agent(None), None);
        assert_eq!((unknown.state, unknown.mark(), unknown.why), (State::Unknown, "?", UNKNOWN_WHY));
        assert!(!unknown.ended() && !unknown.running());
    }
}
