//! Tool lines recede behind the answers in every theme: no accent and no full-strength text on a
//! finished run of calls, while a failure is counted in the warning colour, without a wash.
use std::io::Write;

use mdshared_text::Bitmap;
use mdshared_ui::gfx::Gfx;
use serde_json::{Value, json};

use super::super::rows::CHAT_TOOL_H;
use super::*;

fn record(file: &mut std::fs::File, sec: u8, role: &str, content: Value) {
    let line = json!({
        "timestamp": format!("2026-11-27T10:00:{sec:02}Z"), "type": role,
        "message": {"role": role, "content": content}
    });
    writeln!(file, "{line}").unwrap();
}

fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({"type":"tool_use", "id":id, "name":name, "input":input})
}

fn result(id: &str, text: &str, error: bool) -> Value {
    json!({"type":"tool_result", "tool_use_id":id, "content":text, "is_error":error})
}

/// A transcript with a run of two calls (one failed), a run of five and answers between them.
pub(super) fn fixture(dir: &std::path::Path, extra: &[Value]) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("quiet.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    record(&mut file, 0, "user", json!("Check the caption files."));
    record(&mut file, 1, "assistant", json!("I’m reading the notes first."));
    record(
        &mut file,
        2,
        "assistant",
        json!([
            tool("r1", "Read", json!({"file_path": r"C:\Users\demo\notes\field-notes.md"})),
            tool("b1", "Bash", json!({"command": "cargo test --workspace"}))
        ]),
    );
    record(&mut file, 3, "user", json!([result("r1", "Look west.", false), result("b1", "exit 101", true)]));
    record(&mut file, 4, "assistant", json!("One test failed. Now the five plates."));
    let ids = ["g1", "g2", "g3", "g4", "g5"];
    record(
        &mut file,
        5,
        "assistant",
        json!(ids.map(|id| tool(id, "Read", json!({"file_path": format!(r"C:\Users\demo\plates\{id}.md")})))),
    );
    record(&mut file, 6, "user", json!(ids.map(|id| result(id, "plate", false))));
    record(&mut file, 7, "assistant", json!("All five plates read. The captions are fine."));
    for value in extra {
        writeln!(file, "{value}").unwrap();
    }
    path
}

pub(super) fn model(path: &std::path::Path, cwd: &std::path::Path) -> Model {
    let source = crate::view::model::Source::File(path.into());
    let (wake, events) = std::sync::mpsc::channel();
    let mut model = Model::new(source, cwd.into(), move || {
        let _ = wake.send(());
    });
    model.poll();
    events.recv_timeout(std::time::Duration::from_secs(3)).expect("transcript worker wake");
    assert!(model.collect());
    model
}

/// Draws the Chat view at 900 px; `CS_SHOT_DIR` keeps a picture of it for a look by eye.
pub(super) fn paint(ui: &mut Ui, model: &mut Model, pal: &Palette, shot: &str) -> Bitmap {
    let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(900, 900));
    let view = Rect::new(0.0, 0.0, 900.0, 900.0);
    ui.set_chat(true);
    ui.cache = None;
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    ui.draw(&mut p, pal, view, model, (0.0, 0.0), std::time::Instant::now(), false);
    if let Ok(dir) = std::env::var("CS_SHOT_DIR") {
        bmp.write_png(&std::path::Path::new(&dir).join(format!("{shot}.png"))).unwrap();
    }
    bmp
}

fn pixels(bmp: &Bitmap, r: Rect) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    for y in r.y.max(0.0) as u32..(r.bottom() as u32).min(bmp.height) {
        for x in r.x.max(0.0) as u32..(r.right() as u32).min(bmp.width) {
            let i = (y * bmp.width + x) as usize * 4;
            out.push([bmp.pixels[i + 2], bmp.pixels[i + 1], bmp.pixels[i]]);
        }
    }
    out
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 12)
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(y))).sum()
}

/// How far a colour stands from the page: the largest channel difference.
fn reach(a: [u8; 3], page: [u8; 3]) -> u8 {
    a.iter().zip(page).map(|(x, y)| x.abs_diff(y)).max().unwrap_or(0)
}

