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
/// The dialog about an API key from the environment; its first row, above the highlighted "No",
/// is "Yes".
const KEY_DIALOG: &str = "Do you want to use this API key?";
const TRUST_DIALOG: &str = "trust this folder";

fn unescape(s: &str) -> Vec<u8> {
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
                    out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).expect("two hex digits after \\x"));
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
    out
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

/// Why a script ended before its last line.
#[derive(Debug)]
pub enum Abort {
    /// Asked to stop from outside.
    Stopped,
    /// The CLI did not reach its input.
    Start(String),
    /// A `wait` or `gone` ran out of time (only when the recorder is strict).
    Timeout(String),
}

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Abort::Stopped => f.write_str("stopped"),
            Abort::Start(why) => write!(f, "Claude Code did not reach its input: {why}"),
            Abort::Timeout(what) => write!(f, "the screen never showed what the script waited for: {what}"),
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
}

impl Recorder {
    pub fn spawn(launch: &Launch, out: PathBuf) -> std::io::Result<Recorder> {
        // As Claude Switch starts a Claude tab (`core::launch::child_removals`, `termapp`): no
        // CLAUDE_* or ANTHROPIC_* variable reaches the child except the account's config folder;
        // the classic renderer; TERM=xterm-256color unless a Start-menu environment is asked for.
        let mut cmd = Command::new(launch.program).cwd(launch.cwd);
        for (name, _) in std::env::vars() {
            let upper = name.to_ascii_uppercase();
            let claude = upper.starts_with("CLAUDE_") && upper != "CLAUDE_CONFIG_DIR";
            if claude || upper.starts_with("ANTHROPIC_") || upper == "CLAUDECODE" {
                cmd = cmd.env_remove(name);
            }
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
        for arg in launch.args {
            cmd = cmd.arg(arg);
        }
        let pty = Pty::spawn(&cmd, launch.cols, launch.rows, || {})?;
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
        })
    }

    /// Runs a script's lines; the log is written to `log.txt` however it ends.
    pub fn run(&mut self, script: &str) -> Result<(), Abort> {
        let mut result = Ok(());
        for line in script.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
            self.note(&format!(">> {line}"));
            result = self.step(op, rest);
            if let Err(why) = &result {
                self.note(&format!("ABORT {why}"));
                self.snap("aborted");
                break;
            }
        }
        let _ = self.pty.kill();
        self.publish(true);
        std::fs::write(self.out.join("log.txt"), &self.log).expect("write log");
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

    fn snap(&mut self, name: &str) {
        std::fs::write(self.out.join(format!("{name}.ansi")), redact(&self.ansi())).expect("write snapshot");
        std::fs::write(self.out.join(format!("{name}.txt")), redact(&self.text())).expect("write snapshot");
        let c = self.screen.cursor();
        let quiet = self.last_output.elapsed().as_millis();
        self.note(&format!("snap {name} cursor {} {} quiet {quiet} ms", c.row, c.col));
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

    /// Whether `done` came true within `max_ms`.
    fn until(&mut self, max_ms: &str, mut done: impl FnMut(&Recorder) -> bool) -> Result<bool, Abort> {
        let end = Instant::now() + Duration::from_millis(max_ms.parse().expect("milliseconds"));
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
            if self.until("200", ready)? {
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

    fn step(&mut self, op: &str, rest: &str) -> Result<(), Abort> {
        let ms = |s: &str| Duration::from_millis(s.parse().expect("milliseconds"));
        match op {
            "sleep" => self.run_for(ms(rest))?,
            "send" => {
                self.pump();
                self.write(&unescape(rest));
            }
            "wait" | "gone" => {
                let (text, max) = rest.rsplit_once(' ').expect("TEXT MS");
                let want = op == "wait";
                if !self.until(max, |r| r.has(text) == want)? {
                    self.note(&format!("TIMEOUT {op} {text}"));
                    if self.strict {
                        return Err(Abort::Timeout(format!("{op} {text}")));
                    }
                }
            }
            "quiet" => {
                let (time, max) = rest.split_once(' ').expect("MS MAX");
                let time = ms(time);
                if !self.until(max, |r| r.last_output.elapsed() >= time)? {
                    self.note("TIMEOUT quiet");
                }
            }
            "idle" => {
                let (time, name) = rest.split_once(' ').expect("MS NAME");
                self.verbose = true;
                let ran = self.run_for(ms(time));
                self.verbose = false;
                ran?;
                self.snap(name);
            }
            "watch" => {
                let (time, name) = rest.split_once(' ').expect("MS NAME");
                let end = Instant::now() + ms(time);
                let (mut last, mut n) = (String::new(), 0);
                while Instant::now() < end {
                    self.run_for(Duration::from_secs(1))?;
                    let text = self.text();
                    if text != last {
                        last = text;
                        n += 1;
                        self.snap(&format!("{name}-{n:03}"));
                    }
                }
            }
            "resize" => {
                let (cols, rows) = rest.split_once(' ').expect("COLS ROWS");
                let (cols, rows): (u16, u16) = (cols.parse().expect("columns"), rows.parse().expect("rows"));
                self.pump();
                self.screen.resize(cols.into(), rows.into());
                self.pty.resize(cols, rows).expect("resize");
            }
            "snap" => {
                self.pump();
                self.snap(rest);
            }
            // The newest transcript in REC_PROJECT, as it is on disk now.
            "tscopy" => {
                let dir = std::env::var("REC_PROJECT").expect("REC_PROJECT for tscopy");
                let newest = std::fs::read_dir(&dir)
                    .expect("REC_PROJECT folder")
                    .filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                    .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
                match newest {
                    Some(e) => {
                        let len = std::fs::copy(e.path(), self.out.join(format!("{rest}.jsonl"))).expect("copy");
                        self.note(&format!("tscopy {rest} {len} bytes"));
                    }
                    None => self.note("tscopy: no transcript"),
                }
            }
            "start" => self.start()?,
            _ => panic!("unknown op {op}"),
        }
        Ok(())
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
    fn escapes_in_a_send_become_the_keys_they_name() {
        assert_eq!(unescape(r"a\sb\r"), b"a b\r");
        assert_eq!(unescape(r"\e[A\x03\\"), b"\x1b[A\x03\\");
    }
}
