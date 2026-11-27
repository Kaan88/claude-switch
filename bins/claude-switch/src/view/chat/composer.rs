use claude_switch_core::transcript::ChatInfo;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::theme::Palette;

use super::{Chat, Chip, Control, Drawer, Gate, commands, editor, paint::button, popups, submission};
use crate::state::Target;
use crate::view::look;

pub(super) struct Draw<'a> {
    pub pal: &'a Palette,
    pub focused: bool,
    pub wide: bool,
    pub hover: Option<Target>,
    pub hits: &'a mut Vec<(Rect, Target)>,
    pub pins: &'a [String],
    /// The quick-command chips in their order (Settings › Chat).
    pub chips: &'a [Control],
    /// Own command lines by slot; an empty one has no chip.
    pub own: &'a [String],
}

/// The chips in their default order.
pub(super) const DEFAULT_CHIPS: [Control; 7] = [
    Control::Model,
    Control::Effort,
    Control::Context,
    Control::Mode,
    Control::Compact,
    Control::Clear,
    Control::Commands,
];

/// The chip a Settings key names; `None` for a key this version does not know.
pub(crate) fn chip_control(key: &str) -> Option<Control> {
    let at = claude_switch_core::config::QUICK_CHIPS.iter().position(|(k, _)| *k == key)?;
    DEFAULT_CHIPS.get(at).copied()
}

/// The quick-command chips Settings shows, in order.
pub(crate) fn chips_of(prefs: &claude_switch_core::config::ChatPrefs) -> Vec<Control> {
    prefs.shown().filter_map(chip_control).collect()
}

const MODE_W: f32 = 125.0;
/// An own command's widest chip.
const OWN_MAX: f32 = 150.0;

fn chip_width(control: Control) -> f32 {
    match control {
        Control::Model => MODEL_W,
        Control::Effort => EFFORT_CHIP_MAX,
        Control::Context => CONTEXT_CHIP_MAX,
        Control::Mode => MODE_W,
        Control::Compact => 62.0,
        Control::Clear => 44.0,
        _ => 32.0,
    }
}

/// What the left group takes on the toolbar's line: its chips but the permission mode (shown
/// only when there is room left), the gaps between them and the row's 1 px inset.
pub(super) fn left_group(chips: &[Control]) -> f32 {
    let fixed: Vec<f32> = chips.iter().filter(|c| **c != Control::Mode).map(|c| chip_width(*c)).collect();
    if fixed.is_empty() { 0.0 } else { fixed.iter().sum::<f32>() + 4.0 * (fixed.len() - 1) as f32 + 1.0 }
}

/// The centre of the editor's first row, from its top.
const FIRST_ROW: f32 = 15.0;
/// The round Send button.
const SEND: f32 = 32.0;
const PICKER_HEADER: f32 = 22.0;
const PICKER_ROW: f32 = 25.0;
const PILL_FONT: f32 = 11.5;

pub(super) struct Suggestions {
    bounds: Rect,
    matches: Vec<usize>,
    selected: usize,
    first: usize,
    visible: usize,
}

impl Suggestions {
    pub fn pointer_behind(&self, mouse: (f32, f32)) -> (f32, f32) {
        if self.bounds.contains(mouse.0, mouse.1) { (-1.0, -1.0) } else { mouse }
    }
}

/// What the composer says while a read-only panel card is up.
pub(super) const PANEL_UP: &str = "Close Claude Code's panel above to send.";
/// Notices about the terminal's prompt that a menu or panel card makes redundant.
const SCREEN_NOTICES: [&str; 3] =
    [super::submission::CHECK, super::submission::NO_QUIET_PROMPT, super::submission::NOT_EMPTY];

impl Chat {
    /// The picker and transcript use the same final rectangle, not the layout's maximum envelope.
    pub(super) fn suggestion_plan(&self, pins: &[String]) -> Option<Suggestions> {
        let r = self.layout.suggestions;
        let max_rows = ((r.h - PICKER_HEADER - 4.0) / PICKER_ROW).max(0.0) as usize;
        if !self.commands.open || max_rows == 0 {
            return None;
        }
        let matches = self.commands.matches(&self.editor.text, pins);
        let visible = matches.len().max(1).min(max_rows);
        let selected = self.commands.selected.min(matches.len().saturating_sub(1));
        let first = selected.saturating_sub(visible - 1);
        let h = PICKER_HEADER + PICKER_ROW * visible as f32 + 4.0;
        let bounds = Rect::new(r.x, r.bottom() - h, r.w, h);
        Some(Suggestions { bounds, matches, selected, first, visible })
    }

