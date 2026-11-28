//! Splits: a terminal tab divided into up to four panes, side by side or stacked, as a small
//! tree. The tab's own terminal (`Tab::term`, the program it was opened for) is pane 0 and stays
//! the one Chat, the session view, attention and the safeguard read; the others are shells
//! started in the tab's folder and live here. Keys, the mouse, paste and find go to the focused
//! pane. In Chat's layout only pane 0 is shown (in the drawer) and has the keyboard.

use std::time::Instant;

use mdshared_ui::gfx::{Painter, Rect, mix};
use mdshared_ui::theme::Palette;

use crate::dialogs::{Dialog, Pending};
use crate::state::AppState;
use crate::tabs::Tab;
use crate::term::Term;

/// A pane of a tab: 0 is the tab's own terminal.
pub type PaneId = u32;
pub const MAIN: PaneId = 0;
pub const MAX_PANES: usize = 4;
/// The gap between two panes, the divider's line in its middle.
pub const GAP: f32 = 6.0;
/// The least a pane keeps along the axis it was split on, in logical px.
const MIN_SIDE: f32 = 60.0;

/// Which way a pane is split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side, a vertical divider ("Split right").
    Right,
    /// Stacked, a horizontal divider ("Split down").
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    /// Splits the leaf `at` into it and `new` (after it). True when `at` was found.
    fn split(&mut self, at: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Node::Leaf(id) if *id == at => {
                *self = Node::Split { axis, ratio: 0.5, a: Box::new(Node::Leaf(at)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(at, new, axis) || b.split(at, new, axis),
        }
    }

    /// Takes the leaf `id` out; its sibling takes the parent's place. False when it is the root.
    fn remove(&mut self, id: PaneId) -> bool {
        let Node::Split { a, b, .. } = self else { return false };
        if **a == Node::Leaf(id) {
            *self = std::mem::replace(b, Node::Leaf(MAIN));
            return true;
        }
        if **b == Node::Leaf(id) {
            *self = std::mem::replace(a, Node::Leaf(MAIN));
            return true;
        }
        a.remove(id) || b.remove(id)
    }

    /// The rectangle of each leaf in `area` and each split's divider, splits numbered in
    /// pre-order.
    fn layout(&self, area: Rect, leaves: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(id) => leaves.push((*id, area)),
            Node::Split { axis, ratio, a, b } => {
                let (ra, gap, rb) = split_rect(area, *axis, *ratio);
                dividers.push(Divider { rect: gap, axis: *axis, area });
                a.layout(ra, leaves, dividers);
                b.layout(rb, leaves, dividers);
            }
        }
    }

    /// The `n`th split in pre-order.
    fn nth_split(&mut self, n: &mut usize) -> Option<&mut f32> {
        let Node::Split { ratio, a, b, .. } = self else { return None };
        if *n == 0 {
            return Some(ratio);
        }
        *n -= 1;
        if let Some(r) = a.nth_split(n) {
            return Some(r);
        }
        b.nth_split(n)
    }
}

fn split_rect(area: Rect, axis: Axis, ratio: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::Right => {
            let w = ((area.w - GAP) * ratio).round().max(0.0);
            let a = Rect::new(area.x, area.y, w, area.h);
            let gap = Rect::new(a.right(), area.y, GAP, area.h);
            (a, gap, Rect::new(gap.right(), area.y, (area.right() - gap.right()).max(0.0), area.h))
        }
        Axis::Down => {
            let h = ((area.h - GAP) * ratio).round().max(0.0);
            let a = Rect::new(area.x, area.y, area.w, h);
            let gap = Rect::new(area.x, a.bottom(), area.w, GAP);
            (a, gap, Rect::new(area.x, gap.bottom(), area.w, (area.bottom() - gap.bottom()).max(0.0)))
        }
    }
}

/// A divider between two panes: its gap and the area of the split it divides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub axis: Axis,
    pub area: Rect,
}

