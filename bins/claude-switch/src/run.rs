//! Running a [`Launch`]: the one place that starts a process. A later step puts terminal tabs
//! inside the window; then the `Place::Terminal` arm of [`run`] hands the launch to the tab strip
//! instead of an external terminal, and nothing else changes.
//!
//! - `Place::Detached`: the program alone, with no console window (`CREATE_NO_WINDOW`), not
//!   waited for.
//! - `Place::Terminal`: Windows Terminal when `wt.exe` is on the `PATH`
//!   (`wt.exe -d <cwd> -- <program> <args…>`), else `cmd /c start "" /d <cwd> <program> <args…>`.
//!
//! The launch's `env` and `env_remove` are applied to the child in both cases. Arguments are
//! passed one by one to [`Command`]; no command line is built by joining strings.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use claude_switch_core::{Launch, Place};

/// Starts the launch and returns once the process exists. Errors are the system's.
pub fn run(launch: &Launch) -> io::Result<()> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    let mut cmd = command(launch, &path, &pathext);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(drop)
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
