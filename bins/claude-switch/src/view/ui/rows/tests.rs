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
    let prompt =
        || item(ItemKind::Prompt { text: "show outputs".into(), pictures: vec![], synthetic: false, queued: false });
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
        assert!((0..32).all(|i| chip_rect(chips, i).right() <= width));
    }
}

#[test]
fn both_views_say_runs_in_words_show_edits_as_rows_and_put_only_successful_named_markdown_under_the_turn() {
    let demo = crate::demo::Transcript::new().unwrap();
    let name = "field-notes.md";
    let file = crate::view::links::resolve(name, demo.cwd(), &Default::default()).unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let edit = || ToolCard::Edit {
        path: name.into(),
        added: 1,
        removed: 1,
        lines: vec!["+ hi".into(), "− bye".into()],
        patch: None,
    };
    let shell = ToolCard::Shell { command: "echo enter/leave".into(), result: vec!["enter/leave".into()] };
    let prompt = || item(ItemKind::Prompt { text: "short".into(), pictures: vec![], synthetic: false, queued: false });
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
        tool("Grep", ToolState::Ok, None),
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
        Group::Tools(vec![id(8), id(9)]),
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
            offer: None,
            awaiting: None,
            answers: Default::default(),
        };
        rows.conversation(
            &model,
            &if open { [id(4), id(8)].into() } else { BTreeSet::new() },
            &if open { [id(2), id(4)].into() } else { BTreeSet::new() },
        );
        rows.list
    };
    for chat in [false, true] {
        let line = tool_h(chat);
        let closed = project(chat, false);
        // The two edits that went through are rows of their own; the calls after them are one
        // run, and so are the failed edit and the search of the second turn.
        assert_eq!(
            closed.iter().map(|r| r.style).collect::<Vec<_>>(),
            [
                Style::Prompt,
                Style::Answer,
                Style::Edit,
                Style::Edit,
                Style::Group,
                Style::Answer,
                Style::Files,
                Style::Prompt,
                Style::Group
            ]
        );
        let written = &closed[2];
        assert_eq!((written.target.as_str(), written.counts, written.h), (name, (1, 1), EDIT_H));
        assert!(!written.open && written.body.is_none());
        // A running call names the run, in the text colour; the failure is counted beside it.
        let run = &closed[4];
        assert_eq!((run.title.as_str(), run.target.as_str()), ("Reading", "line one line two"));
        assert!(run.running && run.stamp == "1 failed" && matches!(run.tone, Tone::Warning) && run.h == line);
        let second = &closed[8];
        assert_eq!((second.title.as_str(), second.stamp.as_str()), ("Edited a file, searched the code", "1 failed"));
        assert!(!second.running);
        assert_eq!(closed[6].files.as_slice(), std::slice::from_ref(&file));
        let open = project(chat, true);
        let step = |i| open.iter().find(|r| r.id == Id::Tool(id(i))).unwrap();
        // The run's steps, in order, on its timeline; the opened one shows what it printed.
        let bash = step(4);
        assert_eq!((bash.title.as_str(), bash.stamp.as_str(), bash.step), ("Ran", "failed", Some((true, false))));
        assert_eq!(bash.body.as_ref().unwrap().text(), "echo enter/leave\n\nenter/leave");
        assert!(bash.h > line && bash.boxed);
        let read = step(5);
        assert_eq!((read.title.as_str(), read.step, read.h), ("Reading", Some((false, true)), line));
        assert!(read.running && read.stamp.is_empty());
        // A failed edit changed nothing: it is a step that says so, not an edit's row.
        let failed = step(8);
        assert_eq!((failed.style, failed.title.as_str(), failed.stamp.as_str()), (Style::Tool, "Edited", "failed"));
        assert_eq!(step(9).duration, "420 ms");
        // An opened edit shows its changed lines, signs apart, without line numbers it does not have.
        let diff = step(2);
        assert!(diff.open && diff.h > EDIT_H);
        assert_eq!(diff.body.as_ref().unwrap().text(), "hi\nbye");
        assert_eq!(diff.diff.lines.iter().map(|l| (l.number, l.sign)).collect::<Vec<_>>(), [(None, '+'), (None, '-')]);
    }
    assert!(
        project(true, false)[1].body.as_ref().unwrap().height()
            > project(false, false)[1].body.as_ref().unwrap().height()
    );
}