    pub(super) fn notice(&self, info: &ChatInfo) -> Option<&'static str> {
        // With a menu or panel card up, the card says what the screen shows: the composer only
        // says why it waits, not that the terminal's prompt is not visible.
        let card = self
            .question
            .menu
            .as_ref()
            .filter(|_| self.question.visible())
            .map(|menu| if menu.panel() { PANEL_UP } else { super::Reason::Menu.label() })
            .or_else(|| {
                (self.question.blocked_shown() && self.question.visible()).then_some(super::Reason::Menu.label())
            });
        let error = match (self.error, card) {
            (Some(error), Some(card)) if SCREEN_NOTICES.contains(&error) => Some(card),
            (error, _) => error,
        };
        let reason = |r: super::Reason| match card {
            Some(card) if r == super::Reason::Menu => card,
            _ => r.label(),
        };
        self.reason
            .filter(|r| *r == super::Reason::Permission)
            .map(super::Reason::label)
            .or(error)
            .or_else(|| self.recovery_notice())
            .or_else(|| self.reason.map(reason))
            .or_else(|| self.waiting())
            .or_else(|| self.held_notice())
            .or_else(|| self.recall.waiting().then_some("Loading earlier prompts…"))
            .or_else(|| {
                self.queue.as_ref().map(|queue| {
                    if queue.sends_now() {
                        "Sends as soon as Claude Code's input reads empty."
                    } else if self.last_gate == Some(Gate::Running) {
                        "A turn is running. This will be sent when it ends."
                    } else if self.drawer_focus {
                        "Queue paused while the terminal has focus."
                    } else {
                        "Queued. Waiting for a verified, empty CLI prompt."
                    }
                })
            })
            .or_else(|| self.can_stop(info).then_some("A turn is running. You can queue the next message."))
    }

    pub(super) fn attachment_pages(&self) -> usize {
        self.chips.iter().filter(|c| matches!(c, Chip::Path(_))).count().div_ceil(3).max(1)
    }
}

