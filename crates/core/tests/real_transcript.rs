//! Reads an explicitly supplied transcript and checks PNG/JPEG payload signatures, not image decoding.
//! `cargo test -p claude-switch-core --test real_transcript -- --ignored --nocapture`

use claude_switch_core::transcript::{ItemKind, Reader};

#[test]
#[ignore = "requires CS_REAL_TRANSCRIPT pointing at a transcript with PNG/JPEG attachments"]
fn supplied_transcript_loads_picture_payloads_with_matching_png_or_jpeg_signatures() {
    let path = std::env::var_os("CS_REAL_TRANSCRIPT")
        .filter(|p| !p.is_empty())
        .expect("set CS_REAL_TRANSCRIPT to a transcript with PNG/JPEG attachments");
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
    let mut signatures = 0;
    for p in &pictures {
        let bytes = reader.load_picture(p).unwrap();
        let expected: &[u8] = match p.media_type.as_str() {
            "image/png" => b"\x89PNG\r\n\x1a\n",
            "image/jpeg" => b"\xff\xd8\xff",
            _ => continue,
        };
        assert!(bytes.starts_with(expected), "a picture payload did not match its declared signature");
        signatures += 1;
    }
    assert!(signatures > 0, "no PNG/JPEG attachments found in the scanned chunks");
    println!(
        "opened in {opened:?}; scanned {chunks} chunks in {:?}; prompts {prompts}, texts {texts}, tools {tools}, notices {notices}, loaded pictures {}, PNG/JPEG signatures {signatures}, tasks {}, subagents {}",
        start.elapsed(),
        pictures.len(),
        reader.tasks().len(),
        reader.subagents().count()
    );
}
