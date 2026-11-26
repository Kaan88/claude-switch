//! Open a GUI without keeping the caller's stdout/stderr pipes alive.
use std::path::Path;
use std::time::{Duration, Instant};

fn words(id: &str, account: &str, file: &str) -> Result<String, String> {
    let word = |s: &str| {
        !s.is_empty()
            && s.len() <= 160
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    if ![id, account, file].into_iter().all(word) {
        return Err("Invalid handover window arguments.".into());
    }
    Ok(format!("--compact-open-window {id} {account} {file}"))
}

pub(super) fn spawn(exe: &Path, id: &str, account: &str, file: &str) -> Result<(), String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    let key = format!("cs-handover-{}-{}", std::process::id(), now.as_nanos());
    let dir = ack_dir(&key).ok_or("Invalid handover acknowledgement")?;
    std::fs::create_dir(&dir).map_err(|_| "Could not create a handover acknowledgement directory.")?;
    let result = (|| {
        let arguments = format!("{} {key}", words(id, account, file)?);
        let until = Instant::now() + Duration::from_secs(30);
        spawn_window(exe, &arguments, until).map_err(|_| "Could not start the handover window.")?;
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
        Err("Timed out waiting for the new window to confirm the Claude tab.".into())
    })();
    let _ = std::fs::remove_file(dir.join("result"));
    let _ = std::fs::remove_dir(dir);
    result
}

fn ack_dir(key: &str) -> Option<std::path::PathBuf> {
    let (pid, nonce) = key.strip_prefix("cs-handover-")?.split_once('-')?;
    pid.parse::<u32>().ok()?;
    nonce.parse::<u128>().ok()?;
    (key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| std::env::temp_dir().join(key))
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

    #[test]
    fn handoff_accepts_only_four_fixed_native_words_and_no_path_syntax() {
        assert_eq!(
            words("old-id", "work", "old-id-001.md").unwrap(),
            "--compact-open-window old-id work old-id-001.md"
        );
        for bad in ["one two", "../other", "a\\b", "a&b", "-option", "with\"quote", "line\nbreak"] {
            assert!(words("old-id", "work", bad).is_err());
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
