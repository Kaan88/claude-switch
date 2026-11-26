//! One new native Claude process, with a preallocated UUID for its transcript and provenance.
use crate::{compact_runner, source::Source, state::AppState};
use claude_switch_core::{App, Config, Launch, Roots, Session, accounts, launch};
use std::path::PathBuf;

pub struct NewHandover {
    pub launch: Launch,
    pub id: String,
    pub account: String,
    pub from: String,
}

pub fn validate_prompt(text: &str) -> Result<(), String> {
    // Windows quotes an argument containing whitespace and can double slashes and quotes.
    // 15,000 UTF-16 units leaves >2,700 units of overhead even at 2x expansion.
    if text.encode_utf16().count() > 15_000 {
        return Err("The handover exceeds 15,000 UTF-16 units. Shorten it, or save it for manual use.".into());
    }
    if text.trim().is_empty() || text.trim_start().starts_with(['/', '!']) {
        return Err("The first prompt must be nonempty text, not a slash or shell command.".into());
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')) {
        return Err("The handover contains an unsupported control character.".into());
    }
    Ok(())
}

fn session_id() -> Result<String, String> {
    let mut b = compact_runner::nonce()?;
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let mut out = String::with_capacity(36);
    for (i, b) in b.into_iter().enumerate() {
        use std::fmt::Write as _;
        if [4, 6, 8, 10].contains(&i) {
            out.push('-');
        }
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

pub fn prepare(
    roots: &Roots,
    config: &Config,
    session: &Session,
    from: String,
    account: &str,
    text: &str,
) -> Result<NewHandover, String> {
    validate_prompt(text)?;
    if !session.cwd.is_dir() {
        return Err("The original session folder is not available.".into());
    }
    let program = compact_runner::native_program("claude")
        .ok_or("A native claude.exe on PATH is required to pass a multiline prompt safely.")?;
    let selected = claude_switch_core::account(config, account).map_err(|e| e.to_string())?;
    if !accounts::status(roots, selected).cli_logged_in {
        return Err(format!("Sign in to Claude Code as {} first.", selected.name));
    }
    let id = session_id()?;
    let mut launch = launch::new_session(roots, config, account, App::Cli, &session.cwd).map_err(|e| e.to_string())?;
    launch.program = program;
    launch.args = vec!["--session-id".into(), id.clone(), "--".into(), text.into()];
    Ok(NewHandover { launch, id, account: account.into(), from })
}

impl AppState {
    pub fn open_handover_tab(&mut self, request: NewHandover) -> Result<(), String> {
        let Source::Real(roots) = &self.source else { return Ok(()) };
        claude_switch_core::account(&self.config, &request.account).map_err(|e| e.to_string())?;
        if self.marks_read_only {
            return Err("Repair marks.json before starting a handover session.".into());
        }
        // Persist under the exact UUID, never infer it from file creation time.
        let old = self.marks.clone();
        if !self.marks.set_handover_from(&request.id, &request.from) {
            return Err("Could not record the new session's provenance.".into());
        }
        if let Err(e) = self.marks.save(roots) {
            self.marks = old;
            return Err(e.to_string());
        }
        let result = self.open_tab(&request.launch, Some(&request.id), Some(&request.account));
        if result.is_err() {
            // Roll back only the mark we added. The original session was never touched.
            self.marks.clear_handover_from(&request.id);
            self.save_marks();
        }
        result.map(drop).map_err(|_| "Could not start Claude Code in a terminal tab.".into())
    }
}

pub fn transcript_path(config: &Config, request: &Launch, account: &str, id: &str) -> Option<PathBuf> {
    let cwd = request.cwd.as_deref()?;
    let account = claude_switch_core::account(config, account).ok()?;
    Some(crate::view::handover_transcript(&account.cli_dir, cwd, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_prompt_checks_windows_length_and_control_input() {
        assert!(validate_prompt(&"😺".repeat(7500)).is_ok());
        assert!(validate_prompt(&"😺".repeat(7501)).is_err());
        assert!(validate_prompt("## Goal\n\nThe file is \"a file\".\r\n").is_ok());
        assert!(validate_prompt(" /resume something").is_err());
        assert!(validate_prompt("\0").is_err());
    }

    #[test]
    fn allocator_returns_distinct_version_four_identifiers() {
        let a = session_id().unwrap();
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(a, session_id().unwrap());
    }
}
