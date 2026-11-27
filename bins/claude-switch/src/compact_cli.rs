//! `compact` is dispatched before the window or any window state is created.
use crate::{
    compact_open::{self, names},
    compact_runner, handover_pipe,
};
use claude_switch_core::{Roots, Session, compact, config, marks::Marks, sessions};
use std::io::{self, Write};
use std::path::Path;

mod background;
mod window;

pub use background::{Notice, clean_old_jobs, decode_notice};

const USAGE: &str = "Usage: claude-switch compact --session <id|latest> [--cwd <folder>] \
    [--model <provider/model>] [--as <account>] [--focus-file <file>] \
    [--print | --open [--agent <name>] [--name <title>] [--background]] [--save]";

#[derive(Default, Debug)]
struct Options {
    session: String,
    cwd: Option<String>,
    model: Option<String>,
    account: Option<String>,
    agent: Option<String>,
    name: Option<String>,
    focus_file: Option<String>,
    /// Focus text already read (a background job's), used instead of `focus_file`.
    focus: Option<String>,
    open: bool,
    save: bool,
    /// Make and open the handover in a detached process and return at once.
    background: bool,
}

/// Why a run failed, and whether the running window has shown it already.
#[derive(Debug)]
struct Failed {
    message: String,
    shown: bool,
}

impl From<String> for Failed {
    fn from(message: String) -> Self {
        Failed { message, shown: false }
    }
}

impl From<&str> for Failed {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut session = None;
    let mut mode = None;
    while let Some(flag) = args.next() {
        if let Some(name) = flag.strip_prefix("--name=") {
            if options.name.replace(name.to_owned()).is_some() {
                return Err("Duplicate compact option.".into());
            }
            continue;
        }
        let field = match flag.as_str() {
            "--session" => &mut session,
            "--cwd" => &mut options.cwd,
            "--model" => &mut options.model,
            "--as" => &mut options.account,
            "--agent" => &mut options.agent,
            "--name" => &mut options.name,
            "--focus-file" => &mut options.focus_file,
            "--save" if !options.save => {
                options.save = true;
                continue;
            }
            "--background" if !options.background => {
                options.background = true;
                continue;
            }
            "--print" | "--open" if mode.is_none() => {
                mode = Some(flag == "--open");
                continue;
            }
            _ => return Err("Unknown, duplicate or conflicting compact option.".into()),
        };
        if field.is_some() {
            return Err("Duplicate compact option.".into());
        }
        *field = Some(
            args.next()
                .filter(|s| !s.is_empty() && !s.starts_with("--"))
                .ok_or("A compact option is missing its value.")?,
        );
    }
    options.session = session.ok_or("Specify --session <id|latest>.")?;
    options.open = mode.unwrap_or(false);
    if options.background && !options.open {
        return Err("--background makes and opens the handover in the background and requires --open.".into());
    }
    if let Some(name) = &options.agent {
        claude_switch_core::launch::session_agent_args(name).map_err(|error| error.to_string())?;
        if !options.open {
            return Err("--agent selects the new session's agent and requires --open.".into());
        }
    }
    if let Some(name) = &mut options.name {
        if !options.open {
            return Err("--name selects the new session's name and requires --open.".into());
        }
        *name = names::validate(name)?.to_owned();
    }
    Ok(options)
}

fn find<'a>(sessions: &'a [Session], id: &str, cwd: Option<&Path>) -> Option<&'a Session> {
    let cwd = cwd.map(sessions::folder_key);
    let by_folder = |s: &&Session| cwd.as_ref().is_none_or(|key| *key == sessions::folder_key(&s.cwd));
    let mut all = sessions.iter().filter(by_folder);
    if id == "latest" {
        all.max_by(|a, b| a.transcript_ms.cmp(&b.transcript_ms).then_with(|| b.id.cmp(&a.id)))
    } else {
        all.find(|s| s.id == id)
    }
}

fn name_of(roots: &Roots, session: &Session) -> String {
    Marks::load(roots)
        .ok()
        .and_then(|marks| marks.name(&session.id).map(str::to_owned))
        .unwrap_or_else(|| session.title.clone())
}