/// The panes of a tab beyond its own terminal, how they are arranged and which has the keyboard.
pub struct Panes {
    pub extra: Vec<(PaneId, Term)>,
    /// The folder each shell pane was started in, kept for reopening the tab (`saved`).
    pub folders: Vec<(PaneId, std::path::PathBuf)>,
    pub tree: Node,
    pub focus: PaneId,
    next: PaneId,
    /// Where each pane and divider was laid out last (the shown tab's, for the pointer).
    pub rects: Vec<(PaneId, Rect)>,
    pub dividers: Vec<Divider>,
    /// The divider being dragged, by its number.
    pub drag: Option<usize>,
}

impl Default for Panes {
    fn default() -> Panes {
        Panes {
            extra: Vec::new(),
            folders: Vec::new(),
            tree: Node::Leaf(MAIN),
            focus: MAIN,
            next: MAIN + 1,
            rects: Vec::new(),
            dividers: Vec::new(),
            drag: None,
        }
    }
}

impl Panes {
    pub fn is_split(&self) -> bool {
        !self.extra.is_empty()
    }

    pub fn count(&self) -> usize {
        self.extra.len() + 1
    }

    pub fn ids(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.tree.leaves(&mut out);
        out
    }

    pub fn get(&self, id: PaneId) -> Option<&Term> {
        self.extra.iter().find(|(p, _)| *p == id).map(|(_, t)| t)
    }

    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Term> {
        self.extra.iter_mut().find(|(p, _)| *p == id).map(|(_, t)| t)
    }

    /// Adds `term` beside the focused pane and focuses it. `None` when the tab is full.
    pub fn split(&mut self, axis: Axis, term: Term) -> Option<PaneId> {
        if self.count() >= MAX_PANES {
            return None;
        }
        let id = self.next;
        self.next += 1;
        if !self.tree.split(self.focus, id, axis) {
            return None;
        }
        self.extra.push((id, term));
        self.focus = id;
        Some(id)
    }

    /// Takes a pane other than the tab's own out; the focus goes to the pane that took its place.
    pub fn remove(&mut self, id: PaneId) -> Option<Term> {
        if id == MAIN {
            return None;
        }
        let order = self.ids();
        let at = order.iter().position(|p| *p == id)?;
        if !self.tree.remove(id) {
            return None;
        }
        let i = self.extra.iter().position(|(p, _)| *p == id)?;
        let (_, term) = self.extra.remove(i);
        self.folders.retain(|(p, _)| *p != id);
        if self.focus == id {
            // The pane before it in reading order, or the first.
            let left = self.ids();
            self.focus = left[at.saturating_sub(1).min(left.len() - 1)];
        }
        self.drag = None;
        Some(term)
    }

    /// Lays the panes out in `area`, keeping the places for the pointer.
    pub fn layout(&mut self, area: Rect) -> Vec<(PaneId, Rect)> {
        let (mut leaves, mut dividers) = (Vec::new(), Vec::new());
        self.tree.layout(area, &mut leaves, &mut dividers);
        self.rects = leaves.clone();
        self.dividers = dividers;
        leaves
    }

    pub fn at(&self, x: f32, y: f32) -> Option<PaneId> {
        self.rects.iter().find(|(_, r)| r.contains(x, y)).map(|(id, _)| *id)
    }

    pub fn divider_at(&self, x: f32, y: f32) -> Option<usize> {
        self.dividers.iter().position(|d| d.rect.inset(-1.0).contains(x, y))
    }

    /// Moves the dragged divider to the pointer. True when it moved.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(n) = self.drag else { return false };
        let Some(d) = self.dividers.get(n).copied() else { return false };
        let (pos, start, len) = match d.axis {
            Axis::Right => (x, d.area.x, d.area.w),
            Axis::Down => (y, d.area.y, d.area.h),
        };
        let span = len - GAP;
        if span <= 2.0 * MIN_SIDE {
            return false;
        }
        let ratio = ((pos - start - GAP / 2.0) / span).clamp(MIN_SIDE / span, 1.0 - MIN_SIDE / span);
        let Some(r) = self.tree.nth_split(&mut { n }) else { return false };
        let changed = (*r - ratio).abs() > f32::EPSILON;
        *r = ratio;
        changed
    }
}

