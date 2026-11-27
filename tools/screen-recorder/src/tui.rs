//! The screen of `recorder compat`: the scripts with their state, the terminal being recorded,
//! and what does not match. Only the terminal's own colours and a few named ones, so it reads on
//! a dark and on a light theme.
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};

use crate::compat::{Row, Run, Shared, Status, mmss};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const KEYS: &str = " ↑↓ select · Enter details · r record again · q quit";
const KEYS_DETAILS: &str = " ↑↓ scroll · Enter or Esc back · r record again · q quit";

struct View {
    selected: usize,
    details: bool,
    scroll: u16,
    began: Instant,
    quitting: bool,
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn mark(row: &Row, tick: usize) -> (String, Style) {
    let spin = SPINNER[tick % SPINNER.len()].to_owned();
    match row.status {
        Status::Queued => ("·".to_owned(), dim()),
        Status::Recording => (spin, Style::new().fg(Color::Cyan)),
        Status::Checking => (spin, Style::new().fg(Color::Magenta)),
        Status::Pass => ("✓".to_owned(), Style::new().fg(Color::Green)),
        Status::Fail | Status::Error(_) => ("✗".to_owned(), Style::new().fg(Color::Red)),
        Status::Skipped(_) => ("-".to_owned(), dim()),
    }
}

fn word(row: &Row) -> String {
    let states = || format!("{}/{}", row.states.len() - row.failed().len(), row.states.len());
    let again = if row.attempt > 1 { format!(", try {}", row.attempt) } else { String::new() };
    match &row.status {
        Status::Queued if row.attempt > 0 => "queued again".to_owned(),
        Status::Queued => "queued".to_owned(),
        Status::Recording => format!("recording{again}"),
        Status::Checking => "checking".to_owned(),
        Status::Pass => format!("pass {}{again}", states()),
        Status::Fail => format!("FAIL {}{again}", states()),
        Status::Skipped(why) => format!("skipped: {why}"),
        Status::Error(_) => "ERROR".to_owned(),
    }
}

/// The first thing wrong with a row, for its line in the list.
fn trouble(row: &Row) -> String {
    if let Status::Error(why) = &row.status {
        return why.lines().next().unwrap_or_default().to_owned();
    }
    match row.failed().first() {
        Some(state) => {
            let d = &state.diffs[0];
            format!("{}: {} expected \"{}\", read \"{}\"", state.name, d.key, d.expected, d.got)
        }
        None => row.note.clone(),
    }
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn header(frame: &mut Frame, area: Rect, shared: &Shared, view: &View) {
    let [top, second, gauge] = Layout::vertical([Constraint::Length(1); 3]).areas(area);
    let last = shared.last_tested.as_deref().unwrap_or("never");
    let line = Line::from(vec![
        Span::styled(" Chat and Claude Code ", bold()),
        Span::styled(shared.version.clone(), bold().fg(Color::Cyan)),
        Span::raw(format!(" · last checked {last} · account {} · model {}", shared.account, shared.model)),
    ]);
    frame.render_widget(Paragraph::new(line), top);
    let elapsed = shared.took.unwrap_or_else(|| shared.started.elapsed());
    let phase = if view.quitting { "Stopping and cleaning up" } else { shared.phase.as_str() };
    let cost = format!("about ${:.3} ({} requests)", shared.cost.dollars, shared.cost.requests);
    let line = format!(" {} elapsed · {cost} · {phase}", mmss(elapsed));
    frame.render_widget(Paragraph::new(line), second);
    let wanted = shared.rows.iter().filter(|r| !matches!(r.status, Status::Skipped(_))).count();
    let done = shared.rows.iter().filter(|r| r.finished() && !matches!(r.status, Status::Skipped(_))).count();
    let (_, fail, _) = shared.counts();
    let label = if shared.done {
        format!("{done}/{wanted} scripts · done")
    } else {
        format!("{done}/{wanted} scripts · about {} left", mmss(Duration::from_secs(shared.left())))
    };
    let colour = if fail > 0 { Color::Red } else { Color::Green };
    let ratio = if wanted == 0 { 1.0 } else { done as f64 / wanted as f64 };
    let gauge_area = Rect { x: gauge.x + 1, width: gauge.width.saturating_sub(2), ..gauge };
    frame.render_widget(Gauge::default().gauge_style(Style::new().fg(colour)).ratio(ratio).label(label), gauge_area);
}

fn list(frame: &mut Frame, area: Rect, shared: &Shared, view: &View, tick: usize) {
    let block = Block::new().borders(Borders::TOP).title(" Scripts ").title_style(bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;
    let lines: Vec<Line> = shared
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let (mark, style) = mark(row, tick);
            let time = match (row.took, row.started) {
                (Some(took), _) => mmss(took),
                (None, Some(started)) if !row.finished() => mmss(started.elapsed()),
                _ => String::new(),
            };
            let head = format!(" {:<12} {:<22} {:>5}  ", row.script.name, word(row), time);
            let rest = clip(&trouble(row), width.saturating_sub(head.chars().count() + 3));
            let mut line = Line::from(vec![
                Span::raw(if i == view.selected { "▸" } else { " " }),
                Span::styled(mark, style),
                Span::styled(head, if row.finished() && row.status != Status::Pass { style } else { Style::new() }),
                Span::styled(rest, dim()),
            ]);
            if i == view.selected {
                line = line.style(Style::new().add_modifier(Modifier::REVERSED));
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The last rows of the screen being recorded.
fn live(frame: &mut Frame, area: Rect, run: &Run, shared: &Shared) {
    let name = shared.rows.iter().find(|r| r.status == Status::Recording).map(|r| r.script.name.as_str());
    let title = match name {
        Some(name) => format!(" Claude Code's terminal: {name} "),
        None => " Claude Code's terminal ".to_owned(),
    };
    let block = Block::new().borders(Borders::TOP).title(title).title_style(bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = run.live.lock().unwrap().clone();
    let end = rows.iter().rposition(|row| !row.trim().is_empty()).map_or(0, |i| i + 1);
    let start = end.saturating_sub(inner.height as usize);
    let lines: Vec<Line> = rows[start..end].iter().map(|row| Line::raw(clip(row, inner.width as usize))).collect();
    frame.render_widget(Paragraph::new(lines).style(dim()), inner);
}

fn diff_lines(shared: &Shared, row: &Row) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let red = Style::new().fg(Color::Red);
    if let Status::Error(why) = &row.status {
        lines.extend(why.lines().map(|l| Line::styled(format!("  {l}"), red)));
    }
    if !row.note.is_empty() {
        lines.push(Line::styled(format!("  {}", row.note), red));
    }
    for state in &row.states {
        if state.diffs.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  ✓ ", Style::new().fg(Color::Green)),
                Span::raw(format!("{}/{}", row.script.name, state.name)),
            ]));
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled("  ✗ ", red),
            Span::styled(format!("{}/{}", row.script.name, state.name), bold()),
        ]));
        for d in &state.diffs {
            lines.push(Line::raw(format!("      {}: Chat expected  {}", d.key, d.expected)));
            lines.push(Line::styled(
                format!("      {}  it read        {}", " ".repeat(d.key.chars().count()), d.got),
                red,
            ));
        }
        let path = shared.out.join(&row.script.name).join(format!("{}.txt", state.name));
        lines.push(Line::styled(format!("      recording: {}", path.display()), dim()));
    }
    lines
}