fn focus_from_file(path: &Path) -> Result<String, String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|_| "Could not open the focus file.")?;
    let mut text = String::new();
    file.take(64_001).read_to_string(&mut text).map_err(|_| "The focus file is not readable UTF-8.")?;
    if text.len() > 64_000 {
        return Err("The focus file exceeds 64,000 bytes.".into());
    }
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

/// Everything checked before any transcript is read: consent, OpenCode, the session, the account.
struct Ready {
    roots: Roots,
    config: claude_switch_core::Config,
    model: String,
    program: std::path::PathBuf,
    session: Session,
    account: String,
}

fn ready(options: &Options) -> Result<Ready, String> {
    let roots = Roots::from_env().ok_or("USERPROFILE or APPDATA is not set.")?;
    let config = config::load(&roots);
    let model = options.model.clone().unwrap_or_else(|| config.compact.model.clone());
    let provider = compact::provider(&model).ok_or("Choose a provider/model in the window or specify --model.")?;
    if !config.compact.consents.contains(provider) {
        return Err(format!(
            "No consent for {provider}. Run Compact and open once from the window and approve this provider."
        ));
    }
    let program = compact_runner::opencode()?;
    let cwd = match &options.cwd {
        Some(cwd) => Some(std::path::absolute(cwd).map_err(|_| "Invalid working folder.")?),
        None if options.session == "latest" => {
            Some(std::env::current_dir().map_err(|_| "Could not read the working folder.")?)
        }
        None => None,
    };
    let sessions = sessions::scan(&roots, &config);
    let session = find(&sessions, &options.session, cwd.as_deref()).ok_or("No matching transcript was found.")?.clone();
    let account = options.account.clone().unwrap_or_else(|| config.current.clone());
    if options.open || options.account.is_some() {
        claude_switch_core::account(&config, &account).map_err(|e| e.to_string())?;
    }
    Ok(Ready { roots, config, model, program, session, account })
}

/// The note that the Settings default agent was dropped, for a new tab, is returned for the
/// background process to show; the foreground has printed it.
fn run(options: Options) -> Result<Option<String>, Failed> {
    let Ready { roots, config, model, program, session, account } = ready(&options)?;
    let account = account.as_str();
    let data = compact::Handover::read(session.clone(), name_of(&roots, &session))
        .map_err(|e| format!("Could not read the transcript: {e}"))?;
    // An empty focus file (the plugin with no focus text) means no focus of the user's own.
    let own = match options.focus.clone() {
        Some(text) => Some(text),
        None => options.focus_file.as_deref().map(Path::new).map(focus_from_file).transpose()?,
    };
    let focus = match own.filter(|text| !text.trim().is_empty()) {
        Some(text) => text,
        None => data.chosen_focus(&config.compact).1,
    };
    let text = compact_runner::make(&roots, &program, &model, &data, &focus, || false)?;
    if !options.open {
        print(&roots, &session.id, &text, options.save, io::stdout().lock())?;
        return Ok(None);
    }
    compact_open::validate_prompt(&text)?;
    let file = compact::save(&roots, &session.id, &text).map_err(|e| format!("Could not keep the handover: {e}"))?;
    let exe = std::env::current_exe().map_err(|_| "Could not locate Claude Switch.")?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or("Invalid saved handover path.")?;
    let (agent, dropped) = resolve_agent(&roots, &config, account, &session.cwd, options.agent.as_deref());
    let request = Handoff {
        id: session.id.clone(),
        account: account.to_owned(),
        file: name.to_owned(),
        agent: agent.map(str::to_owned),
        name: options.name.clone(),
        explicit_account: options.account.is_some(),
        basis: Some(data.basis()),
    };
    match handover_pipe::send(&encode_request(&request)?) {
        handover_pipe::Sent::Shown(message) => return Err(Failed { message, shown: true }),
        handover_pipe::Sent::Answered(Err(message)) => return Err(message.into()),
        handover_pipe::Sent::Answered(Ok(handover_pipe::Outcome::NewTab)) => {
            eprintln!("Claude Switch: opened the handover tab in the running window.{}", note(dropped.as_deref()));
        }
        handover_pipe::Sent::Answered(Ok(handover_pipe::Outcome::InPlace)) => {
            eprintln!(
                "Claude Switch: continuing in this session's tab (/clear, then the handover as the first message)."
            );
            return Ok(None);
        }
        handover_pipe::Sent::Answered(Ok(handover_pipe::Outcome::Shown)) => {
            return Err("The running Claude Switch did not confirm opening a Claude tab.".into());
        }
        handover_pipe::Sent::NoAnswer(message) => return Err(message.into()),
        handover_pipe::Sent::NotRunning => {
            window::spawn(&exe, &session.id, account, name, agent, options.name.as_deref())?;
            eprintln!("Claude Switch: started Claude Switch with the handover tab.{}", note(dropped.as_deref()));
        }
    }
    Ok(dropped)
}