impl Draw<'_> {
    pub(super) fn shell_chip(&mut self, p: &mut Painter, area: Rect, count: usize) {
        if count > 0 && area.h > 0.0 {
            let label = crate::view::shells::running_label(count);
            let w = p.measure(Face::Semibold, PILL_FONT, &label) + 18.0;
            if w <= area.w {
                self.pill(p, Rect::new(area.x + 3.0, area.y + 1.0, w, 21.0), &label, Control::Shells, false, true);
            }
        }
    }

    pub fn composer(
        mut self,
        p: &mut Painter,
        chat: &mut Chat,
        info: &ChatInfo,
        suggestions: Option<Suggestions>,
        popup: Option<popups::Plan>,
    ) {
        let l = chat.layout;
        let clip = p.clip(l.composer);
        // A raised box: with the keyboard its border is a little stronger, not the accent.
        look::raised_box(p, self.pal, l.box_frame, self.focused);
        self.attachments(p, chat);
        self.editor(p, chat);
        self.shell_chip(p, l.shell_chip, chat.shell_count);
        self.notice(p, chat, info);
        self.toolbar(p, chat, info);
        p.restore(clip);
        chat.layout.suggestions =
            suggestions.as_ref().map(|s| s.bounds).or(popup.as_ref().map(|s| s.bounds)).unwrap_or_default();
        if let Some(suggestions) = suggestions {
            self.suggestions(p, chat, suggestions);
        } else if let Some(popup) = popup {
            self.popup(p, chat, popup);
        }
    }

    /// The `@` file list or the Ctrl+R prompt search, drawn like the command list.
    fn popup(&mut self, p: &mut Painter, chat: &Chat, plan: popups::Plan) {
        let popups::Plan { kind, bounds: r, items, selected, first, visible } = plan;
        p.rbox(r, 8.0, self.pal.popup_line);
        p.rbox(r.inset(1.0), 7.0, self.pal.popup);
        self.hits.push((r, Target::Chat(Control::CommandBackground)));
        let clip = p.clip(r);
        let heading = match kind {
            popups::Kind::Files if chat.popups.loading_files() => "Files · reading the session folder…".to_owned(),
            popups::Kind::Files => "Files · ↑↓ choose · Tab or Enter inserts · Esc closes".to_owned(),
            popups::Kind::History => {
                format!("Earlier prompts · search: {}▏ · Ctrl+R next · Enter fills", chat.popups.query)
            }
        };
        p.text(Face::Ui, 11.0, r.x + 10.0, r.y + 11.0, &heading, self.pal.muted, r.w - 20.0);
        if items.is_empty() {
            let empty = match kind {
                popups::Kind::Files if chat.popups.loading_files() => "",
                popups::Kind::Files => "No matching file in the session folder.",
                popups::Kind::History => "No earlier prompt matches. Up loads older history.",
            };
            p.text(
                Face::Ui,
                12.0,
                r.x + 10.0,
                r.y + popups::HEADER + popups::ROW / 2.0,
                empty,
                self.pal.muted,
                r.w - 20.0,
            );
        }
        for (n, item) in items.iter().enumerate().skip(first).take(visible) {
            let row = Rect::new(
                r.x + 4.0,
                r.y + popups::HEADER + (n - first) as f32 * popups::ROW,
                r.w - 8.0,
                popups::ROW - 1.0,
            );
            let target = Target::Chat(Control::Popup(n));
            if n == selected || self.hover == Some(target) {
                p.rbox(row, 5.0, if n == selected { self.pal.on } else { self.pal.hover });
            }
            let shown = item.split_whitespace().collect::<Vec<_>>().join(" ");
            let face = if kind == popups::Kind::Files { Face::Mono } else { Face::Ui };
            p.text(face, 12.0, row.x + 8.0, row.cy(), &shown, self.pal.text, row.w - 16.0);
            self.hits.push((row, target));
        }
        p.restore(clip);
    }

    fn hit(&mut self, p: &mut Painter, r: Rect, label: &str, control: Control, picked: bool) {
        let target = Target::Chat(control);
        button(p, self.pal, r, label, self.hover == Some(target), picked);
        self.hits.push((r, target));
    }

    fn pill(&mut self, p: &mut Painter, r: Rect, label: &str, control: Control, picked: bool, enabled: bool) {
        let target = Target::Chat(control);
        let hover = enabled && self.hover == Some(target);
        let accent = picked && control == Control::Send;
        let fill = if accent {
            self.pal.accent
        } else if hover {
            self.pal.hover
        } else if picked {
            self.pal.on
        } else {
            self.pal.bar
        };
        let ink = if accent {
            self.pal.on_accent
        } else if picked {
            self.pal.accent
        } else if enabled {
            self.pal.text
        } else {
            self.pal.muted
        };
        p.rbox(r, r.h / 2.0, fill);
        debug_assert!(p.measure(Face::Semibold, PILL_FONT, label) <= r.w - 8.0, "{label} exceeds {}", r.w);
        p.text_center(Face::Semibold, PILL_FONT, r, label, ink);
        if enabled {
            self.hits.push((r, target));
        }
    }

    fn attachments(&mut self, p: &mut Painter, chat: &Chat) {
        if chat.chips.is_empty() {
            return;
        }
        let r = chat.layout.attachments;
        let mut x = r.x + 4.0;
        if let Some((i, Chip::Clipboard(pic))) =
            chat.chips.iter().enumerate().find(|(_, c)| matches!(c, Chip::Clipboard(_)))
        {
            let card = Rect::new(x, r.y + 1.0, if pic.thumbnail.is_some() { 65.0 } else { 138.0 }, 42.0);
            p.rbox(card, 5.0, self.pal.bar);
            if let Some(im) = &pic.thumbnail {
                let s = (62.0 / im.w as f32).min(40.0 / im.h as f32);
                let (w, h) = (im.w as f32 * s, im.h as f32 * s);
                p.image(&im.rgba, im.w, im.h, Rect::new(card.cx() - w / 2.0, card.cy() - h / 2.0, w, h), 4.0, 1.0);
            } else {
                p.text(Face::Ui, 11.5, card.x + 7.0, card.cy(), "Clipboard picture", self.pal.text, 111.0);
            }
            if !chat.locked() {
                self.hit(p, Rect::new(card.right() - 17.0, card.y, 17.0, 17.0), "×", Control::Remove(i), false);
            }
            x = card.right() + 6.0;
        }
        let pages = chat.attachment_pages();
        let more = if pages > 1 { 62.0 } else { 0.0 };
        let w = ((r.right() - x - more) / 3.0 - 6.0).clamp(45.0, 150.0);
        for (i, chip) in
            chat.chips.iter().enumerate().filter(|(_, c)| matches!(c, Chip::Path(_))).skip(chat.chip_page * 3).take(3)
        {
            let Chip::Path(path) = chip else { continue };
            let chip = Rect::new(x, r.y + 8.0, w, 26.0);
            p.rbox(chip, 6.0, self.pal.bar);
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("File");
            p.text(Face::Ui, 11.5, chip.x + 7.0, chip.cy(), name, self.pal.text, (chip.w - 26.0).max(0.0));
            if !chat.locked() {
                self.hit(p, Rect::new(chip.right() - 19.0, chip.y, 19.0, chip.h), "×", Control::Remove(i), false);
            }
            x += w + 6.0;
        }
        if pages > 1 {
            let label = format!("{}/{} ›", chat.chip_page + 1, pages);
            self.hit(p, Rect::new(r.right() - 62.0, r.y + 8.0, 60.0, 26.0), &label, Control::MoreChips, false);
        }
    }

    fn editor(&mut self, p: &mut Painter, chat: &mut Chat) {
        let e = chat.layout.editor;
        self.hits.push((e, Target::Chat(Control::Editor)));
        let c = p.clip(e);
        let x = e.x + 8.0;
        let width = editor_text_width(e);
        let next_chip = chat.popups.next_prompt.clone().filter(|_| !chat.locked());
        let ed = &mut chat.editor;
        ed.flow(p, width, e.h);
        let selection = ed.selection();
        let caret_row = ed.caret_row();
        if ed.text.is_empty() {
            p.text(editor::FACE, editor::TEXT_SIZE, x, e.y + FIRST_ROW, "Message Claude Code…", self.pal.muted, width);
            if let Some(next) = next_chip.as_deref() {
                let label = format!("↳ {}  ·  Tab", short(next, 80));
                let w = (p.measure(Face::Ui, PILL_FONT, &label) + 18.0).min(width);
                let r = Rect::new(x - 2.0, e.y + FIRST_ROW + editor::LINE - 10.0, w, 20.0);
                let target = Target::Chat(Control::NextPrompt);
                p.rbox(r, 10.0, if self.hover == Some(target) { self.pal.hover } else { self.pal.bar });
                p.text(Face::Ui, PILL_FONT, r.x + 9.0, r.cy(), &label, self.pal.text, r.w - 14.0);
                self.hits.push((r, target));
            }
        }
        for (i, row) in ed.rows.iter().enumerate().skip(ed.scroll).take(ed.visible) {
            let y = e.y + FIRST_ROW + (i - ed.scroll) as f32 * editor::LINE;
            let span = &ed.text[row.clone()];
            let mut before = |at| p.measure(editor::FACE, editor::TEXT_SIZE, &ed.text[row.start..at]);
            let start = selection.start.max(row.start);
            let end = selection.end.min(row.end);
            let selected = (end > start).then(|| (before(start), before(end)));
            let caret = (self.focused && i == caret_row).then(|| before(ed.caret.min(row.end)));
            if let Some((a, b)) = selected {
                p.fill(Rect::new(x + a, y - 10.0, b - a, 20.0), self.pal.on);
            }
            let painted;
            let span = if editor::hyphen(&ed.text, row) {
                painted = format!("{span}-");
                &painted
            } else {
                span
            };
            if !span.is_empty() {
                // Rows are wrapped by summed advances; shaping the row whole may differ by a hair.
                p.text(editor::FACE, editor::TEXT_SIZE, x, y, span, self.pal.text, width + 8.0);
            }
            if let Some(at) = caret {
                p.fill(Rect::new(x + at, y - 9.0, p.px(), 18.0), self.pal.accent);
            }
        }
        p.restore(c);
    }

    fn notice(&mut self, p: &mut Painter, chat: &Chat, info: &ChatInfo) {
        let Some(note) = chat.notice(info) else { return };
        let r = chat.layout.notice.inset(2.0);
        p.rbox(r, 5.0, if chat.error.is_some() { self.pal.warn_bg } else { self.pal.bar });
        let mut x = r.right() - 3.0;
        let mut add = |text: &str, w: f32, control| {
            x -= w;
            self.hit(p, Rect::new(x, r.y + 2.0, w, r.h - 4.0), text, control, false);
            x -= 4.0;
        };
        if chat.enter_only() {
            add("Cancel", 54.0, Control::CancelSend);
            if chat.reason.is_none() {
                add("Press Enter", 94.0, Control::PressEnter);
            }
        } else if chat.locked() {
            add("Cancel", 54.0, Control::CancelSend);
            if chat.queue.is_some() {
                add("Send now", 76.0, Control::SendNow);
                if chat.last_gate == Some(Gate::Running) && chat.can_stop(info) {
                    add("Stop", 47.0, Control::Stop);
                }
            }
        } else if chat.error == Some(submission::CHECK) && chat.last_gate == Some(Gate::Check) && chat.reason.is_none()
        {
            add("Send anyway", 100.0, Control::Override);
        } else if chat.last_gate == Some(Gate::Running) && chat.can_stop(info) {
            add("Stop", 47.0, Control::Stop);
        }
        p.text(Face::Ui, 11.5, r.x + 7.0, r.cy(), note, self.pal.text, (x - r.x - 11.0).max(0.0));
    }

    fn toolbar(&mut self, p: &mut Painter, chat: &Chat, info: &ChatInfo) {
        let l = chat.layout;
        let r = l.toolbar;
        let mut x = r.x + 1.0;
        // A line of the toolbar is as high as Send; the chips sit in its middle.
        let top = r.y + (SEND - 24.0) / 2.0;
        let model = chat.cli_report.model.as_deref().or(info.model.as_deref()).unwrap_or("Model");
        let model = model_label(p, model);
        let effort = chat.effort_label(info);
        let compacted = info.compacted;
        let after = info.compacted_size.map_or_else(|| "—".to_owned(), tokens);
        let context = match info.context {
            _ if compacted => format!("Context {after} after compact"),
            Some(n) => format!("Context {}", tokens(n)),
            None => "Context —".into(),
        };
        // Reserve fixed widths for values so a change cannot move adjacent controls.
        let level = effort.trim_end_matches(" (default)");
        let effort = format!("{level} ▾");
        let context = if compacted { format!("After {after}") } else { context };
        let right = if l.stacked { 0.0 } else { TOOLBAR_RIGHT + if self.wide { DETAILS_W } else { 0.0 } };
        let room = r.w - right - left_group(self.chips);
        // The permission mode, when the row has room left for it.
        let mode = chat.mode_label();
        let mode_shown = room >= MODE_W + 4.0;
        for &control in self.chips {
            let label = match control {
                Control::Model => model.as_str(),
                Control::Effort => effort.as_str(),
                Control::Context => context.as_str(),
                Control::Mode if mode_shown => mode.as_str(),
                Control::Mode => continue,
                Control::Compact => "Compact",
                Control::Clear => "Clear",
                _ => "/ …",
            };
            let w = chip_width(control);
            let chip = Rect::new(x, top, w, 24.0);
            if control == Control::Effort {
                self.effort_pill(p, chip, &effort, level);
            } else {
                self.toolbar_pill(p, chip, label, control, control == Control::Commands && chat.commands.open, true);
            }
            x += w + 4.0;
        }
        // Own commands after the others, each only when it fits whole before the right group.
        let end = r.right() - right;
        for (n, line) in self.own.iter().enumerate().filter(|(_, line)| !line.is_empty()) {
            let fits = |p: &mut Painter, text: &str| {
                let w = (p.measure(Face::Semibold, PILL_FONT, text) + 16.0).ceil();
                (w <= OWN_MAX).then_some(w)
            };
            let name = line.split(' ').next().unwrap_or(line);
            let Some((label, w)) =
                fits(p, line).map(|w| (line.as_str(), w)).or_else(|| fits(p, name).map(|w| (name, w)))
            else {
                continue;
            };
            if x + w > end {
                break;
            }
            self.toolbar_pill(p, Rect::new(x, top, w, 24.0), label, Control::Own(n), false, !chat.locked());
            x += w + 4.0;
        }
        let row = r.y + if l.stacked { SEND - 2.0 } else { 0.0 };
        let rsend = Rect::new(r.right() - SEND - 1.0, row, SEND, SEND);
        self.send(p, chat, rsend);
        let y = row + (SEND - 24.0) / 2.0;
        // The slot left of Send is kept for the small Stop, so nothing moves when it shows.
        let mut x = rsend.x - STOP_SLOT - 4.0;
        if self.wide {
            x -= 58.0;
            self.toolbar_pill(p, Rect::new(x, y, 58.0, 24.0), "Details", Control::Panel, !chat.side_hidden, true);
            x -= 4.0;
        }
        x -= 82.0;
        self.toolbar_pill(
            p,
            Rect::new(x, y, 82.0, 24.0),
            if chat.drawer == Drawer::Closed { "Terminal ▴" } else { "Terminal ▾" },
            Control::Drawer,
            chat.drawer != Drawer::Closed,
            true,
        );
        x -= 63.0;
        let attach = Rect::new(x, y, 59.0, 24.0);
        self.toolbar_pill(p, attach, "Attach…", Control::Attach, false, !chat.locked());
    }

    /// Send: a round button, the accent with an up arrow when there is something to send and the
    /// gate allows it, grey when not.
    fn send(&mut self, p: &mut Painter, chat: &Chat, r: Rect) {
        let pal = self.pal;
        let target = Target::Chat(Control::Send);
        let enabled = !chat.locked();
        let armed =
            enabled && chat.recovery_notice().is_none() && matches!(chat.last_gate, Some(Gate::Ready | Gate::Running));
        let something = !chat.editor.text.trim().is_empty() || !chat.chips.is_empty();
        let hot = enabled && self.hover == Some(target);
        let ground = look::raised(pal);
        let (fill, ink) = if armed && something {
            (if hot { mdshared_ui::gfx::mix(pal.accent, pal.on_accent, 0.12) } else { pal.accent }, pal.on_accent)
        } else {
            (if hot { pal.hover } else { look::quiet_fill(pal, ground) }, if enabled { pal.muted } else { pal.faint })
        };
        look::round_icon(p, r, fill, look::ARROW_UP, 16.0, ink);
        if enabled {
            self.hits.push((r, target));
        }
    }

    fn effort_pill(&mut self, p: &mut Painter, r: Rect, label: &str, level: &str) {
        let target = Target::Chat(Control::Effort);
        p.rbox(r, 5.0, if self.hover == Some(target) { self.pal.hover } else { self.pal.field });
        let active = commands::EFFORT.iter().position(|&name| name == level).map(|n| n + 1).unwrap_or(0);
        for n in 0..5 {
            let dot = Rect::new(r.x + 5.0 + n as f32 * 5.0, r.cy() - 2.0, 4.0, 4.0);
            p.rbox(dot, 2.0, if n < active { self.pal.accent } else { self.pal.line });
        }
        debug_assert!(p.measure(Face::Semibold, PILL_FONT, label) <= r.w - 33.0);
        p.text_center(Face::Semibold, PILL_FONT, Rect::new(r.x + 29.0, r.y, r.w - 31.0, r.h), label, self.pal.text);
        self.hits.push((r, target));
    }

    fn toolbar_pill(&mut self, p: &mut Painter, r: Rect, label: &str, control: Control, picked: bool, enabled: bool) {
        let target = Target::Chat(control);
        let state = matches!(control, Control::Model | Control::Effort | Control::Context | Control::Mode);
        let panel = matches!(control, Control::Attach | Control::Drawer | Control::Panel);
        let fill = if enabled && self.hover == Some(target) {
            self.pal.hover
        } else if picked {
            self.pal.on
        } else if state {
            self.pal.field
        } else if panel {
            self.pal.bar
        } else {
            self.pal.page
        };
        p.rbox(r, if state { 5.0 } else { 12.0 }, fill);
        let ink = if !enabled {
            self.pal.muted
        } else if picked {
            self.pal.accent
        } else {
            self.pal.text
        };
        debug_assert!(p.measure(Face::Semibold, PILL_FONT, label) <= r.w - 8.0, "{label} exceeds {}", r.w);
        p.text_center(Face::Semibold, PILL_FONT, r, label, ink);
        if enabled {
            self.hits.push((r, target));
        }
    }

    fn suggestions(&mut self, p: &mut Painter, chat: &Chat, plan: Suggestions) {
        let Suggestions { bounds: r, matches, selected, first, visible } = plan;
        p.rbox(r, 8.0, self.pal.popup_line);
        p.rbox(r.inset(1.0), 7.0, self.pal.popup);
        self.hits.push((r, Target::Chat(Control::CommandBackground)));
        let clip = p.clip(r);
        let heading = if chat.commands.loading() {
            "Commands · reading local names…"
        } else {
            "Commands · ↑↓ choose · Tab fills · Enter runs"
        };
        p.text(Face::Ui, 11.0, r.x + 10.0, r.y + 11.0, heading, self.pal.muted, r.w - 20.0);
        if matches.is_empty() {
            p.text(
                Face::Ui,
                12.0,
                r.x + 10.0,
                r.y + PICKER_HEADER + PICKER_ROW / 2.0,
                "No matching suggestions. Enter runs the command you typed.",
                self.pal.muted,
                r.w - 20.0,
            );
        }
        for (n, &index) in matches.iter().enumerate().skip(first).take(visible) {
            let cmd = &chat.commands.catalog[index];
            let row = Rect::new(
                r.x + 4.0,
                r.y + PICKER_HEADER + (n - first) as f32 * PICKER_ROW,
                r.w - 8.0,
                PICKER_ROW - 1.0,
            );
            let star = Rect::new(row.x, row.y, 27.0, row.h);
            let target = Target::Chat(Control::Suggestion(index));
            if n == selected || self.hover == Some(target) {
                p.rbox(row, 5.0, if n == selected { self.pal.on } else { self.pal.hover });
            }
            let pinned = self.pins.contains(&cmd.name);
            self.hit(p, star, if pinned { "★" } else { "☆" }, Control::PinCommand(index), pinned);
            let mut right = row.right() - 3.0;
            for (i, value) in commands::choices(&cmd.name).iter().enumerate().rev() {
                let w = (p.measure(Face::Semibold, 11.5, value) + 12.0).max(33.0);
                right -= w;
                self.hit(p, Rect::new(right, row.y + 2.0, w, row.h - 4.0), value, Control::Argument(index, i), false);
                right -= 4.0;
            }
            let name = format!("/{}", cmd.name);
            let x = star.right() + 5.0;
            let name_w = p.measure(Face::Semibold, 12.0, &name).min(((right - x) * 0.63).max(0.0));
            p.text(Face::Semibold, 12.0, x, row.cy(), &name, self.pal.text, name_w + 1.0);
            let desc = x + name_w + 11.0;
            p.text(Face::Ui, 11.5, desc, row.cy(), &cmd.description, self.pal.muted, (right - desc - 3.0).max(0.0));
            self.hits.push((Rect::new(star.right(), row.y, (right - star.right()).max(0.0), row.h), target));
        }
        p.restore(clip);
    }
}

