use super::*;
use crate::state::{AppState, Target};
use crate::view::chat::Control;
use crate::view::model::Source;
use claude_switch_core::transcript::{Item, Subagent, Task};
use mdshared_text::bitmap::Bitmap;
use mdshared_ui::gfx::{Gfx, Painter, Rect};
use mdshared_ui::theme::DARK;

const TAB: u64 = 7;

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
    assert_eq!(closed.len(), 1);
    let agent = hit(&model, Side::Agents, 0, Part::Expand);
    let Some(Action::Expand(key)) = side.action(&model, TAB, Side::Agents, agent) else { panic!() };
    side.toggle(key);
    let expanded = draw(&mut side, &model, Side::Agents);
    let transcript = hit(&model, Side::Agents, 0, Part::AgentTranscript);
    assert_eq!(expanded[1].1, Target::Chat(Control::Detail(transcript)));
    assert_eq!(side.action(&model, TAB, Side::Agents, transcript), Some(Action::AgentTranscript("scout".into())));
    model.revision += 1;
    model.agents[0].id = "replacement".into();
    assert_eq!(side.action(&model, TAB, Side::Agents, transcript), None);
    assert_eq!(draw(&mut side, &model, Side::Agents).len(), 1);
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
    assert!(!activate(&mut pane, TAB, expand));
    assert!(pane.chat.details.expanded.iter().any(|k| matches!(k, Key::File(_))));
    std::fs::remove_file(demo.cwd().join(md)).unwrap();
    assert!(!activate(&mut pane, TAB, open), "stale metadata must not give the viewer focus");
    assert_eq!(pane.chat.details.message, Some("The file is no longer available."));
}
