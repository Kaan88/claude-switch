//! Claude Code in a ConPTY, a script of keys and waits, and snapshots of its screen.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mdshared_pty::{Command, Pty};
use mdshared_term::{Attrs, Color, Screen};

const ESC: char = '\u{1b}';
/// What a shell sets and a program started from the Start menu does not have (`REC_PLAIN=1`).
const SHELL_VARS: [&str; 12] = [
    "TERM",
    "MSYSTEM",
    "WT_SESSION",
    "WT_PROFILE_ID",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "TERMINAL_EMULATOR",
    "ConEmuTask",
    "SHELL",
    "LANG",
    "PWD",
];
/// First-run screens `start` must never answer: Enter on them changes the account's look or
/// starts a sign-in in the browser.
const SIGN_IN_SCREENS: [&str; 3] = ["Select login method", "Choose the text style", "Paste code here if prompted"];
/// An 8 x 8 PNG of one colour, for `clipimage`.
const DOT_PNG: [u8; 74] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x08, 0x00, 0x00, 0x00, 0x08, 0x08, 0x02, 0x00, 0x00, 0x00, 0x4b, 0x6d, 0x29, 0xdc, 0x00, 0x00, 0x00, 0x11, 0x49,
    0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xb8, 0x59, 0x1e, 0x8e, 0x15, 0x31, 0x0c, 0x2d, 0x09, 0x00, 0x72, 0xaf, 0x69,
    0xc1, 0xf8, 0x50, 0x95, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
/// The question a `branch` asks the resumed copy: which of the words the script's turns asked for
/// it remembers.
const BRANCH_QUESTION: &str = "Which of the words alpha, bravo and charlie have you replied with so far in this \
    conversation? Answer with just those words, separated by commas.";
/// The dialog about an API key from the environment; its first row, above the highlighted "No",
/// is "Yes".
const KEY_DIALOG: &str = "Do you want to use this API key?";
const TRUST_DIALOG: &str = "trust this folder";

fn unescape(s: &str) -> Result<Vec<u8>, String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'e' => out.push(0x1b),
                b'r' => out.push(b'\r'),
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b's' => out.push(b' '),
                b'x' => {
                    let hex = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok());
                    out.push(hex.ok_or_else(|| "two hex digits after \\x".to_owned())?);
                    i += 2;
                }
                c => out.push(c),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// A snapshot without what the CLI shows of an API key ("sk-ant-...last characters" in its
/// question about a key from the environment): recordings are passed around.
fn redact(text: &str) -> String {
    const KEY: &str = "sk-ant-";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        let after = &rest[at + KEY.len()..];
        let end = after.find(|c: char| c.is_whitespace() || c == ESC).unwrap_or(after.len());
        out.push_str(&rest[..at + KEY.len()]);
        out.extend(std::iter::repeat_n('*', after[..end].chars().count()));
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// How to start the program whose screen is recorded.
pub struct Launch<'a> {
    pub program: &'a str,
    pub cwd: &'a Path,
    pub cols: u16,
    pub rows: u16,
    pub args: &'a [String],
    /// Variables for the child, set after the `CLAUDE_*` and `ANTHROPIC_*` ones were removed.
    pub env: &'a [(String, String)],
    /// No `TERM` and none of a shell's other terminal variables.
    pub plain: bool,
}

/// The app's test program (`recorder compat` builds it), for what only the app's own code can do:
/// `branch` writes its copy of a transcript with `core::branch`, as the app does.
pub struct Helper {
    pub program: PathBuf,
    /// Variables it must not see (the API key).
    pub remove: Vec<String>,
}

/// Why a script ended before its last line.
#[derive(Debug)]
pub enum Abort {
    /// Asked to stop from outside.
    Stopped,
    /// The CLI did not reach its input.
    Start(String),
    /// A `wait` or `gone` ran out of time (only when the recorder is strict).
    Timeout(String),
    /// A line of the script does not read.
    Script(String),
    /// A file or the terminal could not be written.
    Io(String),
    /// A step would have changed something it cannot put back (the clipboard).
    Refused(String),
}

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Abort::Stopped => f.write_str("stopped"),
            Abort::Start(why) => write!(f, "Claude Code did not reach its input: {why}"),
            Abort::Timeout(what) => write!(f, "the screen never showed what the script waited for: {what}"),
            Abort::Script(why) => write!(f, "the script does not read: {why}"),
            Abort::Io(why) => write!(f, "the recording could not be written: {why}"),
            Abort::Refused(why) => f.write_str(why),
        }
    }
}

pub struct Recorder {
    pty: Pty,
    screen: Screen,
    start: Instant,
    last_output: Instant,
    pub log: String,
    out: PathBuf,
    verbose: bool,
    /// Print the log to stderr as it grows.
    pub echo: bool,
    /// A `wait` or `gone` that runs out of time ends the script: the keys after it were written
    /// for a screen that is not there.
    pub strict: bool,
    /// Set from outside to end the script.
    pub stop: Option<Arc<AtomicBool>>,
    /// The screen's rows, for whoever watches the recording.
    pub live: Option<Arc<Mutex<Vec<String>>>>,
    last_live: Instant,
    /// The work folder, the variables and arguments `claude` was started with: `clipimage` writes
    /// into the folder, `help` and `branch` start `claude` again with the same account.
    program: String,
    size: (u16, u16),
    plain: bool,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    args: Vec<String>,
    /// The work folder's transcripts (`REC_PROJECT` when not set).
    pub project: Option<PathBuf>,
    pub helper: Option<Helper>,
    /// What the clipboard held before `clipimage`, put back when the script ends.
    clipboard: Option<Saved>,
    /// The sessions `branch` wrote: never taken for the recorded session's own transcript.
    branches: Vec<String>,
}