impl Tab {
    /// The pane that has the keyboard: the tab's own in Chat, where only it is shown.
    pub fn focused_pane(&self) -> PaneId {
        if self.pane.as_ref().is_some_and(|p| p.chat.enabled) { MAIN } else { self.panes.focus }
    }

    pub fn focused_term(&self) -> &Term {
        let id = self.focused_pane();
        self.panes.get(id).unwrap_or(&self.term)
    }

    pub fn focused_term_mut(&mut self) -> &mut Term {
        let id = self.focused_pane();
        match self.panes.extra.iter_mut().find(|(p, _)| *p == id) {
            Some((_, t)) => t,
            None => &mut self.term,
        }
    }

    /// The focused pane's terminal and, when it is the tab's own, the Chat pane that watches it.
    pub fn focused_parts(&mut self) -> (&mut Term, Option<&mut crate::view::Pane>) {
        let id = self.focused_pane();
        let Tab { term, panes, pane, .. } = self;
        match panes.extra.iter_mut().find(|(p, _)| *p == id) {
            Some((_, t)) => (t, None),
            None => (term, pane.as_mut()),
        }
    }

    pub fn term_of(&self, id: PaneId) -> Option<&Term> {
        if id == MAIN { Some(&self.term) } else { self.panes.get(id) }
    }

    pub fn term_of_mut(&mut self, id: PaneId) -> Option<&mut Term> {
        if id == MAIN { Some(&mut self.term) } else { self.panes.get_mut(id) }
    }

    /// The tab's own program or a pane's still runs.
    pub fn any_running(&self) -> bool {
        self.term.running() || self.panes.extra.iter().any(|(_, t)| t.running())
    }

    /// The panes are shown: split, and not in Chat's layout.
    pub fn shows_panes(&self) -> bool {
        self.panes.is_split() && !self.pane.as_ref().is_some_and(|p| p.chat.enabled)
    }
}

/// Draws the dividers of a split tab and a subtle edge round the focused pane.
pub fn draw_frame(p: &mut Painter, pal: &Palette, panes: &Panes, focused: Option<PaneId>, hot: Option<usize>) {
    for (i, d) in panes.dividers.iter().enumerate() {
        p.fill(d.rect, pal.page);
        let active = hot == Some(i) || panes.drag == Some(i);
        let colour = if active { pal.accent } else { pal.line };
        let line = match d.axis {
            Axis::Right => Rect::new(d.rect.cx() - 0.5, d.rect.y, 1.0, d.rect.h),
            Axis::Down => Rect::new(d.rect.x, d.rect.cy() - 0.5, d.rect.w, 1.0),
        };
        p.fill(line, colour);
    }
    if let Some(r) = focused.and_then(|id| panes.rects.iter().find(|(p, _)| *p == id)).map(|(_, r)| *r) {
        let edge = mix(pal.line, pal.accent, 0.7);
        for side in [
            Rect::new(r.x, r.y, r.w, 1.0),
            Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0),
            Rect::new(r.x, r.y, 1.0, r.h),
            Rect::new(r.right() - 1.0, r.y, 1.0, r.h),
        ] {
            p.fill_a(side, edge, 0.85);
        }
    }
}

impl AppState {
    /// The folder a new pane of tab `id` starts in: the tab's launch folder, its session's, or
    /// the one a new shell tab would use.
    fn pane_folder(&self, id: u64) -> std::path::PathBuf {
        let tab = self.tabs.get(id);
        tab.and_then(|t| t.folder.clone())
            .or_else(|| self.tab_folder(id))
            .or_else(|| tab.and_then(|t| t.opencode.clone()))
            .filter(|c| self.source.is_demo() || c.is_dir())
            .unwrap_or_else(|| self.source.home())
    }

    /// Whether tab `id` can be split now: a terminal tab with room for another pane, not in
    /// Chat's layout.
    pub fn can_split(&self, id: u64) -> bool {
        self.tabs
            .get(id)
            .is_some_and(|t| t.panes.count() < MAX_PANES && !t.pane.as_ref().is_some_and(|p| p.chat.enabled))
    }

