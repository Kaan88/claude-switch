//! `compact` is dispatched before the window or any window state is created.
use crate::{compact_open, compact_runner};
use claude_switch_core::{Roots, Session, compact, config, marks::Marks, sessions};
use std::io::{self, Write};
use std::path::Path;

mod window;

const USAGE: &str = "Usage: claude-switch compact --session <id|latest> [--cwd <folder>] \
    [--model <provider/model>] [--as <account>] [--focus-file <file>] [--print | --open [--agent <name>]] [--save]";

#[derive(Default, Debug)]
struct Options {
    session: String,
    cwd: Option<String>,
    model: Option<String>,
    account: Option<String>,
    agent: Option<String>,
    focus_file: Option<String>,
    open: bool,
    save: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut session = None;
    let mut mode = None;
    while let Some(flag) = args.next() {
        let field = match flag.as_str() {
            "--session" => &mut session,
            "--cwd" => &mut options.cwd,
            "--model" => &mut options.model,
            "--as" => &mut options.account,
            "--agent" => &mut options.agent,
            "--focus-file" => &mut options.focus_file,
            "--save" if !options.save => {
                options.save = true;
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
    if let Some(name) = &options.agent {
        claude_switch_core::launch::session_agent_args(name).map_err(|error| error.to_string())?;
        if !options.open {
            return Err("--agent selects the new session's agent and requires --open.".into());
        }
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

fn run(options: Options) -> Result<(), String> {
    let roots = Roots::from_env().ok_or("USERPROFILE or APPDATA is not set.")?;
    let config = config::load(&roots);
    let model = options.model.as_deref().unwrap_or(&config.compact.model);
    let provider = compact::provider(model).ok_or("Choose a provider/model in the window or specify --model.")?;
    if !config.compact.consents.contains(provider) {
        return Err(format!(
            "No consent for {provider}. Run Compact and open once from the window and approve this provider."
        ));
    }
    let program = compact_runner::opencode()?;
    let cwd = match options.cwd {
        Some(cwd) => Some(std::path::absolute(cwd).map_err(|_| "Invalid working folder.")?),
        None if options.session == "latest" => {
            Some(std::env::current_dir().map_err(|_| "Could not read the working folder.")?)
        }
        None => None,
    };
    let sessions = sessions::scan(&roots, &config);
    let session = find(&sessions, &options.session, cwd.as_deref()).ok_or("No matching transcript was found.")?;
    let account = options.account.as_deref().unwrap_or(&config.current);
    if options.open || options.account.is_some() {
        claude_switch_core::account(&config, account).map_err(|e| e.to_string())?;
    }
    let data = compact::Handover::read(session.clone(), name_of(&roots, session))
        .map_err(|_| "Could not read the transcript.")?;
    let focus = options.focus_file.as_deref().map(Path::new).map(focus_from_file).transpose()?;
    let text = compact_runner::make(
        &roots,
        &program,
        model,
        &data,
        focus.as_deref().or_else(|| data.focus()).unwrap_or(""),
        || false,
    )?;
    if !options.open {
        return print(&roots, &session.id, &text, options.save, io::stdout().lock());
    }
    compact_open::validate_prompt(&text)?;
    let file = compact::save(&roots, &session.id, &text).map_err(|e| format!("Could not keep the handover: {e}"))?;
    let exe = std::env::current_exe().map_err(|_| "Could not locate Claude Switch.")?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or("Invalid saved handover path.")?;
    let agent = config.compact.agent_for(options.agent.as_deref());
    window::spawn(&exe, &session.id, account, name, agent)?;
    eprintln!("Claude Switch: opened the handover tab in a new window.");
    Ok(())
}

fn print(roots: &Roots, id: &str, text: &str, save: bool, mut out: impl Write) -> Result<(), String> {
    if save {
        compact::save(roots, id, text).map_err(|e| format!("Could not keep the handover: {e}"))?;
    }
    out.write_all(text.as_bytes()).map_err(|e| format!("Could not write stdout: {e}"))
}

pub fn entry(args: impl Iterator<Item = String>) -> i32 {
    match parse(args) {
        Ok(options) => match run(options) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("claude-switch compact: {e}");
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
}

/// Only app-owned handovers are accepted by the window handoff, never a caller-selected file path.
pub fn handoff(roots: &Roots, args: Handoff) -> Result<compact_open::NewHandover, String> {
    let Handoff { id, account, file, agent } = args;
    let config = config::load(roots);
    let file = compact::list(roots, &id)
        .map_err(|_| "Could not list saved handovers.")?
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == file.as_str()))
        .ok_or("The saved handover was not found.")?;
    let session =
        sessions::scan(roots, &config).into_iter().find(|s| s.id == id).ok_or("The original session was not found.")?;
    let text = std::fs::read_to_string(&file).map_err(|_| "Could not read the saved handover.")?;
    compact_open::prepare(roots, &config, &session, name_of(roots, &session), &account, &text, &file)?
        .with_agent(agent.as_deref())
}

pub fn window_args(mut args: impl Iterator<Item = String>) -> Result<(Handoff, String), String> {
    let invalid = "Invalid handover window arguments.";
    let (id, account, file, ack) = (
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
        args.next().ok_or(invalid)?,
    );
    let agent = args.next();
    if args.next().is_some() || !window::valid_ack(&ack) {
        return Err(invalid.into());
    }
    window::words(&id, &account, &file, agent.as_deref())?;
    Ok((Handoff { id, account, file, agent }, ack))
}

pub(crate) fn acknowledge(key: &str, opened: bool) {
    window::acknowledge(key, opened);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn print_writes_only_stdout_unless_save_is_explicit() {
        let root = std::env::temp_dir().join(format!("cs-review3-print-{}", std::process::id()));
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
