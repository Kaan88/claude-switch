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

pub(super) const WORKING: &str = "cursor 33 2
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

pub(super) const WORKING_LINE: &str = "✻ Shimmying… (12s · ↓ 1.2k tokens)";

/// A command of six lines, as the permission card's well shows it.
pub(super) const LONG_COMMAND: &str = "cargo bench -p tiles --bench eviction -- --save-baseline after
git stash
cargo bench -p tiles --bench eviction -- --save-baseline before
git stash pop
critcmp before after
echo compared";

/// The permission prompt for [`LONG_COMMAND`], as Claude Code draws it.
pub(super) fn long_permission() -> String {
    let rows: String = LONG_COMMAND.lines().map(|line| format!(" {line}\n")).collect();
    shots::PERMISSION
        .replace(
            "● Bash(cargo bench -p tiles --bench eviction)",
            "● Bash(cargo bench -p tiles --bench eviction -- --save-baseline…)",
        )
        .replace(" cargo bench -p tiles --bench eviction\n", &rows)
}

/// A theme of the user's own (Settings › Appearance), as `termtheme::named` builds it.
fn user_theme(dark: bool) -> mdshared_ui::theme::Palette {
    use claude_switch_core::config::{
        AppearancePrefs,
        theme::{Colour, NamedTheme},
    };
    let (page, text, muted, accent) = if dark {
        ([0x1e, 0x1a, 0x2e], [0xee, 0xe6, 0xff], [0x9a, 0x90, 0xb8], [0xff, 0x6a, 0xc1])
    } else {
        ([0xf6, 0xef, 0xe0], [0x2a, 0x22, 0x18], [0x6e, 0x60, 0x4c], [0x2e, 0x8b, 0x57])
    };
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| -> [u8; 3] {
        std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8)
    };
    let shade = |t: f32| Colour(mix(page, text, t));
    let theme = NamedTheme {
        name: "Mine".into(),
        base: if dark { "dark" } else { "light" }.into(),
        page: Colour(page),
        bar: shade(0.04),
        title: shade(0.07),
        pane: shade(0.03),
        field: shade(0.01),
        popup: shade(0.05),
        text: Colour(text),
        muted: Colour(muted),
        accent: Colour(accent),
        selection: Colour(mix(page, accent, 0.4)),
        terminal_background: Colour(page),
        terminal_foreground: Colour(text),
        terminal_selection: Colour(mix(page, accent, 0.4)),
        ansi: std::array::from_fn(|i| Colour([i as u8 * 9, 40, 80])),
    };
    let prefs = AppearancePrefs { accent_source: "theme".into(), ..Default::default() };
    crate::termtheme::named::palette(&theme, &prefs, None)
}

/// The wide and narrow windows at the other common scale.
const WIDE_100: Size = Size { w: 1280.0, h: 980.0, scale: 1.0 };
const NARROW_150: Size = Size { w: 760.0, h: 720.0, scale: 1.5 };

/// The question of the "preview" pictures, as the transcript's call has it: three layouts for the
/// tile cache, two with a preview (a mock-up drawn with box lines, a tree), one without.
fn layout_question() -> serde_json::Value {
    let row = |s: &str| format!("│ {s:<36}│");
    let flat = [
        format!("┌─ cache {}┐", "─".repeat(29)),
        row("z12_x2154_y1430.png    18 KB  map"),
        row("z12_x2155_y1430.png    21 KB  map"),
        row("z13_x4308_y2860.png   212 KB  dem"),
        row("index.json             3 KB"),
        format!("└{}┘", "─".repeat(37)),
        "One folder, one size limit for both kinds.".to_owned(),
    ]
    .join("\n");
    let nested = [
        "cache/",
        "├── map/",
        "│   ├── 12/2154/1430.png",
        "│   └── 12/2155/1430.png",
        "├── dem/",
        "│   └── 13/4308/2860.png",
        "└── index.json",
        "",
        "fn path(kind: Kind, z: u8, x: u32, y: u32) -> PathBuf {",
        "    root.join(kind.dir()).join(format!(\"{z}/{x}/{y}.png\"))",
        "}",
    ]
    .join("\n");
    serde_json::json!({"questions": [{
        "header": "Layout",
        "question": "Which folder layout should the tile cache use?",
        "multiSelect": false,
        "options": [
            {"label": "Flat folder", "description": "Every tile in one folder, named by kind and place.", "preview": flat},
            {"label": "Nested by kind and zoom", "description": "A folder per kind, then per zoom level.", "preview": nested},
            {"label": "One packed file", "description": "All tiles in one file with an index."}
        ]
    }]})
}

