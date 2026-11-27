//! One new native Claude process, with a preallocated UUID for its transcript and provenance.
use crate::{
    compact_runner,
    source::Source,
    state::{AppState, Msg},
};
use claude_switch_core::marks::Marks;
use claude_switch_core::{App, Config, Launch, Roots, Session, accounts, compact, launch};
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub(crate) mod names;

pub struct NewHandover {
    pub launch: Launch,
    pub id: String,
    pub account: String,
    pub from: String,
    pub parent: String,
    name: String,
    expected_marks: Option<Marks>,
}

impl NewHandover {
    pub fn with_agent(mut self, agent: Option<&str>) -> Result<Self, String> {
        add_agent(&mut self.launch.args, agent)?;
        Ok(self)
    }

    pub fn expecting_marks(mut self, marks: Marks) -> Self {
        self.expected_marks = Some(marks);
        self
    }
}

fn add_name(args: &mut Vec<String>, name: &str) -> Result<String, String> {
    let name = names::validate(name)?.to_owned();
    let boundary = args.iter().position(|arg| arg == "--").ok_or("Missing first-prompt argument boundary.")?;
    // Bind the value to the option so even an option-looking title remains data.
    args.insert(boundary, format!("--name={name}"));
    Ok(name)
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
    Ok(vec!["--session-id".into(), id.into(), "--".into(), first_prompt(text, saved)?])
}

/// The first message: the handover itself, or above the command-line limit a request to read
/// the saved file (the same rule for a new process and for a session continued in place).
pub fn first_prompt(text: &str, saved: &Path) -> Result<String, String> {
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
    Ok(prompt)
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
    (from, name): (String, &str),
    account: &str,
    text: &str,
    saved: &Path,
) -> Result<NewHandover, String> {
    let id = session_id()?;
    let mut args = first_args(&id, text, saved)?;
    let name = add_name(&mut args, name)?;
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
    launch.label = name.clone();
    Ok(NewHandover {
        launch,
        id,
        account: account.into(),
        from,
        parent: session.id.clone(),
        name,
        expected_marks: None,
    })
}

impl AppState {
    pub fn open_handover_tab(&mut self, request: NewHandover) -> Result<(), String> {
        let Source::Real(roots) = &self.source else { return Ok(()) };
        claude_switch_core::account(&self.config, &request.account).map_err(|e| e.to_string())?;
        if self.marks_read_only {
            return Err("Repair marks.json before starting a handover session.".into());
        }
        if request.expected_marks.as_ref().is_some_and(|marks| !marks.unchanged(roots)) {
            return Err("Session names changed while opening the handover. Refresh and try again.".into());
        }
        // Persist under the exact UUID, never infer it from file creation time.
        let old = self.marks.clone();
        let earlier = self.marks.handed_over_to(&request.parent).map(str::to_owned);
        if !self.marks.set_handover_from(&request.id, &request.from)
            || !self.marks.set_parent(&request.id, &request.parent)
            || !self.marks.set_handover_launch_name(&request.id, &request.name, claude_switch_core::usage::now_ms())
            || !self.marks.set_handed_over_to(&request.parent, &request.id)
        {
            self.marks = old;
            return Err("Could not record the new session's provenance.".into());
        }
        if let Err(e) = self.marks.save(roots) {
            self.marks = old;
            return Err(e.to_string());
        }
        let result = self.open_tab(&request.launch, Some(&request.id), Some(&request.account));
        if let Ok(tab) = result
            && let Some(source) = self.tabs.by_session(&request.parent, |t| t.id != tab).map(|t| t.id)
        {
            self.tabs.place_after(tab, source);
        }
        if result.is_err() {
            // Roll back only the mark we added. The original session was never touched.
            self.marks.clear_handover_from(&request.id);
            self.marks.clear_parent(&request.id);
            self.marks.clear_handover_launch_name(&request.id);
            // An earlier handover of the same session keeps its mark.
            match &earlier {
                Some(to) => _ = self.marks.set_handed_over_to(&request.parent, to),
                None => self.marks.clear_handed_over_to(&request.parent),
            }
            self.save_marks();
        }
        result.map(drop).map_err(|_| "Could not start Claude Code in a terminal tab.".into())
    }
}

impl AppState {
    /// Lets a later `claude-switch compact --open` of this user open its tab here.
    pub fn serve_handovers(&mut self) {
        if let Some(reason) = self.compact.listen_failed.take() {
            self.warn(
                "Handovers from the command line cannot reach this window",
                format!(
                    "{reason} `claude-switch compact --open` and /switch-compact will start another window instead."
                ),
            );
        }
        let (Source::Real(roots), Some(proxy)) = (&self.source, self.proxy.clone()) else { return };
        let Some(listener) = self.compact.listener.take() else { return };
        let roots = roots.clone();
        listener.serve(move |bytes| {
            use crate::handover_pipe::Outcome;
            let post = |msg: Msg| -> Result<(), String> {
                proxy.lock().map_err(|_| "Claude Switch is closing.")?.send(msg);
                Ok(())
            };
            // A background handover's failure or note: shown as a notice, nothing else.
            if let Some(notice) = crate::compact_cli::decode_notice(&bytes) {
                let result = notice.and_then(|n| post(Msg::HandoverNotice(n)).map(|()| Outcome::Shown));
                return crate::handover_pipe::reply(&result);
            }
            let result =
                match crate::compact_cli::decode_request(&bytes).and_then(|a| crate::compact_cli::load(&roots, a)) {
                    // Shown here as well, so every `failed:` answer is on screen (the caller may run
                    // in the background, with nobody reading its output).
                    Err(e) => post(Msg::HandoverNotice(crate::compact_cli::Notice::Failed(e.clone()))).and(Err(e)),
                    Ok(request) => (|| {
                        let (tx, rx) = std::sync::mpsc::channel();
                        post(Msg::Handover(Box::new(request), tx))?;
                        rx.recv_timeout(crate::handover_pipe::REPLY_WAIT).map_err(|_| {
                        "Timed out waiting for the window. The outcome is unknown; check for the tab before retrying."
                    })?
                    })(),
                };
            crate::handover_pipe::reply(&result)
        });
    }

