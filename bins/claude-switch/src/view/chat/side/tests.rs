use super::*;
use crate::state::{AppState, Target};
use crate::view::chat::Control;
use crate::view::model::Source;
use claude_switch_core::transcript::{AgentEnd, EndSource, Item, McpStatus, McpTask, Subagent, Task};
use mdshared_text::bitmap::Bitmap;
use mdshared_ui::gfx::{Gfx, Painter, Rect};
use mdshared_ui::theme::DARK;

const TAB: u64 = 7;

#[test]
fn shell_actions_are_bound_to_the_expanded_row_and_refuse_stale_or_finished_stop_hits() {
    use claude_switch_core::transcript::{BackgroundShell, ShellStatus};
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let mut model = Model::new(Source::Demo(demo.path().into()), demo.cwd().into(), || {});
    let id = ItemId { offset: 2, block: 0 };
    model.shells.push(BackgroundShell {
        tool: id,
        id: "b1".into(),
        command: "preview".into(),
        output: demo
            .cwd()
            .join("claude")
            .join(crate::view::model::project_slug(demo.cwd()))
            .join("tasks/b1.output")
            .to_str()
            .unwrap()
            .into(),
        status: ShellStatus::Running,
        start_ms: None,
        end_ms: None,
    });
    let mut details = Details::default();
    details.refresh(&model, demo.cwd(), Side::Shells);
    let stop = hit(&model, Side::Shells, 0, Part::Stop);
    assert_eq!(details.action(&model, TAB, Side::Shells, stop), None);
    details.select_shell(id, model.generation);
    assert_eq!(details.action(&model, TAB, Side::Shells, stop), Some(Action::ShellStop(id)));
    model.revision += 1;
    assert_eq!(details.action(&model, TAB, Side::Shells, stop), None);
    model.shells[0].status = ShellStatus::Finished(Some(4));
    assert_eq!(details.action(&model, TAB, Side::Shells, hit(&model, Side::Shells, 0, Part::Stop)), None);
    assert_eq!(
        details.action(&model, TAB, Side::Shells, hit(&model, Side::Shells, 0, Part::Full)),
        Some(Action::ShellFull(id))
    );
}

fn put_tool(model: &mut Model, cwd: &Path, offset: u64, name: &str, path: &str, state: ToolState) -> ItemId {
    let id = ItemId { offset, block: 0 };
    let kind = ItemKind::Tool {
        id: format!("tool-{offset}"),
        name: name.into(),
        summary: path.into(),
        file_path: Some(path.into()),
        state,
        duration_ms: None,
        pictures: vec![],
        subagent: None,
        denial: None,
        card: None,
    };
    model.items.insert(id, Item { kind, timestamp: Some("2026-11-26T18:42:01Z".into()) });
    if let Some(file) = links::resolve(path, cwd, &BTreeSet::new()) {
        model.files.insert(id, [(path.to_owned(), file)].into());
    }
    id
}

fn hit(model: &Model, side: Side, index: usize, part: Part) -> Hit {
    Hit { tab: TAB, revision: model.revision, side, index, part }
}

