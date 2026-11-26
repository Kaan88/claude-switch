//! A synthetic field-guide editing session, in the same on-disk format as a real reader input.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub struct Transcript {
    cwd: PathBuf,
    path: PathBuf,
    stop: Option<Sender<()>>,
    writer: Option<JoinHandle<()>>,
}

impl Transcript {
    #[cfg(test)]
    pub fn new() -> io::Result<Self> {
        Self::open(false, false)
    }

    #[cfg(test)]
    pub fn new_chat() -> io::Result<Self> {
        Self::open(true, false)
    }

    pub fn new_details() -> io::Result<Self> {
        Self::open(true, true)
    }

    fn open(chat: bool, details: bool) -> io::Result<Self> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let cwd = loop {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let name = format!("claude-switch-demo-{}-{serial}", std::process::id());
            let dir = std::env::temp_dir().join(name);
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        // Own the directory before the first fallible write, so partial setup is cleaned too.
        let path = cwd.join("field-guide.jsonl");
        let mut demo = Self { cwd, path, stop: None, writer: None };
        let mut file = File::create(&demo.path)?;
        seed(&demo, &mut file)?;
        if chat {
            seed_chat(&mut file, &demo.cwd)?;
            if details {
                seed_details(&mut file, &demo.cwd)?;
            }
            return Ok(demo);
        }
        let (stop, signal) = mpsc::channel();
        demo.stop = Some(stop);
        demo.writer = Some(thread::Builder::new().name("demo-transcript".into()).spawn(move || {
            for (i, delay) in [3, 5, 6, 8].into_iter().enumerate() {
                if signal.recv_timeout(Duration::from_secs(delay)) != Err(RecvTimeoutError::Timeout)
                    || append(&mut file, i).is_err()
                {
                    break;
                }
            }
        })?);
        Ok(demo)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    fn stop_writer(&mut self) {
        drop(self.stop.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn seed_details(file: &mut File, cwd: &Path) -> io::Result<()> {
    record(file, 40, "user", json!("Build the preview and show me what it produced."))?;
    let runs = [
        ("watch-preview", "b7watch", "npm run preview -- --watch", "Listening on the demo preview port"),
        ("check-proof", "b4done", "npm run check:proof", "Proof checks passed"),
        ("old-preview", "b2old", "npm run build:old-preview", "Stopped at a checkpoint"),
    ];
    // Each record is separate, exactly like the tool-use/result pairing of a real turn.
    for (i, (tool_id, shell_id, command, last)) in runs.into_iter().enumerate() {
        let log = cwd.join(format!("{shell_id}.output"));
        fs::write(
            &log,
            format!(
                "Checking assets\nLoading cover images\nBuilding the ridge plate\nPreparing styles\n\
             Indexing views\nRendering the north ridge\nWriting the preview\n{last}\n"
            ),
        )?;
        record(
            file,
            41 + i as u8,
            "assistant",
            json!([tool(
                tool_id,
                "Bash",
                json!({
                    "command": command, "run_in_background": true
                })
            )]),
        )?;
        record(
            file,
            42 + i as u8,
            "user",
            json!([result(
                tool_id,
                json!(format!(
                    "Command running in background with ID: {shell_id}. Output is being written to: {}",
                    log.display()
                ))
            )]),
        )?;
    }
    for (i, (id, status, code)) in [("b4done", "completed", 0), ("b2old", "killed", 130)].into_iter().enumerate() {
        record(
            file,
            45 + i as u8,
            "user",
            json!(format!(
                "<task-notification><task-id>{id}</task-id><status>{status}</status><exit-code>{code}</exit-code></task-notification>"
            )),
        )?;
    }
    let mut image = File::create(cwd.join("north-ridge.png"))?;
    let mut encoder = png::Encoder::new(&mut image, 144, 90);
    encoder.set_color(png::ColorType::Rgb);
    let mut rgb = Vec::new();
    for y in 0..90 {
        for x in 0..144 {
            let ridge = 56 - (45i32 - x).abs().min(50) / 2;
            let ink = if (y - (x - 108).abs()) < 39 && y < ridge {
                [248, 202, 120]
            } else if y >= ridge {
                [43, 67, 85]
            } else {
                [98, 155, 178]
            };
            rgb.extend(ink);
        }
    }
    encoder.write_header()?.write_image_data(&rgb)?;
    record(file, 46, "assistant", json!([tool("demo-image", "Write", json!({"file_path":"north-ridge.png"}))]))?;
    record(file, 47, "user", json!([result("demo-image", json!("Saved the north ridge plate."))]))?;
    fs::write(cwd.join("preview-report.txt"), "Demo preview report\n8 assets rendered.\n")?;
    record(file, 48, "assistant", json!([tool("demo-publish", "Bash", json!({"command":"publish preview"}))]))?;
    record(
        file,
        49,
        "user",
        json!([result(
            "demo-publish",
            json!(
                "Published https://claude.ai/artifacts/demo-north-ridge and created https://github.com/example/field-guide/pull/42\nSaved to: preview-report.txt"
            )
        )]),
    )?;
    record(
        file,
        50,
        "assistant",
        json!(
            "The [North ridge preview](https://claude.ai/artifacts/demo-north-ridge) and the \
         [review pull request](https://github.com/example/field-guide/pull/42) are ready. \
         See also the [build notes](https://example.test/field-guide/build)."
        ),
    )
}

fn seed_chat(file: &mut File, cwd: &Path) -> io::Result<()> {
    fs::write(cwd.join("ridge-caption.txt"), "Face west at dusk.\nLook for two ridgelines.\n")?;
    fs::write(cwd.join("plate-index.json"), "{\"night\":\"Moon\"}\n")?;
    record(file, 34, "user", json!("Shorten the north ridge caption."))?;
    record(file, 35, "assistant", json!("I’ll keep the direction and the two silhouettes."))?;
    record(
        file,
        36,
        "assistant",
        json!([
            tool(
                "demo-edit",
                "Edit",
                json!({
                    "file_path":"field-notes.md", "old_string":"Look west at dusk.",
                    "new_string":"Face west at dusk.\nLook for two ridgelines."
                })
            ),
            tool("demo-text", "Write", json!({"file_path":"ridge-caption.txt","content":"Face west at dusk."})),
            tool("demo-index", "Write", json!({"file_path":"plate-index.json","content":"{\"night\":\"Moon\"}"})),
            tool("demo-read", "Read", json!({"file_path":"field-notes.md"}))
        ]),
    )?;
    let results = ["demo-edit", "demo-text", "demo-index", "demo-read"].map(|id| result(id, json!("Done.")));
    record(file, 37, "user", json!(results))?;
    record(
        file,
        38,
        "assistant",
        json!(
            "**The caption is shorter.** Open [the field notes](field-notes.md) to see it. I’ll update the print heading next."
        ),
    )?;
    record(
        file,
        39,
        "assistant",
        json!([tool(
            "demo-wait",
            "Edit",
            json!({
                "file_path":"field-notes.md", "old_string":"# North ridge", "new_string":"# North ridge · Pocket guide"
            })
        )]),
    )?;
    writeln!(
        file,
        "{}",
        json!({"type":"assistant","message":{"model":"claude-sonnet-5-5","stop_reason":"tool_use","usage":{
            "input_tokens":843, "cache_read_input_tokens":3580
        }}})
    )
}

impl Drop for Transcript {
    fn drop(&mut self) {
        self.stop_writer();
        let _ = fs::remove_dir_all(&self.cwd);
    }
}

fn record(file: &mut File, sec: u8, role: &str, content: Value) -> io::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let at = now - 40 + i64::from(sec);
    let minute = mdshared_app::fmt::date_time(at, 0).replace(' ', "T");
    let line = json!({
        "timestamp":format!("{minute}:{:02}Z", at.rem_euclid(60)), "type":role,
        "message":{"role":role, "content":content}
    });
    writeln!(file, "{line}")
}

fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({"type":"tool_use", "id":id, "name":name, "input":input})
}

fn result(id: &str, content: Value) -> Value {
    json!({"type":"tool_result", "tool_use_id":id, "content":content})
}

fn seed(demo: &Transcript, file: &mut File) -> io::Result<()> {
    fs::write(demo.cwd.join("field-notes.md"), "# North ridge\n\nLook west at dusk. The two ridgelines overlap.\n")?;
    let reference = plate(false)?;
    let proof = plate(true)?;
    fs::write(demo.cwd.join("ridge-proof.png"), &proof)?;
    record(
        file,
        0,
        "user",
        json!([
            {"type":"text","text":"Make a north ridge pocket guide. Here’s my colour study."},
            image(&reference)
        ]),
    )?;
    record(file, 1, "assistant", json!("## North ridge · field notes\nUse **two silhouettes** for depth."))?;
    let tasks = tool(
        "plan",
        "TodoWrite",
        json!({"todos":[
            {"content":"Read the observer’s notes","status":"completed"},
            {"content":"Compare dusk and night plates","status":"in_progress"},
            {"content":"Write the pocket-guide caption","status":"pending"}
        ]}),
    );
    let agents = demo.path.with_extension("").join("subagents");
    fs::create_dir_all(&agents)?;
    let mut calls = vec![
        tasks,
        tool("notes", "Read", json!({"file_path":"field-notes.md"})),
        tool("proof", "Read", json!({"file_path":"ridge-proof.png"})),
    ];
    for (id, desc, finished, note) in [
        ("colour", "Compare the sky colours", Some(true), "Copper and slate keep their contrast at pocket size."),
        ("contour", "Trace the ridge contour", Some(false), "Checking where the two ridges cross."),
        ("caption", "Review the plate caption", None, "The caption should tell the reader which way to face."),
    ] {
        let spawn = format!("spawn-{id}");
        calls.push(tool(&spawn, "Agent", json!({"description":desc})));
        let side = agents.join(format!("agent-{id}.jsonl"));
        let mut child = File::create(&side)?;
        record(&mut child, 3, "user", json!(desc))?;
        record(&mut child, 4, "assistant", json!([tool("read", "Read", json!({"file_path":"field-notes.md"}))]))?;
        record(&mut child, 5, "user", json!([result("read", json!("Look west at dusk."))]))?;
        record(&mut child, 6, "assistant", json!(note))?;
        let meta = json!({"agentType":"reviewer", "description":desc, "model":"demo",
            "finished":finished, "toolUseId":spawn, "spawnDepth":1});
        fs::write(side.with_extension("meta.json"), meta.to_string())?;
    }
    record(file, 2, "assistant", json!(calls))?;
    record(
        file,
        7,
        "user",
        json!([
            result("plan", json!("Saved.")),
            result("notes", json!("Look west at dusk.")),
            result("proof", json!([image(&proof)])),
            result("spawn-colour", json!("Contrast checked."))
        ]),
    )?;
    record(
        file,
        8,
        "assistant",
        json!(
            r#"### Proof notes

| Plate | Light |
| --- | --- |
| Dusk | Sun |
| Night | Moon |

1. Compare both silhouettes.
   - [x] Check contrast.
   - [ ] Review at pocket size.
2. Keep the **night** plate.

> Face west. The paler peak stands beyond the saddle.

---

Use `caption` for the printed label. See `field-notes.md` and the [Markdown reference](https://commonmark.org/).

```rust
fn caption(night: bool) -> &'static str {
    if night { "Moon" } else { "Sun" }
}
```

```sh
plate="night"
printf '%s\n' "$plate"
```"#
        ),
    )?;
    Ok(())
}

fn append(file: &mut File, i: usize) -> io::Result<()> {
    let action = |id, verb| tool(id, "mcp__ccd_turn__update_plan", json!({"action":verb}));
    match i {
        0 => {
            record(file, 9, "user", json!("Keep the night version. Give me a caption I can read by torchlight."))?;
            record(file, 12, "assistant", json!([action("caption-start", "advance")]))?;
        }
        1 => {
            record(file, 17, "user", json!([result("caption-start", json!("Saved."))]))?;
            record(
                file,
                17,
                "assistant",
                json!("> Face west. Follow the dark ridge down to the saddle; the paler peak stands beyond it."),
            )?;
        }
        2 => record(file, 23, "assistant", json!([action("caption-done", "done")]))?,
        _ => {
            record(file, 31, "user", json!([result("caption-done", json!("Saved."))]))?;
            record(
                file,
                31,
                "assistant",
                json!("**Pocket guide ready.** The night plate and its caption fit on one page."),
            )?;
        }
    }
    Ok(())
}

// Two small original, flat-colour landscape plates. Keep them legible at thumbnail size.
fn plate(night: bool) -> io::Result<Vec<u8>> {
    let (w, h) = (256, 128);
    let colors = if night {
        [[30, 45, 77], [244, 225, 173], [79, 111, 141], [43, 69, 92], [18, 38, 54]]
    } else {
        [[241, 175, 133], [255, 225, 158], [186, 122, 122], [134, 90, 100], [76, 66, 83]]
    };
    let mut pixels = Vec::with_capacity(w * h * 3);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let disk = (x - 179).pow(2) + (y - 33).pow(2) < 19 * 19;
            let crescent = (x - 187).pow(2) + (y - 28).pow(2) > 16 * 16;
            let star = night && (x * 29 + y * 67) % 433 == 0 && y < 53;
            let c = if y > 97 + (x - 177).abs() / 9 - (x - 103).abs() / 5 {
                4
            } else if y > 66 + (x - 149).abs() / 3 {
                3
            } else if y > 44 + (x - 68).abs() / 2 || y > 55 + (x - 220).abs() / 2 {
                2
            } else if star || disk && (!night || crescent) {
                1
            } else {
                0
            };
            pixels.extend_from_slice(&colors[c]);
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(&pixels).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)?;
    }
    Ok(bytes)
}

fn image(bytes: &[u8]) -> Value {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut data = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for trio in bytes.chunks(3) {
        let bits = u32::from_be_bytes([0, trio[0], *trio.get(1).unwrap_or(&0), *trio.get(2).unwrap_or(&0)]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            data.push(if i > trio.len() { '=' } else { ABC[((bits >> shift) & 63) as usize] as char });
        }
    }
    json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":data}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::transcript::{ItemKind, Reader, TaskStatus};

    #[test]
    fn demo_uses_real_reader_shapes_and_streams_more_records() {
        let mut demo = Transcript::new().unwrap();
        demo.stop_writer();
        let mut reader = Reader::open(demo.path(), 512 * 1024).unwrap();
        let pictures: Vec<_> = reader
            .items()
            .flat_map(|(_, item)| match &item.kind {
                ItemKind::Prompt { pictures, .. } | ItemKind::Tool { pictures, .. } => pictures.as_slice(),
                _ => &[],
            })
            .cloned()
            .collect();
        assert_eq!(pictures.len(), 2);
        for picture in pictures {
            assert!(reader.load_picture(&picture).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        }
        let states: Vec<_> = reader.subagents().map(|agent| agent.finished).collect();
        assert_eq!(states, [None, Some(true), Some(false)]);
        assert_eq!(
            reader.tasks().iter().map(|task| task.status).collect::<Vec<_>>(),
            [TaskStatus::Completed, TaskStatus::InProgress, TaskStatus::Pending]
        );
        let mut file = File::options().append(true).open(demo.path()).unwrap();
        append(&mut file, 0).unwrap();
        let changes = reader.poll().unwrap();
        assert!(changes.tasks_changed);
        assert!(!changes.added.is_empty());
        assert_eq!(reader.tasks()[2].status, TaskStatus::InProgress);
    }
}
