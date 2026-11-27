//! Tool rows recede behind the answers in every theme: no accent and no full-strength text on a
//! finished call or a group of calls, while a failure keeps the warning colour.
use std::io::Write;

use mdshared_text::Bitmap;
use mdshared_ui::gfx::Gfx;
use serde_json::{Value, json};

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

/// A transcript with two single calls (one failed), a group of five and answers between them.
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
fn tool_rows_are_quiet_in_light_dark_and_custom_themes_and_failures_still_show() {
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
        let tools = rows(Style::Tool).collect::<Vec<_>>();
        let groups = rows(Style::Group).collect::<Vec<_>>();
        assert_eq!((tools.len(), groups.len()), (2, 1), "{name}: two single calls and one group");
        assert!(tools.iter().chain(&groups).all(|r| r.h == TOOL_H && !r.open), "{name}: one closed line each");
        let (body, offset) = (ui.body, ui.offset);
        let header = |row: &Row| Rect::new(body.x + row.geo.x, body.y + row.top - offset, row.geo.w, TOOL_H);
        let answer = rows(Style::Answer).next().unwrap();
        let answer_px = pixels(&bmp, Rect::new(body.x + answer.geo.tx, body.y + answer.top, 200.0, 19.0));
        let loudest = answer_px.iter().map(|&c| reach(c, pal.page)).max().unwrap();
        for row in [tools[0], groups[0]] {
            let px = pixels(&bmp, header(row));
            assert!(!px.iter().any(|&c| near(c, pal.accent) && !near(pal.accent, pal.muted)), "{name}: no accent");
            assert!(
                px.iter().all(|&c| reach(c, pal.page) < loudest),
                "{name}: {} stays quieter than the answer text",
                row.title
            );
            assert!(px.iter().any(|&c| reach(c, pal.page) > 30), "{name}: {} is still readable", row.title);
            // No box: the row's edges are the page itself.
            let r = header(row);
            let edge = pixels(&bmp, Rect::new(r.x, r.y, r.w, 1.0));
            assert!(edge.iter().all(|&c| c == pal.page), "{name}: no border on {}", row.title);
        }
        let failed = tools[1];
        assert!(matches!(failed.tone, Tone::Warning));
        let wash = mix(pal.page, pal.warn_bg, 0.75);
        let warn = legible(pal.warn, wash, 4.5);
        let px = pixels(&bmp, header(failed));
        assert!(
            px.iter()
                .any(|&c| distance(c, warn) < distance(c, pal.muted).min(distance(c, wash)) && distance(c, warn) < 90),
            "{name}: a failure shows in the warning colour"
        );
        assert!(px.iter().filter(|&&c| c == wash).count() > px.len() / 2, "{name}: on a soft warning wash");

        // Opened, a call and a group stay clickable lines over their details.
        let Id::Tool(failed_id) = failed.id else { unreachable!() };
        let Id::Group(group_id) = groups[0].id else { unreachable!() };
        ui.tool_expanded.insert(failed_id);
        ui.expanded.insert(group_id);
        paint(&mut ui, &mut model, &pal, &format!("{name}-open"));
        let open = ui.rows.iter().find(|r| r.id == Id::Tool(failed_id)).unwrap();
        assert!(open.open && open.h > TOOL_H && open.body.is_some(), "{name}: the failed call opens");
        assert_eq!(ui.rows.iter().filter(|r| r.style == Style::Tool).count(), 7, "{name}: the group shows its calls");
        let header = ui.rows.iter().find(|r| r.id == Id::Group(group_id)).map(header).unwrap();
        assert!(ui.hits.iter().any(|(r, a)| *a == Action::Group(group_id) && r.contains(header.cx(), header.cy())));
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
            titles[2..],
            [
                ("/model", "Set model to `Opus 5.5` and saved as your default for new sessions"),
                ("/context", ""),
                ("Shell", "! git status --short"),
                ("Shell", "! git lg")
            ],
            "{name}"
        );
        let context = tools[3];
        assert!(context.body.as_ref().unwrap().text().contains("Free space: 159k"), "{name}: escapes stripped, kept");
        assert!(!context.body.as_ref().unwrap().text().contains('\u{1b}'));
        assert_eq!(tools[4].body.as_ref().unwrap().text(), " M docs/TODO.md\n?? notes.txt", "{name}: indent kept");
        assert!(matches!(tools[5].tone, Tone::Warning), "{name}: stderr alone is a failure");
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
