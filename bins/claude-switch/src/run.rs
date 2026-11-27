//! Running a [`Launch`]: the one place that starts a process. A later step puts terminal tabs
//! inside the window; then the `Place::Terminal` arm of [`run`] hands the launch to the tab strip
//! instead of an external terminal, and nothing else changes.
//!
//! - `Place::Detached`: the program alone, with no console window (`CREATE_NO_WINDOW`), not
//!   waited for. A launch that hands a session to the desktop app (`claude --desktop ...`) is the
//!   exception: the CLI refuses that without a terminal, so it runs in a pseudo-terminal nobody
//!   sees ([`needs_terminal`], [`run_unseen`]).
//! - `Place::Terminal`: launch directly in a new console, never through a shell's `start`.
//!
//! The launch's `env` and `env_remove` are applied to the child in both cases. Arguments are
//! passed one by one to [`Command`]; no command line is built by joining strings.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use claude_switch_core::{Launch, Place};

/// Starts the launch and returns once the process exists. Errors are the system's.
pub fn run(launch: &Launch) -> io::Result<()> {
    run_cancellable(launch, || false)
}

pub fn run_cancellable(launch: &Launch, cancelled: impl Fn() -> bool) -> io::Result<()> {
    if needs_terminal(launch) {
        return Err(io::Error::other("Desktop handoff requires a worker"));
    }
    #[cfg(windows)]
    let desktop_guard = if claude_switch_core::launch::is_desktop_request(launch) {
        Some(claude_switch_core::launch::guard_desktop_exe().map_err(io::Error::other)?)
    } else if launch.place == Place::Detached
        && launch.program.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("Claude.exe") || name.eq_ignore_ascii_case("claude-desktop.exe")
        })
    {
        return Err(io::Error::other("Claude Desktop requires a verified installation request."));
    } else {
        None
    };
    #[cfg(windows)]
    let program = match &desktop_guard {
        Some(guard) => guard.program().to_owned(),
        None => native_program(&launch.program)?,
    };
    #[cfg(not(windows))]
    let program = native_program(&launch.program)?;
    let mut cmd = command_for(launch, &program);
    if launch.place == Place::Detached {
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    }
    if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    cmd.spawn().map(drop)
}

pub fn needs_desktop_worker(launch: &Launch) -> bool {
    #[cfg(windows)]
    if claude_switch_core::launch::is_desktop_request(launch) {
        return true;
    }
    needs_terminal(launch)
}

/// A detached launch the CLI only does with a terminal: `--desktop` ends at once with "can't run
/// non-interactively" when its output is not one.
pub fn needs_terminal(launch: &Launch) -> bool {
    launch.place == Place::Detached && launch.args.iter().any(|a| a == "--desktop")
}

pub fn run_unseen(launch: &Launch, cancelled: impl Fn() -> bool) -> io::Result<()> {
    let mut launch = launch.clone();
    launch.program = native_program(&launch.program)?;
    let mut pty = mdshared_pty::Pty::spawn(&crate::term::command(&launch), 200, 30, || {})?;
    let start = Instant::now();
    let mut printed = Vec::new();
    let collect = |printed: &mut Vec<u8>, output: Vec<u8>| {
        const CAP: usize = 8 * 1024;
        let output = &output[output.len().saturating_sub(CAP)..];
        let remove = (printed.len() + output.len()).saturating_sub(CAP);
        printed.drain(..remove);
        printed.extend_from_slice(output);
    };
    wait_for_handoff(
        || start.elapsed(),
        cancelled,
        || {
            collect(&mut printed, pty.take_output());
            let exit = pty.exit()?;
            collect(&mut printed, pty.take_output());
            let said = plain_text(&printed);
            Some(if exit.success() {
                Ok(())
            } else if said.is_empty() {
                Err(io::Error::other("The Claude Code CLI ended with an error and said nothing."))
            } else {
                Err(io::Error::other(said))
            })
        },
        || std::thread::sleep(Duration::from_millis(50)),
    )
}

