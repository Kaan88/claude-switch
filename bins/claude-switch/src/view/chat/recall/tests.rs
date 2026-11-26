use std::sync::mpsc;
use std::time::Duration;

use mdshared_ui::keys::Mods;

use super::*;
use crate::view::model;

fn fixture() -> Model {
    // Manual fixtures below don't start the reader or touch any path.
    Model::new(model::Source::File(Default::default()), Default::default(), || {})
}

fn prompt(model: &mut Model, offset: u64, text: &str, synthetic: bool) {
    model.items.insert(
        ItemId { offset, block: 0 },
        Item { timestamp: None, kind: ItemKind::Prompt { text: text.into(), pictures: Vec::new(), synthetic } },
    );
}

#[test]
fn up_down_browse_distinct_adjacent_prompts_preserving_the_draft_and_selection() {
    let mut model = fixture();
    for (offset, body, synthetic) in [
        (10, "latest", false),
        (20, "oldest but different", false),
        (30, "latest", false),
        (40, " latest ", false),
        (50, "synthetic", true),
        (60, " \n ", false),
    ] {
        prompt(&mut model, offset, body, synthetic);
    }
    let mut chat = Chat::default();
    chat.editor.insert("unsent café");
    chat.editor.move_to(2, false);
    chat.editor.move_to(6, true);
    let keys = [KeyName::Up, KeyName::Up, KeyName::Up, KeyName::Up, KeyName::Down, KeyName::Down, KeyName::Down];
    let seen: Vec<_> = keys
        .into_iter()
        .map(|key| {
            assert!(chat.recall_key(&model, &key));
            chat.editor.text.clone()
        })
        .collect();
    assert_eq!(
        seen,
        [" latest ", "oldest but different", "latest", "latest", "oldest but different", "latest", "unsent café"]
    );
    assert_eq!(chat.editor.selection(), 2..6);
    assert_eq!(chat.editor.caret, 6);
    assert!(!chat.recall.active());
}

#[test]
fn history_boundaries_preserve_normal_vertical_motion_with_explicit_and_visual_wraps() {
    let mut model = fixture();
    prompt(&mut model, 10, "first\nlast", false);
    prompt(&mut model, 20, "newest", false);
    let mut chat = Chat::default();
    chat.editor.insert("one two");
    chat.editor.rows = vec![0..4, 4..7];
    // Start on the last displayed row, even though the draft has no explicit newline.
    assert!(!chat.recall_key(&model, &KeyName::Up));
    chat.editor.key(&KeyName::Up, Mods::default(), None);
    assert!(chat.editor.on_first_line());
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert_eq!(chat.editor.text, "first\nlast");
    assert_eq!(chat.editor.caret, 0);
    assert!(!chat.recall_key(&model, &KeyName::Down));
    // No paint yet: moving to the next explicit line still works.
    chat.editor.key(&KeyName::Down, Mods::default(), None);
    assert!(chat.editor.on_last_line());
    assert!(chat.recall_key(&model, &KeyName::Down));
    assert_eq!(chat.editor.text, "newest");
    assert!(chat.recall_key(&model, &KeyName::Down));
    assert_eq!(chat.editor.text, "one two");
    assert!(chat.editor.on_first_line());
}

#[test]
fn escape_restores_the_unsent_draft_even_after_editing_the_recalled_text() {
    let mut model = fixture();
    prompt(&mut model, 70, "/context", false);
    let mut chat = Chat::default();
    chat.editor.set("keep me\nsecond line".into());
    chat.editor.move_to(3, false);
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert_eq!(chat.editor.text, "/context");
    assert!(!chat.commands.open, "history must not open the slash picker");
    chat.editor.insert("edited:");
    assert!(chat.recall_key(&model, &KeyName::Escape));
    assert_eq!(chat.editor.text, "keep me\nsecond line");
    assert_eq!(chat.editor.caret, 3);
    assert!(!chat.recall_key(&model, &KeyName::Escape));
}

#[test]
fn open_slash_picker_keeps_arrows_and_transcript_reset_restores_draft() {
    let mut model = fixture();
    prompt(&mut model, 10, "older from the transcript", false);
    let mut chat = Chat::default();
    chat.editor.insert("draft");
    chat.commands.open = true;
    assert!(!chat.recall_key(&model, &KeyName::Up));
    assert!(!chat.recall_key(&model, &KeyName::Down));
    assert_eq!(chat.editor.text, "draft");
    chat.commands.open = false;
    assert!(chat.recall_key(&model, &KeyName::Up));
    model.generation += 1;
    assert!(chat.recall.advance(&mut chat.editor, &model, true));
    assert_eq!(chat.editor.text, "draft");
    assert!(!chat.recall.active());
}

#[test]
fn boundary_loads_older_terminal_prompts_from_the_worker_and_escape_cancels_recall() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let path = demo.cwd().join("recall-fixture.jsonl");
    let entry = |role: &str, body: &str| {
        serde_json::json!({"type": role, "message": {"role": role, "content": body}}).to_string()
    };
    let rows = [
        entry("user", "earlier prompt entered in terminal"),
        entry("assistant", &"x".repeat(275 * 1024)),
        entry("user", "latest prompt entered in terminal"),
    ];
    std::fs::write(&path, rows.join("\n") + "\n").unwrap();
    let (tx, rx) = mpsc::channel();
    let mut model = Model::new(model::Source::File(path), demo.cwd().into(), move || {
        let _ = tx.send(());
    });
    model.poll();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(model.collect() && model.has_earlier);
    assert_eq!(model.items.values().filter_map(text).collect::<Vec<_>>(), ["latest prompt entered in terminal"]);
    let mut chat = Chat::default();
    chat.editor.insert("not sent");
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert_eq!(chat.editor.text, "latest prompt entered in terminal");
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert!(chat.recall.waiting());
    // A live append can advance the model revision; only a historical page can satisfy the pending request.
    model.revision += 1;
    assert!(!chat.recall.advance(&mut chat.editor, &model, true));
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(model.collect());
    assert!(chat.recall.advance(&mut chat.editor, &model, true));
    assert_eq!(chat.editor.text, "latest prompt entered in terminal");
    assert!(chat.recall.waiting(), "the first historical page contains only assistant output");
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(model.collect());
    assert!(chat.recall.advance(&mut chat.editor, &model, true));
    assert_eq!(chat.editor.text, "earlier prompt entered in terminal");
    assert!(chat.recall_key(&model, &KeyName::Escape));
    assert_eq!(chat.editor.text, "not sent");
}

#[test]
fn escape_while_waiting_prevents_a_late_page_from_replacing_the_draft() {
    let mut model = fixture();
    prompt(&mut model, 70, "newer", false);
    let mut chat = Chat::default();
    chat.editor.insert("unsent");
    assert!(chat.recall_key(&model, &KeyName::Up));
    model.has_earlier = true;
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert!(chat.recall.waiting());
    assert!(chat.recall_key(&model, &KeyName::Escape));
    prompt(&mut model, 60, "older", false);
    model.earlier_version += 1;
    assert!(!chat.recall.advance(&mut chat.editor, &model, true));
    assert_eq!(chat.editor.text, "unsent");
}