impl Recorder {
    /// The program as Claude Switch starts a Claude tab (`core::launch::child_removals`,
    /// `termapp`): no CLAUDE_* or ANTHROPIC_* variable reaches the child except the account's
    /// config folder; the classic renderer; TERM=xterm-256color unless a Start-menu environment is
    /// asked for.
    fn command(launch: &Launch, args: &[String]) -> Command {
        let mut cmd = Command::new(launch.program).cwd(launch.cwd);
        for name in removed_vars() {
            cmd = cmd.env_remove(name);
        }
        cmd = cmd.env("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN", "1");
        if launch.plain {
            for name in SHELL_VARS {
                cmd = cmd.env_remove(name);
            }
        } else {
            cmd = cmd.env("TERM", "xterm-256color");
        }
        for (name, value) in launch.env {
            cmd = cmd.env(name, value);
        }
        for arg in args {
            cmd = cmd.arg(arg);
        }
        cmd
    }

    pub fn spawn(launch: &Launch, out: PathBuf) -> std::io::Result<Recorder> {
        let pty = Pty::spawn(&Self::command(launch, launch.args), launch.cols, launch.rows, || {})?;
        let now = Instant::now();
        Ok(Recorder {
            pty,
            screen: Screen::new(launch.cols.into(), launch.rows.into(), 2000),
            start: now,
            last_output: now,
            log: String::new(),
            out,
            verbose: false,
            echo: false,
            strict: false,
            stop: None,
            live: None,
            last_live: now,
            program: launch.program.to_owned(),
            size: (launch.cols, launch.rows),
            plain: launch.plain,
            cwd: launch.cwd.to_owned(),
            env: launch.env.to_vec(),
            args: launch.args.to_vec(),
            project: None,
            helper: None,
            clipboard: None,
            branches: Vec::new(),
        })
    }

    /// Runs a script's lines; the log is written to `log.txt` however it ends. Every line is read
    /// first: a mistake in the script ends it before a key goes.
    pub fn run(&mut self, script: &str) -> Result<(), Abort> {
        let lines: Vec<&str> =
            script.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        let steps: Result<Vec<Step>, Abort> = lines
            .iter()
            .map(|line| Step::read(line).map_err(|why| Abort::Script(format!("{line:?}: {why}"))))
            .collect();
        let mut result = Ok(());
        match steps {
            Err(why) => {
                self.note(&format!("ABORT {why}"));
                result = Err(why);
            }
            Ok(steps) => {
                for (line, step) in lines.iter().zip(steps) {
                    self.note(&format!(">> {line}"));
                    result = self.step(step);
                    if let Err(why) = &result {
                        self.note(&format!("ABORT {why}"));
                        let _ = self.snap("aborted");
                        break;
                    }
                }
            }
        }
        let _ = self.pty.kill();
        if let Some(saved) = self.clipboard.take() {
            match clipboard_put_back(saved) {
                Ok(true) => self.note("the clipboard is as it was (its text)"),
                Ok(false) => self.note("the clipboard changed since clipimage: it was left as it is"),
                Err(why) => self.note(&format!("the clipboard could not be put back: {why}")),
            }
        }
        self.publish(true);
        if let Err(e) = std::fs::write(self.out.join("log.txt"), &self.log)
            && result.is_ok()
        {
            result = Err(Abort::Io(format!("log.txt: {e}")));
        }
        result
    }

    fn note(&mut self, text: &str) {
        let line = format!("{:>7} {text}\n", self.start.elapsed().as_millis());
        if self.echo {
            eprint!("{line}");
        }
        self.log.push_str(&line);
    }

    fn publish(&mut self, now: bool) {
        let Some(live) = &self.live else { return };
        if !now && self.last_live.elapsed() < Duration::from_millis(120) {
            return;
        }
        self.last_live = Instant::now();
        let rows = (0..self.screen.rows()).map(|r| self.screen.row_text(r).trim_end().to_owned()).collect();
        *live.lock().unwrap() = rows;
    }

    fn pump(&mut self) {
        let bytes = self.pty.take_output();
        if bytes.is_empty() {
            return;
        }
        self.screen.feed(&bytes);
        self.last_output = Instant::now();
        let reply = self.screen.take_reply();
        if !reply.is_empty() {
            let _ = self.pty.write(&reply);
        }
        if self.screen.take_bell() {
            self.note("BELL");
        }
        if let Some(title) = self.screen.take_title() {
            self.note(&format!("TITLE {title:?}"));
        }
        if self.verbose {
            let shown: String =
                String::from_utf8_lossy(&bytes).chars().take(60).flat_map(char::escape_default).collect();
            self.note(&format!("out {} bytes: {shown}", bytes.len()));
        }
        self.publish(false);
    }

    fn text(&self) -> String {
        let c = self.screen.cursor();
        let mut s = format!("cursor {} {}\n", c.row, c.col);
        for r in 0..self.screen.rows() {
            s.push_str(self.screen.row_text(r).trim_end());
            s.push('\n');
        }
        s
    }

    /// The screen as the fixtures' `.ansi` form: cursor, size, wrapped rows, modes, then each
    /// row with its SGR styles.
    fn ansi(&self) -> String {
        let c = self.screen.cursor();
        let m = self.screen.modes();
        let mut s = format!("cursor {} {}\n@size {} {}\n", c.row, c.col, self.screen.cols(), self.screen.rows());
        let wrapped: Vec<String> = (0..self.screen.rows())
            .filter(|&r| self.screen.line(r).is_some_and(|l| l.wrapped()))
            .map(|r| r.to_string())
            .collect();
        if !wrapped.is_empty() {
            let _ = writeln!(s, "@wrapped {}", wrapped.join(" "));
        }
        let paste = if m.bracketed_paste { " paste" } else { "" };
        let alt = if m.alt_screen { " alt" } else { "" };
        let _ = writeln!(s, "@modes{paste}{alt}{}", if c.visible { " cursor" } else { "" });
        let color = |c: Color, base: u8| match c {
            Color::Default => String::new(),
            Color::Indexed(n) => format!(";{base};5;{n}"),
            Color::Rgb(r, g, b) => format!(";{base};2;{r};{g};{b}"),
        };
        let bits = [
            (Attrs::BOLD, 1),
            (Attrs::DIM, 2),
            (Attrs::ITALIC, 3),
            (Attrs::UNDERLINE, 4),
            (Attrs::BLINK, 5),
            (Attrs::INVERSE, 7),
            (Attrs::HIDDEN, 8),
            (Attrs::STRIKE, 9),
        ];
        let plain = format!("{ESC}[0m");
        for r in 0..self.screen.rows() {
            let Some(line) = self.screen.line(r) else { continue };
            let cells = line.cells();
            let end = cells
                .iter()
                .rposition(|c| c.ch != ' ' || c.attrs != Attrs::NONE || c.bg != Color::Default)
                .map_or(0, |e| e + 1);
            let mut last = plain.clone();
            for cell in cells[..end].iter().filter(|c| !c.is_spacer()) {
                let mut sgr = format!("{ESC}[0");
                for (bit, code) in bits {
                    if cell.attrs.contains(bit) {
                        let _ = write!(sgr, ";{code}");
                    }
                }
                sgr.push_str(&color(cell.fg, 38));
                sgr.push_str(&color(cell.bg, 48));
                sgr.push('m');
                if sgr != last {
                    s.push_str(&sgr);
                    last = sgr;
                }
                s.push(cell.ch);
            }
            if last != plain {
                s.push_str(&plain);
            }
            s.push('\n');
        }
        s
    }