/// The editor's wrapping width inside its rectangle (8 px of padding on each side).
pub(super) fn editor_text_width(e: Rect) -> f32 {
    (e.w - 16.0).max(0.0)
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.into() } else { s.chars().take(max - 1).chain(['…']).collect() }
}

fn tokens(n: u64) -> String {
    for (unit, suffix) in [
        (1_000_000_000_000_000_000, "E"),
        (1_000_000_000_000_000, "P"),
        (1_000_000_000_000, "T"),
        (1_000_000_000, "B"),
        (1_000_000, "M"),
        (1_000, "k"),
    ] {
        if n >= unit {
            let value = n / unit;
            return if value < 100 {
                format!("{value}.{}{suffix}", n % unit / (unit / 10))
            } else {
                format!("{value}{suffix}")
            };
        }
    }
    n.to_string()
}

fn compact_model(model: &str) -> String {
    let model = model.strip_prefix("Demo ").unwrap_or(model);
    if let Some(model) = model.strip_prefix("claude-") {
        for family in ["sonnet", "opus", "haiku", "fable"] {
            if model.split('-').any(|part| part == family) {
                let mut versions = model.split('-').filter(|s| s.len() == 1 && s.bytes().all(|b| b.is_ascii_digit()));
                let family = format!("{}{}", family[..1].to_ascii_uppercase(), &family[1..]);
                return match (versions.next(), versions.next()) {
                    (Some(major), Some(minor)) => format!("{family} {major}.{minor}"),
                    (Some(major), None) => format!("{family} {major}"),
                    _ => family,
                };
            }
        }
    }
    for family in ["Sonnet", "Opus", "Haiku", "Fable"] {
        if let Some(rest) = model.strip_prefix(family) {
            let version = rest.split_whitespace().next().filter(|s| {
                s.len() <= 5 && s.as_bytes()[0].is_ascii_digit() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            });
            return version.map_or_else(|| family.to_owned(), |v| format!("{family} {v}"));
        }
    }
    short(model, 12)
}

