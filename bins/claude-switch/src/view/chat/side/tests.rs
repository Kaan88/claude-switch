use super::*;
use crate::state::{AppState, Target};
use crate::view::chat::Control;
use crate::view::model::Source;
use claude_switch_core::transcript::{Item, McpStatus, McpTask, Subagent, Task};
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
        details.draw(&mut p, &DARK, Frame { tab: TAB, area, side: page, model, cwd: demo.cwd(), hover: None })
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
    let frame = Frame {
        tab: TAB,
        area: pane.chat.layout.side,
        side: Side::Files,
        model: &pane.model,
        cwd: demo.cwd(),
        hover: Some(Target::Chat(Control::Detail(open))),
    };
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
        let frame = Frame {
            tab: TAB,
            area: Rect::new(0.0, 0.0, 280.0, 500.0),
            side: Side::Shells,
            model,
            cwd: demo.cwd(),
            hover: None,
        };
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