fn summary(frame: &mut Frame, area: Rect, shared: &Shared) {
    let block = Block::new().borders(Borders::TOP).title(" Summary ").title_style(bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (pass, fail, skipped) = shared.counts();
    let verdict = match (&shared.fatal, fail) {
        (Some(why), _) => Line::styled(
            format!(" Could not run: {}", why.lines().next().unwrap_or_default()),
            Style::new().fg(Color::Red),
        ),
        (None, 0) => Line::styled(" Every state read as Chat expects.", bold().fg(Color::Green)),
        _ => Line::styled(" Some screens no longer read as Chat expects.", bold().fg(Color::Red)),
    };
    let took = shared.took.unwrap_or_default();
    let mut lines = vec![
        verdict,
        Line::raw(format!(
            " {pass} passed · {fail} failed · {skipped} skipped · {} · about ${:.3} ({} requests)",
            mmss(took),
            shared.cost.dollars,
            shared.cost.requests
        )),
        Line::raw(format!(" Report: {}", shared.report.as_ref().map(|p| p.display().to_string()).unwrap_or_default())),
        Line::raw(""),
    ];
    for row in shared.rows.iter().filter(|r| matches!(r.status, Status::Fail | Status::Error(_))) {
        lines.extend(
            diff_lines(shared, row).into_iter().filter(|l| !l.spans.first().is_some_and(|s| s.content.contains('✓'))),
        );
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn details(frame: &mut Frame, area: Rect, shared: &Shared, view: &View) {
    let row = &shared.rows[view.selected];
    let block =
        Block::new().borders(Borders::TOP).title(format!(" {}: {} ", row.script.name, word(row))).title_style(bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = diff_lines(shared, row);
    // The screen of the first state that does not match, else of the last one.
    let shown = row.failed().first().map(|s| s.name.clone()).or_else(|| row.states.last().map(|s| s.name.clone()));
    let shown = if row.note.is_empty() { shown } else { Some("aborted".to_owned()) };
    if let Some(state) = shown {
        let path = shared.out.join(&row.script.name).join(format!("{state}.txt"));
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("  The screen at \"{state}\" ({})", path.display()), bold()));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "(no recording)".to_owned());
        let rows: Vec<&str> = text.lines().skip(1).collect();
        let end = rows.iter().rposition(|r| !r.trim().is_empty()).map_or(0, |i| i + 1);
        lines.extend(rows[..end].iter().map(|r| Line::styled(clip(r, inner.width as usize), dim())));
    } else if row.finished() {
        lines.push(Line::raw("  Nothing was recorded."));
    } else {
        lines.push(Line::raw("  Not finished yet."));
    }
    frame.render_widget(Paragraph::new(lines).scroll((view.scroll, 0)), inner);
}

fn draw(frame: &mut Frame, run: &Run, view: &View) {
    let shared = run.shared.lock().unwrap();
    let tick = (view.began.elapsed().as_millis() / 100) as usize;
    let rows = shared.rows.len() as u16 + 1;
    let [head, scripts, rest, keys] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(rows), Constraint::Min(3), Constraint::Length(1)])
            .areas(frame.area());
    header(frame, head, &shared, view);
    list(frame, scripts, &shared, view, tick);
    if view.details {
        details(frame, rest, &shared, view);
    } else if shared.done {
        summary(frame, rest, &shared);
    } else {
        live(frame, rest, run, &shared);
    }
    let keys_line = if view.details { KEYS_DETAILS } else { KEYS };
    frame.render_widget(Paragraph::new(keys_line).style(dim()), keys);
}