#[test]
fn an_edits_diff_takes_line_numbers_and_context_from_the_recorded_patch() {
    use claude_switch_core::transcript::{Hunk, Patch};
    let hunk = |old_start, new_start, lines: &[&str], more| Hunk {
        old_start,
        new_start,
        lines: lines.iter().map(|l| (*l).to_owned()).collect(),
        more,
    };
    let card = ToolCard::Edit {
        path: "src/a.rs".into(),
        added: 2,
        removed: 1,
        lines: vec!["− old".into(), "+ new".into(), "+ more".into()],
        patch: Some(Box::new(Patch {
            hunks: vec![
                hunk(86, 86, &[" fn a() {", "-    old", "+    new", "+    more", " }"], 0),
                hunk(120, 121, &["-x"], 3),
            ],
        })),
    };
    let (diff, texts) = diff_of(&card);
    assert_eq!(texts, ["fn a() {", "    old", "    new", "    more", "}", "x"]);
    // A removed line has the old file's number; the others the new file's.
    assert_eq!(
        diff.lines.iter().map(|l| (l.number, l.sign)).collect::<Vec<_>>(),
        [(Some(86), ' '), (Some(87), '-'), (Some(87), '+'), (Some(88), '+'), (Some(89), ' '), (Some(120), '-')]
    );
    assert_eq!((diff.breaks.as_slice(), diff.more), (&[5][..], 3));
    // Without a patch: the reader's changed lines, in colour, with what it left out counted.
    let ToolCard::Edit { added, removed, lines, .. } = card.clone() else { unreachable!() };
    let plain = ToolCard::Edit { path: "a".into(), added: added + 30, removed, lines, patch: None };
    let (diff, texts) = diff_of(&plain);
    assert_eq!(texts, ["old", "new", "more"]);
    assert!(diff.lines.iter().all(|l| l.number.is_none()) && diff.more == 30 && diff.breaks.is_empty());
}

