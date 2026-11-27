//! Records what Claude Code draws, for Chat's screen fixtures. See README.md.
//!
//! recorder <script> <workdir> <cols> <rows> <outdir> [claude args...]
use std::fmt::Write as _;
use std::path::PathBuf;
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

struct Recorder {
    pty: Pty,
    screen: Screen,
    start: Instant,
    last_output: Instant,
    log: String,
    out: PathBuf,
    verbose: bool,
}

impl Recorder {
    fn note(&mut self, text: &str) {
        let line = format!("{:>7} {text}\n", self.start.elapsed().as_millis());
        eprint!("{line}");
        self.log.push_str(&line);
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
        std::fs::write(self.out.join(format!("{name}.ansi")), self.ansi()).expect("write snapshot");
        std::fs::write(self.out.join(format!("{name}.txt")), self.text()).expect("write snapshot");
        let c = self.screen.cursor();
        let quiet = self.last_output.elapsed().as_millis();
        self.note(&format!("snap {name} cursor {} {} quiet {quiet} ms", c.row, c.col));
    }

    fn has(&self, text: &str) -> bool {
        (0..self.screen.rows()).any(|r| self.screen.row_text(r).contains(text))
    }

    fn run_for(&mut self, time: Duration) {
        let end = Instant::now() + time;
        while Instant::now() < end {
            self.pump();
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn until(&mut self, max_ms: &str, mut done: impl FnMut(&Recorder) -> bool) -> bool {
        let end = Instant::now() + Duration::from_millis(max_ms.parse().expect("milliseconds"));
        loop {
            self.pump();
            if done(self) {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn step(&mut self, op: &str, rest: &str) {
        let ms = |s: &str| Duration::from_millis(s.parse().expect("milliseconds"));
        match op {
            "sleep" => self.run_for(ms(rest)),
            "send" => {
                self.pump();
                self.pty.write(&unescape(rest)).expect("write to the terminal");
            }
            "wait" | "gone" => {
                let (text, max) = rest.rsplit_once(' ').expect("TEXT MS");
                let want = op == "wait";
                if !self.until(max, |r| r.has(text) == want) {
                    self.note(&format!("TIMEOUT {op} {text}"));
                }
            }
            "quiet" => {
                let (time, max) = rest.split_once(' ').expect("MS MAX");
                let time = ms(time);
                if !self.until(max, |r| r.last_output.elapsed() >= time) {
                    self.note("TIMEOUT quiet");
                }
            }
            "idle" => {
                let (time, name) = rest.split_once(' ').expect("MS NAME");
                self.verbose = true;
                self.run_for(ms(time));
                self.verbose = false;
                self.snap(name);
            }
            "watch" => {
                let (time, name) = rest.split_once(' ').expect("MS NAME");
                let end = Instant::now() + ms(time);
                let (mut last, mut n) = (String::new(), 0);
                while Instant::now() < end {
                    self.run_for(Duration::from_secs(1));
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
            "snap" => self.snap(rest),
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
            // Start-up: answers the folder trust dialog when it shows, then waits for the input.
            "start" => {
                let ready = |r: &Recorder| r.has("shift+tab to cycle") || r.has("for shortcuts");
                if !self.until("40000", |r| r.has("trust this folder") || ready(r)) {
                    self.note("TIMEOUT start");
                }
                if self.has("trust this folder") {
                    self.run_for(Duration::from_millis(800));
                    self.pty.write(b"\x1b[B").expect("write");
                    self.run_for(Duration::from_millis(400));
                    self.pty.write(b"\r").expect("write");
                    if !self.until("40000", ready) {
                        self.note("TIMEOUT start after trust");
                    }
                }
                self.run_for(Duration::from_millis(1500));
            }
            _ => panic!("unknown op {op}"),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("recorder <script> <workdir> <cols> <rows> <outdir> [claude args...]   (see README.md)");
        std::process::exit(2);
    }
    let script = std::fs::read_to_string(&args[1]).expect("script file");
    let (cols, rows): (u16, u16) = (args[3].parse().expect("columns"), args[4].parse().expect("rows"));
    let out = PathBuf::from(&args[5]);
    std::fs::create_dir_all(&out).expect("output folder");
    // As Claude Switch starts a Claude tab (`core::launch::child_removals`, `termapp`): no CLAUDE_*
    // or ANTHROPIC_* variable reaches the child except the account's config folder; the classic
    // renderer; TERM=xterm-256color unless REC_PLAIN asks for a Start-menu environment.
    let mut cmd = Command::new("claude").cwd(&args[2]);
    for (name, _) in std::env::vars() {
        let upper = name.to_ascii_uppercase();
        let claude = upper.starts_with("CLAUDE_") && upper != "CLAUDE_CONFIG_DIR";
        if claude || upper.starts_with("ANTHROPIC_") || upper == "CLAUDECODE" {
            cmd = cmd.env_remove(name);
        }
    }
    cmd = cmd.env("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN", "1");
    if std::env::var("REC_PLAIN").is_ok_and(|v| v == "1") {
        for name in SHELL_VARS {
            cmd = cmd.env_remove(name);
        }
    } else {
        cmd = cmd.env("TERM", "xterm-256color");
    }
    for pair in std::env::var("REC_ENV").unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').expect("REC_ENV as NAME=value;NAME=value");
        cmd = cmd.env(name, value);
    }
    for arg in &args[6..] {
        cmd = cmd.arg(arg);
    }
    let pty = Pty::spawn(&cmd, cols, rows, || {}).expect("start claude");
    let now = Instant::now();
    let screen = Screen::new(cols.into(), rows.into(), 2000);
    let mut recorder = Recorder { pty, screen, start: now, last_output: now, log: String::new(), out, verbose: false };
    for line in script.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
        recorder.note(&format!(">> {line}"));
        recorder.step(op, rest);
    }
    let _ = recorder.pty.kill();
    std::fs::write(recorder.out.join("log.txt"), &recorder.log).expect("write log");
}