    /// A request from another process, read by the pipe thread: in place in the tab that runs
    /// the session (as its account, when `--as` named one), else in a new tab.
    pub fn handover_requested(
        &mut self,
        request: crate::compact_cli::Loaded,
    ) -> Result<crate::handover_pipe::Outcome, String> {
        use crate::handover_pipe::Outcome;
        let account = request.explicit_account.then_some(request.account.as_str());
        let target = request.basis.and_then(|basis| Some((self.in_place_target(&request.session.id, account)?, basis)));
        let result = if let Some((tab, basis)) = target {
            let names = (request.from.clone(), request.name.clone());
            self.continue_in_place(tab, &request.session, names, &request.text, (request.file.clone(), basis))
                .map(|()| Outcome::InPlace)
        } else {
            let Source::Real(roots) = &self.source else { return Err("Demo".into()) };
            let roots = roots.clone();
            request.new_tab(&roots, &self.config).and_then(|r| self.open_handover_tab(r)).map(|()| Outcome::NewTab)
        };
        if let Err(e) = &result {
            self.error("Could not open handover", e.clone());
        }
        result
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
    fn fake_claude_gets_one_literal_name_and_short_text_or_reads_the_full_saved_handover() {
        let fake = fixture::fake_program();
        let root = fake.with_file_name("test home with spaces");
        let roots = Roots { home: root.clone(), appdata: root };
        let mut previous = None;
        for (text, from_file, agent) in [
            ("## Short\n\nText with \"quotes\" and \\slashes.\n😺".to_owned(), false, None),
            ("## Long\n\nlong private marker 😺\n".repeat(1000), true, Some("workshop:nested:reviewer")),
        ] {
            let saved = keep(&roots, "source-id", &text, previous.as_deref()).unwrap();
            assert_ne!(previous.as_ref(), Some(&saved), "edited text needs a new revision");
            assert_eq!(keep(&roots, "source-id", &text, Some(&saved)).unwrap(), saved);
            assert_eq!(std::fs::read_to_string(&saved).unwrap(), text);
            let mut spec = launch::shell(fake.parent().unwrap());
            spec.program = fake.clone();
            spec.args = first_args("fake-id", &text, &saved).unwrap();
            let name = r#"--not-an-option "Review & compare" C:\work\ 😺"#;
            assert_eq!(add_name(&mut spec.args, name).unwrap(), name);
            spec.env.insert("FAKE_CLAUDE_NAME".into(), name.into());
            add_agent(&mut spec.args, agent).unwrap();
            assert_eq!(spec.args.last().unwrap() != &text, from_file);
            spec.env_remove.push("FAKE_CLAUDE_AGENT".into());
            if let Some(agent) = agent {
                spec.env.insert("FAKE_CLAUDE_AGENT".into(), agent.into());
            }
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
    fn failed_spawn_rolls_back_the_pending_name_and_only_the_new_provenance() {
        let root = std::env::temp_dir().join(format!("cs-handover-name-{}", session_id().unwrap()));
        let roots = Roots { home: root.join("home"), appdata: root.join("appdata") };
        let mut st = AppState::new(Source::Real(roots.clone()), None, Default::default());
        let parent = session_id().unwrap();
        let id = session_id().unwrap();
        let earlier = session_id().unwrap();
        assert!(st.marks.rename(&parent, Path::new("unused"), "Source name"));
        assert!(st.marks.set_handed_over_to(&parent, &earlier), "handed over once before");
        st.marks.save(&roots).unwrap();
        let account = st.config.current.clone();
        let mut launch = launch::shell(&root);
        launch.program = root.join("not-a-cli.exe");
        let request = NewHandover {
            launch,
            id: id.clone(),
            parent: parent.clone(),
            from: "Source name".into(),
            name: "Source name · continued".into(),
            account,
            expected_marks: None,
        };
        assert!(st.open_handover_tab(request).unwrap_err().contains("Could not start"));
        let marks = Marks::load(&roots).unwrap();
        assert_eq!(marks.name(&parent), Some("Source name"));
        assert_eq!(marks.handover_from(&id), None);
        assert_eq!(marks.parent(&id), None);
        assert_eq!(marks.handover_launch_names(claude_switch_core::usage::now_ms()).count(), 0);
        assert_eq!(marks.handed_over_to(&parent), Some(earlier.as_str()), "the earlier handover's mark stays");
        std::fs::remove_dir_all(root).unwrap();
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