#[test]
fn file_history_groups_named_successful_writes_and_edits_in_reverse_order() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let cwd = demo.cwd();
    let mut m = Model::new(Source::File(demo.path().into()), cwd.into(), || {});
    let md = "field-notes.md";
    let absolute = cwd.join(md);
    let name = "ridge-caption.txt";
    let old = put_tool(&mut m, cwd, 1, "Write", md, ToolState::Ok);
    let recent = put_tool(&mut m, cwd, 2, "Edit", absolute.to_str().unwrap(), ToolState::Ok);
    put_tool(&mut m, cwd, 3, "Read", md, ToolState::Ok);
    put_tool(&mut m, cwd, 4, "Edit", md, ToolState::Failed);
    put_tool(&mut m, cwd, 5, "Edit", md, ToolState::Running);
    put_tool(&mut m, cwd, 6, "Write", name, ToolState::Ok);
    let missing = put_tool(&mut m, cwd, 7, "Write", "absent.md", ToolState::Ok);
    std::fs::write(cwd.join("unsupported.exe"), b"not an executable").unwrap();
    put_tool(&mut m, cwd, 8, "Write", "unsupported.exe", ToolState::Ok);

    let mut ui = Details::default();
    ui.refresh(&m, cwd, Side::Files);
    assert_eq!(ui.files.len(), 4);
    assert_eq!(ui.files[3].edits.iter().map(|e| e.id).collect::<Vec<_>>(), [recent, old]);
    assert_eq!(ui.files[1].edits[0].id, missing);
    let action = |ui: &Details, i, part| ui.action(&m, TAB, Side::Files, hit(&m, Side::Files, i, part));
    assert_eq!(action(&ui, 0, Part::Open), None, "unsafe association is not actionable");
    assert_eq!(action(&ui, 1, Part::Open), None, "missing file retains its history, not an open action");
    assert_eq!(
        action(&ui, 2, Part::Open),
        Some(Action::DefaultApp(links::resolve(name, cwd, &BTreeSet::new()).unwrap()))
    );
    assert_eq!(action(&ui, 3, Part::Open), Some(Action::Preview(links::resolve(md, cwd, &BTreeSet::new()).unwrap())));
    assert_eq!(action(&ui, 3, Part::Edit(1)), None);
    let Some(Action::Expand(key)) = action(&ui, 3, Part::Expand) else { panic!("expected a disclosure action") };
    ui.toggle(key.clone());
    assert_eq!(action(&ui, 3, Part::Edit(0)), Some(Action::Jump(recent)));
    assert_eq!(action(&ui, 3, Part::Edit(1)), Some(Action::Jump(old)));
    ui.toggle(key);
    assert_eq!(action(&ui, 3, Part::Edit(1)), None);
}

#[test]
fn task_and_agent_hits_expand_detail_and_refuse_reassigned_rows() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let mut model = Model::new(Source::File(demo.path().into()), demo.cwd().into(), || {});
    model.tasks = vec![
        Task {
            content: "A task with a short title\nThe second line only appears inside the expanded row.".into(),
            status: TaskStatus::InProgress,
        },
        Task { content: "Next task".into(), status: TaskStatus::Pending },
    ];
    model.agents = vec![Subagent { id: "scout".into(), model: Some("demo".into()), ..Default::default() }];
    let area = Rect::new(0.0, 0.0, 260.0, 400.0);
    let mut bmp = Bitmap::new(260, 400);
    let mut gfx = Gfx::new();
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    let mut side = Details::default();
    let mut draw = |details: &mut Details, model: &Model, page| {
        details.draw(&mut p, &DARK, Frame::page(TAB, area, page, model, demo.cwd(), None))
    };
    let closed = draw(&mut side, &model, Side::Tasks);
    let task = hit(&model, Side::Tasks, 0, Part::Expand);
    let Some(Action::Expand(key)) = side.action(&model, TAB, Side::Tasks, task) else { panic!() };
    side.toggle(key);
    let expanded = draw(&mut side, &model, Side::Tasks);
    assert!(expanded[1].0.y > closed[1].0.y + 25.0);
    assert_eq!(side.action(&model, TAB + 1, Side::Tasks, task), None);
    assert_eq!(side.action(&model, TAB, Side::Agents, task), None);

    let closed = draw(&mut side, &model, Side::Agents);
    assert_eq!(closed.len(), 2, "the arrow expands and the name opens the agent's transcript");
    assert_eq!(
        side.action(&model, TAB, Side::Agents, hit(&model, Side::Agents, 0, Part::Open)),
        Some(Action::AgentTranscript("scout".into()))
    );
    let agent = hit(&model, Side::Agents, 0, Part::Expand);
    let Some(Action::Expand(key)) = side.action(&model, TAB, Side::Agents, agent) else { panic!() };
    side.toggle(key);
    let expanded = draw(&mut side, &model, Side::Agents);
    let transcript = hit(&model, Side::Agents, 0, Part::AgentTranscript);
    assert_eq!(expanded[2].1, Target::Chat(Control::Detail(transcript)));
    assert_eq!(side.action(&model, TAB, Side::Agents, transcript), Some(Action::AgentTranscript("scout".into())));
    model.revision += 1;
    model.agents[0].id = "replacement".into();
    assert_eq!(side.action(&model, TAB, Side::Agents, transcript), None);
    assert_eq!(draw(&mut side, &model, Side::Agents).len(), 2);
    model.agent = Some("replacement".into());
    assert_eq!(draw(&mut side, &model, Side::Agents).len(), 1, "the agent being shown does not open itself");
}