fn wait_for_handoff(
    elapsed: impl Fn() -> Duration,
    cancelled: impl Fn() -> bool,
    mut poll: impl FnMut() -> Option<io::Result<()>>,
    mut pause: impl FnMut(),
) -> io::Result<()> {
    const LIMIT: Duration = Duration::from_secs(30);
    while elapsed() < LIMIT && !cancelled() {
        if let Some(result) = poll() {
            return result;
        }
        pause();
    }
    Err(if cancelled() {
        io::Error::new(io::ErrorKind::Interrupted, "Desktop handoff was cancelled.")
    } else {
        io::Error::new(io::ErrorKind::TimedOut, "Desktop handoff timed out after 30 seconds.")
    })
}

/// What a terminal's output says as one line: escape sequences and control characters removed,
/// runs of white space made one space.
pub fn plain_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '' {
            match chars.next() {
                // CSI: parameters, then one final character.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '' {
                            break;
                        }
                        if c == '' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
fn command(launch: &Launch, path: &OsString, pathext: &OsString) -> io::Result<Command> {
    let _ = pathext;
    Ok(command_for(launch, &native_on_path(&launch.program, path)?))
}

pub fn native_program(program: &Path) -> io::Result<PathBuf> {
    native_on_path(program, &std::env::var_os("PATH").unwrap_or_default())
}

fn native_on_path(program: &Path, path: &OsString) -> io::Result<PathBuf> {
    let native = |p: &Path| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    let found = if program.is_absolute() {
        (native(program) && program.is_file()).then(|| program.to_owned())
    } else if program.extension().is_none() || native(program) {
        resolve(program, path, &".EXE".into())
    } else {
        None
    };
    found.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "A native .exe is required; batch shims are not supported")
    })
}