const MODEL_W: f32 = 104.0;
/// The room kept left of Send for the small Stop button.
const STOP_SLOT: f32 = 26.0;
/// The right-hand group on the toolbar's line: Attach, Terminal, the Stop slot and Send with
/// their gaps.
const TOOLBAR_RIGHT: f32 = 59.0 + 4.0 + 82.0 + 4.0 + STOP_SLOT + 4.0 + SEND + 1.0;
/// The Details chip and its gap, when the right group has it.
const DETAILS_W: f32 = 62.0;
/// The widest short Effort chip ("Effort medium ▾") and Context chip ("Context 999.9k"):
/// the layout keeps room for them, and a test measures them against these.
pub(super) const EFFORT_CHIP_MAX: f32 = 99.0;
pub(super) const CONTEXT_CHIP_MAX: f32 = 92.0;

/// The toolbar width the left chips need to share one line with the right group. Narrower, the
/// layout puts the groups on two lines rather than let a chip shrink below its label.
pub(super) fn toolbar_one_line(wide: bool, left: f32) -> f32 {
    TOOLBAR_RIGHT + if wide { DETAILS_W } else { 0.0 } + left
}

fn model_label(p: &mut Painter, model: &str) -> String {
    let label = format!("{} ▾", compact_model(model));
    if label.contains('…') || p.measure(Face::Semibold, PILL_FONT, &label) > MODEL_W - 10.0 {
        "Model ▾".into()
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::chat::{Layout, clipboard};
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;

    /// Every chip label fits its chip (the pill's debug assertion) and the chips stay apart and
    /// inside the toolbar: each effort level, set or a model's default, each permission mode and
    /// Context after a compaction, from the minimum window width up, at several display scales.
    #[test]
    fn toolbar_chips_fit_their_longest_labels_at_every_width_and_scale() {
        use crate::view::chat::commands::EFFORT;
        use crate::view::chat::effort::Start;
        let mut gfx = Gfx::new();
        let app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
        let pal = app.pal();
        // (how the effort is known, the model): every level set, and the defaults with "(default)".
        let mut efforts: Vec<(Start, &str)> = EFFORT.iter().map(|&e| (Start::Set(e), "claude-sonnet-5-5")).collect();
        for model in ["claude-opus-5-5", "claude-opus-4-7", "claude-sonnet-4-6"] {
            efforts.push((Start::Default, model));
        }
        let modes = [
            None,
            Some("Default"),
            Some("Accept edits"),
            Some("Plan"),
            Some("Auto"),
            Some("Don't ask"),
            Some("Bypass"),
            Some("Manual"),
        ];
        let mut cases = Vec::new();
        for (start, model) in &efforts {
            cases.push((start.clone(), *model, Some("Don't ask")));
        }
        for mode in modes {
            cases.push((Start::Default, "claude-opus-4-7", mode));
        }
        {
            // The layout's reserve for the short labels holds what they measure.
            let mut bitmap = Bitmap::new(8, 8);
            let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
            for level in EFFORT.iter().chain(&["—"]) {
                let w = p.measure(Face::Semibold, PILL_FONT, &format!("Effort {level} ▾")) + 10.0;
                assert!(w <= EFFORT_CHIP_MAX, "Effort {level}: {w}");
            }
            for n in [0, 999, 99_999, 999_949, 9_999_999, 999_999_999_999] {
                let w = p.measure(Face::Semibold, PILL_FONT, &format!("Context {}", tokens(n))) + 10.0;
                assert!(w <= CONTEXT_CHIP_MAX, "Context {n}: {w}");
            }
        }
        // Every 20 px, and around where the Details panel appears and the measure reaches its cap.
        let widths = (760..=1600).step_by(20).chain([899, 900, 901, 1010, 1019, 1020, 1021, 1047, 1048, 1049]);
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for width in widths.clone() {
                let area = Rect::new(0.0, 0.0, width as f32, 480.0);
                let mut bitmap = Bitmap::new((width as f32 * scale) as u32, (480.0 * scale) as u32);
                let mut p = Painter::new(&mut bitmap, &mut gfx, scale);
                for panel in [false, true] {
                    let layout = Layout::new(area, panel, false, None, false, false);
                    for compacted in [false, true] {
                        let mut info = ChatInfo::default();
                        info.context = Some(987_654);
                        info.compacted = compacted;
                        info.compacted_size = Some(987_654);
                        for (start, model, mode) in &cases {
                            info.model = Some((*model).into());
                            let mut chat = Chat { layout, last_gate: Some(Gate::Ready), ..Default::default() };
                            chat.set_starting_effort(start.clone());
                            chat.mode_shown = *mode;
                            let mut hits = Vec::new();
                            let wide = area.w >= 900.0 || layout.side.w > 0.0;
                            Draw {
                                pal,
                                focused: true,
                                wide,
                                hover: None,
                                hits: &mut hits,
                                pins: &[],
                                chips: &DEFAULT_CHIPS,
                                own: &[],
                            }
                            .toolbar(&mut p, &chat, &info);
                            let what = format!("{width} px at {scale}x, panel {panel}, {start:?} {model} {mode:?}");
                            for (i, (a, _)) in hits.iter().enumerate() {
                                assert!(a.x >= layout.toolbar.x && a.right() <= layout.toolbar.right() + 0.5, "{what}");
                                for (b, _) in hits.iter().skip(i + 1) {
                                    let apart =
                                        a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                                    assert!(apart, "{what}: {a:?} overlaps {b:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Settings › Chat: the chips in the chosen order, hidden ones absent, own commands after them
    /// only where they fit whole, and fewer chips keep the row on one line at a narrower width.
    #[test]
    fn configured_chips_follow_their_order_and_own_commands_fit_or_stay_away() {
        let mut gfx = Gfx::new();
        let app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
        let mut bitmap = Bitmap::new(1400, 480);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let chips = [Control::Clear, Control::Model];
        let own = ["/review --strict".to_owned(), String::new(), format!("/{}", "x".repeat(90))];
        let area = Rect::new(0.0, 0.0, 1100.0, 480.0);
        let left = left_group(&chips);
        assert!(left < left_group(&DEFAULT_CHIPS));
        let layout = Layout::with_shells(area, true, false, None, false, false, false, false, 1, left, None);
        assert!(!layout.stacked);
        let chat = Chat { layout, ..Default::default() };
        let mut hits = Vec::new();
        Draw {
            pal: app.pal(),
            focused: false,
            wide: true,
            hover: None,
            hits: &mut hits,
            pins: &[],
            chips: &chips,
            own: &own,
        }
        .toolbar(&mut p, &chat, &ChatInfo::default());
        let order: Vec<_> = hits.iter().map(|(_, t)| *t).collect();
        assert_eq!(
            order[..3],
            [Target::Chat(Control::Clear), Target::Chat(Control::Model), Target::Chat(Control::Own(0))],
            "{order:?}"
        );
        assert!(!order.contains(&Target::Chat(Control::Own(2))), "a name too long for a chip is left out");
        assert!(!order.contains(&Target::Chat(Control::Compact)));
        let own_chip = hits.iter().find(|(_, t)| *t == Target::Chat(Control::Own(0))).unwrap().0;
        let attach = hits.iter().find(|(_, t)| *t == Target::Chat(Control::Attach)).unwrap().0;
        assert!(own_chip.right() <= attach.x);
        // On a narrow toolbar an own command that does not fit is not drawn over the right group.
        let narrow = Layout::with_shells(
            Rect::new(0.0, 0.0, 760.0, 480.0),
            false,
            false,
            None,
            false,
            false,
            false,
            false,
            1,
            left_group(&DEFAULT_CHIPS),
            None,
        );
        let chat = Chat { layout: narrow, ..Default::default() };
        let mut hits = Vec::new();
        Draw {
            pal: app.pal(),
            focused: false,
            wide: false,
            hover: None,
            hits: &mut hits,
            pins: &[],
            chips: &DEFAULT_CHIPS,
            own: &own,
        }
        .toolbar(&mut p, &chat, &ChatInfo::default());
        for (r, t) in &hits {
            assert!(r.right() <= narrow.toolbar.right() + 0.5, "{t:?} leaves the toolbar");
        }
        let settings = claude_switch_core::config::ChatPrefs {
            chips: vec!["clear".into(), "from-a-newer-version".into(), "model".into()],
            ..Default::default()
        };
        assert_eq!(chips_of(&settings), chips);
    }

    #[test]
    fn changing_session_values_does_not_move_toolbar_controls() {
        use crate::view::chat::effort::Start;
        let mut gfx = Gfx::new();
        let app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
        let mut bitmap = Bitmap::new(1400, 480);
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        for width in [760.0, 1400.0] {
            let layout = Layout::new(Rect::new(0.0, 0.0, width, 480.0), true, false, None, false, false);
            let mut positions = Vec::new();
            for (level, context) in [("low", 1), ("medium", 999_999), ("max", u64::MAX)] {
                let mut chat = Chat { layout, ..Default::default() };
                chat.set_starting_effort(Start::Set(level));
                let mut info = ChatInfo::default();
                info.context = Some(context);
                let mut hits = Vec::new();
                Draw {
                    pal: app.pal(),
                    focused: false,
                    wide: width >= 900.0,
                    hover: None,
                    hits: &mut hits,
                    pins: &[],
                    chips: &DEFAULT_CHIPS,
                    own: &[],
                }
                .toolbar(&mut p, &chat, &info);
                positions.push(hits);
            }
            assert_eq!(positions[0], positions[1]);
            assert_eq!(positions[1], positions[2]);
        }
    }

    #[test]
    fn narrow_and_wide_composers_rasterize_inside_box_and_toolbar_hits_do_not_overlap() {
        let mut gfx = Gfx::new();
        let app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
        let pal = app.pal();
        let mut info = ChatInfo::default();
        info.model = Some("Demo Sonnet 5.5 (default)".into());
        info.context = Some(12_321);
        for width in [760, 900, 987, 988, 999, 1100] {
            let area = Rect::new(0.0, 0.0, width as f32, 480.0);
            let layout = Layout::new(area, true, false, None, true, true);
            let mut chat =
                Chat { layout, last_gate: Some(Gate::Check), error: Some(submission::CHECK), ..Default::default() };
            chat.chips.push(Chip::Clipboard(clipboard::Picture {
                seq: 42,
                thumbnail: Some(clipboard::Thumbnail { rgba: vec![161, 75, 32, 255], w: 1, h: 1 }),
            }));
            chat.chips.push(Chip::Path("fixture.txt".into()));
            chat.editor.set("/model".into());
            chat.commands.open = true;
            let pins = ["model".into()];
            let suggestions = chat.suggestion_plan(&pins);
            let popup = suggestions.as_ref().unwrap();
            assert_eq!(popup.pointer_behind((popup.bounds.cx(), popup.bounds.cy())), (-1.0, -1.0));
            let above = (popup.bounds.cx(), popup.bounds.y - 1.0);
            assert_eq!(popup.pointer_behind(above), above);
            let mut hits = Vec::new();
            let mut bitmap = Bitmap::new(width, 480);
            let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
            assert_eq!(model_label(&mut p, info.model.as_deref().unwrap()), "Sonnet 5.5 ▾");
            assert_eq!(model_label(&mut p, "claude-sonnet-5-5"), "Sonnet 5.5 ▾");
            p.fill(area, pal.page);
            Draw {
                pal,
                focused: true,
                wide: width >= 900,
                hover: None,
                hits: &mut hits,
                pins: &pins,
                chips: &DEFAULT_CHIPS,
                own: &[],
            }
            .composer(&mut p, &mut chat, &info, suggestions, None);
            let tool = hits.iter().filter(|(r, _)| layout.toolbar.contains(r.cx(), r.cy())).collect::<Vec<_>>();
            // The mode chip shows only where the row has room for it.
            let mode = tool.iter().filter(|(_, t)| *t == Target::Chat(Control::Mode)).count();
            assert_eq!(tool.len() - mode, if width >= 900 { 10 } else { 9 });
            for (i, (a, _)) in tool.iter().enumerate() {
                assert!(layout.box_frame.contains(a.x, a.y));
                for (b, _) in tool.iter().skip(i + 1).copied() {
                    assert!(a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y);
                }
            }
            assert!(hits.iter().any(|(r, t)| *t == Target::Chat(Control::Remove(0))
                && layout.attachments.contains(r.cx(), r.cy())));
            assert!(hits.iter().any(|(_, t)| *t == Target::Chat(Control::Override)));
            assert_eq!(hits.iter().filter(|(_, t)| matches!(t, Target::Chat(Control::Argument(_, _)))).count(), 6);
            let pixel =
                ((layout.attachments.y + 21.0) as usize * width as usize + (layout.attachments.x + 36.0) as usize) * 4;
            assert_eq!(&bitmap.pixels[pixel..pixel + 4], &[32, 75, 161, 255]);
            if width == 1100
                && let Some(root) = std::env::var_os("CS_COMPOSER_SHOTS")
            {
                bitmap.write_png(&std::path::Path::new(&root).join("chat-composer-dark.png")).unwrap();
            }
        }
    }
}