#[test]
fn painted_file_name_and_arrow_are_disjoint_and_removed_markdown_is_not_opened() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let app = AppState::new(crate::source::Source::Demo, None, Default::default());
    let mut pane = app.new_session_pane(Source::File(demo.path().into()), demo.cwd().into());
    pane.chat.side = Side::Files;
    pane.chat.layout.side = Rect::new(0.0, 0.0, 260.0, 400.0);
    let md = "field-notes.md";
    put_tool(&mut pane.model, demo.cwd(), 1, "Write", md, ToolState::Ok);
    let mut bmp = Bitmap::new(260, 400);
    let mut gfx = Gfx::new();
    let expand = hit(&pane.model, Side::Files, 0, Part::Expand);
    let open = hit(&pane.model, Side::Files, 0, Part::Open);
    let hover = Some(Target::Chat(Control::Detail(open)));
    let frame = Frame::page(TAB, pane.chat.layout.side, Side::Files, &pane.model, demo.cwd(), hover);
    bmp.clear(DARK.pane);
    let hits = pane.chat.details.draw(&mut Painter::new(&mut bmp, &mut gfx, 1.0), &DARK, frame);
    assert_eq!(hits[0].1, Target::Chat(Control::Detail(expand)));
    assert_eq!(hits[1].1, Target::Chat(Control::Detail(open)));
    assert!(!hits[0].0.contains(hits[1].0.cx(), hits[1].0.cy()));
    let r = hits[1].0;
    let pixel = (r.cy() as usize * bmp.width as usize + (r.right() - 10.0) as usize) * 4;
    let [red, green, blue] = DARK.hover;
    assert_eq!(bmp.pixels[pixel..pixel + 4], [blue, green, red, 255], "the name independently paints the hover fill");
    assert!(matches!(activate(&mut pane, TAB, Side::Files, expand), Activated::None));
    assert!(pane.chat.details.expanded.iter().any(|k| matches!(k, Key::File(_))));
    std::fs::remove_file(demo.cwd().join(md)).unwrap();
    assert!(
        matches!(activate(&mut pane, TAB, Side::Files, open), Activated::None),
        "stale metadata must not give the viewer focus"
    );
    assert_eq!(pane.chat.details.message, Some("The file is no longer available."));
}

fn mcp_task(offset: u64, id: &str, status: McpStatus, result: Option<&str>) -> McpTask {
    McpTask {
        tool: ItemId { offset, block: 0 },
        id: id.into(),
        job: format!("job_{offset}"),
        name: "fixture/long_job".into(),
        status,
        start_ms: Some(1_000),
        end_ms: (status != McpStatus::Running).then_some(126_000),
        summary: result.map(|_| format!("MCP task job_{offset} (fixture/long_job) completed.")),
        result: result.map(str::to_owned),
    }
}

#[test]
fn mcp_tasks_list_after_shells_and_open_their_result_or_their_call() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let app = AppState::new(crate::source::Source::Demo, None, Default::default());
    let mut pane = app.new_session_pane(Source::File(demo.path().into()), demo.cwd().into());
    pane.chat.side = Side::Shells;
    pane.chat.layout.side = Rect::new(0.0, 0.0, 280.0, 500.0);
    pane.model.mcp_tasks = vec![
        mcp_task(4, "krunning", McpStatus::Running, None),
        mcp_task(2, "kdone", McpStatus::Completed, Some("# Report\n\nAll **done**.")),
    ];
    let model = &pane.model;
    let details = &mut pane.chat.details;
    let mut bmp = Bitmap::new(280, 500);
    let mut gfx = Gfx::new();
    let mut draw = |details: &mut Details, model: &Model| {
        let frame = Frame::page(TAB, Rect::new(0.0, 0.0, 280.0, 500.0), Side::Shells, model, demo.cwd(), None);
        details.draw(&mut Painter::new(&mut bmp, &mut gfx, 1.0), &DARK, frame)
    };
    let hits = draw(details, model);
    assert_eq!(hits.len(), 4, "each task has an arrow and a name");
    let action =
        |details: &Details, i, part| details.action(model, TAB, Side::Shells, hit(model, Side::Shells, i, part));
    assert_eq!(
        action(details, 0, Part::Open),
        Some(Action::Jump(ItemId { offset: 4, block: 0 })),
        "no result yet: the call"
    );
    assert_eq!(action(details, 1, Part::Open), Some(Action::McpResult("kdone".into())));
    assert_eq!(action(details, 1, Part::Full), None, "row buttons need the row open");
    assert_eq!(action(details, 2, Part::Open), None, "past the list");
    let Some(Action::Expand(key)) = action(details, 1, Part::Expand) else { panic!("expected a disclosure") };
    details.toggle(key);
    let expanded = draw(details, model);
    let full = hit(model, Side::Shells, 1, Part::Full);
    let call = hit(model, Side::Shells, 1, Part::Call);
    assert!(expanded.iter().any(|(_, t)| *t == Target::Chat(Control::Detail(full))));
    assert!(expanded.iter().any(|(_, t)| *t == Target::Chat(Control::Detail(call))));
    assert_eq!(action(details, 1, Part::Call), Some(Action::Jump(ItemId { offset: 2, block: 0 })));

    assert!(matches!(activate(&mut pane, TAB, Side::Shells, full), Activated::Focus));
    assert_eq!(
        pane.ui.viewer(),
        Some(("fixture/long_job · result", "MCP task kdone · completed · from the transcript"))
    );
}

