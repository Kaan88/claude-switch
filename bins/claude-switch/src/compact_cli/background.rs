//! `compact --open --background`, what `/switch-compact` runs: the quick checks run at once, then
//! a detached Claude Switch process (no console window, none of the caller's handles, so the
//! caller's shell call ends) makes and opens the handover, and the caller returns. Nothing is
//! reported back into the old chat: the running window shows the tab, or the failure as a
//! notice; with no window to show it, a message box does.
use super::{Failed, Options};
use crate::handover_pipe::{self, Outcome, Sent};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The hidden option the detached process is started with; its one value names the job file.
pub(super) const JOB_FLAG: &str = "--background-job";
const PREFIX: &str = "cs-handover-job-";
const MAX_JOB: u64 = 256 * 1024;
const MAX_ARGS: usize = 16;
const FAILED_VERSION: &str = "claude-switch-handover-failed-1";
/// The notice's text, in bytes (the request stays under `handover_pipe::MAX_REQUEST`).
const MAX_NOTICE: usize = 1500;

/// Checks what can be checked quickly, hands the rest to a detached process and returns the
/// one line for the caller.
pub(super) fn start(options: Options) -> Result<String, String> {
    let ready = super::ready(&options)?;
    // Read now: the plugin removes its focus file as soon as this returns.
    let focus = options.focus_file.as_deref().map(Path::new).map(super::focus_from_file).transpose()?;
    let label = super::name_of(&ready.roots, &ready.session);
    let job = serde_json::json!({
        "args": job_args(&options, &ready.session.id),
        "focus": focus,
        "label": label,
    });
    let key = new_key()?;
    let path = job_path(&key).ok_or("Invalid background job name.")?;
    write_new(&path, job.to_string().as_bytes()).map_err(|_| "Could not write the background handover job.")?;
    let started = std::env::current_exe().map_err(|_| "Could not locate Claude Switch.".to_owned()).and_then(|exe| {
        let until = Instant::now() + Duration::from_secs(30);
        super::window::spawn_window(&exe, &format!("compact {JOB_FLAG} {key}"), until)
            .map_err(|_| "Could not start the background handover.".to_owned())
    });
    if let Err(e) = started {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(format!(
        "Claude Switch is making the handover of \u{201c}{label}\u{201d} in the background and will show the \
         result in its window. Nothing more to do in this chat."
    ))
}

/// The detached process: runs the job and shows a failure the window has not shown.
pub(super) fn run_job(mut args: impl Iterator<Item = String>) -> i32 {
    let (key, extra) = (args.next(), args.next());
    let job = match key.filter(|_| extra.is_none()) {
        Some(key) => read_job(&key),
        None => Err("Invalid background handover job.".into()),
    };
    let (options, label) = match job {
        Ok(job) => job,
        Err(e) => {
            report(&e);
            return 2;
        }
    };
    match super::run(options) {
        Ok(()) => 0,
        Err(Failed { shown: true, .. }) => 1,
        Err(Failed { message, .. }) => {
            report(&format!("\u{201c}{label}\u{201d}: {message}"));
            1
        }
    }
}

/// The same options as given, with the session fixed to the one found (`latest` could change)
/// and without the focus file, background flag or working folder.
fn job_args(options: &Options, id: &str) -> Vec<String> {
    let mut args = vec!["--session".to_owned(), id.to_owned(), "--open".to_owned()];
    for (flag, value) in [("--model", &options.model), ("--as", &options.account), ("--agent", &options.agent)] {
        if let Some(value) = value {
            args.extend([flag.to_owned(), value.clone()]);
        }
    }
    if let Some(name) = &options.name {
        args.push(format!("--name={name}"));
    }
    args
}

/// Reads and removes the job, then validates it as if typed: the same parser and limits.
fn read_job(key: &str) -> Result<(Options, String), String> {
    let invalid = "Invalid background handover job.";
    let path = job_path(key).ok_or(invalid)?;
    let mut bytes = Vec::new();
    let read = std::fs::File::open(&path).and_then(|file| file.take(MAX_JOB + 1).read_to_end(&mut bytes));
    let _ = std::fs::remove_file(&path);
    read.map_err(|_| "Could not read the background handover job.")?;
    parse_job(&bytes)
}

fn parse_job(bytes: &[u8]) -> Result<(Options, String), String> {
    let invalid = "Invalid background handover job.";
    if bytes.len() as u64 > MAX_JOB {
        return Err(invalid.into());
    }
    let job: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid)?;
    let args = job["args"].as_array().filter(|a| a.len() <= MAX_ARGS).ok_or(invalid)?;
    let args = args.iter().map(|a| a.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>().ok_or(invalid)?;
    let mut options = super::parse(args.into_iter())?;
    if !options.open || options.background || options.focus_file.is_some() || options.cwd.is_some() {
        return Err(invalid.into());
    }
    options.focus = match &job["focus"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) if text.len() <= 64_000 => Some(text.clone()),
        _ => return Err(invalid.into()),
    };
    let label = job["label"].as_str().map(|l| one_line(l, 200)).unwrap_or_default();
    Ok((options, label))
}