#[test]
fn a_run_reads_in_words_by_kind_and_a_call_a_card_asks_about_waits() {
    assert_eq!(
        summary(&[Kind::Tasks, Kind::Command, Kind::Read, Kind::Read, Kind::Thought, Kind::Read, Kind::Search]),
        "Ran a command, read 3 files, searched the code"
    );
    assert_eq!(summary(&[Kind::Thought, Kind::Thought]), "Thought");
    assert_eq!(summary(&[Kind::Tasks, Kind::Tasks]), "Updated the task list");
    assert_eq!(summary(&[Kind::Tasks, Kind::Thought]), "Updated the task list, thought");
    assert_eq!(
        summary(&[Kind::Web, Kind::Agent, Kind::Agent, Kind::Other, Kind::Edit]),
        "Looked at a web page, ran 2 agents, used a tool, edited a file"
    );
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let shell = |command: &str| ToolCard::Shell { command: command.into(), result: Vec::new() };
    let call = |name: &str, summary: &str, state, card| {
        let mut call = tool(name, state, card);
        if let ItemKind::Tool { summary: s, .. } = &mut call.kind {
            *s = summary.into();
        }
        call
    };
    model.items = [
        item(ItemKind::Prompt { text: "go".into(), pictures: vec![], synthetic: false, queued: false }),
        call("Bash", "cargo test -p tiles", ToolState::Ok, Some(shell("cargo test -p tiles"))),
        item(ItemKind::Assistant { markdown: "Now the benchmark.".into() }),
        call("Bash", "cargo bench", ToolState::Running, Some(shell("cargo bench"))),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    let project = |awaiting, open: bool| {
        let pal = &mdshared_ui::theme::DARK;
        let mut rows = Rows {
            list: vec![],
            previous: BTreeMap::new(),
            end: 0.0,
            width: 700.0,
            scale: 1.0,
            pal,
            chat: true,
            offer: None,
            awaiting,
            answers: Default::default(),
        };
        rows.conversation(&model, &BTreeSet::new(), &if open { [id(1)].into() } else { BTreeSet::new() });
        rows.list
    };
    // A single call reads as itself, at the column's edge.
    let rows = project(None, false);
    let done = rows.iter().find(|r| r.id == Id::Tool(id(1))).unwrap();
    assert_eq!((done.style, done.title.as_str(), done.target.as_str()), (Style::Tool, "Ran", "cargo test -p tiles"));
    assert!(done.step.is_none() && !done.running && done.geo.x == 0.0 && done.h == CHAT_TOOL_H);
    let running = rows.iter().find(|r| r.id == Id::Tool(id(3))).unwrap();
    assert_eq!((running.title.as_str(), running.target.as_str()), ("Running", "cargo bench"));
    assert!(running.running);
    // Opened, the row already shows the one-line command: the box holds only what it printed.
    let open = project(None, true);
    let done = open.iter().find(|r| r.id == Id::Tool(id(1))).unwrap();
    assert_eq!(done.body.as_ref().unwrap().text(), "No text result.");
    // While a card asks about the call it has not started.
    let waiting = project(Some(id(3)), false);
    let asked = waiting.iter().find(|r| r.id == Id::Tool(id(3))).unwrap();
    assert_eq!((asked.title.as_str(), asked.target.as_str()), ("Waiting to run", "cargo bench"));
}

#[test]
fn notices_are_compact_rows_that_expand_and_thoughts_are_steps_that_do_not_split_or_name_a_run() {
    use claude_switch_core::transcript::NoticeKind;
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let notice = |kind, text: &str| item(ItemKind::Notice { kind, text: text.into() });
    model.items = [
        item(ItemKind::Prompt { text: "go".into(), pictures: vec![], synthetic: false, queued: false }),
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
        let mut rows = Rows {
            list: vec![],
            previous: BTreeMap::new(),
            end: 0.0,
            width: 700.0,
            scale: 1.0,
            pal,
            chat: true,
            offer: None,
            awaiting: None,
            answers: Default::default(),
        };
        rows.conversation(&model, &open, &tools);
        rows.list
    };
    let closed = project(BTreeSet::new(), BTreeSet::new());
    assert_eq!(
        closed[1].title, "Read a file, edited a file, ran a command, searched the code",
        "thoughts are not named beside the calls"
    );
    let hook = &closed[2];
    assert_eq!(
        (hook.style, hook.title.as_str(), hook.target.as_str()),
        (Style::Tool, "Stop hook feedback", "Stop hook feedback:")
    );
    assert_eq!(hook.h, CHAT_TOOL_H);
    assert_eq!(closed[3].style, Style::Card, "a compaction keeps its card");
    let open = project([id(1)].into(), [id(1), id(7)].into());
    let thought = open.iter().find(|r| r.id == Id::Tool(id(1))).unwrap();
    assert_eq!((thought.title.as_str(), thought.kind, thought.step), ("Thought", Kind::Thought, Some((true, false))));
    assert!(thought.target.is_empty(), "a thought's step is its word alone");
    assert!(thought.body.as_ref().unwrap().text().contains("read"), "an open thought shows its text");
    let hook = open.iter().find(|r| r.id == Id::Tool(id(7))).unwrap();
    assert!(hook.h > CHAT_TOOL_H && hook.body.as_ref().unwrap().text().contains("Run the tests first."));
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
        item(ItemKind::Prompt { text: "/model".into(), pictures: vec![], synthetic: false, queued: false }),
        local("/model", "", "Set model to Opus 5.5", ToolState::Ok),
        item(ItemKind::Prompt { text: "/context".into(), pictures: vec![], synthetic: false, queued: false }),
        local("/context", "", "Context Usage\n⛁ ⛁ ⛀ 12k / 200k", ToolState::Ok),
        local("Shell", "! git status", "On branch main", ToolState::Ok),
        local("Shell", "! git lg", "git: 'lg' is not a git command.", ToolState::Failed),
        item(ItemKind::Prompt { text: "next".into(), pictures: vec![], synthetic: false, queued: false }),
        local("Shell", "! true", "", ToolState::Ok),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    let project = |tools: BTreeSet<ItemId>| {
        let pal = &mdshared_ui::theme::DARK;
        let mut rows = Rows {
            list: vec![],
            previous: BTreeMap::new(),
            end: 0.0,
            width: 700.0,
            scale: 1.0,
            pal,
            chat: true,
            offer: None,
            awaiting: None,
            answers: Default::default(),
        };
        rows.conversation(&model, &BTreeSet::new(), &tools);
        rows.list
    };
    let rows = project(BTreeSet::new());
    let row = |i| rows.iter().find(|r| r.id == Id::Tool(id(i))).unwrap();
    let model_row = row(1);
    assert_eq!((model_row.title.as_str(), model_row.target.as_str()), ("/model", "Set model to Opus 5.5"));
    assert!(model_row.body.is_none() && model_row.h == CHAT_TOOL_H, "a one-line reply is a closed note");
    let context = row(3);
    assert_eq!(context.body.as_ref().unwrap().text(), "Context Usage\n⛁ ⛁ ⛀ 12k / 200k", "longer output is shown");
    assert!(context.h > CHAT_TOOL_H && context.target.is_empty() && context.boxed);
    let shell = row(4);
    assert_eq!((shell.title.as_str(), shell.target.as_str()), ("Shell", "! git status"));
    assert_eq!(shell.body.as_ref().unwrap().text(), "On branch main", "shell output sits under its command");
    let failed = row(5);
    assert!(matches!(failed.tone, Tone::Warning) && failed.stamp == "failed");
    assert_eq!(row(7).h, CHAT_TOOL_H, "a shell command that printed nothing is one line");
    assert!(rows.iter().all(|r| r.style != Style::Group), "command results are rows of their own, never a run");
    // The one-line reply still opens; a click folds longer output.
    let open = project([id(1), id(3)].into());
    let folded = open.iter().find(|r| r.id == Id::Tool(id(3))).unwrap();
    assert!(!folded.open && folded.body.is_none() && folded.h == CHAT_TOOL_H);
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
        item(ItemKind::Prompt { text: "go".into(), pictures: vec![], synthetic: false, queued: false }),
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
    let mut rows = Rows {
        list: vec![],
        previous: BTreeMap::new(),
        end: 0.0,
        width: 700.0,
        scale: 1.0,
        pal,
        chat: true,
        offer: None,
        awaiting: None,
        answers: Default::default(),
    };
    rows.conversation(&model, &[id(1)].into(), &[id(1)].into());
    let row = |i| rows.list.iter().find(|r| r.id == Id::Tool(id(i))).unwrap();
    let group = rows.list.iter().find(|r| r.style == Style::Group).expect("four calls are one run");
    assert_eq!(group.stamp, "2 denied · 1 failed", "a denied call did not fail: it never ran");
    assert!(matches!(group.tone, Tone::Warning));
    // The step's cross is its icon; the words stay.
    assert!(matches!(row(1).tone, Tone::Warning) && row(1).stamp == "blocked by auto mode");
    assert_eq!(row(2).stamp, "you said no");
    assert_eq!(row(3).stamp, "failed");
    // Opened, the row says why under what was asked for.
    let body = row(1).body.as_ref().unwrap().text();
    assert!(
        body.starts_with("mkdir probe") && body.ends_with("Auto mode's classifier blocked it: Too risky."),
        "{body}"
    );
}

#[test]
fn chat_keeps_room_for_buttons_under_each_prompt_and_each_turns_last_answer() {
    let demo = crate::demo::Transcript::new().unwrap();
    let mut model = Model::new(super::super::super::model::Source::File(demo.path().into()), demo.cwd().into(), || {});
    let prompt =
        |text: &str, queued| item(ItemKind::Prompt { text: text.into(), pictures: vec![], synthetic: false, queued });
    let answer = |text: &str| item(ItemKind::Assistant { markdown: text.into() });
    model.items = [
        prompt("first", false),
        answer("looking"),
        tool("Read", ToolState::Ok, None),
        answer("done with the first"),
        prompt("also this", true),
        answer("done with that too"),
        prompt("/context", false),
        answer("the newest"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, item)| (id(i as u64), item))
    .collect();
    model.groups = super::super::super::model::group(&model.items);
    let pal = &mdshared_ui::theme::DARK;
    for chat in [true, false] {
        let mut rows = Rows {
            list: vec![],
            previous: BTreeMap::new(),
            end: 0.0,
            width: 700.0,
            scale: 1.0,
            pal,
            chat,
            offer: None,
            awaiting: None,
            answers: if chat { answer_actions(&model.items) } else { Default::default() },
        };
        rows.conversation(&model, &BTreeSet::new(), &BTreeSet::new());
        let actions = |i| rows.list.iter().find(|r| r.id == Id::Message(id(i))).unwrap().actions;
        if !chat {
            // The Session view is as it was.
            assert!(rows.list.iter().all(|r| r.actions.is_none()));
            continue;
        }
        // Rewind only where the CLI's rewind list has the prompt: not a slash command. A message
        // sent while a turn ran is listed there too.
        assert_eq!(actions(0), Some(Actions { rewind: true, newest: false }));
        assert_eq!(actions(4), Some(Actions { rewind: true, newest: false }));
        assert_eq!(actions(6), Some(Actions { rewind: false, newest: false }));
        // An answer that goes on into tool calls has none; each turn's last has them.
        assert_eq!(actions(1), None);
        assert_eq!(actions(3), Some(Actions { rewind: false, newest: false }));
        assert_eq!(actions(5), Some(Actions { rewind: false, newest: false }));
        assert_eq!(actions(7), Some(Actions { rewind: false, newest: true }));
        // Their room is kept under the message, shown or not.
        let row = |i| rows.list.iter().find(|r| r.id == Id::Message(id(i))).unwrap();
        let next = |i: u64| rows.list.iter().find(|r| r.id == Id::Message(id(i + 1))).unwrap();
        assert!(next(0).top - (row(0).top + row(0).h) >= ACTIONS_ROOM);
        assert!(next(4).top - (row(4).top + row(4).h) >= ACTIONS_ROOM);
    }
}