#[test]
fn an_mcp_tasks_running_time_and_missing_last_step_are_said_plainly() {
    let running = mcp_task(1, "krun", McpStatus::Running, None);
    assert_eq!(extra::mcp_status(&running, Some(61_000)), "Running · 1:00");
    assert_eq!(extra::mcp_status(&running, None), "Running");
    let stopped = McpTask { result: None, summary: None, ..mcp_task(1, "kstop", McpStatus::Stopped, None) };
    assert_eq!(extra::mcp_status(&stopped, None), "Stopped · 2:05");
}

#[test]
fn a_task_opens_the_newest_plan_update_and_only_when_there_is_one() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let mut model = Model::new(Source::File(demo.path().into()), demo.cwd().into(), || {});
    model.tasks = vec![Task { content: "Plan".into(), status: TaskStatus::Pending }];
    let mut details = Details::default();
    details.refresh(&model, demo.cwd(), Side::Tasks);
    let open = |model: &Model| details.action(model, TAB, Side::Tasks, hit(model, Side::Tasks, 0, Part::Open));
    assert_eq!(open(&model), None);
    put_tool(&mut model, demo.cwd(), 3, "TodoWrite", "plan", ToolState::Ok);
    let newest = put_tool(&mut model, demo.cwd(), 9, "TodoWrite", "plan", ToolState::Ok);
    put_tool(&mut model, demo.cwd(), 12, "Read", "plan", ToolState::Ok);
    assert_eq!(open(&model), Some(Action::Jump(newest)));
}

/// Whether the panel opens by itself is answered from the lists the model keeps; the transcript is
/// scanned for changed files only while nothing else would open it.
#[test]
fn the_panel_scans_for_changed_files_only_while_nothing_else_opens_it() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let cwd = demo.cwd();
    let mut model = Model::new(Source::File(demo.path().into()), cwd.into(), || {});
    let mut details = Details::default();
    assert!(!details.wants_panel(&model, cwd), "an empty session has nothing to show");
    assert_eq!(details.files_revision, Some(model.revision), "scanned, with nothing else to show");
    model.revision += 1;
    model.tasks.push(claude_switch_core::transcript::Task {
        content: "Fix the order".into(),
        status: claude_switch_core::transcript::TaskStatus::InProgress,
    });
    assert!(details.wants_panel(&model, cwd));
    assert_eq!(details.files_revision, Some(model.revision - 1), "not scanned again for this revision");
}

