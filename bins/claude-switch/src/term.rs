//! One terminal of a tab: the program in its pseudo-terminal (`mdshared_pty`), the screen its
//! output is fed into (`mdshared_term`), and what the window keeps of how it is looked at: the
//! scroll offset, the selection, a press that is being reported to the program. No drawing and
//! no events here; [`crate::termapp`] does those.

use std::io;

use claude_switch_core::Launch;
use mdshared_pty::{Command, Exit, Pty};
use mdshared_term::{MouseButton, Point, Screen, Selection};
use mdshared_ui::termview::View;

/// Lines of scrollback a terminal keeps.
pub const SCROLLBACK: usize = 10_000;
/// The size a terminal starts at before the window has laid one out.
pub const FIRST_SIZE: (u16, u16) = (100, 30);

/// The command a terminal tab runs for a launch: its program, arguments, folder and
/// environment, in that order; the environment's removals first, so that a variable that is both
/// removed and set ends up set.
pub fn command(launch: &Launch) -> Command {
    command_with_removals(launch, claude_switch_core::launch::child_removals(launch))
}

fn command_with_removals(launch: &Launch, removals: impl IntoIterator<Item = std::ffi::OsString>) -> Command {
    let mut c = Command::new(&launch.program).args(&launch.args);
    if let Some(dir) = &launch.cwd {
        c = c.cwd(dir);
    }
    for name in removals {
        c = c.env_remove(name);
    }
    for (k, v) in &launch.env {
        c = c.env(k, v);
    }
    c
}

/// The line written under a program's last output when it has ended.
pub fn ended_line(exit: Exit) -> String {
    let code = match exit.code {
        Some(c) if c > 0xFFFF => format!("0x{c:08X}"),
        Some(c) => c.to_string(),
        None => exit.signal.map_or_else(|| "?".to_string(), |s| format!("signal {s}")),
    };
    format!("[ended with code {code} - press Enter to close]")
}

/// What one look at a terminal's program found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Output arrived (and was fed to the screen).
    pub output: bool,
    /// The program set a new title.
    pub title: Option<String>,
    /// The program ended in this look.
    pub ended: bool,
}

pub struct Term {
    /// `None` for a terminal with no program (the tests').
    pub pty: Option<Pty>,
    pub screen: Screen,
    /// How many lines back into the scrollback the view looks.
    pub scroll: usize,
    pub selection: Option<Selection>,
    /// A drag that selects is held.
    pub selecting: bool,
    /// A press reported to the program is held, and the last cell reported.
    pub reported: Option<MouseButton>,
    pub last_cell: Option<(usize, usize)>,
    /// The part of a wheel line not yet scrolled.
    pub wheel_rest: f32,
    /// How the program ended, once it has and its last output is on the screen.
    pub ended: Option<Exit>,
}

impl Term {
    /// Starts `command` in a terminal of `cols` by `rows`. `wake` is called from another thread
    /// on output and at the end.
    pub fn spawn(command: &Command, size: (u16, u16), wake: impl Fn() + Send + Sync + 'static) -> io::Result<Term> {
        let pty = Pty::spawn(command, size.0, size.1, wake)?;
        let mut t = Term::empty(size);
        t.pty = Some(pty);
        Ok(t)
    }

    /// A terminal with no program behind it.
    pub fn empty(size: (u16, u16)) -> Term {
        Term {
            pty: None,
            screen: Screen::new(size.0.into(), size.1.into(), SCROLLBACK),
            scroll: 0,
            selection: None,
            selecting: false,
            reported: None,
            last_cell: None,
            wheel_rest: 0.0,
            ended: None,
        }
    }

    /// The program runs (closing the tab ends it).
    pub fn running(&self) -> bool {
        self.ended.is_none() && self.pty.as_ref().is_some_and(Pty::running)
    }

    /// Bytes for the program; nothing once it has ended.
    pub fn write(&self, bytes: &[u8]) {
        if self.ended.is_some() {
            return;
        }
        if let Some(p) = &self.pty {
            let _ = p.write(bytes);
        }
    }

