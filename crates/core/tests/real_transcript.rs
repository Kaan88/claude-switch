//! Reads a real transcript named by `CS_REAL_TRANSCRIPT` and prints counts only, never content.
//! `cargo test -p claude-switch-core --test real_transcript -- --ignored --nocapture`

use claude_switch_core::transcript::{ItemKind, Reader};

#[test]
#[ignore = "needs CS_REAL_TRANSCRIPT"]
fn a_real_transcript_reads_and_its_pictures_decode() {
    let Ok(path) = std::env::var("CS_REAL_TRANSCRIPT") else { return };
    let start = std::time::Instant::now();
    let mut reader = Reader::open(&path, 2 * 1024 * 1024).unwrap();
    let opened = start.elapsed();
    let mut chunks = 1;
    while reader.has_earlier() && chunks < 40 {
        reader.earlier().unwrap();
        chunks += 1;
    }
    let (mut prompts, mut texts, mut tools, mut notices, mut pictures) = (0, 0, 0, 0, Vec::new());
    for (_, item) in reader.items() {
        match &item.kind {
            ItemKind::Prompt { pictures: p, .. } => {
                prompts += 1;
                pictures.extend(p.iter().cloned());
            }
            ItemKind::Assistant { .. } => texts += 1,
            ItemKind::Tool { pictures: p, .. } => {
                tools += 1;
                pictures.extend(p.iter().cloned());
            }
            ItemKind::Notice { .. } => notices += 1,
        }
    }
    let mut decoded = 0;
    for p in &pictures {
        let bytes = reader.load_picture(p).unwrap();
        let png = bytes.starts_with(&[0x89, b'P', b'N', b'G']);
        let jpeg = bytes.starts_with(&[0xFF, 0xD8]);
        assert!(png || jpeg || !p.media_type.contains("png"), "a picture did not decode to its type");
        decoded += 1;
    }
    println!(
        "opened in {opened:?}; all {chunks} chunks in {:?}; prompts {prompts}, texts {texts}, tools {tools}, notices {notices}, pictures {decoded}, tasks {}, subagents {}",
        start.elapsed(),
        reader.tasks().len(),
        reader.subagents().len()
    );
}
