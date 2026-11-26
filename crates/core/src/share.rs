//! What a further account shares with the first: links in its CLI folder, and the MCP server list.

use crate::{Account, Refusal, Roots};

/// The names shared by link, in order: `projects`, `skills`, `agents`, `plugins`, `mcp-servers`
/// (folders), `settings.json`, `CLAUDE.md`, `keybindings.json` (files).
pub const SHARED: [&str; 8] =
    ["projects", "skills", "agents", "plugins", "mcp-servers", "settings.json", "CLAUDE.md", "keybindings.json"];

/// How one shared name stands in an account's CLI folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// It is a link to (or, for a file, the same file as) the first account's.
    Linked,
    /// The first account has no such folder or file, so there is nothing to share.
    NoSource,
    /// Nothing is there yet; [`apply`] will make the link.
    Missing,
    /// Something of the account's own is there (a real folder with content, a different file). It is
    /// never deleted or replaced; the owner decides.
    Conflict,
}

/// One row of [`status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareState {
    /// One of [`SHARED`].
    pub name: &'static str,
    /// How it stands.
    pub state: LinkState,
}

/// How each shared name stands for the account. For the first account every row is `Linked`.
pub fn status(roots: &Roots, account: &Account) -> Vec<ShareState> {
    let _ = (roots, account);
    todo!()
}

/// Makes every `Missing` link (directory junctions for folders; hard links for files, and an empty
/// real folder in the way is removed first) and returns the states afterwards. A `Conflict` is left
/// exactly as it is. Does nothing for the first account. An error making one link does not stop the
/// others; the first such error is returned after all were tried.
pub fn apply(roots: &Roots, account: &Account) -> Result<Vec<ShareState>, Refusal> {
    let _ = (roots, account);
    todo!()
}

/// Copies the `mcpServers` key of the first account's `.claude.json` into the account's own
/// `.claude.json`, changing no other key and creating the file as `{"mcpServers": …}` when missing.
/// Returns whether the file changed. Does nothing (`Ok(false)`) for the first account or when the
/// first account's file has no such key. A target that is not a JSON object is left alone
/// ([`Refusal::Io`]).
pub fn sync_mcp(roots: &Roots, account: &Account) -> Result<bool, Refusal> {
    let _ = (roots, account);
    todo!()
}
