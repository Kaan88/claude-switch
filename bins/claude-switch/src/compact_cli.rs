//! `compact` is dispatched before the window or any window state is created.
use crate::{compact_open, compact_runner};
use claude_switch_core::{Roots, Session, compact, config, marks::Marks, sessions};
use std::io::{self, Write};
use std::path::Path;

mod window;

const USAGE: &str = "Usage: claude-switch compact --session <id|latest> [--cwd <folder>] \
    [--model <provider/model>] [--as <account>] [--focus-file <file>] [--print | --open]";

#[derive(Default, Debug)]
struct Options {
    session: String,
    cwd: Option<String>,
    model: Option<String>,
    account: Option<String>,
    focus_file: Option<String>,
    open: bool,
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
            "--focus-file" => &mut options.focus_file,
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
    let text =
        compact_runner::make(&program, model, &data, focus.as_deref().or_else(|| data.focus()).unwrap_or(""), || {
            false
        })?;
    let file = compact::save(&roots, &session.id, &text).map_err(|e| format!("Could not keep the handover: {e}"))?;
    if !options.open {
        return io::stdout().lock().write_all(text.as_bytes()).map_err(|e| format!("Could not write stdout: {e}"));
    }
    compact_open::validate_prompt(&text)?;
    let exe = std::env::current_exe().map_err(|_| "Could not locate Claude Switch.")?;
    let name = file.file_name().and_then(|n| n.to_str()).ok_or("Invalid saved handover path.")?;
    window::spawn(&exe, &session.id, account, name)?;
    eprintln!("Claude Switch: opened the handover tab in a new window.");
    Ok(())
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

/// Only app-owned handovers are accepted by the window handoff, never a caller-selected file path.
pub fn handoff(roots: &Roots, args: (String, String, String)) -> Result<compact_open::NewHandover, String> {
    let (id, account, name) = args;
    let config = config::load(roots);
    let file = compact::list(roots, &id)
        .map_err(|_| "Could not list saved handovers.")?
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == name.as_str()))
        .ok_or("The saved handover was not found.")?;
    let session =
        sessions::scan(roots, &config).into_iter().find(|s| s.id == id).ok_or("The original session was not found.")?;
    let text = std::fs::read_to_string(&file).map_err(|_| "Could not read the saved handover.")?;
    compact_open::prepare(roots, &config, &session, name_of(roots, &session), &account, &text, &file)
}

pub fn window_args(args: &mut impl Iterator<Item = String>) -> Option<(String, String, String)> {
    Some((args.next()?, args.next()?, args.next()?))
}

pub(crate) fn acknowledge(key: &str, opened: bool) {
    window::acknowledge(key, opened);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