    /// Takes the program's output into the screen, answers what it asked, and notes its end.
    pub fn pump(&mut self) -> Pumped {
        let mut out = Pumped::default();
        let Some(pty) = &mut self.pty else { return out };
        let bytes = pty.take_output();
        let exit = pty.exit();
        if !bytes.is_empty() {
            self.feed(&bytes);
            out.output = true;
            let reply = self.screen.take_reply();
            self.write(&reply);
        }
        if let Some(title) = self.screen.take_title() {
            out.title = Some(title);
        }
        if self.ended.is_none()
            && let Some(exit) = exit
        {
            self.ended = Some(exit);
            // The cursor is hidden: nothing more can be typed.
            let line = format!("\x1b[0m\r\n{}\x1b[?25l", ended_line(exit));
            self.screen.feed(line.as_bytes());
            out.ended = true;
            out.output = true;
        }
        out
    }

    /// Feeds output to the screen, keeping a view that looks back on the same lines and
    /// dropping a selection whose text was rewritten.
    pub fn feed(&mut self, bytes: &[u8]) {
        let before = self.screen.screen_start();
        self.screen.feed(bytes);
        if self.scroll > 0 {
            let moved = self.screen.screen_start().saturating_sub(before) as usize;
            self.scroll = (self.scroll + moved).min(self.screen.scrollback_len());
        }
        if let Some(sel) = &mut self.selection
            && !sel.is_empty()
            && !sel.follow(&self.screen)
        {
            self.selection = None;
            self.selecting = false;
        }
    }