/// Chat's panel: one list of sections in place of the tabs. A section shows only when it has
/// something, its head folds it, and every target says which page it belongs to, so what a tab's
/// row did is still reachable.
#[test]
fn chats_panel_lists_the_sections_that_have_something_and_each_folds() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let cwd = demo.cwd();
    let mut model = Model::new(Source::File(demo.path().into()), cwd.into(), || {});
    let mut details = Details::default();
    details.refresh_all(&model, cwd);
    assert!(!details.has_sections(&model), "an empty session has nothing to show");
    let area = Rect::new(0.0, 0.0, 280.0, 600.0);
    let mut bmp = Bitmap::new(280, 600);
    let mut gfx = Gfx::new();
    let mut draw = |details: &mut Details, model: &Model| {
        let diff = crate::view::look::Diff::fallback(&DARK);
        let frame = Frame { tab: TAB, area, side: Side::Tasks, sections: true, model, cwd, hover: None, diff };
        let hits = details.draw(&mut Painter::new(&mut bmp, &mut gfx, 1.0), &DARK, frame);
        hits.into_iter()
            .map(|(_, target)| match target {
                Target::Chat(Control::Detail(hit)) => (hit.side, hit.index, hit.part),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>()
    };
    assert!(draw(&mut details, &model).is_empty());

    std::fs::write(cwd.join("notes.md"), "# Notes").unwrap();
    model.tasks = vec![
        Task { content: "Done".into(), status: TaskStatus::Completed },
        Task { content: "Doing".into(), status: TaskStatus::InProgress },
    ];
    model.agents = vec![
        Subagent { id: "done".into(), end: Some((AgentEnd::Finished, EndSource::Sidecar)), ..Default::default() },
        Subagent { id: "busy".into(), ..Default::default() },
    ];
    // Running: the terminal's agent list shows it so.
    model.listed.insert("busy".into(), crate::view::agent_status::Listed::Running);
    put_tool(&mut model, cwd, 2, "Edit", "notes.md", ToolState::Ok);
    model.revision += 1;
    details.refresh_all(&model, cwd);
    assert!(details.has_sections(&model));
    assert_eq!((details.files.len(), details.files[0].added, details.files[0].removed), (1, 0, 0));
    let hits = draw(&mut details, &model);
    let heads: Vec<Side> = hits.iter().filter(|h| h.2 == Part::Fold).map(|h| h.0).collect();
    assert_eq!(heads, [Side::Tasks, Side::Shells, Side::Files], "Progress, Background, Changed files; no Outputs");
    // Each task's row expands it.
    assert_eq!(
        hits.iter().filter(|h| h.0 == Side::Tasks && h.2 == Part::Expand).map(|h| h.1).collect::<Vec<_>>(),
        [0, 1]
    );
    // In Background the running agent comes first; each keeps its own index.
    assert_eq!(
        hits.iter().filter(|h| h.0 == Side::Agents && h.2 == Part::Expand).map(|h| h.1).collect::<Vec<_>>(),
        [1, 0]
    );
    // A changed file's name opens it and its arrow lists its edits.
    assert!(hits.contains(&(Side::Files, 0, Part::Open)) && hits.contains(&(Side::Files, 0, Part::Expand)));
    // The targets act as the tabs' rows did.
    let open = hit(&model, Side::Agents, 1, Part::Open);
    assert_eq!(details.action(&model, TAB, Side::Agents, open), Some(Action::AgentTranscript("busy".into())));
    let fold = hit(&model, Side::Tasks, 0, Part::Fold);
    assert_eq!(details.action(&model, TAB, Side::Tasks, fold), Some(Action::Fold(Side::Tasks)));
    // Folded, a section keeps its head and hides its rows.
    details.folded.insert(Side::Tasks);
    let hits = draw(&mut details, &model);
    assert!(hits.contains(&(Side::Tasks, 0, Part::Fold)));
    assert!(!hits.iter().any(|h| h.0 == Side::Tasks && h.2 == Part::Expand));
    assert!(hits.iter().any(|h| h.0 == Side::Agents));
}

/// Item 49: after a rewind's next prompt, the panel's sections drop what the replaced turn did
/// (its written file, its plan, an ended shell), through the worker as Chat reads it.
#[test]
fn the_panel_drops_what_a_rewound_turn_did_once_the_next_prompt_is_written() {
    use serde_json::json;
    let dir = crate::testing::unique("cs-side-rewind");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    let rec = |mut v: serde_json::Value, uuid: &str, parent: Option<&str>| {
        v["uuid"] = json!(uuid);
        v["parentUuid"] = json!(parent);
        format!("{v}\n")
    };
    let call = |uuid: &str, parent: &str, id: &str, name: &str, input: serde_json::Value| {
        rec(
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}),
            uuid,
            Some(parent),
        )
    };
    let result = |uuid: &str, parent: &str, id: &str, text: &str| {
        rec(
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":text}]}}),
            uuid,
            Some(parent),
        )
    };
    let prompt = |uuid: &str, parent: Option<&str>, text: &str| {
        rec(json!({"type":"user","message":{"content":text}}), uuid, parent)
    };
    let answer = |uuid: &str, parent: &str| {
        rec(
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"done"}],"stop_reason":"end_turn"}}),
            uuid,
            Some(parent),
        )
    };
    let todos = |content: &str| json!({"todos":[{"content":content,"status":"pending","activeForm":content}]});
    let text = prompt("u1", None, "one")
        + &call("t1", "u1", "todo1", "TodoWrite", todos("first plan"))
        + &result("t1r", "t1", "todo1", "ok")
        + &call("w1", "t1r", "write1", "Write", json!({"file_path":"kept.md","content":"x"}))
        + &result("w1r", "w1", "write1", "ok")
        + &answer("a1", "w1r")
        + &prompt("u2", Some("a1"), "two")
        + &call("t2", "u2", "todo2", "TodoWrite", todos("replaced plan"))
        + &result("t2r", "t2", "todo2", "ok")
        + &call("w2", "t2r", "write2", "Write", json!({"file_path":"replaced.md","content":"x"}))
        + &result("w2r", "w2", "write2", "ok")
        + &call("b2", "w2r", "bash2", "Bash", json!({"command":"watch","run_in_background":true}))
        + &result(
            "b2r",
            "b2",
            "bash2",
            "Command running in background with ID: done2. Output is being written to: /tmp/done2.output",
        )
        + &rec(
            json!({"type":"user","isMeta":true,"message":{"content":
            "<task-notification><task-id>done2</task-id><status>completed</status><summary>Command ended (exit code 0)</summary></task-notification>"}}),
            "n2",
            Some("b2r"),
        )
        + &answer("a2", "n2");
    std::fs::write(&path, text).unwrap();
    let mut model = Model::new(Source::File(path.clone()), dir.clone(), || {});
    let settle = |model: &mut Model, done: &dyn Fn(&Model) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(model) && std::time::Instant::now() < deadline {
            model.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
            model.collect();
        }
    };
    let names = |model: &Model| -> Vec<String> { changed_files(model, &dir).into_iter().map(|f| f.path).collect() };
    settle(&mut model, &|m| m.tasks.iter().any(|t| t.content == "replaced plan") && !m.shells.is_empty());
    assert_eq!(names(&model), ["replaced.md", "kept.md"]);
    assert_eq!(model.shells.len(), 1);

    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, prompt("u3", Some("a1"), "two again").as_bytes()).unwrap();
    settle(&mut model, &|m| m.tasks.iter().any(|t| t.content == "first plan"));
    assert_eq!(model.tasks.iter().map(|t| t.content.as_str()).collect::<Vec<_>>(), ["first plan"]);
    assert_eq!(names(&model), ["kept.md"], "the replaced turn's file is not a changed file");
    assert!(model.shells.is_empty(), "its ended shell is gone");
    drop(model);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An agent's row says its state in a word, and its tooltip where that was read or why nothing