/// Shows a failure in the running window, or in a message box when no window can show it.
fn report(text: &str) {
    match handover_pipe::send(&encode_failure(text)) {
        Sent::Answered(Ok(Outcome::Shown)) => {}
        _ => crate::cli_args::message_box(&format!("The handover did not finish.\n\n{text}"), true),
    }
}

/// The failure notice request: the version line, then one line of text.
pub(super) fn encode_failure(text: &str) -> Vec<u8> {
    format!("{FAILED_VERSION}\n{}", one_line(text, MAX_NOTICE)).into_bytes()
}

/// `None` when the request is not a failure notice; otherwise its text, revalidated.
pub fn decode_failure(bytes: &[u8]) -> Option<Result<String, String>> {
    let rest = bytes.strip_prefix(format!("{FAILED_VERSION}\n").as_bytes())?;
    let text = std::str::from_utf8(rest).ok().filter(|t| {
        t.len() <= MAX_NOTICE && !t.trim().is_empty() && !t.chars().any(|c| c.is_control() || is_separator(c))
    });
    Some(text.map(str::to_owned).ok_or_else(|| "Invalid handover notice.".into()))
}

fn is_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

/// Control characters become spaces; cut to `limit` bytes at a character boundary.
fn one_line(text: &str, limit: usize) -> String {
    let mut line: String = text.chars().map(|c| if c.is_control() || is_separator(c) { ' ' } else { c }).collect();
    if line.len() > limit {
        let end = (0..=limit).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
        line.truncate(end);
    }
    line.trim().to_owned()
}

fn new_key() -> Result<String, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    Ok(format!("{PREFIX}{}-{}", std::process::id(), now.as_nanos()))
}

