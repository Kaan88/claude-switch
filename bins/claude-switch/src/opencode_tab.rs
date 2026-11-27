//! OpenCode tabs: the owner's normal OpenCode in a folder, as a terminal tab of its own kind. Not a
//! Claude session: no account, transcript, live record, send gate or Chat view. See DESIGN.md,
//! "OpenCode tabs".
//!
//! The installed CLI (checked with `opencode --help`, v2.4.1) takes `opencode [flags] [<directory>]`
//! with `--prompt <text>` for a first prompt and `--continue` for the folder's last session.

use std::path::{Path, PathBuf};

use claude_switch_core::{Launch, Session, launch, sessions::folder_name};

use crate::state::AppState;

/// The tab's icon: a terminal window with a prompt, drawn where Claude tabs show the account colour.
pub const GLYPH: &str =
    "M2.5 3h11a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM4.5 6.3L6.6 8l-2.1 1.7M8 10.2h3.5";

/// How the tab's OpenCode starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// A new session.
    Fresh,
    /// Reopened at startup: the folder's last OpenCode session, and never a prompt again.
    Continue,
    /// A handover: this text is the first prompt.
    Prompt(String),
}

pub fn label(folder: &Path) -> String {
    format!("OpenCode · {}", folder_name(folder))
}

/// The launch: a plain shell's sanitized environment (Claude Code's variables removed) and no
/// OpenCode overrides, so the owner's own config, plugins and Magic Context apply. Options come
/// before the folder, which OpenCode takes as its last argument.
pub fn launch(program: &Path, folder: &Path, start: &Start) -> Launch {
    let mut l = launch::shell(folder);
    l.program = program.to_owned();
    // OpenCode gives PWD precedence over the process folder.
    l.env_remove.push("PWD".into());
    match start {
        Start::Fresh => {}
        Start::Continue => l.args.push("--continue".into()),
        Start::Prompt(text) => l.args.extend(["--prompt".into(), text.clone()]),
    }
    l.args.push(folder.to_string_lossy().into_owned());
    l.label = label(folder);
    l
}

/// The first prompt for a handover: the text itself, or above the command-line limit a request to
/// read the saved file (the same rule as for Claude Code). A text that starts like an option is
/// passed as the file request too, so OpenCode's parser cannot take it for a flag.
pub fn first_prompt(text: &str, saved: &Path) -> Result<String, String> {
    let prompt = crate::compact_open::first_prompt(text, saved)?;
    if !prompt.trim_start().starts_with('-') {
        return Ok(prompt);
    }
    let path = std::path::absolute(saved).map_err(|_| "Could not resolve the absolute handover path.")?;
    let path = path
        .to_str()
        .filter(|s| !s.contains('"') && !s.chars().any(char::is_control))
        .ok_or("Could not represent the absolute handover path safely.")?;
    Ok(format!("Read the handover file at \"{path}\" in full before doing anything else. Then continue from it."))
}

fn program() -> Result<PathBuf, String> {
    crate::compact_runner::opencode()
}

impl AppState {
    pub fn ask_new_opencode_tab(&mut self) {
        self.dialog = Some(crate::dialogs::new_opencode_dialog(self));
    }

    /// Opens an OpenCode tab in `folder` and shows it. The demo opens a placeholder tab.
    pub fn open_opencode_tab(&mut self, folder: &Path, start: Start) -> Result<u64, String> {
        if !self.source.is_demo() && !folder.is_dir() {
            return Err(format!("{} is not an available folder.", folder.display()));
        }
        let id = if self.source.is_demo() {
            let mut term = crate::term::Term::empty(self.grid);
            let what = match &start {
                Start::Fresh => "new session",
                Start::Continue => "--continue",
                Start::Prompt(_) => "--prompt with the handover",
            };
            term.feed(format!("Demo OpenCode tab ({what}); no program started\r\n").as_bytes());
            let color = self.pal().muted;
            self.tabs.open(crate::tabs::NewTab { label: label(folder), color, session: None, account: None }, term)
        } else {
            let l = launch(&program()?, folder, &start);
            self.open_tab(&l, None, None).map_err(|e| format!("Could not start OpenCode: {e}"))?
        };
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.opencode = Some(folder.to_owned());
        }
        Ok(id)
    }

    /// "In OpenCode" in the handover preview: a new OpenCode tab in the session's folder with the
    /// handover as its first prompt, beside the source session's tab. The Claude session is left as
    /// it is and no provenance is recorded (there is no new Claude session to link).
    pub fn continue_in_opencode(&mut self, session: &Session, text: &str, saved: &Path) -> Result<u64, String> {
        if !self.source.is_demo() && !session.cwd.is_dir() {
            return Err("The original session folder is not available.".into());
        }
        let prompt = first_prompt(text, saved)?;
        let tab = self.open_opencode_tab(&session.cwd, Start::Prompt(prompt))?;
        if let Some(source) = self.tabs.by_session(&session.id, |t| t.id != tab).map(|t| t.id) {
            self.tabs.place_after(tab, source);
        }
        Ok(tab)
    }
}

#[cfg(test)]
mod tests;
