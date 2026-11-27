use std::path::Path;

use claude_switch_core::transcript::TaskStatus;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap, mix};
use mdshared_ui::glyph;
use mdshared_ui::theme::Palette;

use super::{Details, Hit, Key, Model, Part, Side, plan_update, task_key, task_status};
use crate::state::Target;
use crate::view::chat::Control;
use crate::view::look;

pub(super) const ROW: f32 = 26.0;
const INDENT: f32 = 22.0;
const BODY_SIZE: f32 = 11.5;
/// A section's head, and a row of a section's own list (a task, a changed file).
const SECTION: f32 = 30.0;
const ITEM: f32 = 28.0;

pub(in crate::view) struct Frame<'a> {
    pub tab: u64,
    pub area: Rect,
    /// The page shown (the Session view's tab), or the section last asked for (Chat).
    pub side: Side,
    /// Chat's panel: every section that has something, in one scrolling list. Otherwise the
    /// one page `side` names, under its heading (the Session view).
    pub sections: bool,
    pub model: &'a Model,
    pub cwd: &'a Path,
    pub hover: Option<Target>,
    /// The colours of added and removed lines.
    pub diff: look::Diff,
}

impl<'a> Frame<'a> {
    /// One page under its heading, as the Session view shows it.
    #[cfg(test)]
    pub(super) fn page(
        tab: u64,
        area: Rect,
        side: Side,
        model: &'a Model,
        cwd: &'a Path,
        hover: Option<Target>,
    ) -> Self {
        let diff = look::Diff::fallback(&mdshared_ui::theme::DARK);
        Self { tab, area, side, sections: false, model, cwd, hover, diff }
    }
}

