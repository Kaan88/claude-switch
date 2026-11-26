//! Open a GUI without keeping the caller's stdout/stderr pipes alive.
use std::path::Path;

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
    let arguments = words(id, account, file)?;
    spawn_window(exe, &arguments).map_err(|_| "Could not start the handover window.".into())
}

#[cfg(windows)]
fn spawn_window(exe: &Path, arguments: &str) -> std::io::Result<()> {
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
    let status = crate::compact_runner::launched_command(&spec)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other("Windows did not start the window")) }
}

#[cfg(not(windows))]
fn spawn_window(exe: &Path, arguments: &str) -> std::io::Result<()> {
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
}