/// Shows the run until the user leaves; `q` ends the recording in progress and the worker.
pub fn run(run: &Run) -> std::io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let mut view = View { selected: 0, details: false, scroll: 0, began: Instant::now(), quitting: false };
    let result = loop {
        if let Err(e) = terminal.draw(|frame| draw(frame, run, &view)) {
            break Err(e);
        }
        // Once asked to quit: until the worker has wrapped up (the report) and gone idle.
        if view.quitting && run.shared.lock().unwrap().done {
            break Ok(());
        }
        if !event::poll(Duration::from_millis(80))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let count = run.shared.lock().unwrap().rows.len();
        let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => view.quitting = true,
            _ if ctrl_c => view.quitting = true,
            KeyCode::Up | KeyCode::Char('k') if view.details => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') if view.details => view.scroll += 1,
            KeyCode::PageUp if view.details => view.scroll = view.scroll.saturating_sub(10),
            KeyCode::PageDown if view.details => view.scroll += 10,
            KeyCode::Up | KeyCode::Char('k') => view.selected = view.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => view.selected = (view.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Enter => (view.details, view.scroll) = (!view.details, 0),
            KeyCode::Esc => view.details = false,
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let mut shared = run.shared.lock().unwrap();
                let fatal = shared.fatal.is_some();
                if shared.rows[view.selected].finished() && !fatal && !view.quitting {
                    shared.rows[view.selected].status = Status::Queued;
                    shared.queue.push_back(view.selected);
                    (shared.done, shared.took) = (false, None);
                }
            }
            _ => {}
        }
        if view.quitting {
            run.stop.store(true, Ordering::Relaxed);
            run.quit.store(true, Ordering::Relaxed);
        }
    };
    ratatui::restore();
    result
}