fn valid_key(key: &str) -> bool {
    key.strip_prefix(PREFIX)
        .and_then(|tail| tail.split_once('-'))
        .is_some_and(|(pid, nonce)| pid.parse::<u32>().is_ok() && nonce.parse::<u128>().is_ok())
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Only app-named files in this user's temporary folder, never a caller-chosen path.
fn job_path(key: &str) -> Option<PathBuf> {
    valid_key(key).then(|| std::env::temp_dir().join(format!("{key}.json")))
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::create_new(path)?;
    let wrote = file.write_all(bytes);
    drop(file);
    if wrote.is_err() {
        let _ = std::fs::remove_file(path);
    }
    wrote
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "5eed3dde-1c2d-4e5f-8a9b-000000033232";

    fn parse(args: &[&str]) -> Options {
        super::super::parse(args.iter().map(|a| a.to_string())).unwrap()
    }

    #[test]
    fn background_needs_open_and_the_job_carries_the_same_validated_options() {
        let bad = |args: &[&str]| super::super::parse(args.iter().map(|a| a.to_string())).is_err();
        assert!(bad(&["--session", ID, "--background"]), "--print is not a background job");
        assert!(bad(&["--session", ID, "--print", "--background"]));
        assert!(bad(&["--session", ID, "--open", "--background", "--background"]));
        let options = parse(&[
            "--session",
            "latest",
            "--open",
            "--background",
            "--cwd",
            "C:\\Work dir",
            "--agent",
            "plugin:review",
            "--as",
            "work",
            "--model",
            "openai/gpt",
            "--name=--Dev's \"notes\" 😺",
            "--focus-file",
            "C:\\Temp\\focus.txt",
        ]);
        assert!(options.background && options.open);
        let args = job_args(&options, ID);
        assert_eq!(&args[..3], ["--session", ID, "--open"]);
        assert!(!args.iter().any(|a| a == "--background" || a == "--focus-file" || a == "--cwd"));
        let job = serde_json::json!({"args": args, "focus": "Keep the plan.\n", "label": "Name\nwith a break"});
        let (back, label) = parse_job(job.to_string().as_bytes()).unwrap();
        assert_eq!(back.session, ID);
        assert_eq!(back.agent.as_deref(), Some("plugin:review"));
        assert_eq!(back.account.as_deref(), Some("work"));
        assert_eq!(back.model.as_deref(), Some("openai/gpt"));
        assert_eq!(back.name.as_deref(), Some("--Dev's \"notes\" 😺"));
        assert_eq!(back.focus.as_deref(), Some("Keep the plan.\n"));
        assert!(back.open && !back.background && back.focus_file.is_none());
        assert_eq!(label, "Name with a break");
    }

    #[test]
    fn a_job_is_revalidated_and_cannot_name_a_file_or_start_another_job() {
        let job = |args: serde_json::Value, focus: serde_json::Value| {
            parse_job(serde_json::json!({"args": args, "focus": focus}).to_string().as_bytes())
        };
        let ok = serde_json::json!(["--session", ID, "--open"]);
        assert!(job(ok.clone(), serde_json::Value::Null).is_ok());
        for args in [
            serde_json::json!(["--session", ID]),
            serde_json::json!(["--session", ID, "--open", "--background"]),
            serde_json::json!(["--session", ID, "--open", "--focus-file", "C:\\other.txt"]),
            serde_json::json!(["--session", ID, "--open", "--cwd", "C:\\"]),
            serde_json::json!(["--session", ID, "--open", "--agent", "bad agent"]),
            serde_json::json!(["--session", ID, "--open", 1]),
            serde_json::json!(vec!["--save"; 17]),
            serde_json::json!("--session"),
        ] {
            assert!(job(args.clone(), serde_json::Value::Null).is_err(), "{args}");
        }
        assert!(job(ok.clone(), serde_json::json!(5)).is_err());
        assert!(job(ok, serde_json::json!("x".repeat(64_001))).is_err());
        assert!(parse_job(b"not json").is_err());
        assert!(parse_job(&vec![b' '; MAX_JOB as usize + 1]).is_err());
    }

    #[test]
    fn job_names_are_app_owned_words_in_the_temporary_folder() {
        let key = new_key().unwrap();
        assert!(valid_key(&key));
        assert_eq!(job_path(&key).unwrap().parent(), Some(std::env::temp_dir().as_path()));
        for bad in [
            "cs-handover-job-1-../../x",
            "cs-handover-job-1",
            "cs-handover-1-2",
            "cs-handover-job-a-2",
            "C:\\x",
            "cs-handover-job-1-2 x",
        ] {
            assert!(job_path(bad).is_none(), "{bad}");
        }
        assert!(read_job("cs-handover-job-1-2").is_err(), "a missing job is an error, not a guess");
    }

    #[test]
    fn a_written_job_is_read_once_and_removed() {
        let key = new_key().unwrap();
        let path = job_path(&key).unwrap();
        let job = serde_json::json!({"args": ["--session", ID, "--open"], "focus": null, "label": "Work"});
        write_new(&path, job.to_string().as_bytes()).unwrap();
        assert!(write_new(&path, b"{}").is_err(), "never overwrites");
        let (options, label) = read_job(&key).unwrap();
        assert_eq!((options.session.as_str(), label.as_str()), (ID, "Work"));
        assert!(!path.exists());
        assert!(read_job(&key).is_err());
    }

    #[test]
    fn failure_notices_round_trip_as_one_bounded_line() {
        let bytes = encode_failure("“Work”: Could not read the transcript:\nline two\x1b[2J");
        assert_eq!(decode_failure(&bytes).unwrap().unwrap(), "“Work”: Could not read the transcript: line two [2J");
        let long = encode_failure(&"😺".repeat(1000));
        assert!(long.len() <= handover_pipe::MAX_REQUEST);
        assert!(decode_failure(&long).unwrap().is_ok());
        let request = super::super::encode_request(&super::super::Handoff {
            id: ID.into(),
            account: "work".into(),
            file: "handover-001.md".into(),
            agent: None,
            name: None,
            explicit_account: false,
            basis: None,
        })
        .unwrap();
        assert!(decode_failure(&request).is_none(), "a handover request is not a notice");
        for bad in [
            format!("{FAILED_VERSION}\n"),
            format!("{FAILED_VERSION}\n  "),
            format!("{FAILED_VERSION}\na\nb"),
            format!("{FAILED_VERSION}\na\u{2028}b"),
            format!("{FAILED_VERSION}\n{}", "x".repeat(MAX_NOTICE + 1)),
        ] {
            assert!(decode_failure(bad.as_bytes()).unwrap().is_err(), "{bad:?}");
        }
        assert!(decode_failure(format!("{FAILED_VERSION}\n\u{ff}").as_bytes()).unwrap().is_ok());
        assert!(decode_failure(&[FAILED_VERSION.as_bytes(), b"\n\xff"].concat()).unwrap().is_err());
    }
}