/// tells.
#[test]
fn an_agent_row_names_its_state_and_its_tooltip_says_why() {
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let mut model = Model::new(Source::File(demo.path().into()), demo.cwd().into(), || {});
    model.agents = vec![
        Subagent { id: "gone".into(), description: Some("Trace".into()), ..Default::default() },
        Subagent {
            id: "done".into(),
            description: Some("Compare".into()),
            end: Some((AgentEnd::Interrupted, EndSource::Transcript)),
            ..Default::default()
        },
    ];
    model.revision += 1;
    let tip = |model: &Model, index| Details::tip(model, hit(model, Side::Agents, index, Part::Open));
    assert_eq!(
        tip(&model, 0).as_deref(),
        Some(format!("Unknown. {}", crate::view::agent_status::UNKNOWN_WHY).as_str())
    );
    assert_eq!(tip(&model, 1).as_deref(), Some("Interrupted. From the end of its own transcript."));
    assert!(model.set_listed([("gone".into(), crate::view::agent_status::Listed::Running)].into()));
    assert_eq!(tip(&model, 0).as_deref(), Some("Running. The terminal's agent list shows it running."));
    assert!(!model.set_listed([("gone".into(), crate::view::agent_status::Listed::Running)].into()), "the same");
    // Another section's target has no tooltip; a stale one neither.
    assert_eq!(Details::tip(&model, hit(&model, Side::Tasks, 0, Part::Open)), None);
    let stale = hit(&model, Side::Agents, 0, Part::Open);
    model.revision += 1;
    assert_eq!(Details::tip(&model, stale), None);
}
