//! The window app's command line. Anything it does not know is refused with a short usage text
//! instead of being ignored: an ignored typo such as `--dmeo` would otherwise start the real app.
//! The `compact` subcommand is read by `compact_cli` before this parser runs.

use crate::compact_cli::{self, Handoff};
use crate::demo_terminal::Preview;
use crate::termtheme;

/// What the command line asked for.
#[derive(Default)]
pub struct Options {
    pub demo: bool,
    pub demo_notifications: bool,
    pub demo_usage_hover: bool,
    pub demo_permission: bool,
    pub demo_working: bool,
    pub demo_terminal: Option<Preview>,
    pub demo_terminal_settings: bool,
    pub forced_dark: Option<bool>,
    pub usage_shape: Option<Option<String>>,
    pub usage_labels: bool,
    pub handoff: Option<(Handoff, String)>,
    pub demo_compact: Option<bool>,
    pub demo_compact_long: bool,
    pub demo_search: Option<String>,
    pub demo_offscreen: bool,
    pub demo_workers: bool,
    pub shell: Option<String>,
    pub typed: Vec<String>,
    pub list_tab: bool,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Options { .. }")
    }
}

/// Why the app should not start.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// `--help`: print [`HELP`] and exit 0.
    Help,
    /// A usage error: print the message and [`SHORT_USAGE`], exit 2.
    Usage(String),
    /// A malformed internal handover: print the message alone, exit 2.
    Handoff(String),
}

pub const SHORT_USAGE: &str = "\
Usage: claude-switch [--demo] [--theme light|dark]
       claude-switch compact ...
Run `claude-switch --help` for every option.";

pub const HELP: &str = "\
Claude Switch: lists the Claude Code sessions of this computer and opens them.

Usage: claude-switch [options]
       claude-switch compact [options]   Hand a session over to a new chat (`compact --help`)

Options:
  --demo                     Made-up sessions and accounts; reads and writes no personal data
  --theme light|dark         Force the look instead of following Settings
  -h, --help                 Show this text

Hidden, for testing (meant to be combined with --demo):
  --demo-notifications       Two synthetic tabs and usage readings for notifications
  --demo-usage-hover         Echo terminal keys into a process-free demo tab
  --demo-permission          A synthetic permission dialog with a draft
  --demo-working             A synthetic working line in the session view
  --demo-terminal SHAPE,OPACITY,BACKDROP
                             Preview the terminal look, e.g. bar,80,mica
  --demo-terminal-settings   Open the terminal settings page
  --demo-search TEXT         Open the Sessions tab with an example search
  --demo-compact             Open the handover dialog
  --demo-compact-preview     Open the handover dialog's preview
  --demo-compact-long        The preview with a long transcript
  --demo-workers             Open Settings -> OpenCode
  --demo-offscreen           Start with the window placed off screen
  --shell PROGRAM            Open a terminal tab running PROGRAM (debug builds only)
  --type TEXT                Type TEXT into that tab, one per two seconds (debug builds only)
  --sessions-tab             Show the Sessions list after that tab or a demo tab opens

Hidden, diagnostic:
  --usage-shape [--labels] [ACCOUNT]
                             Print the shape of one usage reply and exit; uses the real login
                             token unless --demo is given
  --labels                   With --usage-shape, also print the labels

Hidden, internal (started by `compact --open`, not for typing):
  --compact-open-window ...  Open a handover in a new window";

