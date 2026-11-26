//! Running a [`Launch`]: the one place that starts a process. A later step puts terminal tabs
//! inside the window; then the `Place::Terminal` arm of [`run`] hands the launch to the tab strip
//! instead of an external terminal, and nothing else changes.
//!
//! - `Place::Detached`: the program alone, with no console window (`CREATE_NO_WINDOW`), not
//!   waited for. A launch that hands a session to the desktop app (`claude --desktop ...`) is the
//!   exception: the CLI refuses that without a terminal, so it runs in a pseudo-terminal nobody
//!   sees ([`needs_terminal`], [`run_unseen`]).
//! - `Place::Terminal`: Windows Terminal when `wt.exe` is on the `PATH`
//!   (`wt.exe -d <cwd> -- <program> <args…>`), else `cmd /c start "" /d <cwd> <program> <args…>`.
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
    if needs_terminal(launch) {
        return run_unseen(launch);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    let mut cmd = command(launch, &path, &pathext);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(drop)
}

/// A detached launch the CLI only does with a terminal: `--desktop` ends at once with "can't run
/// non-interactively" when its output is not one.
pub fn needs_terminal(launch: &Launch) -> bool {
    launch.place == Place::Detached && launch.args.iter().any(|a| a == "--desktop")
}

/// How long [`run_unseen`] waits for a refusal before it reports the launch as started.
const REFUSAL_WAIT: Duration = Duration::from_secs(4);
/// How long the unseen terminal is kept for a program that does not end.
const UNSEEN_LIMIT: Duration = Duration::from_secs(120);

/// Runs the launch in a pseudo-terminal that is shown nowhere. A program that ends with an error
/// within [`REFUSAL_WAIT`] makes this an error carrying what it printed (the CLI says why it will
/// not hand the session over); otherwise its output is thrown away until it ends.
fn run_unseen(launch: &Launch) -> io::Result<()> {
    let mut pty = mdshared_pty::Pty::spawn(&crate::term::command(launch), 200, 30, || {})?;
    let start = Instant::now();
    let mut printed = Vec::new();
    while start.elapsed() < REFUSAL_WAIT {
        printed.extend(pty.take_output());
        if let Some(exit) = pty.exit() {
            printed.extend(pty.take_output());
            let said = plain_text(&printed);
            return if exit.success() {
                Ok(())
            } else if said.is_empty() {
                Err(io::Error::other("The Claude Code CLI ended with an error and said nothing."))
            } else {
                Err(io::Error::other(said))
            };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::spawn(move || {
        while pty.running() && start.elapsed() < UNSEEN_LIMIT {
            drop(pty.take_output());
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    Ok(())
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

/// The command [`run`] spawns, with `path` and `pathext` standing for the environment's.
pub fn command(launch: &Launch, path: &OsString, pathext: &OsString) -> Command {
    let program = resolve(&launch.program, path, pathext).unwrap_or_else(|| launch.program.clone());
    let mut cmd = match launch.place {
        Place::Detached => {
            let mut c = Command::new(&program);
            c.args(&launch.args);
            no_window(&mut c);
            if let Some(dir) = &launch.cwd {
                c.current_dir(dir);
            }
            c
        }
        Place::Terminal => match resolve(Path::new("wt.exe"), path, pathext) {
            Some(wt) => {
                let mut c = Command::new(wt);
                if let Some(dir) = &launch.cwd {
                    c.arg("-d").arg(dir);
                    c.current_dir(dir);
                }
                c.arg("--").arg(&program).args(&launch.args);
                c
            }
            None => {
                let mut c = Command::new("cmd");
                c.args(["/c", "start", ""]);
                if let Some(dir) = &launch.cwd {
                    c.arg("/d").arg(dir);
                    c.current_dir(dir);
                }
                c.arg(&program).args(&launch.args);
                no_window(&mut c);
                c
            }
        },
    };
    for name in &launch.env_remove {
        cmd.env_remove(name);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    cmd
}

#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_c: &mut Command) {}

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

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("claude-switch-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
        l.program = "cmd.exe".into();
        l.args = vec!["/c".into(), "echo no such session& exit 3".into()];
        assert_eq!(run_unseen(&l).unwrap_err().to_string(), "no such session");
        l.args = vec!["/c".into(), "echo fine".into()];
        assert!(run_unseen(&l).is_ok());
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
        let result = run_unseen(&l);
        println!("{result:?} after {:?}", started.elapsed());
        assert!(result.is_ok());
    }

    #[test]
    fn a_detached_launch_runs_the_program_itself_in_its_folder_with_its_environment() {
        let c = command(&launch(Place::Detached), &OsString::new(), &".EXE".into());
        assert_eq!(c.get_program(), "claude");
        assert_eq!(args(&c), ["--resume", "abc def"]);
        assert_eq!(c.get_current_dir(), Some(Path::new(r"C:\work\my project")));
        let e = envs(&c);
        assert!(e.contains(&("CLAUDE_CONFIG_DIR".into(), Some(r"C:\x\cli".into()))));
        assert!(e.contains(&("ANTHROPIC_API_KEY".into(), None)));
    }

    #[test]
    fn a_terminal_launch_without_windows_terminal_goes_through_cmd_start_with_each_argument_apart() {
        let c = command(&launch(Place::Terminal), &OsString::new(), &".EXE".into());
        assert_eq!(c.get_program(), "cmd");
        assert_eq!(args(&c), ["/c", "start", "", "/d", r"C:\work\my project", "claude", "--resume", "abc def"]);
        assert!(envs(&c).contains(&("CLAUDE_CONFIG_DIR".into(), Some(r"C:\x\cli".into()))));
    }

    #[test]
    fn a_terminal_launch_uses_windows_terminal_when_it_is_on_the_path() {
        let d = scratch("wt");
        std::fs::write(d.join("wt.exe"), b"").unwrap();
        std::fs::write(d.join("claude.cmd"), b"").unwrap();
        let c = command(&launch(Place::Terminal), &d.clone().into_os_string(), &".EXE;.CMD".into());
        assert_eq!(c.get_program(), d.join("wt.exe").as_os_str());
        let a = args(&c);
        assert_eq!(a[..3], ["-d".to_string(), r"C:\work\my project".into(), "--".into()]);
        assert_eq!(a[3], d.join("claude.cmd").to_string_lossy());
        assert_eq!(a[4..], ["--resume".to_string(), "abc def".into()]);
        assert!(envs(&c).contains(&("ANTHROPIC_API_KEY".into(), None)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_launch_with_no_folder_names_none() {
        let mut l = launch(Place::Terminal);
        l.cwd = None;
        let c = command(&l, &OsString::new(), &".EXE".into());
        assert_eq!(args(&c), ["/c", "start", "", "claude", "--resume", "abc def"]);
        assert_eq!(c.get_current_dir(), None);
    }

    #[test]
    fn resolve_finds_a_bare_name_by_its_extensions_and_leaves_paths_and_unknown_names_alone() {
        let d = scratch("resolve");
        std::fs::write(d.join("tool.cmd"), b"").unwrap();
        let path = d.clone().into_os_string();
        assert_eq!(resolve(Path::new("tool"), &path, &".EXE;.CMD".into()), Some(d.join("tool.cmd")));
        assert_eq!(resolve(Path::new("tool.cmd"), &path, &".EXE".into()), Some(d.join("tool.cmd")));
        assert_eq!(resolve(Path::new("missing"), &path, &".EXE;.CMD".into()), None);
        assert_eq!(resolve(&d.join("tool.cmd"), &path, &".CMD".into()), None);
        assert_eq!(resolve(Path::new(""), &path, &".CMD".into()), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