fn note(dropped: Option<&str>) -> String {
    dropped.map(|notice| format!(" {notice}")).unwrap_or_default()
}

/// An explicit `--agent` is passed as given; the Settings default only when the target account
/// and folder have it (otherwise none, and the notice).
fn resolve_agent<'a>(
    roots: &Roots,
    config: &'a claude_switch_core::Config,
    account: &str,
    cwd: &Path,
    explicit: Option<&'a str>,
) -> (Option<&'a str>, Option<String>) {
    if explicit.is_some() {
        return (explicit, None);
    }
    let Some(default) = config.compact.default_agent.as_deref() else { return (None, None) };
    let Ok(target) = claude_switch_core::account(config, account) else { return (None, None) };
    let found = crate::compact_dialog::agents::discover(roots, target, cwd, || false);
    match crate::compact_dialog::agents::checked_default(default, &found, &target.name) {
        Ok(()) => (Some(default), None),
        Err(notice) => (None, Some(notice)),
    }
}

fn print(roots: &Roots, id: &str, text: &str, save: bool, mut out: impl Write) -> Result<(), String> {
    if save {
        compact::save(roots, id, text).map_err(|e| format!("Could not keep the handover: {e}"))?;
    }
    out.write_all(text.as_bytes()).map_err(|e| format!("Could not write stdout: {e}"))
}

pub fn entry(args: impl Iterator<Item = String>) -> i32 {
    let mut args = args.peekable();
    if args.next_if(|word| word == background::JOB_FLAG).is_some() {
        return background::run_job(args);
    }
    match parse(args) {
        Ok(options) if options.background => match background::start(options) {
            Ok(line) => {
                eprintln!("{line}");
                0
            }
            Err(e) => {
                eprintln!("claude-switch compact: {e}");
                1
            }
        },
        Ok(options) => match run(options) {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("claude-switch compact: {}", e.message);
                1
            }
        },
        Err(e) => {
            eprintln!("claude-switch compact: {e}\n{USAGE}");
            2
        }
    }
}

pub struct Handoff {
    id: String,
    account: String,
    file: String,
    agent: Option<String>,
    name: Option<String>,
    /// `--as` was given: continue only in a tab of that account, else open a new tab.
    explicit_account: bool,
    /// The transcript's length when the handover was made (`compact::Handover::basis`); without
    /// it the window never continues in place.
    basis: Option<u64>,
}

const REQUEST_VERSION: &str = "claude-switch-handover-3";

/// The running window's request: the same validated words as the window arguments, one a line.
pub fn encode_request(request: &Handoff) -> Result<Vec<u8>, String> {
    window::words(&request.id, &request.account, &request.file, request.agent.as_deref())?;
    let name = request.name.as_deref().map(names::encode).transpose()?;
    let lines = [
        REQUEST_VERSION,
        &request.id,
        &request.account,
        &request.file,
        request.agent.as_deref().unwrap_or("-"),
        name.as_deref().unwrap_or("-"),
        if request.explicit_account { "as" } else { "-" },
        &request.basis.map_or_else(|| "-".to_owned(), |n| n.to_string()),
    ];
    let bytes = lines.join("\n").into_bytes();
    if bytes.len() > handover_pipe::MAX_REQUEST {
        return Err("The handover request is too long.".into());
    }
    Ok(bytes)
}