/// Reads the arguments after the program name, without the `compact` subcommand.
pub fn parse(args: impl IntoIterator<Item = String>, debug_build: bool) -> Result<Options, Stop> {
    let mut o = Options::default();
    let mut args = args.into_iter().peekable();
    while let Some(a) = args.next() {
        let mut value = |what: &str| args.next().ok_or_else(|| Stop::Usage(format!("{a} needs {what}.")));
        match a.as_str() {
            "-h" | "--help" | "/?" | "-?" => return Err(Stop::Help),
            "--demo" => o.demo = true,
            "--demo-notifications" => o.demo_notifications = true,
            "--demo-usage-hover" => o.demo_usage_hover = true,
            "--demo-permission" => o.demo_permission = true,
            "--demo-working" => o.demo_working = true,
            "--demo-terminal" => {
                let v = value("SHAPE,OPACITY,BACKDROP")?;
                let preview = Preview::parse(&v);
                o.demo_terminal = Some(preview.ok_or_else(|| Stop::Usage(format!("{a}: cannot read `{v}`.")))?);
            }
            "--demo-terminal-settings" => o.demo_terminal_settings = true,
            "--theme" => {
                let v = value("light or dark")?;
                let dark = termtheme::window_dark(&v);
                o.forced_dark = Some(dark.ok_or_else(|| Stop::Usage(format!("{a} takes light or dark, not `{v}`.")))?);
            }
            "--shell" => o.shell = debug_build.then_some(value("a program")?),
            "--type" => o.typed.extend(debug_build.then_some(value("some text")?)),
            "--sessions-tab" => o.list_tab = true,
            "--demo-search" => o.demo_search = Some(value("a search")?),
            "--demo-compact" => o.demo_compact = Some(false),
            "--demo-compact-preview" => o.demo_compact = Some(true),
            "--demo-compact-long" => {
                o.demo_compact = Some(true);
                o.demo_compact_long = true;
            }
            "--demo-workers" => o.demo_workers = true,
            "--demo-offscreen" => o.demo_offscreen = true,
            // Takes every remaining argument and checks their number itself.
            "--compact-open-window" => {
                o.handoff = Some(compact_cli::window_args(&mut args).map_err(Stop::Handoff)?);
            }
            "--usage-shape" => {
                if args.next_if(|s| s == "--labels").is_some() {
                    o.usage_labels = true;
                }
                o.usage_shape = Some(args.next_if(|s| !s.starts_with('-')));
            }
            "--labels" => o.usage_labels = true,
            _ if a.starts_with('-') => return Err(Stop::Usage(format!("Unknown option `{a}`."))),
            _ => return Err(Stop::Usage(format!("Unexpected argument `{a}`."))),
        }
    }
    Ok(o)
}

/// The text to show for a stop and the exit code.
pub fn message(stop: &Stop) -> (String, i32) {
    match stop {
        Stop::Help => (HELP.to_string(), 0),
        Stop::Usage(why) => (format!("claude-switch: {why}\n{SHORT_USAGE}"), 2),
        Stop::Handoff(why) => (format!("claude-switch compact: {why}"), 2),
    }
}

/// Shows the text where the user can read it: the console the app was started from, or a
/// message box when there is none (a shortcut or the Run dialog).
pub fn show(text: &str, error: bool) {
    if has_console() {
        if error {
            eprintln!("{text}");
        } else {
            println!("{text}");
        }
        return;
    }
    message_box(text, error);
}

/// A message box with no owner window (a background process has nobody reading its output).
pub fn message_box(text: &str, error: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW,
        };
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (body, title) = (wide(text), wide("Claude Switch"));
        let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
        // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | icon | MB_SETFOREGROUND);
        }
    }
    #[cfg(not(windows))]
    eprintln!("{text}");
}