    /// Splits the focused pane of tab `id` and starts a shell in the new one.
    pub fn split_pane(&mut self, now: Instant, id: u64, axis: Axis) {
        if !self.can_split(id) {
            let full = self.tabs.get(id).is_some_and(|t| t.panes.count() >= MAX_PANES);
            let why = if full { "A tab holds up to four panes." } else { "Switch to the terminal layout to split it." };
            self.toast(why.into(), now);
            return;
        }
        let cwd = self.pane_folder(id);
        // About the half it will get, so the shell starts near its size (the next frame fits it).
        let size = self.tabs.get(id).map_or(self.grid, |t| {
            let s = &t.focused_term().screen;
            let (cols, rows) = (s.cols() as u16, s.rows() as u16);
            match axis {
                Axis::Right => ((cols.saturating_sub(1) / 2).max(1), rows),
                Axis::Down => (cols, (rows.saturating_sub(1) / 2).max(1)),
            }
        });
        let term = match self.shell_term(&cwd, size) {
            Ok(t) => t,
            Err(e) => return self.error("Could not start a shell", e),
        };
        let before = self.tabs.get(id).map(|t| t.focused_pane());
        let Some(tab) = self.tabs.get_mut(id) else { return };
        if let Some(pane) = tab.panes.split(axis, term) {
            tab.panes.folders.push((pane, cwd));
            self.pane_focus_moved(id, before);
        }
    }

    /// A pane's shell in `cwd` (in the demo, a terminal with no program that says so).
    pub(crate) fn shell_term(&self, cwd: &std::path::Path, size: (u16, u16)) -> Result<Term, String> {
        if self.source.is_demo() {
            let mut t = Term::empty(size);
            t.feed(format!("Demo: no shell started in {}\r\n", cwd.display()).as_bytes());
            return Ok(t);
        }
        let mut l = claude_switch_core::launch::shell(cwd);
        let spawned = crate::run::native_program(&l.program).and_then(|program| {
            l.program = program;
            Term::spawn(&crate::term::command(&l), size, self.term_wake())
        });
        spawned.map_err(|e| format!("{}: {e}", l.program.display()))
    }

    /// Closes the focused pane of tab `id`, asking while its program runs. The last pane closes
    /// the tab. The tab's own pane never closes alone, since Chat and the session follow it:
    /// with other panes open, closing it asks to close the whole tab.
    pub fn ask_close_pane(&mut self, id: u64) {
        let Some(tab) = self.tabs.get(id) else { return };
        if !tab.panes.is_split() {
            return self.ask_close_tab(id);
        }
        let pane = tab.focused_pane();
        if pane == MAIN {
            // Chat, the session view and the safeguard follow this pane: it never goes alone.
            // Always asked, running or not, since the other panes go with it.
            let others = tab.panes.count() - 1;
            let panes = if others == 1 { "the other pane".to_string() } else { format!("the {others} other panes") };
            let why = if tab.pane.is_some() { "Chat and the session view need this pane, so " } else { "" };
            self.dialog = Some(Dialog::Confirm {
                title: "Close the whole tab?".into(),
                text: format!("{why}closing it closes the tab with {panes}. Their programs end."),
                ok: "Close tab".into(),
                then: Pending::CloseTab(id),
            });
            return;
        }
        if tab.panes.get(pane).is_some_and(Term::running) {
            self.dialog = Some(Dialog::Confirm {
                title: "Close this pane?".into(),
                text: "Its program ends.".into(),
                ok: "Close".into(),
                then: Pending::ClosePane(id, pane),
            });
        } else {
            self.close_pane_now(id, pane);
        }
    }

    /// Closes a pane; dropping its terminal ends its program.
    pub fn close_pane_now(&mut self, id: u64, pane: PaneId) {
        let Some(tab) = self.tabs.get_mut(id) else { return };
        let before = Some(tab.focused_pane());
        if tab.panes.remove(pane).is_some() {
            self.pane_focus_moved(id, before.filter(|b| *b != pane));
        }
    }