/// Revalidates everything: the sender is the same user, but not trusted to have checked.
pub fn decode_request(bytes: &[u8]) -> Result<Handoff, String> {
    let invalid = "Invalid handover request.";
    let text = std::str::from_utf8(bytes).map_err(|_| invalid)?;
    let lines: Vec<&str> = text.split('\n').collect();
    let [version, id, account, file, agent, name, explicit, basis] = lines[..] else { return Err(invalid.into()) };
    if version != REQUEST_VERSION || !matches!(explicit, "as" | "-") {
        return Err(invalid.into());
    }
    let basis = match basis {
        "-" => None,
        n if !n.is_empty() && n.len() <= 20 && n.bytes().all(|b| b.is_ascii_digit()) => {
            Some(n.parse::<u64>().map_err(|_| invalid)?)
        }
        _ => return Err(invalid.into()),
    };
    let agent = (agent != "-").then(|| agent.to_owned());
    window::words(id, account, file, agent.as_deref())?;
    let name = (name != "-").then(|| names::decode(name)).transpose()?;
    Ok(Handoff {
        id: id.into(),
        account: account.into(),
        file: file.into(),
        agent,
        name,
        explicit_account: explicit == "as",
        basis,
    })
}

/// Only app-owned handovers are accepted by the window handoff, never a caller-selected file path.
/// A requested handover with everything read from disk (off the UI thread), before the window
/// decides whether it continues in place or in a new tab.
pub struct Loaded {
    pub session: Session,
    pub from: String,
    pub name: String,
    pub text: String,
    pub file: std::path::PathBuf,
    pub account: String,
    pub explicit_account: bool,
    pub basis: Option<u64>,
    agent: Option<String>,
    marks: Option<Marks>,
}

impl Loaded {
    pub fn new_tab(
        self,
        roots: &Roots,
        config: &claude_switch_core::Config,
    ) -> Result<compact_open::NewHandover, String> {
        let Loaded { session, from, name, text, file, account, agent, marks, .. } = self;
        let request = compact_open::prepare(roots, config, &session, (from, &name), &account, &text, &file)?
            .with_agent(agent.as_deref())?;
        Ok(match marks {
            Some(marks) => request.expecting_marks(marks),
            None => request,
        })
    }
}

pub fn handoff(roots: &Roots, args: Handoff) -> Result<compact_open::NewHandover, String> {
    load(roots, args)?.new_tab(roots, &config::load(roots))
}

pub fn load(roots: &Roots, args: Handoff) -> Result<Loaded, String> {
    let Handoff { id, account, file, agent, name, explicit_account, basis } = args;
    let config = config::load(roots);
    let file = compact::list(roots, &id)
        .map_err(|_| "Could not list saved handovers.")?
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == file.as_str()))
        .ok_or("The saved handover was not found.")?;
    let sessions = sessions::scan(roots, &config);
    let session = sessions.iter().find(|s| s.id == id).ok_or("The original session was not found.")?;
    let marks = Marks::load(roots).map_err(|e| e.to_string())?;
    let from = names::displayed(&marks, session).to_owned();
    let automatic = name.is_none();
    let name = name.unwrap_or_else(|| names::suggest(&marks, &sessions, &from));
    let text = std::fs::read_to_string(&file).map_err(|_| "Could not read the saved handover.")?;
    Ok(Loaded {
        session: session.clone(),
        from,
        name,
        text,
        file,
        account,
        explicit_account,
        basis,
        agent,
        marks: automatic.then_some(marks),
    })
}

pub fn window_args(mut args: impl Iterator<Item = String>) -> Result<(Handoff, String), String> {
    let invalid = "Invalid handover window arguments.";
    let (id, account, file, ack) = (
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
    );
    let mut agent = args.next();
    let name = if agent.as_deref().is_some_and(|word| word.starts_with("name=")) { agent.take() } else { args.next() };
    if args.next().is_some() || !window::valid_ack(&ack) {
        return Err(invalid.into());
    }
    window::words(&id, &account, &file, agent.as_deref())?;
    let name = name.map(|word| names::decode(word.strip_prefix("name=").ok_or(invalid)?)).transpose()?;
    Ok((Handoff { id, account, file, agent, name, explicit_account: false, basis: None }, ack))
}

