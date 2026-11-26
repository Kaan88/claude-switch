//! One new native Claude process, with a preallocated UUID for its transcript and provenance.
use crate::{compact_runner, source::Source, state::AppState};
use claude_switch_core::{App, Config, Launch, Roots, Session, accounts, compact, launch};
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub struct NewHandover {
    pub launch: Launch,
    pub id: String,
    pub account: String,
    pub from: String,
    pub parent: String,
}

impl NewHandover {
    pub fn with_agent(mut self, agent: Option<&str>) -> Result<Self, String> {
        add_agent(&mut self.launch.args, agent)?;
        Ok(self)
    }
}

fn add_agent(args: &mut Vec<String>, agent: Option<&str>) -> Result<(), String> {
    if let Some(agent) = agent {
        let [flag, name] = launch::session_agent_args(agent).map_err(|e| e.to_string())?;
        let boundary = args.iter().position(|arg| arg == "--").ok_or("Missing first-prompt argument boundary.")?;
        args.splice(boundary..boundary, [flag.into(), name.into()]);
    }
    Ok(())
}

pub fn validate_prompt(text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.trim_start().starts_with(['/', '!']) {
        return Err("The first prompt must be nonempty text, not a slash or shell command.".into());
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')) {
        return Err("The handover contains an unsupported control character.".into());
    }
    Ok(())
}

pub fn uses_file(text: &str) -> bool {
    // Allow for Windows quoting and the rest of the command line.
    text.encode_utf16().count() > 15_000
}

pub fn keep(roots: &Roots, id: &str, text: &str, previous: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = previous
        && compact::list(roots, id).is_ok_and(|files| files.iter().any(|f| f == path))
        && let Ok(file) = std::fs::File::open(path)
    {
        let mut bytes = Vec::new();
        if file.take(text.len() as u64 + 1).read_to_end(&mut bytes).is_ok() && bytes == text.as_bytes() {
            return Ok(path.to_owned());
        }
    }
    compact::save(roots, id, text).map_err(|e| format!("Could not keep the handover: {e}"))
}

fn first_args(id: &str, text: &str, saved: &Path) -> Result<Vec<String>, String> {
    validate_prompt(text)?;
    let prompt = if uses_file(text) {
        let path = std::path::absolute(saved).map_err(|_| "Could not resolve the absolute handover path.")?;
        let path = path
            .to_str()
            .filter(|s| !s.contains('"') && !s.chars().any(char::is_control))
            .ok_or("Could not represent the absolute handover path safely.")?;
        let prompt =
            format!("Read the handover file at \"{path}\" in full before doing anything else. Then continue from it.");
        if uses_file(&prompt) {
            return Err("The absolute handover path is too long to pass to Claude Code.".into());
        }
        prompt
    } else {
        text.into()
    };
    Ok(vec!["--session-id".into(), id.into(), "--".into(), prompt])
}

pub(crate) fn session_id() -> Result<String, String> {
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
    saved: &Path,
) -> Result<NewHandover, String> {
    let id = session_id()?;
    let args = first_args(&id, text, saved)?;
    if !session.cwd.is_dir() {
        return Err("The original session folder is not available.".into());
    }
    let program = compact_runner::native_program("claude")
        .ok_or("A native claude.exe on PATH is required to pass a multiline prompt safely.")?;
    let selected = claude_switch_core::account(config, account).map_err(|e| e.to_string())?;
    if !accounts::status(roots, selected).cli_logged_in {
        return Err(format!("Sign in to Claude Code as {} first.", selected.name));
    }
    let mut launch = launch::new_session(roots, config, account, App::Cli, &session.cwd).map_err(|e| e.to_string())?;
    launch.program = program;
    launch.args = args;
    Ok(NewHandover { launch, id, account: account.into(), from, parent: session.id.clone() })
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
        if !self.marks.set_handover_from(&request.id, &request.from)
            || !self.marks.set_parent(&request.id, &request.parent)
        {
            self.marks = old;
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
            self.marks.clear_parent(&request.id);
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

    mod fixture {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/opencode/tests/support/mod.rs"));
    }

    #[test]
    fn agent_is_one_option_before_the_literal_prompt_and_default_adds_nothing() {
        let saved = Path::new("unused.md");
        let first = "Review this text with --agent embedded";
        let mut args = first_args("new-id", first, saved).unwrap();
        add_agent(&mut args, None).unwrap();
        assert_eq!(args, ["--session-id", "new-id", "--", first]);
        for invalid in ["--help", "notes:--bad", "two words", "bad\0name", "", "notes::review"] {
            assert!(add_agent(&mut args, Some(invalid)).is_err(), "{invalid:?}");
            assert_eq!(args, ["--session-id", "new-id", "--", first]);
        }
        add_agent(&mut args, Some("plugin:core:review")).unwrap();
        assert_eq!(args, ["--session-id", "new-id", "--agent", "plugin:core:review", "--", first]);
    }

    #[test]
    fn fake_claude_gets_short_text_or_reads_full_saved_handover() {
        let fake = fixture::fake_program();
        let root = fake.with_file_name("test home with spaces");
        let roots = Roots { home: root.clone(), appdata: root };
        let mut previous = None;
        for (text, from_file) in [
            ("## Short\n\nText with \"quotes\" and \\slashes.\n😺".to_owned(), false),
            ("## Long\n\nlong private marker 😺\n".repeat(1000), true),
        ] {
            let saved = keep(&roots, "source-id", &text, previous.as_deref()).unwrap();
            assert_ne!(previous.as_ref(), Some(&saved), "edited text needs a new revision");
            assert_eq!(keep(&roots, "source-id", &text, Some(&saved)).unwrap(), saved);
            assert_eq!(std::fs::read_to_string(&saved).unwrap(), text);
            let mut spec = launch::shell(fake.parent().unwrap());
            spec.program = fake.clone();
            spec.args = first_args("fake-id", &text, &saved).unwrap();
            assert_eq!(spec.args[3] != text, from_file);
            let received = saved.with_extension("fake-result");
            spec.env.insert("FAKE_CLAUDE_OUT".into(), received.to_str().unwrap().into());
            let mut pty = mdshared_pty::Pty::spawn(&crate::term::command(&spec), 100, 30, || {}).unwrap();
            let started = std::time::Instant::now();
            let mut output = Vec::new();
            loop {
                output.extend(pty.take_output());
                if let Some(exit) = pty.exit() {
                    assert!(exit.success(), "fake Claude failed: {exit:?}: {}", String::from_utf8_lossy(&output));
                    break;
                }
                assert!(started.elapsed() < std::time::Duration::from_secs(8), "fake Claude did not exit");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert_eq!(std::fs::read_to_string(received).unwrap(), text);
            previous = Some(saved);
        }
    }

    #[test]
    fn first_prompt_checks_windows_length_and_control_input() {
        assert!(!uses_file(&"😺".repeat(7500)));
        assert!(uses_file(&"😺".repeat(7501)));
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