/// Whether standard output reaches someone: inherited redirects, or the parent's console. A
/// GUI-subsystem program gets neither when it is started from a shortcut.
fn has_console() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        };
        // SAFETY: these calls take only Win32 constants and do not dereference caller memory.
        unsafe {
            let valid = |h: windows_sys::Win32::Foundation::HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
            valid(GetStdHandle(STD_OUTPUT_HANDLE)) && valid(GetStdHandle(STD_ERROR_HANDLE))
                || AttachConsole(ATTACH_PARENT_PROCESS) != 0
        }
    }
    #[cfg(not(windows))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Options, Stop> {
        parse(args.iter().map(|a| a.to_string()), true)
    }

    #[test]
    fn known_flags_keep_working() {
        let o = run(&["--demo", "--theme", "dark", "--sessions-tab", "--demo-search", "width splitter"]).unwrap();
        assert!(o.demo && o.list_tab);
        assert_eq!(o.forced_dark, Some(true));
        assert_eq!(o.demo_search.as_deref(), Some("width splitter"));
        let o = run(&["--demo", "--demo-compact-long", "--demo-workers", "--demo-offscreen"]).unwrap();
        assert!(o.demo_compact == Some(true) && o.demo_compact_long && o.demo_workers && o.demo_offscreen);
        let o = run(&["--demo-terminal", "bar,80,mica", "--demo-terminal-settings"]).unwrap();
        assert!(o.demo_terminal.is_some() && o.demo_terminal_settings);
        let o = run(&["--usage-shape", "--labels", "work"]).unwrap();
        assert_eq!(o.usage_shape, Some(Some("work".into())));
        assert!(o.usage_labels);
        let o = run(&["--usage-shape", "--demo"]).unwrap();
        assert!(o.demo && o.usage_shape == Some(None));
        assert!(run(&[]).unwrap().handoff.is_none());
    }

    #[test]
    fn shell_and_typed_arguments_are_discarded_in_release() {
        let args = || ["--shell", "cmd", "--type", "dir\\r"].map(String::from);
        let release = parse(args(), false).unwrap();
        assert!(release.shell.is_none() && release.typed.is_empty());
        let debug = parse(args(), true).unwrap();
        assert_eq!(debug.shell.as_deref(), Some("cmd"));
        assert_eq!(debug.typed, ["dir\\r"]);
    }

    #[test]
    fn unknown_arguments_and_missing_values_are_refused() {
        for args in [&["--dmeo"][..], &["--demo", "-x"], &["stray"], &["--demo", "--theme"], &["--theme", "blue"]] {
            assert!(matches!(run(args), Err(Stop::Usage(_))), "{args:?}");
        }
        assert!(matches!(run(&["--demo-terminal", "nope"]), Err(Stop::Usage(_))));
        assert!(matches!(run(&["--compact-open-window", "x"]), Err(Stop::Handoff(_))));
        let (text, code) = message(&run(&["--dmeo"]).unwrap_err());
        assert_eq!(code, 2);
        assert!(text.contains("`--dmeo`") && text.contains("--help"));
    }

    #[test]
    fn help_lists_every_flag_and_marks_the_hidden_ones() {
        for args in [&["--help"][..], &["-h"], &["--demo", "/?"]] {
            assert_eq!(run(args).unwrap_err(), Stop::Help);
        }
        assert_eq!(message(&Stop::Help).1, 0);
        let hidden = HELP.find("Hidden").unwrap();
        for flag in [
            "--demo-notifications",
            "--demo-usage-hover",
            "--demo-permission",
            "--demo-working",
            "--demo-terminal ",
            "--demo-terminal-settings",
            "--demo-search",
            "--demo-compact ",
            "--demo-compact-preview",
            "--demo-compact-long",
            "--demo-workers",
            "--demo-offscreen",
            "--shell",
            "--type",
            "--usage-shape",
            "--sessions-tab",
            "--labels",
            "--compact-open-window",
        ] {
            let at = HELP.find(flag).unwrap_or_else(|| panic!("{flag} missing from --help"));
            assert!(at > hidden, "{flag} should be under a Hidden heading");
            let known = !matches!(run(&[flag.trim()]), Err(Stop::Usage(m)) if m.starts_with("Unknown"));
            assert!(known, "{flag} is listed but not parsed");
        }
        for flag in ["--demo ", "--theme", "compact"] {
            assert!(HELP.find(flag).unwrap() < hidden, "{flag}");
        }
    }
}
