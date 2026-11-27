//! `compact --open --background`, what `/switch-compact` runs: the quick checks run at once, then
//! a detached Claude Switch process (no console window, none of the caller's handles, so the
//! caller's shell call ends) makes and opens the handover, and the caller returns. Nothing is
//! reported back into the old chat: the running window shows the tab, the failure or a note (the
//! Settings default agent was dropped) as a notice; with no window to show it, a message box does.
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
const NOTE_VERSION: &str = "claude-switch-handover-note-1";
/// The notice's text, in bytes (the request stays under `handover_pipe::MAX_REQUEST`).
const MAX_NOTICE: usize = 1500;

/// Checks what can be checked quickly, hands the rest to a detached process and returns the
/// one line for the caller.
pub(super) fn start(options: Options) -> Result<String, String> {
    let ready = super::ready(&options)?;
    // Read now: the plugin removes its focus file as soon as this returns.
    let focus = options.focus_file.as_deref().map(Path::new).map(super::focus_from_file).transpose()?;
    let label = super::name_of(&ready.roots, &ready.session);
    let job = job_bytes(&job_args(&options, &ready.session.id), focus.as_deref(), &label)?;
    let key = new_key()?;
    let path = job_path(&key).ok_or("Invalid background job name.")?;
    write_new(&path, &job).map_err(|_| "Could not write the background handover job.")?;
    let launched = std::env::current_exe().map_err(|_| std::io::Error::other("no exe")).and_then(|exe| {
        let until = Instant::now() + Duration::from_secs(30);
        super::window::spawn_window(&exe, &format!("compact {JOB_FLAG} {key}"), until)
    });
    match launched_line(launched, &label) {
        Ok(line) => Ok(line),
        Err(e) => {
            // Not started for certain (the launcher failed before Windows started it), so the
            // job will never be read.
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// The caller's line for the launcher's result. A launcher that ran out of time may already
/// have started the process, so that is neither a failure nor a reason to remove the job (the
/// process removes it when it reads it; an unread one goes with the day-old cleanup).
fn launched_line(launched: std::io::Result<()>, label: &str) -> Result<String, String> {
    match launched {
        Ok(()) => Ok(format!(
            "Claude Switch is making the handover of \u{201c}{label}\u{201d} in the background and will show the \
             result in its window. Nothing more to do in this chat."
        )),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(format!(
            "Windows was slow to start Claude Switch's background handover of \u{201c}{label}\u{201d}; it may have \
             started. If it did, Claude Switch shows the result in its window: check there before retrying. Nothing \
             more to do in this chat."
        )),
        Err(_) => Err("Could not start the background handover.".into()),
    }
}

/// The job as written, refused here (not in the detached process, after the caller was told
/// it started) when it would exceed what the process reads.
fn job_bytes(args: &[String], focus: Option<&str>, label: &str) -> Result<Vec<u8>, String> {
    let job = serde_json::json!({ "args": args, "focus": focus, "label": label }).to_string().into_bytes();
    if job.len() as u64 > MAX_JOB {
        return Err(format!(
            "The focus text is too large for a background handover ({} KiB once written; at most {} KiB). Shorten it \
             or remove control characters, then try again.",
            job.len().div_ceil(1024),
            MAX_JOB / 1024
        ));
    }
    Ok(job)
}

/// The detached process: runs the job and shows a failure the window has not shown, or the
/// note that the Settings default agent was dropped.
pub(super) fn run_job(mut args: impl Iterator<Item = String>) -> i32 {
    let (key, extra) = (args.next(), args.next());
    let job = match key.filter(|_| extra.is_none()) {
        Some(key) => read_job(&key),
        None => Err("Invalid background handover job.".into()),
    };
    let (options, label) = match job {
        Ok(job) => job,
        Err(e) => {
            report(Notice::Failed(e));
            return 2;
        }
    };
    match super::run(options) {
        Ok(None) => 0,
        Ok(Some(note)) => {
            report(Notice::Note(format!("\u{201c}{label}\u{201d}: {note}")));
            0
        }
        Err(Failed { shown: true, .. }) => 1,
        Err(Failed { message, .. }) => {
            report(Notice::Failed(format!("\u{201c}{label}\u{201d}: {message}")));
            1
        }
    }
}

/// Removes job files older than a day from the user's temporary folder: only names of the
/// job pattern, only plain files (not links or folders), nothing else.
pub fn clean_old_jobs() {
    let _ = clean_jobs_in(&std::env::temp_dir(), std::time::SystemTime::now(), Duration::from_secs(24 * 3600));
}

fn clean_jobs_in(dir: &Path, now: std::time::SystemTime, age: Duration) -> std::io::Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        let Some(key) = name.to_str().and_then(|n| n.strip_suffix(".json")) else { continue };
        if !valid_key(key) {
            continue;
        }
        // `symlink_metadata`: a link is never followed or removed.
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        let old = meta.modified().ok().and_then(|at| now.duration_since(at).ok()).is_some_and(|d| d > age);
        if meta.file_type().is_file() && old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
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

/// What a background handover tells the running window. Shown only; it acts on nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    /// The handover did not finish, and why (an error notice).
    Failed(String),
    /// It opened, with something to know (an info notice): a Settings default agent was dropped.
    Note(String),
}

/// Shows a notice in the running window, or in a message box when no window takes it.
fn report(notice: Notice) {
    if let Sent::Answered(Ok(Outcome::Shown)) = handover_pipe::send(&encode_notice(&notice)) {
        return;
    }
    match notice {
        Notice::Failed(text) => crate::cli_args::message_box(&format!("The handover did not finish.\n\n{text}"), true),
        Notice::Note(text) => crate::cli_args::message_box(&text, false),
    }
}

/// The notice request: its version line, then one line of text.
pub(super) fn encode_notice(notice: &Notice) -> Vec<u8> {
    let (version, text) = match notice {
        Notice::Failed(text) => (FAILED_VERSION, text),
        Notice::Note(text) => (NOTE_VERSION, text),
    };
    format!("{version}\n{}", one_line(text, MAX_NOTICE)).into_bytes()
}

/// `None` when the request is not a notice; otherwise the notice, its text revalidated.
pub fn decode_notice(bytes: &[u8]) -> Option<Result<Notice, String>> {
    let (kind, rest): (fn(String) -> Notice, _) = match bytes.strip_prefix(format!("{FAILED_VERSION}\n").as_bytes()) {
        Some(rest) => (Notice::Failed, rest),
        None => (Notice::Note, bytes.strip_prefix(format!("{NOTE_VERSION}\n").as_bytes())?),
    };
    let text = std::str::from_utf8(rest).ok().filter(|t| {
        t.len() <= MAX_NOTICE && !t.trim().is_empty() && !t.chars().any(|c| c.is_control() || is_separator(c))
    });
    Some(text.map(|t| kind(t.to_owned())).ok_or_else(|| "Invalid handover notice.".into()))
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
    fn notices_round_trip_as_one_bounded_line_of_their_kind() {
        let bytes = encode_notice(&Notice::Failed("“Work”: Could not read the transcript:\nline two\x1b[2J".into()));
        assert_eq!(
            decode_notice(&bytes).unwrap().unwrap(),
            Notice::Failed("“Work”: Could not read the transcript: line two [2J".into())
        );
        let note = "“Work”: The default agent “review” in Settings was not found for work, so the new chat starts as plain Claude.";
        assert_eq!(
            decode_notice(&encode_notice(&Notice::Note(note.into()))).unwrap().unwrap(),
            Notice::Note(note.into())
        );
        let long = encode_notice(&Notice::Failed("😺".repeat(1000)));
        assert!(long.len() <= handover_pipe::MAX_REQUEST);
        assert!(decode_notice(&long).unwrap().is_ok());
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
        assert!(decode_notice(&request).is_none(), "a handover request is not a notice");
        assert!(decode_notice(b"claude-switch-handover-note-2\nx").is_none());
        for version in [FAILED_VERSION, NOTE_VERSION] {
            for bad in [
                format!("{version}\n"),
                format!("{version}\n  "),
                format!("{version}\na\nb"),
                format!("{version}\na\u{2028}b"),
                format!("{version}\n{}", "x".repeat(MAX_NOTICE + 1)),
            ] {
                assert!(decode_notice(bad.as_bytes()).unwrap().is_err(), "{bad:?}");
            }
            assert!(decode_notice(format!("{version}\n\u{ff}").as_bytes()).unwrap().is_ok());
            assert!(decode_notice(&[version.as_bytes(), b"\n\xff"].concat()).unwrap().is_err());
        }
    }

    #[test]
    fn startup_cleanup_removes_only_old_job_files() {
        let dir = std::env::temp_dir().join(format!("cs-job-clean-{}", new_key().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        let job = |name: &str| {
            let path = dir.join(name);
            std::fs::write(&path, b"{}").unwrap();
            path
        };
        let old_job = job("cs-handover-job-12-34.json");
        let other = [
            job("cs-handover-job-12-34.txt"),
            job("cs-handover-job-12.json"),
            job("cs-handover-12-34.json"),
            job("notes.json"),
        ];
        let folder = dir.join("cs-handover-job-56-78.json");
        std::fs::create_dir(&folder).unwrap();
        let day = Duration::from_secs(24 * 3600);
        let now = std::time::SystemTime::now();
        assert_eq!(clean_jobs_in(&dir, now, day).unwrap(), 0, "nothing is a day old yet");
        assert!(old_job.exists());
        assert_eq!(clean_jobs_in(&dir, now + day + Duration::from_secs(60), day).unwrap(), 1);
        assert!(!old_job.exists());
        assert!(other.iter().all(|p| p.exists()) && folder.is_dir(), "only the job pattern, only files");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_slow_launcher_is_not_a_failure_and_keeps_the_job() {
        let ok = launched_line(Ok(()), "Work").unwrap();
        assert!(ok.contains("in the background") && ok.contains("Nothing more to do"));
        let slow = launched_line(Err(std::io::ErrorKind::TimedOut.into()), "Work").unwrap();
        assert!(slow.contains("may have started") && slow.contains("before retrying"), "{slow}");
        assert!(launched_line(Err(std::io::Error::other("Windows did not start the window")), "Work").is_err());
    }

    #[test]
    fn an_oversized_job_is_refused_before_anything_starts() {
        let args = ["--session".to_owned(), ID.to_owned(), "--open".to_owned()];
        // Under the 64,000-byte focus limit, but six bytes a character once escaped.
        let control = "\u{1}".repeat(63_000);
        let e = job_bytes(&args, Some(&control), "Work").unwrap_err();
        assert!(e.contains("too large") && e.contains("256 KiB"), "{e}");
        // The largest plain focus fits and is read back as written.
        let plain = "x".repeat(64_000);
        let job = job_bytes(&args, Some(&plain), "Work").unwrap();
        let (options, label) = parse_job(&job).unwrap();
        assert_eq!((options.focus.as_deref(), label.as_str()), (Some(plain.as_str()), "Work"));
        assert!(parse_job(&job_bytes(&args, None, "Work").unwrap()).unwrap().0.focus.is_none());
    }
}
