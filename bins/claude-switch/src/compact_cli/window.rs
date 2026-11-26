//! Open a GUI without keeping the caller's stdout/stderr pipes alive.
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) fn words(id: &str, account: &str, file: &str, agent: Option<&str>) -> Result<String, String> {
    claude_switch_core::launch::validate_session_id(id).map_err(|e| e.to_string())?;
    let word = |s: &str| {
        !s.is_empty()
            && s.len() <= 160
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    if ![id, account, file].into_iter().all(word) {
        return Err("Invalid handover window arguments.".into());
    }
    if let Some(agent) = agent {
        claude_switch_core::launch::session_agent_args(agent).map_err(|e| e.to_string())?;
    }
    Ok(format!("--compact-open-window {id} {account} {file}"))
}

fn arguments(id: &str, account: &str, file: &str, ack: &str, agent: Option<&str>) -> Result<String, String> {
    let words = words(id, account, file, agent)?;
    Ok(match agent {
        Some(name) => format!("{words} {ack} {name}"),
        None => format!("{words} {ack}"),
    })
}

pub(super) fn spawn(exe: &Path, id: &str, account: &str, file: &str, agent: Option<&str>) -> Result<(), String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    let key = format!("cs-handover-{}-{}", std::process::id(), now.as_nanos());
    let arguments = arguments(id, account, file, &key, agent)?;
    let dir = ack_dir(&key).ok_or("Invalid handover acknowledgement")?;
    std::fs::create_dir(&dir).map_err(|_| "Could not create a handover acknowledgement directory.")?;
    let result = (|| {
        let until = Instant::now() + Duration::from_secs(30);
        spawn_window(exe, &arguments, until).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                unknown_outcome()
            } else {
                "Could not start the handover window.".into()
            }
        })?;
        await_ack(&dir, until)
    })();
    let _ = std::fs::remove_file(dir.join("result"));
    let _ = std::fs::remove_dir(dir);
    result
}