    fn snap(&mut self, name: &str) -> Result<(), Abort> {
        for (path, text) in [
            (self.out.join(format!("{name}.ansi")), redact(&self.ansi())),
            (self.out.join(format!("{name}.txt")), redact(&self.text())),
        ] {
            std::fs::write(&path, text).map_err(|e| Abort::Io(format!("{}: {e}", path.display())))?;
        }
        let c = self.screen.cursor();
        let quiet = self.last_output.elapsed().as_millis();
        self.note(&format!("snap {name} cursor {} {} quiet {quiet} ms", c.row, c.col));
        Ok(())
    }

    fn has(&self, text: &str) -> bool {
        (0..self.screen.rows()).any(|r| self.screen.row_text(r).contains(text))
    }

    fn stopped(&self) -> Result<(), Abort> {
        match &self.stop {
            Some(stop) if stop.load(Ordering::Relaxed) => Err(Abort::Stopped),
            _ => Ok(()),
        }
    }

    fn run_for(&mut self, time: Duration) -> Result<(), Abort> {
        let end = Instant::now() + time;
        while Instant::now() < end {
            self.pump();
            self.stopped()?;
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok(())
    }

    /// Whether `done` came true within `max`.
    fn until(&mut self, max: Duration, mut done: impl FnMut(&Recorder) -> bool) -> Result<bool, Abort> {
        let end = Instant::now() + max;
        loop {
            self.pump();
            self.stopped()?;
            if done(self) {
                return Ok(true);
            }
            if Instant::now() > end {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        // A program that has ended takes no keys; the script's waits say so.
        if self.pty.write(bytes).is_err() {
            self.note("write failed: the program has ended");
        }
    }

    /// Start-up: waits for the input box. Answers the folder trust dialog and the question about
    /// an API key from the environment; never a sign-in or first-run screen.
    fn start(&mut self) -> Result<(), Abort> {
        let ready = |r: &Recorder| r.has("shift+tab to cycle") || r.has("for shortcuts");
        let end = Instant::now() + Duration::from_secs(60);
        let (mut key, mut trust) = (false, false);
        loop {
            if self.until(Duration::from_millis(200), ready)? {
                break;
            }
            if let Some(screen) = SIGN_IN_SCREENS.iter().find(|s| self.has(s)) {
                return Err(Abort::Start(format!("it shows \"{screen}\": this account is not signed in")));
            }
            if !self.pty.running() {
                return Err(Abort::Start("it ended".to_owned()));
            }
            if Instant::now() > end {
                return Err(Abort::Start("no input box within 60 s".to_owned()));
            }
            // Each dialog once: its keys are for the row it starts on.
            if self.has(KEY_DIALOG) && !key {
                key = true;
                self.note("the API key dialog: Yes");
                self.run_for(Duration::from_millis(800))?;
                self.write(b"\x1b[A");
                self.run_for(Duration::from_millis(400))?;
                self.write(b"\r");
            } else if self.has(TRUST_DIALOG) && !self.has(KEY_DIALOG) && !trust {
                trust = true;
                self.note("the trust dialog: Yes");
                self.run_for(Duration::from_millis(800))?;
                self.write(b"\x1b[B");
                self.run_for(Duration::from_millis(400))?;
                self.write(b"\r");
            }
        }
        self.run_for(Duration::from_millis(1500))
    }

    /// What the live record of the recorded process names (Claude Switch follows a tab's session by
    /// it), with a copy of every transcript of the work folder: `NAME.live.json` holds the pid and
    /// the record (`null` when there is none), `NAME.transcripts/<id>.jsonl` the transcripts.
    /// The recorded account's config folder: `CLAUDE_CONFIG_DIR` as `claude` got it, else the
    /// first account's.
    fn config_dir(&self) -> Result<PathBuf, Abort> {
        self.env
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR"))
            .map(|(_, value)| PathBuf::from(value))
            .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".claude")))
            .ok_or_else(|| Abort::Script("no CLAUDE_CONFIG_DIR and no USERPROFILE".into()))
    }