pub(super) enum Line {
    /// A row: its arrow expands it; with `split`, its name is a separate target that opens the
    /// item's work when `opens` (otherwise the name is muted), and without, the whole row expands.
    Header {
        label: String,
        index: usize,
        expanded: bool,
        active: bool,
        split: bool,
        opens: bool,
    },
    Detail(String),
    Button {
        label: String,
        index: usize,
        part: Part,
    },
    Heading(&'static str),
    ShellMeta(String),
    Tail(claude_switch_core::transcript::ItemId, Vec<String>),
    Actions(usize, Vec<(&'static str, Part)>),
    Output(usize, String, String),
    /// A section's head: its name, a note at the right (a count), and whether it is folded.
    Section {
        title: &'static str,
        note: String,
        folded: bool,
    },
    /// The task list's progress: done of all.
    Bar(usize, usize),
    /// A task with its mark: done, current or waiting. The row expands to the whole text.
    Task {
        label: String,
        status: TaskStatus,
        index: usize,
        expanded: bool,
    },
    /// A changed file: its name opens it when `opens`, the arrow lists its edits.
    File {
        name: String,
        added: usize,
        removed: usize,
        index: usize,
        expanded: bool,
        opens: bool,
    },
    /// A hairline between sections.
    Rule,
}

pub(super) struct Lines {
    /// Each line's top, height, the page its targets belong to, and what it is.
    lines: Vec<(f32, f32, Side, Line)>,
    pub(super) height: f32,
    reveal: Option<(f32, f32)>,
    /// The page the lines being added belong to.
    side: Side,
}

impl Lines {
    pub(super) fn push(&mut self, h: f32, line: Line) {
        self.lines.push((self.height, h, self.side, line));
        self.height += h;
    }

    pub(super) fn detail(&mut self, p: &mut Painter, width: f32, text: String) {
        let h = p.paragraph_height_wrapped(Face::Ui, BODY_SIZE, width, &text, Wrap::Anywhere);
        self.push(h + 10.0, Line::Detail(text));
    }
}

impl Details {
    pub(in crate::view) fn draw(&mut self, p: &mut Painter, pal: &Palette, f: Frame<'_>) -> Vec<(Rect, Target)> {
        if f.sections {
            self.refresh_all(f.model, f.cwd);
        } else {
            self.refresh(f.model, f.cwd, f.side);
        }
        self.visible_shells.clear();
        let area = f.area;
        // A page has its heading over the list; Chat's sections name themselves.
        let top = if f.sections { 8.0 } else { 64.0 };
        if !f.sections {
            let heading = match f.side {
                Side::Tasks => "Tasks",
                Side::Agents => "Agents",
                Side::Files => "Changed files · from tool results",
                Side::Shells if f.model.mcp_tasks.is_empty() => "Background shells",
                Side::Shells => "Background shells and MCP tasks",
                Side::Outputs => "Session outputs",
            };
            p.text(Face::Semibold, 12.0, area.x + 12.0, area.y + 53.0, heading, pal.muted, area.w - 24.0);
        }
        let body = Rect::new(
            area.x + 8.0,
            area.y + top,
            (area.w - 18.0).max(0.0),
            (area.h - top - if self.message.is_some() { 37.0 } else { 6.0 }).max(0.0),
        );
        if let Some(message) = self.message {
            p.text(Face::Ui, BODY_SIZE, body.x + 4.0, body.bottom() + 15.0, message, pal.warn, body.w - 8.0);
        }
        if body.w <= INDENT || body.h <= 0.0 {
            return Vec::new();
        }
        let width = (body.w - INDENT - 9.0).max(1.0);
        let rows = if f.sections { self.sections(p, &f, width) } else { self.lines(p, &f, width) };
        self.max = (rows.height - body.h).max(0.0);
        if let Some((start, end)) = rows.reveal {
            self.offset = self.offset.max(end - body.h).min(start);
        }
        self.reveal = None;
        self.offset = self.offset.clamp(0.0, self.max);
        let c = p.clip(body);
        let mut hits = Vec::new();
        if rows.lines.is_empty() {
            p.text(Face::Ui, 12.0, body.x + 4.0, body.y + 15.0, "Nothing to show yet", pal.muted, body.w - 8.0);
        }
        let ink = f.diff.on(pal.pane);
        let first = rows.lines.partition_point(|(y, h, _, _)| *y + *h <= self.offset);
        for (y, h, side, line) in &rows.lines[first..] {
            let r = Rect::new(body.x, body.y + y - self.offset, body.w, *h);
            if r.y >= body.bottom() {
                break;
            }
            let mut button = |r: Rect, index, part| {
                let t = Target::Chat(Control::Detail(Hit {
                    tab: f.tab,
                    revision: f.model.revision,
                    side: *side,
                    index,
                    part,
                }));
                let y = r.y.max(body.y);
                let h = r.bottom().min(body.bottom()) - y;
                if h > 0.0 {
                    hits.push((Rect::new(r.x, y, r.w, h), t));
                }
                f.hover == Some(t)
            };
            match line {
                Line::Header { label, index, expanded, active, split, opens } => {
                    let arrow = Rect::new(r.x, r.y + 1.0, INDENT, r.h - 2.0);
                    let main = Rect::new(arrow.right(), arrow.y, r.w - INDENT, arrow.h);
                    let toggle = if *split { arrow } else { Rect::new(r.x, arrow.y, r.w, arrow.h) };
                    let toggle_hot = button(toggle, *index, Part::Expand);
                    let open_hot = *opens && button(main, *index, Part::Open);
                    if toggle_hot || *expanded {
                        p.rbox(toggle, 5.0, if toggle_hot { pal.hover } else { pal.on });
                    }
                    if open_hot {
                        p.rbox(main, 5.0, pal.hover);
                    }
                    p.text_center(
                        Face::Semibold,
                        12.0,
                        arrow,
                        if *expanded { "▾" } else { "▸" },
                        if *expanded { pal.accent } else { pal.muted },
                    );
                    let color = if *active || open_hot {
                        pal.accent
                    } else if *split && !*opens {
                        pal.muted
                    } else {
                        pal.text
                    };
                    p.text(Face::Ui, 12.0, main.x + 4.0, main.cy(), label, color, main.w - 8.0);
                }
                Line::Detail(text) => {
                    p.paragraph_wrapped(
                        Face::Ui,
                        BODY_SIZE,
                        r.x + INDENT + 4.0,
                        r.y + 4.0,
                        width,
                        text,
                        pal.muted,
                        Wrap::Anywhere,
                    );
                }
                Line::Button { label, index, part } => {
                    let r = Rect::new(r.x + INDENT, r.y + 1.0, r.w - INDENT, r.h - 2.0);
                    let hot = button(r, *index, *part);
                    if hot {
                        p.rbox(r, 5.0, pal.hover);
                    }
                    p.text(
                        Face::Ui,
                        BODY_SIZE,
                        r.x + 5.0,
                        r.cy(),
                        label,
                        if hot { pal.accent } else { pal.text },
                        (r.w - 18.0).max(0.0),
                    );
                    p.text(Face::Ui, 14.0, r.right() - 12.0, r.cy(), "›", pal.muted, 9.0);
                }
                Line::Heading(label) => {
                    p.text(Face::Semibold, 11.5, r.x + 4.0, r.cy(), label, pal.muted, r.w - 8.0);
                }
                Line::ShellMeta(label) => {
                    p.text(Face::Ui, 10.5, r.x + INDENT + 4.0, r.cy(), label, pal.muted, r.w - INDENT - 8.0);
                }
                Line::Tail(tool, lines) => {
                    self.visible_shells.push(*tool);
                    let box_ = Rect::new(r.x + 5.0, r.y + 2.0, r.w - 10.0, r.h - 4.0);
                    p.rbox(box_, 5.0, pal.field);
                    for (i, line) in lines.iter().enumerate() {
                        p.text(
                            Face::Mono,
                            10.5,
                            box_.x + 6.0,
                            box_.y + 11.0 + i as f32 * 15.0,
                            line,
                            pal.text,
                            box_.w - 12.0,
                        );
                    }
                }
                Line::Actions(index, actions) => {
                    let mut x = r.x + 4.0;
                    for (label, part) in actions {
                        let w = p.measure(Face::Semibold, 10.5, label) + 12.0;
                        let b = Rect::new(x, r.y + 2.0, w.min((r.right() - x).max(0.0)), r.h - 4.0);
                        if b.w > 5.0 {
                            let hot = button(b, *index, *part);
                            p.rbox(b, 4.0, if hot { pal.hover } else { pal.bar });
                            p.text_center(Face::Semibold, 10.5, b, label, pal.text);
                        }
                        x += w + 4.0;
                    }
                }
                Line::Output(index, title, meta) => {
                    // In Chat's panel an output lines up with the sections' rows.
                    let inset = if f.sections { 8.0 } else { 2.0 };
                    let r = Rect::new(r.x + inset, r.y + 2.0, (r.w - 2.0 * inset).max(0.0), r.h - 4.0);
                    let hot = button(r, *index, Part::Open);
                    p.frame(r, 5.0, if hot { pal.hover } else { pal.bar }, pal.line);
                    let icon = f.model.output_list.get(*index).map(|o| o.embed().icon());
                    if let Some(icon) = icon {
                        p.icon(icon, 14.0, r.x + 14.0, r.cy(), pal.muted);
                    }
                    let tx = r.x + if icon.is_some() { 28.0 } else { 7.0 };
                    p.text(Face::Semibold, 11.5, tx, r.y + 10.0, title, pal.text, r.right() - tx - 7.0);
                    p.text(Face::Ui, 10.5, tx, r.y + 25.0, meta, pal.muted, r.right() - tx - 7.0);
                }
                Line::Section { title, note, folded } => {
                    let row = Rect::new(r.x, r.y + 2.0, r.w, r.h - 4.0);
                    let hot = button(row, 0, Part::Fold);
                    if hot {
                        p.rbox(row, 5.0, pal.hover);
                    }
                    let chevron = if *folded { glyph::RIGHT } else { glyph::DOWN };
                    p.icon(chevron, 10.0, row.right() - 11.0, row.cy() + 0.5, pal.faint);
                    let note_w = p.text_right(Face::Ui, 12.0, row.right() - 24.0, row.cy(), note, pal.muted);
                    let room = (row.w - 40.0 - note_w).max(0.0);
                    p.text(Face::Semibold, 12.5, row.x + 8.0, row.cy(), title, pal.text, room);
                }
                Line::Bar(done, all) => {
                    let track = Rect::new(r.x + 8.0, r.y + 2.0, (r.w - 16.0).max(0.0), 4.0);
                    p.rbox(track, 2.0, pal.line);
                    if *done > 0 && *all > 0 {
                        let w = (track.w * *done as f32 / *all as f32).round().max(4.0);
                        p.rbox(Rect::new(track.x, track.y, w, track.h), 2.0, pal.accent);
                    }
                }
                Line::Task { label, status, index, expanded } => {
                    let row = Rect::new(r.x, r.y + 1.0, r.w, r.h - 2.0);
                    let hot = button(row, *index, Part::Expand);
                    if hot || *expanded {
                        p.rbox(row, 5.0, if hot { pal.hover } else { pal.on });
                    }
                    let ground = match (hot, *expanded) {
                        (true, _) => pal.hover,
                        (false, true) => pal.on,
                        _ => pal.pane,
                    };
                    let dot = Rect::new(row.x + 8.0, row.cy() - 8.0, 16.0, 16.0);
                    match status {
                        TaskStatus::Completed => {
                            p.rbox(dot, 8.0, mix(ground, pal.accent, 0.28));
                            p.icon(glyph::CHECK, 10.0, dot.cx(), dot.cy(), pal.accent);
                        }
                        TaskStatus::InProgress => {
                            p.rbox(dot, 8.0, pal.accent);
                            p.rbox(dot.inset(1.5), 6.5, ground);
                            p.rbox(dot.inset(5.0), 3.0, pal.accent);
                        }
                        TaskStatus::Pending => {
                            p.rbox(dot, 8.0, pal.faint);
                            p.rbox(dot.inset(1.5), 6.5, ground);
                        }
                    }
                    let (face, ink) = match status {
                        TaskStatus::Completed => (Face::Ui, pal.muted),
                        TaskStatus::InProgress => (Face::Semibold, pal.text),
                        TaskStatus::Pending => (Face::Ui, pal.text),
                    };
                    p.text(face, 12.5, row.x + 34.0, row.cy(), label, ink, (row.w - 42.0).max(0.0));
                }
                Line::File { name, added, removed, index, expanded, opens } => {
                    let row = Rect::new(r.x, r.y + 1.0, r.w, r.h - 2.0);
                    let arrow = Rect::new(row.right() - 24.0, row.y, 24.0, row.h);
                    let main = Rect::new(row.x, row.y, row.w - 24.0, row.h);
                    let arrow_hot = button(arrow, *index, Part::Expand);
                    // Without a file to open, the whole row lists its edits.
                    let main_hot =
                        if *opens { button(main, *index, Part::Open) } else { button(main, *index, Part::Expand) };
                    if main_hot {
                        p.rbox(main, 5.0, pal.hover);
                    }
                    if arrow_hot || *expanded {
                        p.rbox(arrow, 5.0, if arrow_hot { pal.hover } else { pal.on });
                    }
                    p.icon(glyph::DOC, 14.0, row.x + 16.0, row.cy(), pal.muted);
                    let chevron = if *expanded { glyph::DOWN } else { glyph::RIGHT };
                    p.icon(chevron, 10.0, arrow.cx(), arrow.cy() + 0.5, if *expanded { pal.accent } else { pal.faint });
                    let end = arrow.x - 6.0;
                    let minus = p.text_right(Face::Semibold, 11.5, end, row.cy(), &format!("−{removed}"), ink.del);
                    let plus =
                        p.text_right(Face::Semibold, 11.5, end - minus - 6.0, row.cy(), &format!("+{added}"), ink.add);
                    let room = (end - minus - plus - 16.0 - (row.x + 34.0)).max(0.0);
                    p.text(
                        Face::Ui,
                        12.5,
                        row.x + 34.0,
                        row.cy(),
                        name,
                        if main_hot { pal.accent } else { pal.text },
                        room,
                    );
                }
                Line::Rule => {
                    p.fill(Rect::new(r.x + 8.0, r.cy(), (r.w - 16.0).max(0.0), p.px()), pal.line);
                }
            }
        }
        p.restore(c);
        if self.max > 0.0 {
            let h = (body.h * (body.h / rows.height)).clamp(body.h.min(12.0), body.h);
            let top = body.y + (body.h - h) * (self.offset / self.max);
            p.rbox(Rect::new(body.right() + 3.0, top, 3.0, h), 1.5, pal.thumb);
        }
        hits
    }

    /// Chat's panel: Progress, Background, Changed files and Outputs, each only when it has
    /// something, each with a head that folds it.
    fn sections(&self, p: &mut Painter, f: &Frame<'_>, width: f32) -> Lines {
        let mut out = Lines { lines: Vec::new(), height: 0.0, reveal: None, side: Side::Tasks };
        let m = f.model;
        let mut started = false;
        let mut section = |out: &mut Lines, side: Side, title: &'static str, note: String| {
            if std::mem::replace(&mut started, true) {
                out.push(13.0, Line::Rule);
            }
            out.side = side;
            let folded = self.folded.contains(&side);
            out.push(SECTION, Line::Section { title, note, folded });
            !folded
        };
        if !m.tasks.is_empty() {
            let done = m.tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
            if section(&mut out, Side::Tasks, "Progress", format!("{done} of {}", m.tasks.len())) {
                out.push(14.0, Line::Bar(done, m.tasks.len()));
                self.side_lines(&mut out, p, f, width, Side::Tasks);
            }
        }
        let running = m.agents.iter().filter(|a| a.finished == Some(false)).count()
            + m.running_shells()
            + m.mcp_tasks.iter().filter(|t| t.status == claude_switch_core::transcript::McpStatus::Running).count();
        let background = m.agents.len() + m.shells.len() + m.mcp_tasks.len();
        if background > 0 {
            let note = if running > 0 { format!("{running} running") } else { background.to_string() };
            // The section folds under the shells' page; its agents keep their own targets.
            if section(&mut out, Side::Shells, "Background", note) {
                self.side_lines(&mut out, p, f, width, Side::Agents);
                self.side_lines(&mut out, p, f, width, Side::Shells);
            }
        }
        if !self.files.is_empty() && section(&mut out, Side::Files, "Changed files", self.files.len().to_string()) {
            self.side_lines(&mut out, p, f, width, Side::Files);
        }
        if !m.output_list.is_empty() && section(&mut out, Side::Outputs, "Outputs", m.output_list.len().to_string()) {
            self.side_lines(&mut out, p, f, width, Side::Outputs);
        }
        out
    }

    /// One page, as the Session view shows it under its tab.
    fn lines(&self, p: &mut Painter, f: &Frame<'_>, width: f32) -> Lines {
        let mut out = Lines { lines: Vec::new(), height: 0.0, reveal: None, side: f.side };
        self.side_lines(&mut out, p, f, width, f.side);
        out
    }

    /// The rows of one page, added to `out` with that page's targets.
    fn side_lines(&self, out: &mut Lines, p: &mut Painter, f: &Frame<'_>, width: f32, side: Side) {
        out.side = side;
        match side {
            Side::Tasks => {
                let opens = plan_update(f.model).is_some();
                for (index, task) in f.model.tasks.iter().enumerate() {
                    let key = task_key(index, &task.content);
                    let expanded = self.expanded.contains(&key);
                    let (mark, status) = task_status(task.status);
                    if f.sections {
                        let label = first_line(&task.content).to_owned();
                        out.push(ITEM, Line::Task { label, status: task.status, index, expanded });
                    } else {
                        let label = format!("{mark}  {}", first_line(&task.content));
                        out.push(
                            ROW,
                            Line::Header {
                                label,
                                index,
                                expanded,
                                active: task.status == TaskStatus::InProgress,
                                split: opens,
                                opens,
                            },
                        );
                    }
                    if expanded {
                        let top = out.height;
                        out.detail(p, width, format!("{status}\n{}", task.content));
                        if opens {
                            out.push(
                                ROW,
                                Line::Button { label: "Show the latest plan update".into(), index, part: Part::Open },
                            );
                        }
                        self.reveal(out, &key, top);
                    }
                }
            }
            Side::Agents => {
                // In Chat's Background section the running agents come first.
                let mut order: Vec<usize> = (0..f.model.agents.len()).collect();
                if f.sections {
                    order.sort_by_key(|&i| f.model.agents[i].finished != Some(false));
                }
                for index in order {
                    let agent = &f.model.agents[index];
                    let key = Key::Agent(agent.id.clone());
                    let expanded = self.expanded.contains(&key);
                    let (mark, status) = match agent.finished {
                        Some(true) => ("✓", "Finished"),
                        Some(false) => ("◉", "Running"),
                        None => ("?", "Unknown"),
                    };
                    let desc = agent.description.as_deref().unwrap_or(&agent.id);
                    let label = format!("{mark}  {}", first_line(desc));
                    let opens = f.model.agent.as_deref() != Some(agent.id.as_str());
                    out.push(
                        ROW,
                        Line::Header {
                            label,
                            index,
                            expanded,
                            active: agent.finished == Some(false),
                            split: opens,
                            opens,
                        },
                    );
                    if expanded {
                        let top = out.height;
                        let kind = [agent.agent_type.as_deref(), agent.model.as_deref()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let mut text = format!(
                            "{desc}\n{status} · {} tool {}",
                            agent.tool_calls,
                            if agent.tool_calls == 1 { "call" } else { "calls" }
                        );
                        if !kind.is_empty() {
                            text.push_str(&format!("\n{kind}"));
                        }
                        if let Some(last) = &agent.last_tool_summary {
                            text.push_str(&format!("\nLast tool: {last}"));
                        }
                        out.detail(p, width, text);
                        if f.model.agent.as_deref() != Some(agent.id.as_str()) {
                            out.push(
                                ROW,
                                Line::Button {
                                    label: "Open agent transcript".into(),
                                    index,
                                    part: Part::AgentTranscript,
                                },
                            );
                        }
                        self.reveal(out, &key, top);
                    }
                }
            }
            Side::Files => {
                for (index, file) in self.files.iter().enumerate() {
                    let key = Key::File(file.key.clone());
                    let expanded = self.expanded.contains(&key);
                    let name = Path::new(&file.path).file_name().and_then(|s| s.to_str()).unwrap_or(&file.path);
                    let opens = file.file.as_ref().is_some_and(|f| f.supports_default_app());
                    if f.sections {
                        out.push(
                            ITEM,
                            Line::File {
                                name: first_line(name).to_owned(),
                                added: file.added,
                                removed: file.removed,
                                index,
                                expanded,
                                opens,
                            },
                        );
                    } else {
                        out.push(
                            ROW,
                            Line::Header {
                                label: first_line(name).to_owned(),
                                index,
                                expanded,
                                active: false,
                                split: true,
                                opens,
                            },
                        );
                    }
                    if expanded {
                        let top = out.height;
                        let suffix = if file.file.is_none() {
                            "\nNot available as a local file"
                        } else if !opens {
                            "\nThis file type cannot be opened from Switch."
                        } else {
                            ""
                        };
                        out.detail(p, width, format!("{}{suffix}", file.path));
                        for (j, edit) in file.edits.iter().enumerate() {
                            out.push(ROW, Line::Button { label: edit.label.clone(), index, part: Part::Edit(j) });
                        }
                        self.reveal(out, &key, top);
                    }
                }
            }
            Side::Shells | Side::Outputs => self.extra_lines(out, p, f, width, side),
        }
    }

    pub(super) fn reveal(&self, out: &mut Lines, key: &Key, top: f32) {
        if self.reveal.as_ref() == Some(key) {
            out.reveal = Some((top - ROW, out.height.min(top + ROW * 2.0)));
        }
    }
}

fn first_line(text: &str) -> &str {
    text.trim().split(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']).next().unwrap_or("")
}