/// Configure a command whose executable has already been resolved by the caller.
pub fn command_for(launch: &Launch, program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(&launch.args);
    if let Some(dir) = &launch.cwd {
        cmd.current_dir(dir);
    }
    window_flags(&mut cmd, launch.place);
    for name in claude_switch_core::launch::child_removals(launch) {
        cmd.env_remove(name);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    cmd
}

/// Opens `folder` in Explorer through the shell, as a double-click would. Only an existing folder
/// with an absolute path: never a file, so nothing can be started this way.
pub fn open_folder(folder: &Path) -> io::Result<()> {
    if !folder.is_absolute() || !folder.is_dir() {
        return Err(io::ErrorKind::NotFound.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let path: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
        let verb: Vec<u16> = "open".encode_utf16().chain([0]).collect();
        // SAFETY: both strings are NUL-terminated and outlive the call; the other pointers may be
        // null. The return value is a status code, not a handle to close.
        let status = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        } as isize;
        if status <= 32 { Err(io::Error::other(format!("the shell refused it (code {status})"))) } else { Ok(()) }
    }
    #[cfg(not(windows))]
    {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub fn reveal(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut folder = [0u16; 260];
        // SAFETY: the supplied buffer matches the capacity passed to GetWindowsDirectoryW.
        let n = unsafe {
            windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
                folder.as_mut_ptr(),
                folder.len() as u32,
            )
        } as usize;
        if n == 0 || n >= folder.len() {
            return Err(io::Error::last_os_error());
        }
        use std::os::windows::ffi::OsStringExt as _;
        let program = PathBuf::from(OsString::from_wide(&folder[..n])).join("explorer.exe");
        let launch = Launch {
            program,
            args: vec![],
            cwd: None,
            env: Default::default(),
            env_remove: Vec::new(),
            place: Place::Detached,
            label: String::new(),
        };
        command_for(&launch, &native_program(&launch.program)?).arg("/select,").arg(path).spawn().map(drop)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(windows)]
fn window_flags(c: &mut Command, place: Place) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    c.creation_flags(if place == Place::Terminal { CREATE_NEW_CONSOLE } else { CREATE_NO_WINDOW });
}

#[cfg(not(windows))]
fn window_flags(_c: &mut Command, _place: Place) {}

/// A bare program name found on `path`, trying each of `pathext` when it has no extension (so an
/// npm-installed `claude.cmd` is found as well as `claude.exe`). A name with a folder in it, or
/// one not found, is `None`: the caller passes it on as it is.
pub fn resolve(program: &Path, path: &OsString, pathext: &OsString) -> Option<PathBuf> {
    if program.components().count() != 1 || program.as_os_str().is_empty() {
        return None;
    }
    let exts: Vec<String> =
        pathext.to_string_lossy().split(';').filter(|e| !e.is_empty()).map(str::to_string).collect();
    let has_ext = program.extension().is_some();
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        if has_ext {
            let p = dir.join(program);
            if p.is_file() {
                return Some(p);
            }
            continue;
        }
        for ext in &exts {
            let mut name = program.as_os_str().to_os_string();
            name.push(ext.to_lowercase());
            let p = dir.join(&name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn launch(place: Place) -> Launch {
        let mut env = BTreeMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), r"C:\x\cli".to_string());
        Launch {
            program: "claude".into(),
            args: vec!["--resume".into(), "abc def".into()],
            cwd: Some(PathBuf::from(r"C:\work\my project")),
            env,
            env_remove: vec!["ANTHROPIC_API_KEY".into()],
            place,
            label: "main · test".into(),
        }
    }

    fn args(c: &Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn envs(c: &Command) -> Vec<(String, Option<String>)> {
        c.get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect()
    }

    fn scratch(name: &str) -> crate::testing::Scratch {
        crate::testing::Scratch::created(&format!("claude-switch-run-{name}"))
    }

    #[test]
    fn only_a_detached_hand_over_to_the_desktop_app_needs_a_terminal() {
        let mut l = launch(Place::Detached);
        assert!(!needs_terminal(&l));
        l.args.insert(0, "--desktop".into());
        assert!(needs_terminal(&l));
        l.place = Place::Terminal;
        assert!(!needs_terminal(&l));
    }

    #[cfg(windows)]
    #[test]
    fn desktop_requests_use_the_worker_and_unverified_explicit_executables_are_refused() {
        let mut launch = launch(Place::Detached);
        launch.program = "Claude.exe".into();
        assert!(needs_desktop_worker(&launch));
        assert!(!needs_terminal(&launch));
        let tmp = scratch("unverified-desktop");
        for name in ["Claude.exe", "claude-desktop.exe"] {
            launch.program = tmp.join(name);
            assert_eq!(
                run(&launch).unwrap_err().to_string(),
                "Claude Desktop requires a verified installation request."
            );
        }
        std::fs::remove_dir(tmp).unwrap();
    }

    #[test]
    fn what_a_terminal_printed_becomes_one_plain_line() {
        let raw = b"]0;claude[31mNo session found[0m
  with ID 1.[?25h
";
        assert_eq!(plain_text(raw), "No session found with ID 1.");
        assert_eq!(plain_text(b"[2J"), "");
    }

    #[cfg(windows)]
    #[test]
    fn an_unseen_run_reports_what_a_failing_program_said_and_nothing_for_a_good_one() {
        let mut l = launch(Place::Detached);
        l.cwd = None;
        l.program = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        l.args = vec!["/d".into(), "/c".into(), "echo no such session& exit 3".into()];
        assert_eq!(run_unseen(&l, || false).unwrap_err().to_string(), "no such session");
        l.args = vec!["/d".into(), "/c".into(), "echo fine".into()];
        assert!(run_unseen(&l, || false).is_ok());
    }

    #[test]
    fn desktop_handoff_polls_until_its_thirty_second_deadline_then_returns_timeout() {
        let now = std::cell::Cell::new(29_950);
        let mut polled = false;
        let error = wait_for_handoff(
            || Duration::from_millis(now.get()),
            || false,
            || {
                assert!(!polled, "polled at or beyond the deadline");
                polled = true;
                None
            },
            || now.set(30_000),
        )
        .unwrap_err();
        assert!(polled);
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn desktop_handoff_wait_preserves_a_cli_failure_after_the_old_four_second_cutoff() {
        let now = std::cell::Cell::new(0);
        let error = wait_for_handoff(
            || Duration::from_millis(now.get()),
            || false,
            || (now.get() == 4_200).then(|| Err(io::Error::other("handoff failed"))),
            || {
                assert_eq!(now.replace(4_200), 0, "did not return the ready failure");
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "handoff failed");
    }

    /// A real hand-over: `CS_DESKTOP_SESSION` is the session's id and `CS_DESKTOP_CWD` its folder.
    #[cfg(windows)]
    #[test]
    #[ignore = "opens a session in the desktop app"]
    fn a_real_session_is_handed_to_the_desktop_app() {
        let id = std::env::var("CS_DESKTOP_SESSION").unwrap();
        let l = Launch {
            program: "claude".into(),
            args: vec!["--desktop".into(), "--resume".into(), id],
            cwd: Some(PathBuf::from(std::env::var("CS_DESKTOP_CWD").unwrap())),
            env: BTreeMap::new(),
            env_remove: Vec::new(),
            place: Place::Detached,
            label: String::new(),
        };
        let started = Instant::now();
        let result = run_unseen(&l, || false);
        println!("{result:?} after {:?}", started.elapsed());
        assert!(result.is_ok());
    }

    #[test]
    fn a_detached_launch_runs_the_program_itself_in_its_folder_with_its_environment() {
        let d = scratch("detached");
        let path = d.join("claude.exe");
        std::fs::write(&path, b"").unwrap();
        let c = command(&launch(Place::Detached), &d.to_path_buf().into_os_string(), &".EXE".into()).unwrap();
        assert_eq!(c.get_program(), path.as_os_str());
        assert_eq!(args(&c), ["--resume", "abc def"]);
        assert_eq!(c.get_current_dir(), Some(Path::new(r"C:\work\my project")));
        let e = envs(&c);
        assert!(e.contains(&("CLAUDE_CONFIG_DIR".into(), Some(r"C:\x\cli".into()))));
        assert!(e.contains(&("ANTHROPIC_API_KEY".into(), None)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_terminal_launch_uses_the_executable_and_never_interprets_the_folder_as_a_command() {
        let d = scratch("terminal");
        std::fs::write(d.join("claude.exe"), b"").unwrap();
        let mut l = launch(Place::Terminal);
        l.cwd = Some(d.join("R&D"));
        let c = command(&l, &d.to_path_buf().into_os_string(), &".EXE".into()).unwrap();
        assert_eq!(c.get_program(), d.join("claude.exe").as_os_str());
        assert_eq!(args(&c), ["--resume", "abc def"]);
        assert_eq!(c.get_current_dir(), l.cwd.as_deref());
        assert!(envs(&c).contains(&("CLAUDE_CONFIG_DIR".into(), Some(r"C:\x\cli".into()))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn batch_only_is_refused_even_with_safe_arguments_and_windows_terminal_present() {
        let d = scratch("wt");
        std::fs::write(d.join("wt.exe"), b"").unwrap();
        std::fs::write(d.join("claude.cmd"), b"").unwrap();
        let mut l = launch(Place::Terminal);
        let path = d.to_path_buf().into_os_string();
        assert!(command(&l, &path, &".EXE;.CMD".into()).is_err());
        l.args = vec!["auth".into(), "login".into()];
        assert!(command(&l, &path, &".EXE;.CMD".into()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_launch_with_no_folder_names_none() {
        let d = scratch("no-folder");
        std::fs::write(d.join("claude.exe"), b"").unwrap();
        let mut l = launch(Place::Terminal);
        l.cwd = None;
        let c = command(&l, &d.to_path_buf().into_os_string(), &".EXE".into()).unwrap();
        assert_eq!(args(&c), ["--resume", "abc def"]);
        assert_eq!(c.get_current_dir(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_finds_a_bare_name_by_its_extensions_and_leaves_paths_and_unknown_names_alone() {
        let d = scratch("resolve");
        std::fs::write(d.join("tool.cmd"), b"").unwrap();
        let path = d.to_path_buf().into_os_string();
        assert_eq!(resolve(Path::new("tool"), &path, &".EXE;.CMD".into()), Some(d.join("tool.cmd")));
        assert_eq!(resolve(Path::new("tool.cmd"), &path, &".EXE".into()), Some(d.join("tool.cmd")));
        assert_eq!(resolve(Path::new("missing"), &path, &".EXE;.CMD".into()), None);
        assert_eq!(resolve(&d.join("tool.cmd"), &path, &".CMD".into()), None);
        assert_eq!(resolve(Path::new(""), &path, &".CMD".into()), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