/// A theme a user made in Settings, through the same code the app uses for it.
fn user_theme(dark: bool, page: [u8; 3], text: [u8; 3], muted: [u8; 3], accent: [u8; 3]) -> Palette {
    use claude_switch_core::config::{
        AppearancePrefs,
        theme::{Colour, NamedTheme},
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

#[test]
fn runs_of_calls_are_quiet_in_light_dark_and_custom_themes_and_failures_still_show() {
    let scratch = crate::testing::Scratch::new("cs-quiet-rows");
    let dir = scratch.to_path_buf();
    let path = fixture(&dir, &[]);
    use crate::termtheme::palettes::{DARK_THEMES, LIGHT_THEMES};
    let mut themes = vec![
        ("dark".to_owned(), mdshared_ui::theme::DARK),
        ("light".to_owned(), mdshared_ui::theme::LIGHT),
        (
            "user-dark".to_owned(),
            user_theme(true, [0x1e, 0x1a, 0x2e], [0xee, 0xe6, 0xff], [0x9a, 0x90, 0xb8], [0xff, 0x6a, 0xc1]),
        ),
        (
            "user-light".to_owned(),
            user_theme(false, [0xf6, 0xef, 0xe0], [0x2a, 0x22, 0x18], [0x6e, 0x60, 0x4c], [0x2e, 0x8b, 0x57]),
        ),
    ];
    for (i, theme) in DARK_THEMES.iter().chain(LIGHT_THEMES).enumerate() {
        themes.push((format!("builtin-{i}"), theme.palette));
    }
    for (name, pal) in themes {
        let mut model = model(&path, &dir);
        let mut ui = Ui::default();
        let bmp = paint(&mut ui, &mut model, &pal, &name);
        let rows = |style| ui.rows.iter().filter(move |r| r.style == style);
        let runs = rows(Style::Group).collect::<Vec<_>>();
        assert_eq!((rows(Style::Tool).count(), runs.len()), (0, 2), "{name}: two runs, each one line");
        assert!(runs.iter().all(|r| r.h == CHAT_TOOL_H && !r.open), "{name}: one closed line each");
        assert_eq!((runs[0].title.as_str(), runs[0].stamp.as_str()), ("Read a file, ran a command", "1 failed"));
        assert_eq!((runs[1].title.as_str(), runs[1].stamp.as_str()), ("Read 5 files", ""));
        let (body, offset) = (ui.body, ui.offset);
        let header = |row: &Row| Rect::new(body.x + row.geo.x, body.y + row.top - offset, row.geo.w, row.head);
        let answer = rows(Style::Answer).next().unwrap();
        assert_eq!(runs[1].geo.x, answer.geo.x, "{name}: a run starts at the column's edge, as the answers do");
        let answer_px = pixels(&bmp, Rect::new(body.x + answer.geo.tx, body.y + answer.top, 200.0, 19.0));
        let loudest = answer_px.iter().map(|&c| reach(c, pal.page)).max().unwrap();
        let px = pixels(&bmp, header(runs[1]));
        assert!(!px.iter().any(|&c| near(c, pal.accent) && !near(pal.accent, pal.muted)), "{name}: no accent");
        assert!(px.iter().all(|&c| reach(c, pal.page) < loudest), "{name}: a run stays quieter than the answer text");
        assert!(px.iter().any(|&c| reach(c, pal.page) > 30), "{name}: a run is still readable");
        // No box: the line's edges are the page itself.
        let r = header(runs[1]);
        let edge = pixels(&bmp, Rect::new(r.x, r.y, r.w, 1.0));
        assert!(edge.iter().all(|&c| c == pal.page), "{name}: no border on a run");
        // The failure is counted in the warning colour, on the page: no wash.
        let failed = runs[0];
        assert!(matches!(failed.tone, Tone::Warning));
        let warn = legible(pal.warn, pal.page, 4.5);
        let px = pixels(&bmp, header(failed));
        assert!(
            px.iter()
                .any(|&c| distance(c, warn) < distance(c, pal.muted).min(distance(c, pal.page))
                    && distance(c, warn) < 90),
            "{name}: a failure shows in the warning colour"
        );
        assert!(px.iter().filter(|&&c| c == pal.page).count() > px.len() * 3 / 4, "{name}: on the page, no wash");

        // Opened, a run shows its steps; the failed one has its word and opens to what it printed.
        let Id::Group(first) = failed.id else { unreachable!() };
        let Id::Group(second) = runs[1].id else { unreachable!() };
        ui.expanded.insert(first);
        paint(&mut ui, &mut model, &pal, &format!("{name}-steps"));
        let steps = ui.rows.iter().filter(|r| r.style == Style::Tool).collect::<Vec<_>>();
        assert_eq!(steps.len(), 2, "{name}: the run shows its two steps");
        assert_eq!(
            steps.iter().map(|r| (r.title.as_str(), r.stamp.as_str(), r.step)).collect::<Vec<_>>(),
            [("Read", "", Some((true, false))), ("Ran", "failed", Some((false, true)))]
        );
        assert!(steps[0].target.ends_with("field-notes.md") && steps[1].target == "cargo test --workspace");
        let Id::Tool(failed_id) = steps[1].id else { unreachable!() };
        ui.tool_expanded.insert(failed_id);
        ui.expanded.insert(second);
        paint(&mut ui, &mut model, &pal, &format!("{name}-open"));
        let open = ui.rows.iter().find(|r| r.id == Id::Tool(failed_id)).unwrap();
        assert!(open.open && open.h > open.head && open.boxed, "{name}: the failed call opens");
        assert_eq!(open.body.as_ref().unwrap().text(), "exit 101", "{name}: its box holds what it printed");
        assert_eq!(ui.rows.iter().filter(|r| r.style == Style::Tool).count(), 7, "{name}: both runs show their calls");
        let header = ui.rows.iter().find(|r| r.id == Id::Group(first)).map(header).unwrap();
        assert!(ui.hits.iter().any(|(r, a)| *a == Action::Group(first) && r.contains(header.cx(), header.cy())));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_results_and_shell_mode_are_quiet_mono_cards_under_their_command() {
    let scratch = crate::testing::Scratch::new("cs-quiet-local");
    let dir = scratch.to_path_buf();
    let user = |sec: u8, text: &str| {
        json!({"timestamp": format!("2026-11-27T10:01:{sec:02}Z"), "type": "user",
            "message": {"role": "user", "content": text}})
    };
    let extra = [
        user(
            0,
            "<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args></command-args>",
        ),
        user(
            1,
            "<local-command-stdout>Set model to `Opus 5.5` and saved as your default for new sessions</local-command-stdout>",
        ),
        user(
            2,
            "<command-name>/context</command-name>\n<command-message>context</command-message>\n<command-args></command-args>",
        ),
        user(
            3,
            "<local-command-stdout> \u{1b}[1mContext Usage\u{1b}[22m\n\u{1b}[38;2;136;136;136m⛁ ⛁ ⛁ ⛀ \u{1b}[39m  claude-opus-5-5 · 41k/200k tokens (21%)\n⛶ ⛶ ⛶ ⛶   Free space: 159k</local-command-stdout>",
        ),
        user(4, "<bash-input>git status --short</bash-input>"),
        user(5, "<bash-stdout> M docs/TODO.md\n?? notes.txt</bash-stdout><bash-stderr></bash-stderr>"),
        user(6, "<bash-input>git lg</bash-input>"),
        user(
            7,
            "<bash-stdout></bash-stdout><bash-stderr>git: 'lg' is not a git command. See 'git --help'.</bash-stderr>",
        ),
    ];
    let path = fixture(&dir, &extra);
    for (name, pal) in [("local-dark", mdshared_ui::theme::DARK), ("local-light", mdshared_ui::theme::LIGHT)] {
        let mut model = model(&path, &dir);
        let mut ui = Ui::default();
        let bmp = paint(&mut ui, &mut model, &pal, name);
        let tools = ui.rows.iter().filter(|r| r.style == Style::Tool).collect::<Vec<_>>();
        let titles = tools.iter().map(|r| (r.title.as_str(), r.target.as_str())).collect::<Vec<_>>();
        assert_eq!(
            titles[..],
            [
                ("/model", "Set model to `Opus 5.5` and saved as your default for new sessions"),
                ("/context", ""),
                ("Shell", "! git status --short"),
                ("Shell", "! git lg")
            ],
            "{name}"
        );
        let context = tools[1];
        assert!(context.body.as_ref().unwrap().text().contains("Free space: 159k"), "{name}: escapes stripped, kept");
        assert!(!context.body.as_ref().unwrap().text().contains('\u{1b}'));
        assert_eq!(tools[2].body.as_ref().unwrap().text(), " M docs/TODO.md\n?? notes.txt", "{name}: indent kept");
        assert!(matches!(tools[3].tone, Tone::Warning), "{name}: stderr alone is a failure");
        // The card's text is muted like its row: quieter than the answers.
        let card = Rect::new(
            ui.body.x + context.geo.tx,
            ui.body.y + context.top - ui.offset + context.geo.ty,
            context.geo.tw,
            context.h - context.geo.ty,
        );
        let loud = pixels(&bmp, card).into_iter().map(|c| reach(c, pal.page)).max().unwrap();
        assert!(loud > 30 && loud < reach(pal.text, pal.page) - 20, "{name}: {loud}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
