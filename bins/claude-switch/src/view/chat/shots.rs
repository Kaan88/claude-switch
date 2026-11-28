//! Pictures of Chat's states for a look by eye, drawn by the window's own code from the demo data
//! as the README's pictures are (`readme_shots.rs`): a wide window and the narrowest one, dark and
//! light. To draw them:
//!
//! ```text
//! CS_CHAT_SHOTS=target-shots cargo test -p claude-switch -- --ignored chat_shots
//! ```
//!
//! `CS_CHAT_ONLY=question,empty` draws only those states.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ItemKind, ToolCard, ToolState};
use mdshared_text::Bitmap;

use super::readme_shots::{self as shots, Size};
use crate::demo::Transcript;
use crate::state::AppState;

const WIDE: Size = Size { w: 1280.0, h: 980.0, scale: 1.5 };
/// The narrowest window Chat is drawn in.
const NARROW: Size = Size { w: 760.0, h: 720.0, scale: 1.0 };

const WORKING: &str = "cursor 33 2
@size 100 40




















● I’ll run the eviction benchmark on this version, then on the one before it.

● Bash(cargo bench -p tiles --bench eviction)
  ⎿  Running… (12s)
     (ctrl+b to run in background)

✻ Shimmying… (12s · ↓ 1.2k tokens)

────────────────────────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ accept edits on (shift+tab to cycle) · esc to interrupt
";

const WORKING_LINE: &str = "✻ Shimmying… (12s · ↓ 1.2k tokens)";

fn pane(st: &mut AppState, id: u64) -> &mut crate::view::Pane {
    shots::tab(st, id).pane.as_mut().unwrap()
}

const BENCH: (&str, &str, &str) = (
    "Measure eviction before and after the change.",
    "I’ll run the eviction benchmark on this version, then on the one before it.",
    "cargo bench -p tiles --bench eviction",
);

/// One frame of the window in `state`.
fn shot(state: &str, dark: bool, size: Size) -> Bitmap {
    let now = Instant::now() + Duration::from_secs(1);
    let (demo, mut st, later) = match state {
        "conversation" | "steps" | "scrolled" | "pointing" => {
            // Scrolled back needs a conversation longer than the window: a second, short turn.
            let more = (state == "scrolled").then_some((
                "Does the counter survive a restart?",
                "No. It starts at zero with the process, and that is fine: after a restart every tile is \
                 equally old, and the first reads set the order again. To keep the order, the counter and \
                 each tile's last use could be written to the cache index on shutdown. That costs one small \
                 write, and a crash would lose only the order, never a tile. I would leave it as it is \
                 unless evictions right after startup show up in practice.",
                "",
            ));
            let demo = Transcript::new_showcase(more).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            let later = shots::show(&mut st, id, shots::IDLE, now);
            if state == "conversation" {
                pane(&mut st, id).chat.editor.set("Now add elevation tiles to the same cache".into());
            }
            if state == "steps" {
                // The run of calls with its failed call, and the edit, opened as a click would.
                let pane = pane(&mut st, id);
                let ids: Vec<_> = pane
                    .model
                    .items
                    .iter()
                    .filter(|(_, item)| {
                        matches!(&item.kind, ItemKind::Tool { state: ToolState::Failed, .. })
                            || matches!(&item.kind, ItemKind::Tool { card: Some(ToolCard::Edit { .. }), .. })
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids.into_iter().rev() {
                    pane.ui.jump_to_tool(&pane.model, id);
                }
            }
            if state == "pointing" {
                // The pointer over your message: its buttons show under it.
                let mut anim = mdshared_ui::anim::Animator::new();
                shots::frame_at(&mut st, &mut anim, later, size);
                // Scrolled to the top, where the message is.
                let area = pane(&mut st, id).chat.layout.conversation;
                pane(&mut st, id).ui.wheel(area.cx(), area.cy(), 5000.0);
                shots::frame_at(&mut st, &mut anim, later, size);
                let (x, y) = pane(&mut st, id).ui.prompt_point().expect("a prompt on screen");
                st.mouse = (x, y);
                pane(&mut st, id).ui.moved(x, y);
            }
            if state == "scrolled" {
                let mut anim = mdshared_ui::anim::Animator::new();
                shots::frame_at(&mut st, &mut anim, later, size);
                let area = pane(&mut st, id).chat.layout.conversation;
                pane(&mut st, id).ui.wheel(area.cx(), area.cy(), 190.0);
            }
            (demo, st, later)
        }
        "question" => {
            let demo = Transcript::new_showcase(Some((
                "Now add elevation tiles to the same cache.",
                "Elevation tiles are about ten times larger than map tiles, so where they live matters. \
                 One choice first.",
                "",
            )))
            .unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your answer");
            let later = shots::show(&mut st, id, shots::QUESTION, now);
            (demo, st, later)
        }
        "permission" => {
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your approval");
            let later = shots::show(&mut st, id, shots::PERMISSION, now);
            (demo, st, later)
        }
        "running" => {
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, WORKING_LINE);
            let later = shots::show(&mut st, id, WORKING, now);
            // `show` read the screen; the working line is the host's to set each tick.
            shots::running(&mut st, id, WORKING_LINE);
            pane(&mut st, id).chat.mode_shown = Some("Accept edits");
            (demo, st, later)
        }
        "empty" => {
            let demo = Transcript::new_empty().unwrap();
            let mut st = shots::window(dark);
            let id = shots::claude_tab(&mut st, "Move the map tiles to a worker thread");
            let source = crate::view::model::Source::File(demo.path().into());
            let mut pane = st.new_session_pane(source, demo.cwd().into());
            pane.chat.scripted = true;
            pane.chat.enabled = true;
            pane.chat.identity_unverified = false;
            pane.chat.awaiting_transcript = true;
            pane.ui.set_chat(true);
            pane.model.poll();
            std::thread::sleep(Duration::from_millis(300));
            pane.model.collect();
            let tab = shots::tab(&mut st, id);
            tab.label = "New session".into();
            tab.pane = Some(pane);
            let later = shots::show(&mut st, id, shots::IDLE, now);
            (demo, st, later)
        }
        other => panic!("no such state: {other}"),
    };
    let bmp = shots::settled_at(&mut st, later, size);
    drop(demo);
    bmp
}

/// Draws every picture into `CS_CHAT_SHOTS` (relative to the workspace); without it the test only
/// checks that they draw.
#[test]
#[ignore = "writes pictures of Chat's states; run by hand"]
fn chat_shots() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_CHAT_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-chat-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    let only = std::env::var("CS_CHAT_ONLY").ok();
    let want = |name: &str| only.as_deref().is_none_or(|only| only.split(',').any(|n| n.trim() == name));
    let states = ["conversation", "steps", "question", "permission", "running", "scrolled", "empty", "pointing"];
    for (n, state) in states.into_iter().enumerate().filter(|(_, state)| want(state)) {
        for (size, width) in [(WIDE, "wide"), (NARROW, "narrow")] {
            for (dark, theme) in [(true, "dark"), (false, "light")] {
                let name = format!("{}-{state}-{width}-{theme}.png", n + 1);
                shot(state, dark, size).write_png(&dir.join(name)).unwrap();
            }
        }
    }
}