    /// Gives the keyboard to pane `to` of the shown tab.
    pub fn focus_pane(&mut self, to: PaneId) {
        let Some(id) = self.tabs.active else { return };
        let Some(tab) = self.tabs.get_mut(id) else { return };
        let before = tab.focused_pane();
        if before == to || tab.term_of(to).is_none() {
            return;
        }
        tab.panes.focus = to;
        self.pane_focus_moved(id, Some(before));
    }

    /// After the focus moved between panes of tab `id`: tells the programs that asked, and
    /// points the mouse's grid at the focused pane.
    fn pane_focus_moved(&mut self, id: u64, before: Option<PaneId>) {
        self.clear_session_text_focus();
        let focused = self.window_focused && self.tabs.active == Some(id);
        let Some(tab) = self.tabs.get_mut(id) else { return };
        let now = tab.focused_pane();
        if let Some(t) = before.and_then(|b| tab.term_of_mut(b)) {
            t.stop_pointer();
            // One selection per tab: the one copy and right-click act on is the only one shown.
            t.selection = None;
            // Its find bar stays open but leaves the keyboard with the pane, so coming back
            // (Next pane, a closed neighbour) types into the program again.
            if let Some(find) = &mut t.find {
                find.focused = false;
            }
            if focused && let Some(b) = mdshared_term::input::encode_focus(false, &t.screen.modes()) {
                t.write(&b);
            }
        }
        if focused
            && let Some(t) = tab.term_of(now)
            && let Some(b) = mdshared_term::input::encode_focus(true, &t.screen.modes())
        {
            t.write(&b);
        }
        if let Some(r) = tab.panes.rects.iter().find(|(p, _)| *p == now).map(|(_, r)| *r) {
            self.term_area = crate::termapp::inner(r);
        }
    }

    /// A press on the terminal of the shown tab: the pane under it takes the keyboard.
    pub fn press_pane(&mut self, x: f32, y: f32) {
        let Some(tab) = self.tabs.shown() else { return };
        if !tab.shows_panes() {
            return;
        }
        if let Some(to) = tab.panes.at(x, y) {
            self.focus_pane(to);
        }
    }

    /// A press on a divider starts its drag. True when there was one.
    pub fn press_divider(&mut self, x: f32, y: f32) -> bool {
        let Some(tab) = self.tabs.shown_mut().filter(|t| t.shows_panes()) else { return false };
        tab.panes.drag = tab.panes.divider_at(x, y);
        tab.panes.drag.is_some()
    }

    pub fn divider_dragging(&self) -> bool {
        self.tabs.shown().is_some_and(|t| t.panes.drag.is_some())
    }

    pub fn drag_divider(&mut self, x: f32, y: f32) -> bool {
        self.tabs.shown_mut().is_some_and(|t| t.panes.drag_to(x, y))
    }

    pub fn pane_divider_axis(&self, i: usize) -> Option<Axis> {
        self.tabs.shown()?.panes.dividers.get(i).map(|d| d.axis)
    }

    /// Gives the keyboard to the next pane in reading order, round from the last.
    pub fn focus_next_pane(&mut self) {
        let Some(tab) = self.tabs.shown().filter(|t| t.shows_panes()) else { return };
        let ids = tab.panes.ids();
        let at = ids.iter().position(|p| *p == tab.panes.focus).unwrap_or(0);
        self.focus_pane(ids[(at + 1) % ids.len()]);
    }

    /// The wheel over a pane that has not the keyboard scrolls that pane's own scrollback.
    /// True when it was such a pane.
    pub fn wheel_other_pane(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let Some(tab) = self.tabs.shown_mut().filter(|t| t.shows_panes()) else { return false };
        let Some(id) = tab.panes.at(x, y).filter(|id| *id != tab.panes.focus) else { return false };
        let Some(t) = tab.term_of_mut(id) else { return false };
        let lines = crate::termkeys::wheel_lines(dy, &mut t.wheel_rest);
        t.scroll_by(lines);
        true
    }

    pub fn release_divider(&mut self) -> bool {
        self.tabs.shown_mut().and_then(|t| t.panes.drag.take()).is_some()
    }
}

pub mod saved;

#[cfg(test)]
mod tests;
