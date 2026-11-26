//! The sessions on disk: transcripts joined with the desktop apps' records.

use crate::{Config, Roots, Session};
use std::path::{Path, PathBuf};

/// Every session of every account, newest first. Reads each account's `<cli_dir>/projects` (a folder
/// reached twice through a link counts once: sessions are unique by id, and paths are compared after
/// `canonicalize`) and each account's desktop records. Skips `subagents` folders and files that hold
/// no session lines. A desktop record's `priorCliSessionIds` are not sessions of their own: they are
/// left out when the record's current `cliSessionId` has a transcript. Never fails.
pub fn scan(roots: &Roots, config: &Config) -> Vec<Session> {
    let _ = (roots, config);
    todo!()
}

/// One transcript read alone (head and tail only, see `docs/DESIGN.md`), without desktop records.
/// `None` when the file is not a session's transcript.
pub fn read_transcript(file: &Path) -> Option<Session> {
    let _ = file;
    todo!()
}

/// The sessions of one working folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The folder. Empty for sessions that named none.
    pub cwd: PathBuf,
    /// The folder's last component, for a heading; `"(no folder)"` when empty.
    pub name: String,
    /// Indices into the slice that was grouped, newest first.
    pub sessions: Vec<usize>,
}

/// Groups sessions by working folder (compared without regard to case or a trailing separator).
/// Groups are ordered by their newest session, newest first.
pub fn group_by_folder(sessions: &[Session]) -> Vec<Group> {
    let _ = sessions;
    todo!()
}

/// One piece of a conversation, for a preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewLine {
    /// True for what the user wrote, false for the assistant's text.
    pub from_user: bool,
    /// The text, with runs of white space as one space, cut to 400 characters.
    pub text: String,
}

/// The last `max` user prompts and assistant texts of a transcript, oldest first, read from the
/// file's tail only. Tool calls, tool results, meta lines, sidechains and prompts that start with
/// `<` are left out. Empty when nothing can be read.
pub fn preview(file: &Path, max: usize) -> Vec<PreviewLine> {
    let _ = (file, max);
    todo!()
}