/// A window only just tall enough for Chat.
const TINY: Size = Size { w: 760.0, h: 400.0, scale: 1.0 };
const TINY_WIDE: Size = Size { w: 1280.0, h: 420.0, scale: 1.25 };
/// Scales other than 100 % and 150 %.
const WIDE_125: Size = Size { w: 1280.0, h: 900.0, scale: 1.25 };
const NARROW_200: Size = Size { w: 760.0, h: 720.0, scale: 2.0 };

/// A window too short for a tall card (the room a 1080p screen at 150 % leaves, or the drawer).
const SHORT_WIDE: Size = Size { w: 1280.0, h: 560.0, scale: 1.5 };
const SHORT_NARROW: Size = Size { w: 760.0, h: 520.0, scale: 1.0 };

fn pane(st: &mut AppState, id: u64) -> &mut crate::view::Pane {
    shots::tab(st, id).pane.as_mut().unwrap()
}

const BENCH: (&str, &str, &str) = (
    "Measure eviction before and after the change.",
    "I’ll run the eviction benchmark on this version, then on the one before it.",
    "cargo bench -p tiles --bench eviction",
);

/// A made-up terminal screen `cols` wide: the benchmark's command waits at a question of its
/// own, in colour, over Claude Code's input box.
fn waiting_screen(cols: usize) -> String {
    let rule = format!("\x1b[38;5;244m{}\x1b[0m", "─".repeat(cols));
    [
        "\x1b[2m  ⎿  Read tiles/src/cache.rs (212 lines)\x1b[0m".to_owned(),
        String::new(),
        "\x1b[32m●\x1b[0m \x1b[1mBash\x1b[0m(cargo bench -p tiles --bench eviction)".to_owned(),
        "  ⎿  \x1b[33mwarning\x1b[0m: the baseline \x1b[1mmain\x1b[0m is 3 days old".to_owned(),
        "     Overwrite the saved baseline? \x1b[36m[y/N]\x1b[0m ".to_owned(),
        String::new(),
        rule.clone(),
        "\x1b[1m❯\x1b[0m ".to_owned(),
        rule,
        "  \x1b[2m? for shortcuts\x1b[0m".to_owned(),
    ]
    .join("\r\n")
}

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
        "preview" | "preview-hover" => {
            // A question whose options have previews, from the transcript's pending call: the
            // first option's preview, or (hover) the one under the pointer.
            let demo = Transcript::new_asking(
                "Now add elevation tiles to the same cache.",
                "Elevation tiles are about ten times larger than map tiles, so where they live matters.",
                layout_question(),
            )
            .unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your answer");
            let later = shots::show(&mut st, id, crate::view::chat::question::screens::recorded::PREVIEW_ASKED, now);
            if state == "preview-hover" {
                let mut anim = mdshared_ui::anim::Animator::new();
                shots::frame_at(&mut st, &mut anim, later, size);
                let target = crate::state::Target::Chat(super::Control::Question(super::question::Hit::Option(1)));
                let (r, _) = *st.hits.iter().find(|(_, t)| *t == target).expect("the second option");
                st.mouse = (r.cx(), r.cy());
                st.hover = Some(target);
            }
            (demo, st, later)
        }
        "form-tabs" | "form-submit" => {
            // A form of three questions from the transcript's pending call: its tabs, the
            // multi-select page with two boxes ticked and a typed answer, or the Submit step.
            let mut input = layout_question();
            let extras = serde_json::json!({
                "header": "Extras", "question": "Which extras should the cache keep beside each tile?",
                "multiSelect": true,
                "options": [
                    {"label": "Logs", "description": "Who fetched it and when."},
                    {"label": "Metrics", "description": "Hits and misses per zoom level."},
                    {"label": "Traces", "description": "The full request, for debugging."}
                ]
            });
            let name = serde_json::json!({
                "header": "Names", "question": "How should tile files be named?", "multiSelect": false,
                "options": [{"label": "Short", "description": "z/x/y.png"}, {"label": "Long", "description": "kind_z_x_y.png"}]
            });
            input["questions"].as_array_mut().unwrap().extend([extras, name]);
            let demo = Transcript::new_asking(
                "Now add elevation tiles to the same cache.",
                "A few choices before I start.",
                input,
            )
            .unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your answer");
            let later = shots::show(&mut st, id, shots::IDLE, now);
            use super::question::{Hit, press};
            let tab = shots::tab(&mut st, id);
            press(tab, later, Hit::Option(1));
            press(tab, later, Hit::Page(1));
            press(tab, later, Hit::Option(0));
            press(tab, later, Hit::Option(2));
            tab.pane.as_mut().unwrap().chat.question.field.set_text("alerts");
            if state == "form-submit" {
                press(tab, later, Hit::Page(2));
                press(tab, later, Hit::Option(0));
                press(tab, later, Hit::Page(3));
            }
            (demo, st, later)
        }
        "permission" => {
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your approval");
            let later = shots::show(&mut st, id, shots::PERMISSION, now);
            (demo, st, later)
        }
        "denied-under-card" => {
            // A call denied earlier in the turn, and now a permission card holds the card's place.
            use claude_switch_core::transcript::{Denial, DenialKind, Item, ItemId};
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your approval");
            let pane = pane(&mut st, id);
            let at = pane.model.chat.prompt.unwrap() + 1;
            let denial = Denial {
                kind: DenialKind::AutoMode,
                before_run: None,
                reason: "The server-side auto mode classifier judged this action dangerous.".into(),
                rule_source: None,
            };
            let kind = ItemKind::Tool {
                id: "toolu_denied".into(),
                name: "Bash".into(),
                summary: "rm -rf target/bench-old".into(),
                file_path: None,
                state: ToolState::Failed,
                duration_ms: None,
                pictures: vec![],
                subagent: None,
                denial: Some(Box::new(denial)),
                card: Some(ToolCard::Shell { command: "rm -rf target/bench-old".into(), result: vec![] }),
            };
            pane.model.items.insert(ItemId { offset: at, block: 0 }, Item { kind, timestamp: None });
            let later = shots::show(&mut st, id, shots::PERMISSION, now);
            (demo, st, later)
        }
        // A card read from the screen alone over the showcase: a form's question, a plain menu, a
        // read-only panel, the list of blocked calls.
        "form" | "menu" | "panel" | "blocked" | "preview-screen" => {
            use crate::view::chat::question::screens::recorded::{
                BLOCKED_TWO, MENU_MODEL, PANEL_USAGE, PLAIN_FORM, PREVIEW_ASKED,
            };
            let screen = match state {
                "form" => PLAIN_FORM,
                // A question with previews whose call the transcript does not hold (yet).
                "preview-screen" => PREVIEW_ASKED,
                "menu" => MENU_MODEL,
                "panel" => PANEL_USAGE,
                _ => BLOCKED_TWO,
            };
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your answer");
            let later = shots::show(&mut st, id, screen, now);
            (demo, st, later)
        }
        "rewind" => {
            use crate::view::chat::question::screens::recorded::REWIND_ACTIONS;
            let demo = Transcript::new_showcase(None).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            let later = shots::show(&mut st, id, REWIND_ACTIONS, now);
            pane(&mut st, id).chat.question.rewinding("Fix the flaky tile cache test");
            (demo, st, later)
        }
        "rewind-choices" => {
            // The rewind's Enter went on the prompt's row and Claude Code shows its choices for it
            // (recorded): Chat reads them from the screen, and the card offers them.
            use crate::view::chat::question::screens::recorded::REWIND_ACTIONS;
            let demo = Transcript::new_showcase(None).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            let later = shots::show(&mut st, id, REWIND_ACTIONS, now);
            pane(&mut st, id).chat.question.rewinding("Reply with just the word bravo.");
            let later = later + Duration::from_millis(200);
            pane(&mut st, id).chat.output(later, true, false);
            st.progress_chat_tabs(later, true);
            assert!(pane(&mut st, id).chat.question.rewind_choosing(), "the card offers the choices");
            (demo, st, later)
        }
        "very-short" | "scale-125" | "scale-200" | "custom-theme" => {
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            if state == "custom-theme" {
                st.tokens.palette = user_theme(dark);
            }
            shots::running(&mut st, id, "Waiting for your approval");
            let later = shots::show(&mut st, id, shots::PERMISSION, now);
            (demo, st, later)
        }
        "tall-permission" => {
            let demo = Transcript::new_showcase(Some((BENCH.0, BENCH.1, LONG_COMMAND))).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, "Waiting for your approval");
            let later = shots::show(&mut st, id, &long_permission(), now);
            (demo, st, later)
        }
        "queued" => {
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            shots::running(&mut st, id, WORKING_LINE);
            let later = shots::show(&mut st, id, WORKING, now);
            shots::running(&mut st, id, WORKING_LINE);
            // Two messages sent while the benchmark runs, waiting in Claude Code's queue.
            let pane = pane(&mut st, id);
            let generation = pane.model.chat.generation;
            let chat = &mut pane.chat;
            chat.note_queued("Also compare the memory use.".into(), false, generation, u64::MAX);
            chat.note_queued(
                "And keep the old numbers in the notes, with the date and the commit they were measured at, \
                 so the next run can be compared with them."
                    .into(),
                false,
                generation,
                u64::MAX,
            );
            (demo, st, later)
        }
        "new-session-card" => {
            // A new session whose first answer is a question: the card stands at the window's
            // foot, not in the greeting's place.
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
            let later = shots::show(&mut st, id, shots::QUESTION, now);
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
        "command" | "command-custom" => {
            // `/context` and a shell-mode command, their colour codes as recorded. The custom
            // picture takes a terminal theme against the window's: a light one beside a dark
            // window, a dark one beside a light window.
            let demo = Transcript::new_command(&crate::view::ui::grid::tests::context_fixture()).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            if state == "command-custom" {
                st.config.terminal_scheme = if dark { "tango-light" } else { "campbell" }.into();
                st.retheme();
            }
            let later = shots::show(&mut st, id, shots::IDLE, now);
            (demo, st, later)
        }
        "terminal-card" => {
            // The benchmark's command waits at a question of its own, which Chat cannot read:
            // after the silence, the screen shows as a card above the message box.
            let demo = Transcript::new_showcase(Some(BENCH)).unwrap();
            let (mut st, id) = shots::chat_window(dark, &demo);
            let later = shots::show(&mut st, id, shots::IDLE, now);
            let tab = shots::tab(&mut st, id);
            let (cols, rows) = (tab.term.screen.cols(), tab.term.screen.rows());
            tab.term.screen = mdshared_term::Screen::new(cols, rows, 0);
            tab.term.screen.feed(waiting_screen(cols).as_bytes());
            tab.pane.as_mut().unwrap().chat.output(later, true, false);
            let later = later + Duration::from_secs(6);
            st.progress_chat_tabs(later, true);
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
    let states = [
        "conversation",
        "steps",
        "question",
        "preview",
        "preview-hover",
        "preview-screen",
        "form-tabs",
        "form-submit",
        "permission",
        "running",
        "scrolled",
        "empty",
        "pointing",
        "command",
        "command-custom",
        "terminal-card",
        "tall-permission",
        "new-session-card",
        "queued",
        "denied-under-card",
        "form",
        "menu",
        "panel",
        "blocked",
        "rewind",
        "rewind-choices",
        "very-short",
        "scale-125",
        "scale-200",
        "custom-theme",
    ];
    for (n, state) in states.into_iter().enumerate().filter(|(_, state)| want(state)) {
        let sizes = match state {
            "tall-permission" => [(SHORT_WIDE, "wide"), (SHORT_NARROW, "narrow")],
            "very-short" => [(TINY_WIDE, "wide"), (TINY, "narrow")],
            "scale-125" | "scale-200" => [(WIDE_125, "wide"), (NARROW_200, "narrow")],
            _ => [(WIDE, "wide"), (NARROW, "narrow")],
        };
        // The question cards at both common scales, wide and narrow.
        let more: &[(Size, &str)] =
            if state.starts_with("preview") || state.starts_with("form-") || state == "rewind-choices" {
                &[(WIDE_100, "wide-100"), (NARROW_150, "narrow-150")]
            } else {
                &[]
            };
        let sizes: Vec<(Size, &str)> = sizes.into_iter().chain(more.iter().copied()).collect();
        for (size, width) in sizes {
            for (dark, theme) in [(true, "dark"), (false, "light")] {
                let name = format!("{}-{state}-{width}-{theme}.png", n + 1);
                shot(state, dark, size).write_png(&dir.join(name)).unwrap();
            }
        }
        if state.starts_with("command") {
            for (dark, theme) in [(true, "dark"), (false, "light")] {
                let name = format!("{}-{state}-terminal-{theme}.png", n + 1);
                terminal_shot(state, dark).write_png(&dir.join(name)).unwrap();
            }
        }
    }
}

/// The `/context` output of the "command" pictures as the terminal draws it, with the same
/// window's terminal theme and font: to set beside Chat's.
fn terminal_shot(state: &str, dark: bool) -> Bitmap {
    use mdshared_ui::gfx::{Gfx, Painter, Rect};
    use mdshared_ui::termview::{self, CellFont, View};
    let mut st = shots::window(dark);
    if state == "command-custom" {
        st.config.terminal_scheme = if dark { "tango-light" } else { "campbell" }.into();
        st.retheme();
    }
    let scheme = st.terminal_scheme();
    let scale = WIDE.scale;
    let mut font = CellFont::new();
    let metrics = font.metrics(f32::from(st.config.terminal_font_size), scale);
    let (cols, rows) = (90, 16);
    let mut screen = mdshared_term::Screen::new(cols, rows, 0);
    screen.feed(crate::view::ui::grid::tests::context_fixture().replace('\n', "\r\n").as_bytes());
    let (w, h) = (cols as f32 * metrics.cell_w() + 24.0, rows as f32 * metrics.cell_h() + 24.0);
    let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new((w * scale) as u32, (h * scale) as u32));
    let mut p = Painter::new(&mut bmp, &mut gfx, scale);
    p.fill(Rect::new(0.0, 0.0, w, h), scheme.background);
    let area = Rect::new(12.0, 12.0, w - 24.0, h - 24.0);
    termview::draw_screen(&mut p, &mut font, &scheme, &metrics, area, &screen, None, View::new().cursor_shown(false));
    bmp
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
    // Command output in the panel's narrow column.
    let demo = Transcript::new_command(&crate::view::ui::grid::tests::context_fixture()).unwrap();
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
        let (w, h, scale) = (380.0, 900.0, 1.5);
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new((w * scale) as u32, (h * scale) as u32));
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        ui.draw(&mut p, &pal, Rect::new(0.0, 0.0, w, h), &mut model, (-1.0, -1.0), Instant::now(), false);
        bmp.write_png(&dir.join(format!("8-session-view-command-{theme}.png"))).unwrap();
    }
}

