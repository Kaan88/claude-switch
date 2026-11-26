//! This app's own settings file, and the two keys it writes into the bridge's worker list.

use crate::{Config, Refusal, Roots};

/// Reads [`Roots::config_file`]. A missing or damaged file, or a missing key, gives the defaults:
/// one account (`main`, named "Main", primary, the standard folders), `current` = `main`, workers
/// `Same`, embedded terminal on, archived hidden. The first account's folders always come from
/// `roots`, whatever the file says.
pub fn load(roots: &Roots) -> Config {
    let _ = roots;
    todo!()
}

/// Writes the settings (temporary file beside it, then rename), creating the folder.
pub fn save(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let _ = (roots, config);
    todo!()
}

/// Sets `claudeAccounts` (every account's CLI folder, in order, as strings) and `claudeWorkersUse`
/// (`"same"`, `"other"` or a CLI folder) in [`Roots::workers_file`], leaving every other key of that
/// file exactly as it is. A missing file is created holding only the two keys; a file that is not a
/// JSON object is left alone and is [`Refusal::Io`].
pub fn write_workers(roots: &Roots, config: &Config) -> Result<(), Refusal> {
    let _ = (roots, config);
    todo!()
}
