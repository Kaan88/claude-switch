use super::*;

fn id(offset: u64) -> ItemId {
    ItemId { offset, block: 0 }
}

fn item(kind: ItemKind) -> Item {
    Item { kind, timestamp: None }
}

fn tool(name: &str, state: ToolState, card: Option<ToolCard>) -> Item {
    item(ItemKind::Tool {
        id: name.to_owned(),
        name: name.into(),
        summary: "line one\nline two".into(),
        file_path: match &card {
            Some(ToolCard::Edit { path, .. }) => Some(path.clone()),
            _ => None,
        },
        state,
        duration_ms: (state == ToolState::Ok).then_some(420),
        pictures: Vec::new(),
        subagent: None,
        denial: None,
        card,
    })
}

#[test]
fn tool_only_outputs_follow_the_next_answer_and_repeated_links_use_its_label() {
    use crate::view::outputs::collect;
    use claude_switch_core::transcript::ResultDetails;
    let prompt = || item(ItemKind::Prompt { text: "show outputs".into(), pictures: vec![], synthetic: false });
    let answer = item(ItemKind::Assistant { markdown: "[Named artifact](https://claude.ai/artifacts/1)".into() });
    let call = tool("Bash", ToolState::Ok, None);
    let result =
        ResultDetails { text: "https://claude.ai/artifacts/1 https://example.test/report".into(), timestamp: None };
    let items = BTreeMap::from([(id(1), prompt()), (id(2), call), (id(3), answer), (id(4), prompt())]);
    let outputs = BTreeMap::from([
        (id(2), collect(id(2), &items[&id(2)], Some(&result), None)),
        (id(3), collect(id(3), &items[&id(3)], None, None)),
    ]);
    let cards = output_cards(&items, &outputs);
    assert_eq!(cards.keys().copied().collect::<Vec<_>>(), [id(3)]);
    assert_eq!(
        cards[&id(3)].iter().map(|o| o.title.as_str()).collect::<Vec<_>>(),
        ["example.test/report", "Named artifact"]
    );
}

#[test]
fn geometry_right_aligns_bubbles_and_starts_the_rest_at_the_columns_edge() {
    for width in [240.0, 330.0, 660.0, 780.0] {
        let bubble = Geometry::new(Style::Prompt, width, 105.0);
        let long = Geometry::new(Style::Prompt, width, 999.0);
        let answer = Geometry::new(Style::Answer, width, 0.0);
        let tool = Geometry::new(Style::Tool, width, 0.0);
        let chips = Geometry::new(Style::Files, width, 0.0);
        assert_eq!(bubble.w, 105.0);
        assert!((long.x + long.w - width).abs() < 0.001);
        assert!(long.x > answer.x && long.w <= width * 0.8 && bubble.x > long.x);
        // One column: no mark before an answer and no indent before a tool line.
        assert_eq!((answer.x, answer.tx, answer.w), (0.0, 0.0, width));
        assert!(tool.x == answer.x && chips.x == answer.x && tool.tx + tool.tw <= width);
        assert_eq!(tool.ty, TOOL_H + 6.0);
        assert!((0..32).all(|i| chip_rect(chips, i).right() <= width));
    }
}