fn await_ack(dir: &Path, until: Instant) -> Result<(), String> {
    while Instant::now() < until {
        match std::fs::File::open(dir.join("result")) {
            Ok(file) => {
                use std::io::Read as _;
                let mut bytes = Vec::new();
                file.take(7).read_to_end(&mut bytes).map_err(|_| "Could not read the handover acknowledgement.")?;
                return ack_result(&bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not read the handover acknowledgement.".into()),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(unknown_outcome())
}

fn unknown_outcome() -> String {
    "Timed out waiting for the handover window. The outcome is unknown; check for the tab before retrying.".into()
}

pub(super) fn valid_ack(key: &str) -> bool {
    key.strip_prefix("cs-handover-").and_then(|tail| tail.split_once('-')).is_some_and(|(pid, nonce)| {
        pid.parse::<u32>().is_ok()
            && nonce.parse::<u128>().is_ok()
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn ack_dir(key: &str) -> Option<std::path::PathBuf> {
    valid_ack(key).then(|| std::env::temp_dir().join(key))
}

fn ack_result(bytes: &[u8]) -> Result<(), String> {
    match bytes {
        b"opened" => Ok(()),
        _ => Err("The new window did not confirm opening a Claude tab.".into()),
    }
}

pub(super) fn acknowledge(key: &str, opened: bool) {
    if let Some(dir) = ack_dir(key)
        && std::fs::symlink_metadata(&dir).is_ok_and(|meta| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt as _;
                meta.is_dir() && meta.file_attributes() & 0x400 == 0
            }
            #[cfg(not(windows))]
            {
                meta.is_dir() && !meta.file_type().is_symlink()
            }
        })
    {
        use std::io::Write as _;
        let staging = dir.join("pending");
        if let Ok(mut file) = std::fs::File::create_new(&staging) {
            let wrote = file.write_all(if opened { b"opened" } else { b"failed" });
            drop(file);
            if wrote.is_ok() {
                let _ = std::fs::rename(&staging, dir.join("result"));
            } else {
                let _ = std::fs::remove_file(staging);
            }
        }
    }
}

#[cfg(windows)]
fn spawn_window(exe: &Path, arguments: &str, until: Instant) -> std::io::Result<()> {
    use claude_switch_core::{Place, launch};
    use std::io;
    use std::path::PathBuf;
    use std::process::Stdio;

    // Rust's stable Windows Command inherits extraneous inheritable handles even with all three
    // streams set to null. Windows' ShellExecute path starts the GUI without those handles.
    // This script is fixed: only the app's native path and validated ASCII words are passed as data.
    const OPEN: &str = "$ErrorActionPreference='Stop'; \
        $p = New-Object System.Diagnostics.ProcessStartInfo; \
        $p.FileName = $env:CS_HANDOVER_EXE; \
        $p.Arguments = $env:CS_HANDOVER_ARGS; \
        Remove-Item Env:CS_HANDOVER_EXE,Env:CS_HANDOVER_ARGS; \
        $p.WorkingDirectory = (Get-Location).ProviderPath; \
        $p.UseShellExecute = $true; \
        $child = [Diagnostics.Process]::Start($p); \
        if ($null -eq $child) { exit 1 }; $child.Dispose(); exit 0";
    let root = std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("No SystemRoot"))?;
    let program = PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut spec = launch::shell(&std::env::temp_dir());
    spec.program = program;
    spec.place = Place::Detached;
    spec.args = ["-NoProfile", "-NonInteractive", "-Command", OPEN].map(str::to_owned).to_vec();
    spec.env.insert("CS_HANDOVER_EXE".into(), exe.to_str().ok_or_else(|| io::Error::other("Non-Unicode path"))?.into());
    spec.env.insert("CS_HANDOVER_ARGS".into(), arguments.into());
    let mut child = crate::run::command_for(&spec, &spec.program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() { Ok(()) } else { Err(io::Error::other("Windows did not start the window")) };
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[cfg(not(windows))]
fn spawn_window(exe: &Path, arguments: &str, _: Instant) -> std::io::Result<()> {
    use claude_switch_core::{Place, launch};
    let mut spec = launch::shell(&std::env::temp_dir());
    spec.program = exe.to_owned();
    spec.place = Place::Detached;
    spec.args = arguments.split_ascii_whitespace().map(str::to_owned).collect();
    crate::run::run(&spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "5eed3dde-1c2d-4e5f-8a9b-000000033232";

    #[test]
    fn handover_timeout_reports_unknown_and_warns_before_retry() {
        let error = await_ack(Path::new("unused-after-deadline"), Instant::now()).unwrap_err();
        assert!(error.contains("outcome is unknown") && error.contains("check for the tab before retrying"));
    }

    #[test]
    fn handoff_round_trips_an_optional_validated_name_without_shell_or_path_syntax() {
        let ack = "cs-handover-12-34";
        for agent in [None, Some("plugin:nested:review_2")] {
            let args = arguments(ID, "work", "handover-001.md", ack, agent).unwrap();
            let mut words = args.split_ascii_whitespace();
            assert_eq!(words.next(), Some("--compact-open-window"));
            assert_eq!(words.clone().count(), if agent.is_some() { 5 } else { 4 });
            let (request, key) = crate::compact_cli::window_args(words.map(str::to_owned)).unwrap();
            assert_eq!(request.id, ID);
            assert_eq!(request.account, "work");
            assert_eq!(request.file, "handover-001.md");
            assert_eq!(request.agent.as_deref(), agent);
            assert_eq!(key, ack);
        }
        for bad in ["one two", "../other", "a\\b", "a&b", "-option", "with\"quote", "line\nbreak"] {
            assert!(words(ID, "work", bad, None).is_err());
        }
        assert!(words(ID, "work", "file.md", Some("plugin:bad&name")).is_err());
        for input in [
            format!("{ID} work good.md {ack} --demo"),
            format!("{ID} work good.md {ack} review extra"),
            format!("{ID} work good.md ."),
            format!("not-a-uuid work good.md {ack}"),
        ] {
            assert!(crate::compact_cli::window_args(input.split_ascii_whitespace().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn refusal_is_not_a_successful_tab_acknowledgement() {
        assert!(ack_result(b"opened").is_ok());
        for response in [b"".as_slice(), b"failed", b"opened\n", b"process started"] {
            assert!(ack_result(response).is_err());
        }
        assert!(ack_dir("cs-handover-1-../../other").is_none());
        assert!(ack_dir("cs-handover-1-42").is_some());
    }
}
