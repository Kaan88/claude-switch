//! The README's pictures in `docs/screenshots`, drawn by the window's own code from the demo data
//! (never a real session or account, and no window opens): whole frames of a 1400 x 880 window,
//! at twice the scale for sharp text. To draw them again:
//!
//! ```text
//! CS_README_SHOTS=docs/screenshots cargo test -p claude-switch -- --ignored readme_screenshots
//! ```
//!
//! `CS_README_ONLY=hero,terminal` draws only those.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mdshared_text::Bitmap;
use mdshared_ui::anim::Animator;
use mdshared_ui::gfx::{Gfx, Painter, Rect};
use mdshared_ui::keys::Mods;
use mdshared_ui::splitter::{self, Axis};

use super::question::screens::recorded::recorded;
use super::{ChatPhase, QUIET};
use crate::dialogs::{self, Dialog};
use crate::source::Source;
use crate::state::AppState;
use crate::tabs::NewTab;
use crate::term::Term;
use crate::view::model;

/// A window's size and display scale for a drawn frame.
#[derive(Clone, Copy)]
pub(crate) struct Size {
    pub w: f32,
    pub h: f32,
    pub scale: f32,
}

/// The README's frames.
const README: Size = Size { w: 1400.0, h: 880.0, scale: 2.0 };