    /// A new size for the screen and the program. True when it changed.
    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if (cols, rows) == (self.screen.cols(), self.screen.rows()) {
            return false;
        }
        self.screen.resize(cols, rows);
        if let Some(p) = &mut self.pty {
            let _ = p.resize(cols.min(u16::MAX as usize) as u16, rows.min(u16::MAX as usize) as u16);
        }
        self.scroll = self.scroll.min(self.screen.scrollback_len());
        if let Some(sel) = &mut self.selection
            && !sel.clip(&self.screen)
        {
            self.selection = None;
        }
        true
    }

    /// Scrolls the view by `lines`, back into the scrollback for positive ones.
    pub fn scroll_by(&mut self, lines: i32) {
        let max = self.screen.scrollback_len() as i64;
        self.scroll = (self.scroll as i64 + i64::from(lines)).clamp(0, max) as usize;
    }

    /// The view to draw it with.
    pub fn view(&self, focused: bool) -> View {
        View::new().scrolled(self.scroll).focused(focused)
    }

    /// Extend the drag, reporting only changes to the cells the view highlights.
    pub fn extend_selection(&mut self, at: Point) -> bool {
        let Some(sel) = &mut self.selection else { return false };
        let extent = |s: &Selection| (!s.is_empty()).then(|| s.range());
        let before = extent(sel);
        sel.extend(&self.screen, at);
        before != extent(sel)
    }

    /// Tab switches can lose the matching mouse-up; end any drag on the original tab.
    pub fn stop_pointer(&mut self) {
        self.selecting = false;
        if let (Some(button), Some((row, col))) = (self.reported.take(), self.last_cell.take()) {
            let ev = mdshared_term::MouseEvent {
                kind: mdshared_term::MouseKind::Release,
                button: Some(button),
                col,
                row,
                mods: mdshared_term::Mods::NONE,
            };
            if let Some(bytes) = mdshared_term::input::encode_mouse(ev, &self.screen.modes()) {
                self.write(&bytes);
            }
        }
    }

    /// The selected text, when something is selected.
    pub fn selected_text(&self) -> Option<String> {
        self.selection.filter(|s| !s.is_empty()).map(|s| s.text(&self.screen)).filter(|t| !t.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::Place;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn a_launch_becomes_the_command_with_its_program_arguments_folder_and_environment() {
        let mut env = BTreeMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), r"C:\a\cli".to_string());
        let l = Launch {
            program: "claude".into(),
            args: vec!["--resume".into(), "abc def".into()],
            cwd: Some(PathBuf::from(r"C:\work\my project")),
            env,
            env_remove: vec!["ANTHROPIC_API_KEY".into(), "CLAUDECODE".into()],
            place: Place::Terminal,
            label: "main · x · Resume".into(),
        };
        let want = Command::new("claude")
            .arg("--resume")
            .arg("abc def")
            .cwd(r"C:\work\my project")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("CLAUDECODE")
            .env("CLAUDE_CONFIG_DIR", r"C:\a\cli");
        assert_eq!(command_with_removals(&l, l.env_remove.iter().map(std::ffi::OsString::from)), want);
        let bare = Launch { cwd: None, env: BTreeMap::new(), env_remove: Vec::new(), args: Vec::new(), ..l };
        assert_eq!(command_with_removals(&bare, []), Command::new("claude"));
    }

    #[test]
    fn the_last_line_says_how_the_program_ended() {
        assert_eq!(ended_line(Exit { code: Some(0), signal: None }), "[ended with code 0 - press Enter to close]");
        assert_eq!(ended_line(Exit { code: Some(3), signal: None }), "[ended with code 3 - press Enter to close]");
        assert_eq!(
            ended_line(Exit { code: Some(0xC000_013A), signal: None }),
            "[ended with code 0xC000013A - press Enter to close]"
        );
    }

    #[test]
    fn a_view_scrolled_back_stays_on_its_lines_while_output_comes() {
        let mut t = Term::empty((20, 4));
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.scroll_by(3);
        let top = t.view(true).top_line(&t.screen);
        t.feed(b"more\r\nand more\r\n");
        assert_eq!(t.view(true).top_line(&t.screen), top);
        t.scroll_by(-100);
        assert_eq!(t.scroll, 0);
        t.scroll_by(1000);
        assert_eq!(t.scroll, t.screen.scrollback_len());
    }

    #[test]
    fn a_resize_changes_the_screen_once_and_a_terminal_without_a_program_is_not_running() {
        let mut t = Term::empty((20, 4));
        assert!(!t.running());
        assert!(t.resize(30, 5));
        assert!(!t.resize(30, 5));
        assert_eq!((t.screen.cols(), t.screen.rows()), (30, 5));
        assert_eq!(t.pump(), Pumped::default());
    }

    #[test]
    fn a_drag_reports_only_changes_to_selected_cells() {
        use mdshared_term::SelectionKind;

        let mut t = Term::empty((20, 4));
        t.feed(b"one two\r\nnext");
        let row = t.screen.screen_start();
        let p = |col| Point::new(row, col);
        assert!(!t.extend_selection(p(1)));
        t.selection = Some(Selection::start(&t.screen, p(0), SelectionKind::Stream, 1));
        assert!(!t.extend_selection(p(0)));
        assert!(t.extend_selection(p(1)));
        assert!(!t.extend_selection(p(1)));
        assert!(t.extend_selection(p(0)));
        t.selection = Some(Selection::start(&t.screen, p(0), SelectionKind::Stream, 2));
        assert!(!t.extend_selection(p(2)));
        assert!(t.extend_selection(p(4)));
        assert!(!t.extend_selection(p(6)));
        t.selection = Some(Selection::start(&t.screen, p(0), SelectionKind::Stream, 3));
        assert!(!t.extend_selection(p(6)));
        assert!(t.extend_selection(Point::new(row + 1, 0)));
    }

    #[test]
    fn a_block_drag_can_change_its_endpoint_without_changing_its_highlight() {
        use mdshared_term::SelectionKind;

        let mut t = Term::empty((10, 4));
        t.feed(b"aa bb ccccd xx");
        let row = t.screen.screen_start();
        t.selection = Some(Selection::start(&t.screen, Point::new(row, 6), SelectionKind::Block, 2));
        let before = t.selection;
        // The anchor word wraps. The adjacent word is already inside the block's rectangle.
        assert!(!t.extend_selection(Point::new(row, 3)));
        assert_ne!(before, t.selection, "the drag still tracked the new endpoint");
        assert!(t.extend_selection(Point::new(row + 1, 7)));
    }

    #[test]
    fn losing_the_tab_ends_a_drag_without_erasing_its_selection() {
        let mut term = Term::empty((10, 3));
        let at = Point::new(term.screen.screen_start(), 0);
        term.selection = Some(Selection::start(&term.screen, at, mdshared_term::SelectionKind::Stream, 1));
        term.selecting = true;
        term.reported = Some(MouseButton::Left);
        term.last_cell = Some((0, 0));
        term.stop_pointer();
        assert!(term.selection.is_some() && !term.selecting);
        assert_eq!((term.reported, term.last_cell), (None, None));
    }
}