    /// Copies what Claude Code keeps for the newest session's tasks (its `tasks` and `todos`
    /// entries, named after the session) into `NAME.tasks/<folder>/`, as they are now. Notes how
    /// many entries; none is a result too.
    fn task_files(&mut self, name: &str) -> Result<(), Abort> {
        self.pump();
        let config = self.config_dir()?;
        let Some(session) = self.newest_transcript()? else {
            self.note(&format!("taskfiles {name}: no transcript"));
            return Ok(());
        };
        let id = session.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let to = self.out.join(format!("{name}.tasks"));
        std::fs::create_dir_all(&to).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
        // A task list may be named other than after the session: anything written during this
        // recording is taken too.
        let started = std::time::SystemTime::now() - self.start.elapsed();
        let recent =
            |entry: &std::fs::DirEntry| entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= started);
        let mut copied = 0;
        for folder in ["tasks", "todos"] {
            for entry in std::fs::read_dir(config.join(folder)).into_iter().flatten().filter_map(Result::ok) {
                if !entry.file_name().to_string_lossy().to_lowercase().contains(&id) && !recent(&entry) {
                    continue;
                }
                let target = to.join(folder).join(entry.file_name());
                copied +=
                    copy_tree(&entry.path(), &target).map_err(|e| Abort::Io(format!("{}: {e}", target.display())))?;
            }
        }
        self.note(&format!("taskfiles {name}: {copied} files for {id}"));
        Ok(())
    }

    fn live(&mut self, name: &str) -> Result<(), Abort> {
        self.pump();
        let config = self.config_dir()?;
        let pid = self.pty.pid();
        let record = std::fs::read(config.join("sessions").join(format!("{pid}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .unwrap_or(serde_json::Value::Null);
        let dir = match &self.project {
            Some(dir) => dir.clone(),
            None => PathBuf::from(
                std::env::var("REC_PROJECT").map_err(|_| Abort::Script("this op needs REC_PROJECT".into()))?,
            ),
        };
        let to = self.out.join(format!("{name}.transcripts"));
        std::fs::create_dir_all(&to).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
        let mut copied = 0;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "jsonl") && path.is_file() {
                let target = to.join(entry.file_name());
                std::fs::copy(&path, &target).map_err(|e| Abort::Io(format!("{}: {e}", target.display())))?;
                copied += 1;
            }
        }
        let live = serde_json::json!({ "pid": pid, "record": record });
        let file = self.out.join(format!("{name}.live.json"));
        std::fs::write(&file, serde_json::to_string_pretty(&live).unwrap_or_default())
            .map_err(|e| Abort::Io(format!("{}: {e}", file.display())))?;
        let id = live["record"]["sessionId"].as_str().unwrap_or("none").to_owned();
        self.note(&format!("live {name}: record names {id}, {copied} transcripts"));
        Ok(())
    }

    /// The newest transcript in the work folder's transcript folder, other than the copies
    /// `branch` wrote (resuming one makes it the newest).
    fn newest_transcript(&self) -> Result<Option<PathBuf>, Abort> {
        let dir = match &self.project {
            Some(dir) => dir.clone(),
            None => PathBuf::from(
                std::env::var("REC_PROJECT").map_err(|_| Abort::Script("this op needs REC_PROJECT".into()))?,
            ),
        };
        Ok(std::fs::read_dir(&dir)
            .map_err(|e| Abort::Io(format!("{}: {e}", dir.display())))?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .filter(|e| {
                let stem = e.path().file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                !self.branches.iter().any(|id| id.eq_ignore_ascii_case(&stem))
            })
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path()))
    }

    /// `claude` once more, outside the terminal, with the account and model of the recording;
    /// at most two minutes.
    fn claude(&mut self, args: &[&str]) -> Result<std::process::Output, Abort> {
        let mut command = std::process::Command::new("claude");
        command.current_dir(&self.cwd).stdin(std::process::Stdio::null());
        for name in removed_vars() {
            command.env_remove(name);
        }
        command.envs(self.env.iter().map(|(n, v)| (n, v)));
        if let Some(at) = self.args.iter().position(|a| a == "--model")
            && let Some(model) = self.args.get(at + 1)
            && args.first() != Some(&"--help")
        {
            command.args(["--model", model]);
        }
        command.args(args);
        self.note(&format!("claude {}", args.join(" ")));
        run_for_output(command, Duration::from_secs(120), self.stop.clone())
    }

    /// Ends the recorded Claude Code and starts it again with its own arguments and `extra`.
    fn respawn(&mut self, extra: &[String]) -> Result<(), Abort> {
        self.pump();
        let _ = self.pty.kill();
        let mut args = self.args.clone();
        args.extend(extra.iter().cloned());
        let launch = Launch {
            program: &self.program,
            cwd: &self.cwd,
            cols: self.size.0,
            rows: self.size.1,
            args: &args,
            env: &self.env,
            plain: self.plain,
        };
        let pty = Pty::spawn(&Self::command(&launch, &args), self.size.0, self.size.1, || {})
            .map_err(|e| Abort::Start(format!("restart: {e}")))?;
        self.pty = pty;
        self.screen = Screen::new(self.size.0.into(), self.size.1.into(), 2000);
        Ok(())
    }

    /// The arguments the app gives a new handover session whose handover starts with `text` and is
    /// long enough to be read from its file (the app's own code, through its test program): the
    /// file goes to a `handover` folder beside the work folder, outside it as the app's is.
    fn handover_args(&mut self, name: &str, text: &str) -> Result<Vec<String>, Abort> {
        let helper = self.helper.as_ref().ok_or_else(|| Abort::Script("handover needs `recorder compat`".into()))?;
        let dir = self.cwd.parent().unwrap_or(&self.cwd).join("handover");
        let out = self.out.join(format!("{name}.args.json"));
        let mut command = std::process::Command::new(&helper.program);
        for var in &helper.remove {
            command.env_remove(var);
        }
        command.args(["cli_handover_args", "--ignored", "--nocapture"]).stdin(std::process::Stdio::null());
        command.env("CS_HANDOVER_DIR", &dir).env("CS_HANDOVER_TEXT", text).env("CS_HANDOVER_OUT", &out);
        let made = run_for_output(command, Duration::from_secs(60), self.stop.clone())?;
        let args = std::fs::read_to_string(&out).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok());
        match args {
            Some(args) if made.status.success() => Ok(args),
            _ => Err(Abort::Io(format!("handover {name}: {}", String::from_utf8_lossy(&made.stdout).trim()))),
        }
    }

    /// A branch as the app makes one at a message (`at`: `prompt TEXT` or `answer TEXT`): the
    /// newest transcript copied up to the end of that message's turn under a new id, by the app's
    /// own code through its test program, then that id resumed in print mode with a question.
    /// `NAME.id` holds the id, `NAME.json` the answer.
    fn branch(&mut self, name: &str, at: &str) -> Result<(), Abort> {
        let helper = self.helper.as_ref().ok_or_else(|| Abort::Script("branch needs `recorder compat`".into()))?;
        let source = self.newest_transcript()?.ok_or_else(|| Abort::Io("branch: no transcript".into()))?;
        let id_file = self.out.join(format!("{name}.id"));
        let mut command = std::process::Command::new(&helper.program);
        for var in &helper.remove {
            command.env_remove(var);
        }
        command.args(["cli_branch_write", "--ignored", "--nocapture"]).stdin(std::process::Stdio::null());
        command.env("CS_BRANCH_SOURCE", &source).env("CS_BRANCH_AT", at).env("CS_BRANCH_OUT", &id_file);
        let written = run_for_output(command, Duration::from_secs(60), self.stop.clone())?;
        let id = std::fs::read_to_string(&id_file).unwrap_or_default().trim().to_owned();
        if !written.status.success() || id.len() != 36 {
            let why = String::from_utf8_lossy(&written.stdout);
            self.note(&format!("branch {name}: the copy was not written\n{why}"));
            return Ok(());
        }
        self.note(&format!("branch {name}: copy {id}"));
        self.branches.push(id.clone());
        let output = self.claude(&["-p", "--resume", &id, "--output-format", "json", BRANCH_QUESTION])?;
        let to = self.out.join(format!("{name}.json"));
        std::fs::write(&to, &output.stdout).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
        self.note(&format!("branch {name}: {}", String::from_utf8_lossy(&output.stdout).trim()));
        Ok(())
    }

    fn step(&mut self, step: Step) -> Result<(), Abort> {
        match step {
            Step::Sleep(time) => self.run_for(time)?,
            Step::Send(bytes) => {
                self.pump();
                self.write(&bytes);
            }
            Step::Wait { text, want, max, must } => {
                if !self.until(max, |r| r.has(&text) == want)? {
                    let op = if must {
                        "need"
                    } else if want {
                        "wait"
                    } else {
                        "gone"
                    };
                    self.note(&format!("TIMEOUT {op} {text}"));
                    if self.strict || must {
                        return Err(Abort::Timeout(format!("{op} {text}")));
                    }
                }
            }
            Step::Quiet { time, max } => {
                if !self.until(max, |r| r.last_output.elapsed() >= time)? {
                    self.note("TIMEOUT quiet");
                }
            }
            Step::Idle { time, name } => {
                self.verbose = true;
                let ran = self.run_for(time);
                self.verbose = false;
                ran?;
                self.snap(&name)?;
            }
            Step::Watch { time, name } => {
                let end = Instant::now() + time;
                let (mut last, mut n) = (String::new(), 0);
                while Instant::now() < end {
                    self.run_for(Duration::from_secs(1))?;
                    let text = self.text();
                    if text != last {
                        last = text;
                        n += 1;
                        self.snap(&format!("{name}-{n:03}"))?;
                    }
                }
            }
            Step::Resize { cols, rows } => {
                self.pump();
                self.screen.resize(cols.into(), rows.into());
                self.pty.resize(cols, rows).map_err(|e| Abort::Io(format!("resize: {e}")))?;
            }
            Step::Snap(name) => {
                self.pump();
                self.snap(&name)?;
            }
            // The newest transcript of the work folder, as it is on disk now.
            Step::TsCopy(name) => match self.newest_transcript()? {
                Some(path) => {
                    let to = self.out.join(format!("{name}.jsonl"));
                    let len = std::fs::copy(&path, &to).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
                    self.note(&format!("tscopy {name} {len} bytes"));
                }
                None => self.note("tscopy: no transcript"),
            },
            Step::AgentCopy(name) => match self.newest_transcript()? {
                Some(path) => {
                    let to = self.out.join(format!("{name}.jsonl"));
                    std::fs::copy(&path, &to).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
                    let agents = path.with_extension("").join("subagents");
                    let into = self.out.join(&name).join("subagents");
                    std::fs::create_dir_all(&into).map_err(|e| Abort::Io(format!("{}: {e}", into.display())))?;
                    let mut copied = 0;
                    for entry in std::fs::read_dir(&agents).into_iter().flatten().filter_map(Result::ok) {
                        let file = entry.file_name();
                        let wanted = file.to_str().is_some_and(|f| {
                            f.starts_with("agent-") && (f.ends_with(".jsonl") || f.ends_with(".meta.json"))
                        });
                        if wanted && entry.path().is_file() {
                            let target = into.join(&file);
                            std::fs::copy(entry.path(), &target)
                                .map_err(|e| Abort::Io(format!("{}: {e}", target.display())))?;
                            copied += 1;
                        }
                    }
                    self.note(&format!("agentcopy {name}: the transcript and {copied} agent files"));
                }
                None => self.note("agentcopy: no transcript"),
            },
            // A picture on the clipboard, for Alt+V (as Chat sends a pasted picture). The text the
            // clipboard held is kept and put back when the script ends.
            Step::ClipImage(file) => {
                let path = self.cwd.join(&file);
                std::fs::write(&path, DOT_PNG).map_err(|e| Abort::Io(format!("{}: {e}", path.display())))?;
                let text = match self.clipboard.take() {
                    Some(saved) => saved.text,
                    None => clipboard_text()?,
                };
                let sequence = powershell(
                    &format!(
                        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; {SEQUENCE} \
                        $image = [System.Drawing.Image]::FromFile($env:CS_CLIP_FILE); \
                        [System.Windows.Forms.Clipboard]::SetImage($image); $image.Dispose(); \
                        [Console]::Out.Write([CsRecorder.Clip]::GetClipboardSequenceNumber())"
                    ),
                    &[("CS_CLIP_FILE", &path)],
                    None,
                )?;
                let sequence = sequence.trim().parse().map_err(|_| Abort::Io(format!("clipboard: {sequence:?}")))?;
                self.clipboard = Some(Saved { text, sequence });
                self.note(&format!("clipimage {file}"));
            }
            Step::Help(name) => {
                let output = self.claude(&["--help"])?;
                let to = self.out.join(format!("{name}.help.txt"));
                std::fs::write(&to, &output.stdout).map_err(|e| Abort::Io(format!("{}: {e}", to.display())))?;
                self.note(&format!("help {name}: {} bytes", output.stdout.len()));
            }
            Step::Branch { name, at } => self.branch(&name, &at)?,
            Step::Live(name) => self.live(&name)?,
            Step::TaskFiles(name) => self.task_files(&name)?,
            // Claude Code again, resuming the session `branch` wrote, as the app's Branch opens it.
            Step::Reopen(name) => {
                let id = std::fs::read_to_string(self.out.join(format!("{name}.id"))).unwrap_or_default();
                let id = id.trim().to_owned();
                if id.len() != 36 {
                    return Err(Abort::Start(format!("reopen: no branch {name} was written")));
                }
                self.note(&format!("reopen {name}: claude --resume {id}"));
                self.respawn(&["--resume".to_owned(), id])?;
                self.start()?;
            }
            // Claude Code again, started as the app starts a new handover session.
            Step::Handover { name, text } => {
                let args = self.handover_args(&name, &text)?;
                self.note(&format!("handover {name}: claude {}", args.join(" ")));
                self.respawn(&args)?;
                self.start()?;
            }
            Step::Start => self.start()?,
        }
        Ok(())
    }
}

