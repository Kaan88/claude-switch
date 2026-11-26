//! The accounts: what the files say about each login, and adding, renaming and removing one.

use crate::{Account, AccountStatus, Config, Refusal, Roots};
use std::path::PathBuf;

/// The `.claude.json` of an account: `<home>/.claude.json` for the first account, `<cli_dir>/.claude.json`
/// for any other.
pub fn claude_json(roots: &Roots, account: &Account) -> PathBuf {
    let _ = (roots, account);
    todo!()
}

/// What the files say about the account's login. Never fails: whatever cannot be read is `false`/`None`.
/// Never opens `.credentials.json`; only asks whether it exists.
pub fn status(roots: &Roots, account: &Account) -> AccountStatus {
    let _ = (roots, account);
    todo!()
}

/// A slug for a new account's name: lower-case ASCII letters and digits, other runs become one `-`,
/// trimmed, `"account"` when nothing is left, and `-2`, `-3`… appended until it is not in `taken`.
pub fn slug_for(name: &str, taken: &[&str]) -> String {
    let _ = (name, taken);
    todo!()
}

/// Adds an account: makes its slug, picks the next colour of a fixed list, creates
/// `<accounts_dir>/<slug>/cli` and `…/desktop`, sets up the shared links ([`crate::share::apply`]) and
/// pushes it onto `config.accounts`. Does not save the config. An empty name is [`Refusal::Invalid`].
pub fn add(roots: &Roots, config: &mut Config, name: &str) -> Result<Account, Refusal> {
    let _ = (roots, config, name);
    todo!()
}

/// Changes what an account is called (never its slug). An empty name is [`Refusal::Invalid`].
pub fn rename(config: &mut Config, slug: &str, name: &str) -> Result<(), Refusal> {
    let _ = (config, slug, name);
    todo!()
}

/// Takes an account off the list. Its folders, and so its logins, stay on disk untouched. The first
/// account cannot be removed ([`Refusal::Invalid`]). `config.current` and `config.workers` fall back
/// to the first account / `Same` when they named it.
pub fn remove(config: &mut Config, slug: &str) -> Result<(), Refusal> {
    let _ = (config, slug);
    todo!()
}