/// The Session view (the panel beside a terminal) draws the conversation with the same code at
/// its own size: the showcase with its run and its edit opened, into `CS_CHAT_SHOTS`.
#[test]
#[ignore = "writes pictures of the Session view; run by hand"]
fn session_view_shots() {
    use mdshared_ui::gfx::{Gfx, Painter, Rect};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_CHAT_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-session-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    let demo = Transcript::new_showcase(None).unwrap();
    for (pal, theme) in [(mdshared_ui::theme::DARK, "dark"), (mdshared_ui::theme::LIGHT, "light")] {
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = crate::view::model::Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(Duration::from_secs(5)).expect("the transcript was read");
        assert!(model.collect());
        let mut ui = crate::view::ui::Ui::default();
        let ids: Vec<_> = model
            .items
            .iter()
            .filter(|(_, item)| {
                matches!(&item.kind, ItemKind::Tool { state: ToolState::Failed, .. })
                    || matches!(&item.kind, ItemKind::Tool { card: Some(ToolCard::Edit { .. }), .. })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids.into_iter().rev() {
            ui.jump_to_tool(&model, id);
        }
        let (w, h, scale) = (380.0, 900.0, 1.5);
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new((w * scale) as u32, (h * scale) as u32));
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        ui.draw(&mut p, &pal, Rect::new(0.0, 0.0, w, h), &mut model, (-1.0, -1.0), Instant::now(), false);
        bmp.write_png(&dir.join(format!("8-session-view-{theme}.png"))).unwrap();
    }
}

/// The Sessions page with the selected session's conversation in its details (the shared
/// conversation view, read-only), the details scrolled to their end: into `CS_CHAT_SHOTS`.
#[test]
#[ignore = "writes pictures of the Sessions details; run by hand"]
fn sessions_details_shots() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_CHAT_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-details-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    for (dark, theme) in [(true, "dark"), (false, "light")] {
        let mut st = shots::window(dark);
        let now = Instant::now();
        shots::settled_at(&mut st, now, WIDE);
        let top = shots::settled_at(&mut st, now, WIDE);
        top.write_png(&dir.join(format!("9-sessions-details-top-{theme}.png"))).unwrap();
        st.details_scroll.wheel(-100_000.0);
        let end = shots::settled_at(&mut st, now, WIDE);
        end.write_png(&dir.join(format!("9-sessions-details-end-{theme}.png"))).unwrap();
        assert!(st.details_reader.as_ref().is_some_and(|r| r.loaded()));
    }
}