/// Whether every line of a script reads, without running any of it.
pub fn script_reads(script: &str) -> Result<(), String> {
    for line in script.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        Step::read(line).map_err(|why| format!("{line:?}: {why}"))?;
    }
    Ok(())
}

/// One line of a script, read before anything runs.
#[derive(Debug, PartialEq)]
enum Step {
    Sleep(Duration),
    Send(Vec<u8>),
    /// `wait` (`want`) or `gone` (not `want`): until the screen shows `text`, or no longer does.
    /// `need` is a `wait` whose timeout always ends the script, before any further key.
    Wait {
        text: String,
        want: bool,
        max: Duration,
        must: bool,
    },
    Quiet {
        time: Duration,
        max: Duration,
    },
    Idle {
        time: Duration,
        name: String,
    },
    Watch {
        time: Duration,
        name: String,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Snap(String),
    TsCopy(String),
    /// The newest transcript as `NAME.jsonl`, with its agents' transcripts and sidecars in
    /// `NAME/subagents/` (where the app's reader looks for them beside a transcript).
    AgentCopy(String),
    /// A picture file in the work folder, put on the clipboard.
    ClipImage(String),
    /// `claude --help` into `NAME.help.txt`.
    Help(String),
    /// Claude Code started again with `--resume` and the id of the branch `NAME`.
    Reopen(String),
    /// Claude Code started again as a new handover session whose handover starts with `text`.
    Handover {
        name: String,
        text: String,
    },
    /// The recorded process's live record (`sessions/<pid>.json`) into `NAME.live.json` and every
    /// transcript of the work folder into `NAME.transcripts/`, as they are now.
    Live(String),
    /// What Claude Code keeps for the recorded session's tasks: every entry of the account's
    /// `tasks` and `todos` folders named after the session or written during the recording,
    /// copied into `NAME.tasks/`.
    TaskFiles(String),
    /// A branch at a message: `prompt TEXT` or `answer TEXT`.
    Branch {
        name: String,
        at: String,
    },
    Start,
}

/// What no `claude` started from here may see: a session's or a login's variables, except the
/// account's config folder (as `core::launch::child_removals`).
fn removed_vars() -> Vec<String> {
    std::env::vars()
        .map(|(name, _)| name)
        .filter(|name| {
            let upper = name.to_ascii_uppercase();
            let claude = upper.starts_with("CLAUDE_") && upper != "CLAUDE_CONFIG_DIR";
            claude || upper.starts_with("ANTHROPIC_") || upper == "CLAUDECODE"
        })
        .collect()
}

/// Windows PowerShell (it has the clipboard's picture calls) with a script, in a single-threaded
/// apartment as the clipboard needs; its standard output.
fn powershell(script: &str, env: &[(&str, &Path)], input: Option<&str>) -> Result<String, Abort> {
    use std::io::Write as _;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-STA", "-Command"]);
    command.arg(format!("[Console]::InputEncoding = [Console]::OutputEncoding = [Text.Encoding]::UTF8; {script}"));
    command.envs(env.iter().copied());
    command.stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
    command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| Abort::Io(format!("powershell: {e}")))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).map_err(|e| Abort::Io(format!("powershell: {e}")))?;
    }
    let output = child.wait_with_output().map_err(|e| Abort::Io(format!("powershell: {e}")))?;
    if !output.status.success() {
        return Err(Abort::Io(format!("powershell: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What `clipimage` found on the clipboard, and the clipboard's sequence number once the picture
/// was put there: a different number at the end means someone copied something meanwhile.
struct Saved {
    text: Option<String>,
    sequence: u32,
}

/// Win32's clipboard sequence number, for PowerShell.
const SEQUENCE: &str = "Add-Type -Namespace CsRecorder -Name Clip -MemberDefinition \
    '[DllImport(\"user32.dll\")] public static extern uint GetClipboardSequenceNumber();';";

/// The formats plain text comes in: anything else on the clipboard is not put back.
/// `EnterpriseDataProtectionId` is only a tag Windows adds beside copied text, not content.
const TEXT_FORMATS: &str = "'Text','UnicodeText','OEMText','Locale','System.String','EnterpriseDataProtectionId'";

/// The clipboard's text, `None` when it is empty. Anything else on it (a picture, files,
/// formatted text) could not be put back, so `clipimage` refuses.
fn clipboard_text() -> Result<Option<String>, Abort> {
    let found = powershell(
        &format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $data = [System.Windows.Forms.Clipboard]::GetDataObject(); \
             $formats = @(); if ($data) {{ $formats = @($data.GetFormats()) }}; \
             $other = @($formats | Where-Object {{ $_ -notin @({TEXT_FORMATS}) }}); \
             if ($other.Count -gt 0) {{ [Console]::Out.Write('O' + ($other -join ', ')) }} \
             elseif ([System.Windows.Forms.Clipboard]::ContainsText()) {{ \
             [Console]::Out.Write('T' + [System.Windows.Forms.Clipboard]::GetText()) }} \
             else {{ [Console]::Out.Write('N') }}"
        ),
        &[],
        None,
    )?;
    match found.split_at_checked(1) {
        Some(("T", text)) => Ok(Some(text.to_owned())),
        Some(("N", _)) => Ok(None),
        Some(("O", formats)) => Err(Abort::Refused(format!(
            "clipimage: the clipboard holds more than plain text ({formats}), which could not be put \
             back afterwards; copy some text or nothing and run it again"
        ))),
        _ => Err(Abort::Io(format!("clipboard: {found:?}"))),
    }
}

/// Puts the clipboard's text back (or empties it when it held none), unless it changed since
/// `clipimage`: then what is there now is someone's and stays. True when it was put back.
fn clipboard_put_back(saved: Saved) -> Result<bool, Abort> {
    let now = powershell(
        &format!("{SEQUENCE} [Console]::Out.Write([CsRecorder.Clip]::GetClipboardSequenceNumber())"),
        &[],
        None,
    )?;
    if now.trim().parse::<u32>().ok() != Some(saved.sequence) {
        return Ok(false);
    }
    let script = match saved.text {
        Some(_) => {
            "Add-Type -AssemblyName System.Windows.Forms; \
            [System.Windows.Forms.Clipboard]::SetText([Console]::In.ReadToEnd())"
        }
        None => "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::Clear()",
    };
    powershell(script, &[], Some(saved.text.as_deref().unwrap_or(""))).map(|_| true)
}

/// Runs a program to its end, at most `max`, and gives its output; stops it when asked.
fn run_for_output(
    mut command: std::process::Command,
    max: Duration,
    stop: Option<Arc<AtomicBool>>,
) -> Result<std::process::Output, Abort> {
    let name = command.get_program().to_string_lossy().into_owned();
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Abort::Io(format!("{name}: {e}")))?;
    let id = child.id();
    let done = Arc::new(AtomicBool::new(false));
    let watch = {
        let done = done.clone();
        std::thread::spawn(move || {
            let end = Instant::now() + max;
            while !done.load(Ordering::Relaxed) {
                let stopped = stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed));
                if stopped || Instant::now() > end {
                    // The child's own handle is with `wait_with_output`: end it by its id.
                    let _ = std::process::Command::new("taskkill").args(["/F", "/T", "/PID", &id.to_string()]).output();
                    return stopped;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            false
        })
    };
    let output = child.wait_with_output().map_err(|e| Abort::Io(format!("{name}: {e}")));
    done.store(true, Ordering::Relaxed);
    if watch.join().unwrap_or(false) {
        return Err(Abort::Stopped);
    }
    output
}

/// A bracketed paste as Chat sends a message of several lines (CR between them): `text`, then
/// numbered filler lines until the text is `units` characters long (ASCII, so as many UTF-16
/// units as the app's limits count).
fn long_paste(units: usize, text: &str) -> Vec<u8> {
    let mut body = text.to_owned();
    let mut n = 0;
    while body.len() < units {
        n += 1;
        body.push_str(&format!("\rFiller line {n}: these lines only make the message long; ignore them."));
    }
    body.truncate(units.max(text.len()));
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend(body.as_bytes());
    bytes.extend(b"\x1b[201~");
    bytes
}

fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    text.trim().parse().map_err(|_| format!("{text:?} is not {what}"))
}

impl Step {
    fn read(line: &str) -> Result<Step, String> {
        let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
        let ms = |s: &str| number::<u64>(s, "a number of milliseconds").map(Duration::from_millis);
        let two = |what: &str| rest.split_once(' ').ok_or_else(|| format!("{op} needs {what}"));
        Ok(match op {
            "sleep" => Step::Sleep(ms(rest)?),
            "send" => Step::Send(unescape(rest)?),
            "wait" | "gone" | "need" => {
                let (text, max) = rest.rsplit_once(' ').ok_or_else(|| format!("{op} needs TEXT MS"))?;
                Step::Wait { text: text.to_owned(), want: op != "gone", max: ms(max)?, must: op == "need" }
            }
            "quiet" => {
                let (time, max) = two("MS MAX")?;
                Step::Quiet { time: ms(time)?, max: ms(max)? }
            }
            "idle" | "watch" => {
                let (time, name) = two("MS NAME")?;
                let (time, name) = (ms(time)?, name.to_owned());
                if op == "idle" { Step::Idle { time, name } } else { Step::Watch { time, name } }
            }
            "resize" => {
                let (cols, rows) = two("COLS ROWS")?;
                Step::Resize { cols: number(cols, "a number of columns")?, rows: number(rows, "a number of rows")? }
            }
            "snap" => Step::Snap(rest.to_owned()),
            "longpaste" => {
                let (units, text) = two("UNITS TEXT")?;
                Step::Send(long_paste(number(units, "a number of characters")?, text))
            }
            "tscopy" => Step::TsCopy(rest.to_owned()),
            "agentcopy" if !rest.is_empty() => Step::AgentCopy(rest.to_owned()),
            "agentcopy" => return Err("agentcopy needs NAME".to_owned()),
            "clipimage" if !rest.is_empty() => Step::ClipImage(rest.to_owned()),
            "clipimage" => return Err("clipimage needs FILE".to_owned()),
            "help" => Step::Help(rest.to_owned()),
            "reopen" if !rest.is_empty() => Step::Reopen(rest.to_owned()),
            "handover" => {
                let (name, text) = two("NAME TEXT")?;
                Step::Handover { name: name.to_owned(), text: text.to_owned() }
            }
            "reopen" => return Err("reopen needs NAME".to_owned()),
            "branch" => {
                let (name, at) = two("NAME prompt|answer TEXT")?;
                let (kind, text) = at.split_once(' ').unwrap_or((at, ""));
                if !matches!(kind, "prompt" | "answer") || text.is_empty() {
                    return Err(format!("{op} needs NAME prompt|answer TEXT"));
                }
                Step::Branch { name: name.to_owned(), at: at.to_owned() }
            }
            "live" if !rest.is_empty() => Step::Live(rest.to_owned()),
            "taskfiles" if !rest.is_empty() => Step::TaskFiles(rest.to_owned()),
            "taskfiles" => return Err("taskfiles needs NAME".to_owned()),
            "live" => return Err("live needs NAME".to_owned()),
            "start" => Step::Start,
            _ => return Err(format!("unknown op {op}")),
        })
    }
}

/// `recorder <script> <workdir> <cols> <rows> <outdir> [claude args...]`
pub fn main(args: &[String]) {
    let script = std::fs::read_to_string(&args[1]).expect("script file");
    let (cols, rows): (u16, u16) = (args[3].parse().expect("columns"), args[4].parse().expect("rows"));
    let out = PathBuf::from(&args[5]);
    std::fs::create_dir_all(&out).expect("output folder");
    let env: Vec<(String, String)> = std::env::var("REC_ENV")
        .unwrap_or_default()
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').expect("REC_ENV as NAME=value;NAME=value");
            (name.to_owned(), value.to_owned())
        })
        .collect();
    // REC_PROGRAM records another program's screen (the recorder's own `compat` screen).
    let program = std::env::var("REC_PROGRAM").unwrap_or_else(|_| "claude".to_owned());
    let launch = Launch {
        program: &program,
        cwd: Path::new(&args[2]),
        cols,
        rows,
        args: &args[6..],
        env: &env,
        plain: std::env::var("REC_PLAIN").is_ok_and(|v| v == "1"),
    };
    let mut recorder = Recorder::spawn(&launch, out).expect("start claude");
    recorder.echo = true;
    if let Err(why) = recorder.run(&script) {
        eprintln!("{why}");
        std::process::exit(1);
    }
}