/// The demo's agents under Claude Code's input: one running, one stopped. The demo's third agent
/// (finished) is not listed, as Claude Code drops a finished agent from its list.
const AGENTS_LISTED: &str = "cursor 25 2
@size 100 40
























────────────────────────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts · ↓ to manage

  ● main
  ◯ reviewer  Review the plate caption                                       12s · ↓ 3.1k tokens
  ◯ reviewer  Trace the ridge contour                              stopped · 8s · ↓ 2.0k tokens
";

/// Subagents in each state, the same words everywhere: Chat's Details › Background (the
/// terminal's agent list says which runs), a subagent's conversation with its state in the header,
/// and the Session view's Agents page. A fourth agent whose end nothing records shows `?`. Into
/// `CS_CHAT_SHOTS`.
#[test]
#[ignore = "writes pictures of the subagents' states; run by hand"]
fn agent_state_shots() {
    use mdshared_ui::gfx::{Gfx, Painter, Rect};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("CS_CHAT_SHOTS").map(|d| workspace.join(PathBuf::from(d)));
    let scratch = crate::testing::Scratch::created("cs-agent-shots");
    let dir = dir.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();
    let unknown = claude_switch_core::transcript::Subagent {
        id: "legend".into(),
        agent_type: Some("reviewer".into()),
        description: Some("Check the map legend".into()),
        tool_calls: 2,
        ..Default::default()
    };
    let demo = Transcript::new().unwrap();
    for (dark, theme) in [(true, "dark"), (false, "light")] {
        let now = Instant::now() + Duration::from_secs(1);
        let (mut st, id) = shots::chat_window(dark, &demo);
        pane(&mut st, id).model.agents.push(unknown.clone());
        let later = shots::show(&mut st, id, AGENTS_LISTED, now);
        let model = &pane(&mut st, id).model;
        let words: Vec<_> = model.agents.iter().map(|a| model.agent_status(a).word).collect();
        assert_eq!(words, ["Running", "Finished", "Stopped", "Unknown"], "caption, colour, contour, legend");
        shots::settled_at(&mut st, later, WIDE).write_png(&dir.join(format!("10-agents-details-{theme}.png"))).unwrap();
        // The running agent's conversation: its state beside its task over the conversation.
        let viewed = pane(&mut st, id);
        viewed.ui.open_agent("caption", &viewed.model);
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(20));
            viewed.model.collect();
            if viewed.model.agent.as_deref() == Some("caption") {
                break;
            }
        }
        assert_eq!(viewed.model.agent.as_deref(), Some("caption"));
        let later = shots::show(&mut st, id, AGENTS_LISTED, later);
        shots::settled_at(&mut st, later, NARROW).write_png(&dir.join(format!("10-agent-view-{theme}.png"))).unwrap();
        // The stopped agent's conversation: the box names it, and a message is refused with a word.
        let stopped = pane(&mut st, id);
        stopped.ui.open_agent("contour", &stopped.model);
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(20));
            stopped.model.collect();
            if stopped.model.agent.as_deref() == Some("contour") {
                break;
            }
        }
        stopped.chat.editor.set("Check the second ridge too".into());
        let later = shots::show(&mut st, id, AGENTS_LISTED, later);
        AppState::chat_send(shots::tab(&mut st, id), later, false);
        assert_eq!(pane(&mut st, id).chat.error, Some(super::question::agent_send::ENDED));
        shots::settled_at(&mut st, later, NARROW).write_png(&dir.join(format!("10-agent-ended-{theme}.png"))).unwrap();
    }
    // The Session view's Agents page, from the files alone: no list, so the caption's end is unknown.
    for (pal, theme) in [(mdshared_ui::theme::DARK, "dark"), (mdshared_ui::theme::LIGHT, "light")] {
        let source = crate::view::model::Source::File(demo.path().into());
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = crate::view::model::Model::new(source, demo.cwd().into(), move || {
            let _ = wake.send(());
        });
        model.poll();
        events.recv_timeout(Duration::from_secs(5)).expect("the transcript was read");
        assert!(model.collect());
        model.listed.insert("legend".into(), crate::view::agent_status::Listed::Running);
        model.agents.push(claude_switch_core::transcript::Subagent { id: "legend".into(), ..unknown.clone() });
        let mut ui = crate::view::ui::Ui::default();
        ui.show_agents();
        let (w, h, scale) = (380.0, 640.0, 1.5);
        let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new((w * scale) as u32, (h * scale) as u32));
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        ui.draw(&mut p, &pal, Rect::new(0.0, 0.0, w, h), &mut model, (-1.0, -1.0), Instant::now(), false);
        bmp.write_png(&dir.join(format!("10-agents-page-{theme}.png"))).unwrap();
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