#[test]
fn both_views_expand_tools_and_put_only_successful_named_markdown_under_the_turn() {
    let demo = crate::demo::Transcript::new().unwrap();
    let name = "field-notes.md";
    let file = crate::view::links::resolve(name, demo.cwd(), &Default::default()).unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let edit = || ToolCard::Edit { path: name.into(), added: 1, removed: 1, lines: vec!["+hi".into(), "-bye".into()] };
    let shell = ToolCard::Shell { command: "echo enter/leave".into(), result: vec!["enter/leave".into()] };
    let prompt = || item(ItemKind::Prompt { text: "short".into(), pictures: vec![], synthetic: false });
    model.items = [
        prompt(),
        item(ItemKind::Assistant { markdown: "Here is the answer.".into() }),
        tool("Write", ToolState::Ok, Some(edit())),
        tool("Edit", ToolState::Ok, Some(edit())),
        tool("Bash", ToolState::Failed, Some(shell)),
        tool("Read", ToolState::Running, None),
        item(ItemKind::Assistant { markdown: "Done.".into() }),
        prompt(),
        tool("Edit", ToolState::Failed, Some(edit())),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    for i in [2, 3, 8] {
        model.files.insert(id(i), Files::from([(name.into(), file.clone())]));
    }
    model.groups = vec![
        Group::One(id(0)),
        Group::One(id(1)),
        Group::Tools((2..6).map(id).collect()),
        Group::One(id(6)),
        Group::One(id(7)),
        Group::Tools(vec![id(8)]),
    ];
    let project = |chat, open| {
        let mut rows = Rows {
            list: vec![],
            previous: BTreeMap::new(),
            end: 0.0,
            width: if chat { 700.0 } else { 330.0 },
            scale: 1.0,
            pal: &mdshared_ui::theme::DARK,
            chat,
        };
        rows.conversation(
            &model,
            &if open { [id(2)].into() } else { BTreeSet::new() },
            &if open { [id(4)].into() } else { BTreeSet::new() },
        );
        rows.list
    };
    for chat in [false, true] {
        let closed = project(chat, false);
        assert_eq!(
            closed.iter().map(|r| r.style).collect::<Vec<_>>(),
            [Style::Prompt, Style::Answer, Style::Group, Style::Answer, Style::Files, Style::Prompt, Style::Tool]
        );
        assert_eq!(closed[2].title, "4 tool calls");
        assert_eq!(closed[2].stamp, "1 failed · 1 running");
        assert_eq!(closed[4].files.as_slice(), std::slice::from_ref(&file));
        let open = project(chat, true);
        let bash = open.iter().find(|r| r.id == Id::Tool(id(4))).unwrap();
        assert_eq!(bash.body.as_ref().unwrap().text(), "echo enter/leave\n\nenter/leave");
        assert!(bash.h > TOOL_H);
        assert_eq!(open.iter().find(|r| r.id == Id::Tool(id(2))).unwrap().duration, "420 ms");
        assert_eq!(open.iter().find(|r| r.id == Id::Tool(id(5))).unwrap().stamp, "◌ running");
    }
    assert!(
        project(true, false)[1].body.as_ref().unwrap().height()
            > project(false, false)[1].body.as_ref().unwrap().height()
    );
}

#[test]
fn notices_are_compact_rows_that_expand_and_thoughts_do_not_split_or_count_in_a_group() {
    use claude_switch_core::transcript::NoticeKind;
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let notice = |kind, text: &str| item(ItemKind::Notice { kind, text: text.into() });
    model.items = [
        item(ItemKind::Prompt { text: "go".into(), pictures: vec![], synthetic: false }),
        notice(NoticeKind::Thought, "First I **read** the file.\nThen I edit it."),
        tool("Read", ToolState::Ok, None),
        notice(NoticeKind::Thought, "Now the edit."),
        tool("Edit", ToolState::Ok, None),
        tool("Bash", ToolState::Ok, None),
        tool("Grep", ToolState::Ok, None),
        notice(NoticeKind::StopHook, "Stop hook feedback:\nRun the tests first."),
        notice(NoticeKind::Compaction, "summary"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    assert_eq!(model.groups[1], Group::Tools((1..7).map(id).collect()), "thoughts stay inside the tool group");
    let project = |open: BTreeSet<ItemId>, tools: BTreeSet<ItemId>| {
        let pal = &mdshared_ui::theme::DARK;
        let mut rows =
            Rows { list: vec![], previous: BTreeMap::new(), end: 0.0, width: 700.0, scale: 1.0, pal, chat: true };
        rows.conversation(&model, &open, &tools);
        rows.list
    };
    let closed = project(BTreeSet::new(), BTreeSet::new());
    assert_eq!(closed[1].title, "4 tool calls", "thoughts are not counted as calls");
    let hook = &closed[2];
    assert_eq!(
        (hook.style, hook.title.as_str(), hook.target.as_str()),
        (Style::Tool, "Stop hook feedback", "Stop hook feedback:")
    );
    assert_eq!(hook.h, TOOL_H);
    assert_eq!(closed[3].style, Style::Card, "a compaction keeps its card");
    let open = project([id(1)].into(), [id(1), id(7)].into());
    let thought = open.iter().find(|r| r.id == Id::Tool(id(1))).unwrap();
    assert_eq!(thought.title, "Thought");
    assert!(thought.body.as_ref().unwrap().text().contains("read"), "an open thought shows its text");
    let hook = open.iter().find(|r| r.id == Id::Tool(id(7))).unwrap();
    assert!(hook.h > TOOL_H && hook.body.as_ref().unwrap().text().contains("Run the tests first."));
}

#[test]
fn command_replies_read_on_their_row_and_longer_output_is_a_mono_card() {
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let local = |name: &str, summary: &str, text: &str, state| {
        item(ItemKind::Tool {
            id: format!("local:{name}"),
            name: name.into(),
            summary: summary.into(),
            file_path: None,
            state,
            duration_ms: None,
            pictures: Vec::new(),
            subagent: None,
            denial: None,
            card: Some(ToolCard::Output(text.into())),
        })
    };
    model.items = [
        item(ItemKind::Prompt { text: "/model".into(), pictures: vec![], synthetic: false }),
        local("/model", "", "Set model to Opus 5.5", ToolState::Ok),
        item(ItemKind::Prompt { text: "/context".into(), pictures: vec![], synthetic: false }),
        local("/context", "", "Context Usage\n⛁ ⛁ ⛀ 12k / 200k", ToolState::Ok),
        local("Shell", "! git status", "On branch main", ToolState::Ok),
        local("Shell", "! git lg", "git: 'lg' is not a git command.", ToolState::Failed),
        item(ItemKind::Prompt { text: "next".into(), pictures: vec![], synthetic: false }),
        local("Shell", "! true", "", ToolState::Ok),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    let project = |tools: BTreeSet<ItemId>| {
        let pal = &mdshared_ui::theme::DARK;
        let mut rows =
            Rows { list: vec![], previous: BTreeMap::new(), end: 0.0, width: 700.0, scale: 1.0, pal, chat: true };
        rows.conversation(&model, &BTreeSet::new(), &tools);
        rows.list
    };
    let rows = project(BTreeSet::new());
    let row = |i| rows.iter().find(|r| r.id == Id::Tool(id(i))).unwrap();
    let model_row = row(1);
    assert_eq!((model_row.title.as_str(), model_row.target.as_str()), ("/model", "Set model to Opus 5.5"));
    assert!(model_row.body.is_none() && model_row.h == TOOL_H, "a one-line reply is a closed note");
    let context = row(3);
    assert_eq!(context.body.as_ref().unwrap().text(), "Context Usage\n⛁ ⛁ ⛀ 12k / 200k", "longer output is shown");
    assert!(context.h > TOOL_H && context.target.is_empty());
    let shell = row(4);
    assert_eq!((shell.title.as_str(), shell.target.as_str()), ("Shell", "! git status"));
    assert_eq!(shell.body.as_ref().unwrap().text(), "On branch main", "shell output sits under its command");
    let failed = row(5);
    assert!(matches!(failed.tone, Tone::Warning) && failed.stamp == "✗ failed");
    assert_eq!(row(7).h, TOOL_H, "a shell command that printed nothing is one line");
    // The one-line reply still opens; a click folds longer output.
    let open = project([id(1), id(3)].into());
    let folded = open.iter().find(|r| r.id == Id::Tool(id(3))).unwrap();
    assert!(!folded.open && folded.body.is_none() && folded.h == TOOL_H);
    assert_eq!(
        open.iter().find(|r| r.id == Id::Tool(id(1))).unwrap().body.as_ref().unwrap().text(),
        "Set model to Opus 5.5"
    );
}

#[test]
fn a_denied_call_says_who_refused_it_instead_of_failed() {
    use claude_switch_core::transcript::{Denial, DenialKind};
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let denied = |kind, reason: &str| {
        let shell = ToolCard::Shell { command: "mkdir probe".into(), result: vec![reason.into()] };
        let mut call = tool("Bash", ToolState::Failed, Some(shell));
        if let ItemKind::Tool { denial, .. } = &mut call.kind {
            *denial = Some(Box::new(Denial { kind, before_run: None, reason: reason.into(), rule_source: None }));
        }
        call
    };
    model.items = [
        item(ItemKind::Prompt { text: "go".into(), pictures: vec![], synthetic: false }),
        denied(DenialKind::AutoMode, "Too risky."),
        denied(DenialKind::User, "The user doesn't want to proceed with this tool use."),
        tool("Read", ToolState::Failed, None),
        tool("Read", ToolState::Ok, None),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    let pal = &mdshared_ui::theme::DARK;
    let mut rows =
        Rows { list: vec![], previous: BTreeMap::new(), end: 0.0, width: 700.0, scale: 1.0, pal, chat: true };
    rows.conversation(&model, &[id(1)].into(), &[id(1)].into());
    let row = |i| rows.list.iter().find(|r| r.id == Id::Tool(id(i))).unwrap();
    let group = rows.list.iter().find(|r| r.style == Style::Group).expect("four calls fold into a group");
    assert_eq!(group.stamp, "2 denied · 1 failed", "a denied call did not fail: it never ran");
    assert!(matches!(group.tone, Tone::Warning));
    assert!(matches!(row(1).tone, Tone::Warning) && row(1).stamp == "✗ blocked by auto mode");
    assert_eq!(row(2).stamp, "✗ you said no");
    assert_eq!(row(3).stamp, "✗ failed");
    // Opened, the row says why under what was asked for.
    let body = row(1).body.as_ref().unwrap().text();
    assert!(
        body.starts_with("mkdir probe") && body.ends_with("Auto mode's classifier blocked it: Too risky."),
        "{body}"
    );
}