/// Copies a file, or a folder and everything under it, to `to`; the number of files copied.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<usize> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        let mut count = 0;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            count += copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(count)
    } else if meta.is_file() {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
        Ok(1)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_shown_on_the_screen_is_starred_out_and_nothing_else_changes() {
        let row = "  ANTHROPIC_API_KEY: sk-ant-...23456789abcdefghijAA  \n  Do you want to use this API key?";
        assert_eq!(
            redact(row),
            "  ANTHROPIC_API_KEY: sk-ant-***********************  \n  Do you want to use this API key?"
        );
        assert_eq!(redact("sk-ant-abc\x1b[0m sk-ant-"), "sk-ant-***\x1b[0m sk-ant-");
        assert_eq!(redact("no key here"), "no key here");
    }

    #[test]
    #[cfg(windows)]
    fn the_clipboard_is_left_alone_when_it_changed_since_clipimage() {
        // No sequence number is u32::MAX this soon after start-up: the clipboard counts as changed,
        // and nothing is written to it.
        let saved = Saved { text: Some("not put back".into()), sequence: u32::MAX };
        assert!(!clipboard_put_back(saved).unwrap());
    }

    #[test]
    fn escapes_in_a_send_become_the_keys_they_name() {
        assert_eq!(unescape(r"a\sb\r").unwrap(), b"a b\r");
        assert_eq!(unescape(r"\e[A\x03\\").unwrap(), b"\x1b[A\x03\\");
        assert!(unescape(r"\x4").is_err() && unescape(r"\xzz").is_err(), "no panic on a short or bad \\x");
    }

    #[test]
    fn a_long_paste_is_one_bracketed_paste_of_exactly_the_length_asked_for() {
        let Ok(Step::Send(bytes)) = Step::read("longpaste 15000 Reply: lima.") else { panic!() };
        let body = bytes.strip_prefix(b"\x1b[200~").and_then(|b| b.strip_suffix(b"\x1b[201~")).unwrap();
        assert_eq!(body.len(), 15000);
        assert!(body.starts_with(b"Reply: lima.\rFiller line 1: "));
        assert!(!body.contains(&b'\n') && !body.contains(&0x1b));
        assert!(Step::read("longpaste many Reply").is_err());
    }

    #[test]
    fn a_script_line_that_does_not_read_is_an_error_not_a_panic() {
        let ms = Duration::from_millis;
        assert_eq!(Step::read("sleep 250"), Ok(Step::Sleep(ms(250))));
        assert_eq!(
            Step::read("wait for shortcuts 9000"),
            Ok(Step::Wait { text: "for shortcuts".into(), want: true, max: ms(9000), must: false })
        );
        assert_eq!(
            Step::read("need ⌕ 3000"),
            Ok(Step::Wait { text: "⌕".into(), want: true, max: ms(3000), must: true })
        );
        assert_eq!(Step::read("resize 80 24"), Ok(Step::Resize { cols: 80, rows: 24 }));
        assert_eq!(Step::read("send hi\\r"), Ok(Step::Send(b"hi\r".to_vec())));
        assert_eq!(Step::read("clipimage dot.png"), Ok(Step::ClipImage("dot.png".into())));
        assert_eq!(Step::read("help cli"), Ok(Step::Help("cli".into())));
        assert_eq!(Step::read("agentcopy sent"), Ok(Step::AgentCopy("sent".into())));
        assert_eq!(
            Step::read("handover new Reply with kilo"),
            Ok(Step::Handover { name: "new".into(), text: "Reply with kilo".into() })
        );
        assert_eq!(
            Step::read("branch cut answer alpha"),
            Ok(Step::Branch { name: "cut".into(), at: "answer alpha".into() })
        );
        for bad in [
            "jump 3",
            "sleep soon",
            "wait 9000x",
            "gone",
            "quiet 500",
            "idle x name",
            "resize 80",
            "resize 80 lots",
            "send \\x4",
            "clipimage",
            "agentcopy",
            "reopen",
            "handover new",
            "branch cut",
            "branch cut alpha",
        ] {
            assert!(Step::read(bad).is_err(), "{bad}");
        }
    }
}
