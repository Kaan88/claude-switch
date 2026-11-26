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
fn geometry_right_aligns_bubbles_and_indents_tools_at_session_and_chat_widths() {
    for width in [240.0, 330.0, 660.0, 780.0] {
        let bubble = Geometry::new(Style::Prompt, width, 105.0);
        let long = Geometry::new(Style::Prompt, width, 999.0);
        let answer = Geometry::new(Style::Answer, width, 0.0);
        let tool = Geometry::new(Style::Tool, width, 0.0);
        let chips = Geometry::new(Style::Files, width, 0.0);
        assert_eq!(bubble.w, 105.0);
        assert!((long.x + long.w - width).abs() < 0.001);
        assert!(long.x > answer.x && long.w < width && bubble.x > long.x);
        assert_eq!(answer.tx, answer.x);
        assert!(tool.x > answer.tx && tool.tx + tool.tw <= width);
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