/// The window started for a handover gives it to a Switch that started meanwhile, rather than
/// becoming a second one. True when that window answered (the answer is acknowledged).
pub fn forward_to_running(request: &Handoff, ack: &str) -> bool {
    let Ok(payload) = encode_request(request) else { return false };
    match handover_pipe::send(&payload) {
        handover_pipe::Sent::NotRunning => false,
        handover_pipe::Sent::Answered(result) => {
            if let Err(e) = &result {
                eprintln!("claude-switch compact: {e}");
            }
            acknowledge(ack, result.is_ok());
            true
        }
        handover_pipe::Sent::Shown(e) | handover_pipe::Sent::NoAnswer(e) => {
            eprintln!("claude-switch compact: {e}");
            acknowledge(ack, false);
            true
        }
    }
}

pub(crate) fn acknowledge(key: &str, opened: bool) {
    window::acknowledge(key, opened);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn cli_name_is_optional_only_for_open_and_validated_before_runtime() {
        let parse = |args: &[&str]| super::parse(args.iter().map(|arg| arg.to_string()));
        assert_eq!(parse(&["--session", "latest", "--open"]).unwrap().name, None);
        let name = "Résumé & \"notes\" 😺";
        assert_eq!(parse(&["--session", "latest", "--name", name, "--open"]).unwrap().name.as_deref(), Some(name));
        assert_eq!(
            parse(&["--session", "latest", "--open", "--name=--not-a-flag"]).unwrap().name.as_deref(),
            Some("--not-a-flag")
        );
        for bad in ["", "  ", "ends\n", "a\0b", "a\u{2028}b", &"😺".repeat(101)] {
            assert!(parse(&["--session", "latest", "--open", "--name", bad]).is_err(), "{bad:?}");
        }
        assert!(parse(&["--session", "latest", "--name", "Review"]).is_err());
        assert!(parse(&["--session", "latest", "--open", "--name", "--save"]).is_err());
        assert!(parse(&["--session", "latest", "--open", "--name=one", "--name", "two"]).is_err());
    }

    #[test]
    fn running_window_request_round_trips_validated_words_only() {
        let id = "5eed3dde-1c2d-4e5f-8a9b-000000033232";
        let make = |agent: Option<&str>, name: Option<&str>| Handoff {
            id: id.into(),
            account: "work".into(),
            file: "handover-001.md".into(),
            agent: agent.map(str::to_owned),
            name: name.map(str::to_owned),
            explicit_account: agent.is_some(),
            basis: agent.map(|_| 1234),
        };
        for (agent, name) in [(None, None), (Some("plugin:review"), Some("Dev's \"notes\" & $env:PATH 😺"))] {
            let bytes = encode_request(&make(agent, name)).unwrap();
            let back = decode_request(&bytes).unwrap();
            assert_eq!((back.id.as_str(), back.account.as_str(), back.file.as_str()), (id, "work", "handover-001.md"));
            assert_eq!(back.agent.as_deref(), agent);
            assert_eq!(back.name.as_deref(), name);
            assert_eq!(back.explicit_account, agent.is_some());
            assert_eq!(back.basis, agent.map(|_| 1234));
        }
        assert!(encode_request(&make(Some("bad agent"), None)).is_err());
        assert!(encode_request(&make(None, Some("two\nlines"))).is_err());
        let good = String::from_utf8(encode_request(&make(None, None)).unwrap()).unwrap();
        for bad in [
            good.replace("claude-switch-handover-3", "claude-switch-handover-2"),
            format!(
                "{}
12a",
                good.strip_suffix(
                    "
-"
                )
                .unwrap()
            ),
            format!(
                "{}
",
                good.strip_suffix("-").unwrap()
            ),
            format!(
                "{}
{}",
                good.strip_suffix(
                    "
-"
                )
                .unwrap(),
                "9".repeat(21)
            ),
            good.replace("\n-\n-\n-", "\n-\n-\nyes"),
            good.replace("handover-001.md", "..\\other.md"),
            good.replace("work", "work & calc"),
            format!("{good}\nextra"),
            good.replace(id, "not-a-uuid"),
            good.replace("\n-\n-", "\n--help\n-"),
            good.replace("\n-\n-", "\n-\nzz"),
            good.replace('\n', "\r\n"),
            String::new(),
        ] {
            assert!(decode_request(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert!(decode_request(b"\xff").is_err());
    }

    #[test]
    fn print_writes_only_stdout_unless_save_is_explicit() {
        let scratch = crate::testing::Scratch::new("cs-review3-print");
        let root = scratch.to_path_buf();
        std::fs::create_dir(&root).unwrap();
        let roots = Roots { home: root.clone(), appdata: root.clone() };
        let mut out = Vec::new();
        let options = parse("--session session-1 --print".split_whitespace().map(str::to_owned)).unwrap();
        print(&roots, &options.session, "edited output", options.save, &mut out).unwrap();
        assert_eq!(out, b"edited output");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let options = parse("--session session-1 --print --save".split_whitespace().map(str::to_owned)).unwrap();
        print(&roots, &options.session, "edited output", options.save, io::sink()).unwrap();
        let files = compact::list(&roots, &options.session).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "edited output");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_ambiguous_modes_and_missing_values() {
        let parse = |values: &str| super::parse(values.split_whitespace().map(str::to_owned));
        assert!(parse("--session latest --open --print").is_err());
        assert!(parse("--session latest --focus-file --open").is_err());
        assert!(parse("--session latest --model a/b --model c/d").is_err());
        assert!(parse("--session latest --open").unwrap().open);
        assert!(!parse("--session latest").unwrap().open);
    }

    #[test]
    fn agent_is_only_for_open_and_validated_before_any_runtime_work() {
        let parse = |values: &[&str]| super::parse(values.iter().map(|word| word.to_string()));
        for name in ["", "--help", "plugin::agent", "agent;other", "two words", "ş", &"x".repeat(129)] {
            assert!(parse(&["--session", "latest", "--open", "--agent", name]).is_err(), "{name:?}");
        }
        assert!(parse(&["--session", "latest", "--agent"]).is_err());
        assert!(parse(&["--session", "latest", "--print", "--agent", "review"]).is_err());
        assert!(parse(&["--session", "latest", "--open", "--agent", "review", "--agent", "other"]).is_err());
        let opts = parse(&["--session", "latest", "--agent", "plugin:nested:agent_2", "--open"]).unwrap();
        let mut prefs = compact::Preferences::default();
        assert_eq!(prefs.agent_for(None), None);
        prefs.set_default_agent("review").unwrap();
        assert_eq!(prefs.agent_for(None), Some("review"));
        assert_eq!(prefs.agent_for(opts.agent.as_deref()), Some("plugin:nested:agent_2"));
    }

    #[test]
    fn latest_uses_transcript_time_not_desktop_activity_and_honors_folder() {
        let make = |id: &str, cwd: &str, transcript_ms, last_active_ms| Session {
            id: id.into(),
            cwd: PathBuf::from(cwd),
            transcript_ms,
            last_active_ms,
            title: id.into(),
            last_prompt: String::new(),
            grouping: Default::default(),
            turn_ended: false,
            branch: None,
            bytes: 0,
            last_app: claude_switch_core::App::Cli,
            desktop_id: None,
            desktop_account: None,
            archived: false,
            worker: false,
            file: PathBuf::new(),
        };
        let all = [make("a", "C:/Work", 3, 100), make("b", "c:/work/", 4, 4), make("c", "C:/Other", 10, 10)];
        assert_eq!(find(&all, "latest", Some(Path::new("C:\\Work"))).unwrap().id, "b");
        assert!(find(&all, "c", Some(Path::new("C:\\Work"))).is_none());
    }
}
