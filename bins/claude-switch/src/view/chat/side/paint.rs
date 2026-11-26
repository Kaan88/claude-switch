use std::path::Path;

use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::theme::Palette;

use super::{Details, Hit, Key, Model, Part, Side, plan_update, task_key, task_status};
use crate::state::Target;
use crate::view::chat::Control;

pub(super) const ROW: f32 = 26.0;
const INDENT: f32 = 22.0;
const BODY_SIZE: f32 = 11.5;

pub(in crate::view) struct Frame<'a> {
    pub tab: u64,
    pub area: Rect,
    pub side: Side,
    pub model: &'a Model,
    pub cwd: &'a Path,
    pub hover: Option<Target>,
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
}

pub(super) struct Lines {
    lines: Vec<(f32, f32, Line)>,
    pub(super) height: f32,
    reveal: Option<(f32, f32)>,
}

impl Lines {
    pub(super) fn push(&mut self, h: f32, line: Line) {
        self.lines.push((self.height, h, line));
        self.height += h;
    }

    pub(super) fn detail(&mut self, p: &mut Painter, width: f32, text: String) {
        let h = p.paragraph_height_wrapped(Face::Ui, BODY_SIZE, width, &text, Wrap::Anywhere);
        self.push(h + 10.0, Line::Detail(text));
    }
}

impl Details {
    pub(in crate::view) fn draw(&mut self, p: &mut Painter, pal: &Palette, f: Frame<'_>) -> Vec<(Rect, Target)> {
        self.refresh(f.model, f.cwd, f.side);
        self.visible_shells.clear();
        let area = f.area;
        let heading = match f.side {
            Side::Tasks => "Tasks",
            Side::Agents => "Agents",
            Side::Files => "Changed files · from tool results",
            Side::Shells if f.model.mcp_tasks.is_empty() => "Background shells",
            Side::Shells => "Background shells and MCP tasks",
            Side::Outputs => "Session outputs",
        };
        p.text(Face::Semibold, 12.0, area.x + 12.0, area.y + 53.0, heading, pal.muted, area.w - 24.0);
        let body = Rect::new(
            area.x + 8.0,
            area.y + 64.0,
            (area.w - 18.0).max(0.0),
            (area.h - if self.message.is_some() { 101.0 } else { 70.0 }).max(0.0),
        );
        if let Some(message) = self.message {
            p.text(Face::Ui, BODY_SIZE, body.x + 4.0, body.bottom() + 15.0, message, pal.warn, body.w - 8.0);
        }
        if body.w <= INDENT || body.h <= 0.0 {
            return Vec::new();
        }
        let width = (body.w - INDENT - 9.0).max(1.0);
        let rows = self.lines(p, &f, width);
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
        let first = rows.lines.partition_point(|(y, h, _)| *y + *h <= self.offset);
        for (y, h, line) in &rows.lines[first..] {
            let r = Rect::new(body.x, body.y + y - self.offset, body.w, *h);
            if r.y >= body.bottom() {
                break;
            }
            let mut button = |r: Rect, index, part| {
                let t = Target::Chat(Control::Detail(Hit {
                    tab: f.tab,
                    revision: f.model.revision,
                    side: f.side,
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
                    let r = r.inset(2.0);
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

    fn lines(&self, p: &mut Painter, f: &Frame<'_>, width: f32) -> Lines {
        let mut out = Lines { lines: Vec::new(), height: 0.0, reveal: None };
        match f.side {
            Side::Tasks => {
                let opens = plan_update(f.model).is_some();
                for (index, task) in f.model.tasks.iter().enumerate() {
                    let key = task_key(index, &task.content);
                    let expanded = self.expanded.contains(&key);
                    let (mark, status) = task_status(task.status);
                    let label = format!("{mark}  {}", first_line(&task.content));
                    out.push(
                        ROW,
                        Line::Header {
                            label,
                            index,
                            expanded,
                            active: task.status == claude_switch_core::transcript::TaskStatus::InProgress,
                            split: opens,
                            opens,
                        },
                    );
                    if expanded {
                        let top = out.height;
                        out.detail(p, width, format!("{status}\n{}", task.content));
                        if opens {
                            out.push(
                                ROW,
                                Line::Button { label: "Show the latest plan update".into(), index, part: Part::Open },
                            );
                        }
                        self.reveal(&mut out, &key, top);
                    }
                }
            }
            Side::Agents => {
                for (index, agent) in f.model.agents.iter().enumerate() {
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
                        self.reveal(&mut out, &key, top);
                    }
                }
            }
            Side::Files => {
                for (index, file) in self.files.iter().enumerate() {
                    let key = Key::File(file.key.clone());
                    let expanded = self.expanded.contains(&key);
                    let name = Path::new(&file.path).file_name().and_then(|s| s.to_str()).unwrap_or(&file.path);
                    let opens = file.file.as_ref().is_some_and(|f| f.supports_default_app());
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
                        self.reveal(&mut out, &key, top);
                    }
                }
            }
            Side::Shells | Side::Outputs => self.extra_lines(&mut out, p, f, width),
        }
        out
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