/// The demo window: its accounts, sessions and marks, the first session selected and its
/// details read.
pub(crate) fn window(dark: bool) -> AppState {
    let mut st = AppState::new(Source::Demo, Some(dark), Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.select_first();
    read_details(&mut st);
    st
}

/// Waits for the selected session's last messages, size and related sessions.
fn read_details(st: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while st.details_reader.as_ref().is_some_and(|r| !r.loaded())
        || st.session_size.pending
        || st.grouping.pending.is_some()
    {
        st.take_jobs();
        if let Some(reader) = &st.details_reader {
            reader.poll();
        }
        assert!(Instant::now() < deadline, "the selected session's details were read");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One frame of the whole window at `now`, composed back to front as `app_draw::draw` does it.
fn frame(st: &mut AppState, anim: &mut Animator, now: Instant) -> Bitmap {
    frame_at(st, anim, now, README)
}

/// [`frame`] for a window of another size.
pub(crate) fn frame_at(st: &mut AppState, anim: &mut Animator, now: Instant, size: Size) -> Bitmap {
    let Size { w, h, scale } = size;
    let window = Rect::new(0.0, 0.0, w, h);
    let mut gfx = Gfx::new();
    let mut bmp = Bitmap::new((w * scale) as u32, (h * scale) as u32);
    let mut p = Painter::new(&mut bmp, &mut gfx, scale);
    anim.begin_frame(now);
    let pal = *st.pal();
    st.hits.clear();
    p.fill(window, pal.page);
    crate::topbar::draw(st, &mut p, w, false, false, false);
    let (top, _) = crate::topbar::draw_below_title_with(st, anim, &mut p, w);
    let top = st.draw_tab_restore(&mut p, top, w);
    st.main_top = top;
    let area = Rect::new(0.0, top, w, h - top);
    st.session_view.area = area;
    let term_area = if st.chat_active() { st.layout_chat(area) } else { area };
    if !st.tabs.is_empty() {
        st.layout_terminals(term_area, area, scale);
    }
    if st.terminal_shown() {
        if st.chat_active() {
            st.draw_chat_with(&mut p, area, &pal, now, false, true);
            if term_area.h > 0.0 {
                st.draw_terminal_still(&mut p, term_area);
            }
        } else {
            st.draw_terminal_still(&mut p, term_area);
        }
    } else if st.tabs.workers_shown() {
        crate::workers_tab::draw(st, &mut p, area);
    } else {
        let rects = st.splitter.rects(area, Axis::Horizontal);
        crate::list::draw_with(st, anim, &mut p, rects.0, now);
        crate::details::draw(st, &mut p, rects.2);
        splitter::draw(&mut p, &pal, rects, Axis::Horizontal, false);
    }
    dialogs::draw_with(st, anim, now, &mut p, window, 1.0);
    anim.end_frame();
    bmp
}

/// A frame after the window's animations (tab marks, fades) have settled.
fn settled(st: &mut AppState, now: Instant) -> Bitmap {
    settled_at(st, now, README)
}

/// [`settled`] for a window of another size.
pub(crate) fn settled_at(st: &mut AppState, now: Instant, size: Size) -> Bitmap {
    let mut anim = Animator::new();
    for k in 0..15 {
        frame_at(st, &mut anim, now + Duration::from_millis(16 * k), size);
    }
    frame_at(st, &mut anim, now + Duration::from_secs(1), size)
}

/// A terminal tab beside the shown one, labelled like a session's.
fn background_tab(st: &mut AppState, label: &str, account: &str, waiting: bool) -> u64 {
    let color = st.account_color(account);
    let shown = st.tabs.active;
    let id = st.tabs.open(
        NewTab { label: label.into(), color, session: None, account: Some(account.into()) },
        Term::empty((100, 40)),
    );
    st.tabs.get_mut(id).unwrap().attention.marked = waiting;
    st.tabs.active = shown;
    id
}

/// The shown tab, for a Claude session, with two tabs beside it.
pub(super) fn claude_tab(st: &mut AppState, beside: &str) -> u64 {
    let account = st.current().slug.clone();
    let color = st.account_color(&account);
    let id = st.tabs.open(
        NewTab { label: "Fix the flaky tile cache test".into(), color, session: None, account: Some(account) },
        Term::empty((100, 40)),
    );
    background_tab(st, "Plan the offline mode", "work", true);
    background_tab(st, beside, "main", false);
    st.tabs.active = Some(id);
    id
}

/// A Claude tab in Chat on the showcase conversation, shown.
pub(super) fn chat_window(dark: bool, demo: &crate::demo::Transcript) -> (AppState, u64) {
    let mut st = window(dark);
    let id = claude_tab(&mut st, "Move the map tiles to a worker thread");
    let mut pane = st.new_session_pane(model::Source::File(demo.path().into()), demo.cwd().into());
    pane.chat.scripted = true;
    pane.chat.enabled = true;
    pane.chat.identity_unverified = false;
    pane.ui.set_chat(true);
    let (wake, events) = std::sync::mpsc::channel();
    pane.model = model::Model::new(model::Source::File(demo.path().into()), demo.cwd().into(), move || {
        let _ = wake.send(());
    });
    pane.model.poll();
    events.recv_timeout(Duration::from_secs(5)).expect("the transcript was read");
    assert!(pane.model.collect());
    tab(&mut st, id).pane = Some(pane);
    (st, id)
}

pub(super) fn tab(st: &mut AppState, id: u64) -> &mut crate::tabs::Tab {
    st.tabs.get_mut(id).unwrap()
}

/// The CLI shows `screen` and is then quiet: Chat reads it as a ready prompt or a card.
pub(super) fn show(st: &mut AppState, id: u64, screen: &str, now: Instant) -> Instant {
    tab(st, id).term.screen = recorded(screen);
    tab(st, id).pane.as_mut().unwrap().chat.output(now, true, false);
    let later = now + QUIET + Duration::from_millis(100);
    st.progress_chat_tabs(later, true);
    later
}

/// A turn the transcript shows running: a newer prompt and an unfinished answer, and the line
/// under the conversation that says what it waits for.
pub(super) fn running(st: &mut AppState, id: u64, waiting: &str) {
    let pane = tab(st, id).pane.as_mut().unwrap();
    pane.ui.set_working(Some(crate::view::working::Footer { label: waiting.into(), stop: None }));
    let at = pane.model.chat.prompt.unwrap_or(0) + 100;
    pane.model.chat.prompt = Some(at);
    pane.model.chat.latest = Some((at + 1, ChatPhase::Active));
    pane.chat.fresh = true;
}

/// Claude Code's input at rest under the conversation.
pub(super) const IDLE: &str = "cursor 33 2
@size 100 40




 ▐▛███▜▌   Claude Code v2.1.332
▝▜█████▛▘  Opus 5.5 · Claude Max
  ▘▘ ▝▝    ~\\Desktop\\trail-planner

────────────────────────────────────────────────────────────────────────────────────────────────────
>
────────────────────────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
";

pub(super) const QUESTION: &str = "cursor 30 49
@size 100 40




────────────────────────────────────────────────────────────────────────────────────────────────────
 ☐ Elevation

Where should the elevation tiles be cached?

❯ 1. Beside the map tiles (Recommended)
     One cache folder and one size limit for both kinds of tile.
  2. In a folder of their own
     A separate limit, so large elevation tiles never push map tiles out.
  3. Only in memory
     Nothing written to disk; they are fetched again after a restart.
  4. Type something.
────────────────────────────────────────────────────────────────────────────────────────────────────
  5. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel
";

pub(super) const PERMISSION: &str = "cursor 31 29
@size 100 40




● Bash(cargo bench -p tiles --bench eviction)
  ⎿  Waiting…

────────────────────────────────────────────────────────────────────────────────────────────────────
 Bash command
 Run the eviction benchmark
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 cargo bench -p tiles --bench eviction
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for cargo bench commands in C:\\Users\\demo\\Desktop\\trail-planner
   3. Yes, and switch to auto mode · auto mode handles these prompts for you
   4. No

 Esc to cancel · Tab to amend
";

/// A Claude Code session as the terminal tab shows it, `cols` wide.
fn terminal_text(cols: usize) -> String {
    let rule = format!("{GREY}{}{RESET}\r\n", "─".repeat(cols));
    let mut text = String::new();
    for line in TERMINAL {
        text.push_str(&line.replace("{G}", GREY).replace("{B}", BOLD).replace("{R}", RESET).replace("{D}", DOT));
        text.push_str("\r\n");
    }
    text.push_str(&rule);
    text.push_str("> \x1b[7m \x1b[0m\r\n");
    text.push_str(&rule);
    text.push_str(&format!("  {GREY}⏵⏵ accept edits on (shift+tab to cycle){RESET}\x1b[?25l"));
    text
}

const GREY: &str = "\x1b[38;2;153;153;153m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
/// The green dot of a finished tool call.
const DOT: &str = "\x1b[38;2;78;186;101m●\x1b[0m";

/// The session's lines: `{G}` grey, `{B}` bold, `{R}` reset, `{D}` a finished call's dot.
const TERMINAL: &[&str] = &[
    "\x1b[38;2;215;119;87m ▐▛███▜▌{R}   {B}Claude Code{R} {G}v2.1.332{R}",
    "\x1b[38;2;215;119;87m▝▜█████▛▘{R}  {G}Opus 5.5 · Claude Max{R}",
    "\x1b[38;2;215;119;87m  ▘▘ ▝▝{R}    {G}~\\Desktop\\trail-planner{R}",
    "",
    "{G}>{R} The tile cache test fails about one run in five on Windows. Find out why and fix it.",
    "",
    "● I’ll reproduce it first, then read the eviction code.",
    "",
    "{D} {B}Bash{R}(cargo test -p tiles cache_evicts_oldest -- --exact)",
    "  ⎿  \x1b[38;2;255;107;128mtest cache::tests::cache_evicts_oldest ... FAILED{R}",
    "     {G}assertion failed: evicted == [a]{R}",
    "",
    "{D} {B}Read{R}(src\\tiles\\cache.rs)",
    "  ⎿  Read {B}214{R} lines",
    "",
    "● Eviction picks the tile with the oldest file time, and two tiles written in the",
    "  same tick get the same time on Windows. A use counter makes the order exact.",
    "",
    "{D} {B}Update{R}(src\\tiles\\cache.rs)",
    "  ⎿  Updated {B}src\\tiles\\cache.rs{R} with {B}2{R} additions and {B}2{R} removals",
    "       {G}86{R}       fn touch(&mut self, id: TileId) {",
    "       {G}87{R} \x1b[48;2;92;35;43m-         let now = SystemTime::now();             {R}",
    "       {G}87{R} \x1b[48;2;33;76;43m+         self.clock += 1;                         {R}",
    "       {G}88{R}           if let Some(entry) = self.entries.get_mut(&id) {",
    "       {G}89{R} \x1b[48;2;92;35;43m-             entry.used = now;                    {R}",
    "       {G}89{R} \x1b[48;2;33;76;43m+             entry.used = self.clock;             {R}",
    "       {G}90{R}           }",
    "",
    "{D} {B}Bash{R}(cargo test -p tiles)",
    "  ⎿  test result: \x1b[38;2;78;186;101mok{R}. 48 passed; 0 failed; 0 ignored",
    "",
    "● {B}Fixed.{R} The cache now evicts by a use counter instead of file times. The test",
    "  passed 50 runs in a row, and the whole suite passes.",
    "",
];

fn save(dir: &Path, name: &str, bmp: &Bitmap) {
    bmp.write_png(&dir.join(name)).unwrap();
}

/// Draws every picture into `CS_README_SHOTS` (relative to the workspace); without it the test
/// only checks that they draw.
#[test]
#[ignore = "writes the README's screenshots; run by hand"]
fn readme_screenshots() {
    // A relative folder is the workspace's, not the crate's that tests run in.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_README_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-readme-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    let only = std::env::var("CS_README_ONLY").ok();
    let want = |name: &str| only.as_deref().is_none_or(|only| only.split(',').any(|n| n.trim() == name));
    let now = Instant::now() + Duration::from_secs(1);

    if want("hero") {
        // A Claude tab in Chat after a finished turn, a draft in the composer.
        let demo = crate::demo::Transcript::new_showcase(None).unwrap();
        let (mut st, id) = chat_window(true, &demo);
        let later = show(&mut st, id, IDLE, now);
        tab(&mut st, id).pane.as_mut().unwrap().chat.editor.set("Now add elevation tiles to the same cache".into());
        save(dir, "hero.png", &settled(&mut st, later));
    }

    if want("chat-question") {
        let demo = crate::demo::Transcript::new_showcase(Some((
            "Now add elevation tiles to the same cache.",
            "Elevation tiles are about ten times larger than map tiles, so where they live matters. \
             One choice first.",
            "",
        )))
        .unwrap();
        let (mut st, id) = chat_window(true, &demo);
        running(&mut st, id, "Waiting for your answer");
        let later = show(&mut st, id, QUESTION, now);
        let chat = &tab(&mut st, id).pane.as_ref().unwrap().chat;
        assert_eq!(chat.menu_waiting(), Some("Waiting for your answer"), "the question card shows");
        save(dir, "chat-question.png", &settled(&mut st, later));
    }

    if want("chat-permission") {
        let demo = crate::demo::Transcript::new_showcase(Some((
            "Measure eviction before and after the change.",
            "I’ll run the eviction benchmark on this version, then on the one before it.",
            "cargo bench -p tiles --bench eviction",
        )))
        .unwrap();
        let (mut st, id) = chat_window(true, &demo);
        running(&mut st, id, "Waiting for your approval");
        let later = show(&mut st, id, PERMISSION, now);
        save(dir, "chat-permission.png", &settled(&mut st, later));
        assert!(tab(&mut st, id).pane.as_ref().unwrap().chat.question.visible(), "the permission card shows");
    }

    if want("sessions") {
        // Groups, running and waiting sessions, and a handover being written behind.
        let mut st = window(true);
        // The running session, handed over from an earlier one.
        let first = st.sessions[0].id.clone();
        st.select(&first);
        read_details(&mut st);
        let session = st.sessions[1].clone();
        st.ask_compact_for(session, Some("work"), false);
        let wake = st.wake_fn();
        let (_writing, gate) = std::sync::mpsc::channel();
        let Some(Dialog::Compact(d)) = &mut st.dialog else { panic!("no handover dialog") };
        d.start_test_generation(wake, gate);
        dialogs::click(&mut st, Instant::now(), dialogs::OK, 0.0);
        st.compact.jobs.list[0].started = now - Duration::from_secs(42);
        save(dir, "sessions.png", &settled(&mut st, now));
        st.cancel_background_handovers();
    }

    if want("handover") {
        let mut st = window(true);
        st.compact.demo = Some(true);
        st.start_compact();
        assert!(matches!(st.dialog, Some(Dialog::Compact(_))));
        save(dir, "handover.png", &settled(&mut st, now));
    }

    if want("settings-appearance") {
        let mut st = window(true);
        st.dialog = Some(dialogs::settings(&mut st, Some("Appearance")));
        save(dir, "settings-appearance.png", &settled(&mut st, now));
    }

    if want("settings-peek") {
        // Peek held over Settings, on the light window with the Warm palette chosen.
        let demo = crate::demo::Transcript::new_showcase(None).unwrap();
        let (mut st, id) = chat_window(false, &demo);
        st.config.appearance.light_theme = "warm-light".into();
        st.retheme();
        let later = show(&mut st, id, IDLE, now);
        st.dialog = Some(dialogs::settings(&mut st, Some("Appearance")));
        let mut anim = Animator::new();
        frame(&mut st, &mut anim, later);
        let Some(Dialog::Settings(s)) = &mut st.dialog else { unreachable!() };
        let b = s.peek_button();
        s.press(b.cx(), b.cy(), Mods::default(), 1);
        assert!(st.settings_peeking());
        let mut at = later;
        for _ in 0..30 {
            at += Duration::from_millis(16);
            frame(&mut st, &mut anim, at);
        }
        save(dir, "settings-peek.png", &frame(&mut st, &mut anim, at + Duration::from_millis(16)));
    }

    if want("new-session") {
        let mut st = window(true);
        st.ask_new_session();
        save(dir, "new-session.png", &settled(&mut st, now));
    }

    if want("terminal") {
        // A Claude Code tab in Terminal view, at a larger terminal font.
        let mut st = window(true);
        st.config.terminal_font_size = 16;
        let id = claude_tab(&mut st, "PowerShell");
        frame(&mut st, &mut Animator::new(), now);
        let cols = tab(&mut st, id).term.screen.cols();
        tab(&mut st, id).term.feed(terminal_text(cols).as_bytes());
        save(dir, "terminal.png", &settled(&mut st, now));
    }
}
