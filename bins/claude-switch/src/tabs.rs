//! The tab strip under the top bar: the fixed "Sessions" tab, then one tab a terminal. The model
//! ([`Tabs`]: open, close, switch, which tab holds a session, titles) and the strip's drawing,
//! with the toolkit's `chrome::draw_tab` moved down under the title row. The strip is hidden
//! while there is no terminal tab.

use std::borrow::Cow;
use std::path::PathBuf;

use mdshared_ui::anim::{Animator, Id, document};
use mdshared_ui::chrome::{self, TITLE_H, TabState};
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb, mix};
use mdshared_ui::glyph;
use mdshared_ui::theme::Palette;

use crate::state::Target;
use crate::term::Term;

/// The strip's height: as tall as the title row, whose tabs `draw_tab` draws.
pub const STRIP_H: f32 = TITLE_H;
/// The Sessions tab's id in [`Target::Tab`]; terminal tabs count from 1.
pub const SESSIONS: u64 = 0;
const LEFT: f32 = 12.0;
const ENDED_SUFFIX: &str = " (ended)";

/// One terminal tab.
pub struct Tab {
    pub id: u64,
    /// What the launch called it (`Launch::label`).
    pub label: String,
    /// The title the program set (OSC 0 or 2); empty when it set none.
    pub program_title: String,
    pub rename: Option<String>,
    pub color: [u8; 3],
    /// Current session for list routing, badges and seen marks; never the pane's pinned identity.
    pub session: Option<String>,
    pub account: Option<String>,
    pub signing_in: bool,
    /// Launch folder for a Claude session; shell and sign-in tabs leave it unset.
    pub cwd: Option<PathBuf>,
    /// A resume argument or a live record matched to this tab's process, not a pane guess.
    pub resume_id: Option<String>,
    pub term: Term,
    pub pane: Option<crate::view::Pane>,
    pub attention: crate::attention::TabAttention,
}

impl Tab {
    pub(crate) fn needs_identity(&self) -> bool {
        self.pane.as_ref().is_some_and(|pane| pane.chat.identity_unverified)
    }

    pub(crate) fn apply_live_identity(&mut self, found: Option<&str>) -> bool {
        let Some(pane) = &mut self.pane else { return false };
        let pane_changed = pane.update_identity(found);
        let verified = found.filter(|_| !pane.chat.identity_unverified);
        // An absent PID record must not permit a duplicate resume of an already open tab.
        // A conflicting positive record, on the other hand, clears that routing claim.
        let current = if found.is_none() { self.session.as_deref() } else { verified };
        let changed = self.session.as_deref() != current || self.resume_id.as_deref() != verified;
        if changed {
            self.session = current.map(str::to_owned);
            self.resume_id = verified.map(str::to_owned);
            self.attention.clear_transcript();
        }
        changed || pane_changed
    }

    /// The running program's title or launch label; after exit, identify the original launch.
    pub fn title(&self) -> Cow<'_, str> {
        if self.term.ended.is_some() {
            return format!("{}{}", self.label, ENDED_SUFFIX).into();
        }
        let t = self.program_title.trim();
        if t.is_empty() { self.label.as_str().into() } else { t.into() }
    }
}

/// What a new tab is, apart from its terminal.
pub struct NewTab {
    pub label: String,
    pub color: [u8; 3],
    pub session: Option<String>,
    pub account: Option<String>,
}

#[derive(Default)]
pub struct Tabs {
    pub list: Vec<Tab>,
    /// The tab shown: `None` is the Sessions tab.
    pub active: Option<u64>,
    next: u64,
    pub drag: Option<crate::tabcontrols::Drag>,
}

impl Tabs {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Opens a tab and shows it. Its id.
    pub fn open(&mut self, new: NewTab, term: Term) -> u64 {
        self.next += 1;
        let id = self.next;
        let NewTab { label, color, session, account } = new;
        self.list.push(Tab {
            id,
            label,
            program_title: String::new(),
            rename: None,
            color,
            resume_id: session.clone(),
            session,
            account,
            signing_in: false,
            cwd: None,
            term,
            pane: None,
            attention: Default::default(),
        });
        self.active = Some(id);
        id
    }

    pub fn index(&self, id: u64) -> Option<usize> {
        self.list.iter().position(|t| t.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Tab> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.list.iter_mut().find(|t| t.id == id)
    }

    /// The terminal tab shown, if one is.
    pub fn shown(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn shown_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active?;
        self.get_mut(id)
    }

    /// Removes a tab (which ends its program when the tab is dropped). The tab shown next is the
    /// one before it, or the Sessions tab.
    pub fn close(&mut self, id: u64) -> Option<Tab> {
        let i = self.index(id)?;
        let tab = self.list.remove(i);
        if self.active == Some(id) {
            self.active = if i == 0 { None } else { Some(self.list[i - 1].id) };
        }
        Some(tab)
    }

    /// Shows a tab ([`SESSIONS`] for the Sessions tab). False for an id that is not there.
    pub fn activate(&mut self, id: u64) -> bool {
        if id == SESSIONS {
            self.active = None;
            return true;
        }
        let there = self.index(id).is_some();
        if there {
            self.active = Some(id);
        }
        there
    }

    /// Shows the next tab to the right (`forward`) or to the left, round past the ends; the
    /// Sessions tab is one of them.
    pub fn step(&mut self, forward: bool) {
        let n = self.list.len() + 1;
        let at = self.active.and_then(|id| self.index(id)).map_or(0, |i| i + 1);
        let to = if forward { (at + 1) % n } else { (at + n - 1) % n };
        self.active = if to == 0 { None } else { Some(self.list[to - 1].id) };
    }

    pub fn numbered(&self, number: usize) -> Option<u64> {
        if number == 1 { Some(SESSIONS) } else { self.list.get(number.checked_sub(2)?).map(|tab| tab.id) }
    }

    pub fn by_session(&self, session: &str, eligible: impl Fn(&Tab) -> bool) -> Option<&Tab> {
        self.list.iter().find(|t| t.session.as_deref() == Some(session) && eligible(t))
    }

    /// Moves tab `id` to just right of `anchor`; false if either is missing.
    pub fn place_after(&mut self, id: u64, anchor: u64) -> bool {
        let (Some(from), Some(_)) = (self.index(id), self.index(anchor)) else { return false };
        if id == anchor {
            return false;
        }
        let tab = self.list.remove(from);
        let to = self.index(anchor).map_or(self.list.len(), |i| i + 1);
        self.list.insert(to, tab);
        true
    }

    /// How many tabs have a program that still runs.
    pub fn running(&self) -> usize {
        self.list.iter().filter(|t| t.term.running()).count()
    }
}

/// The strip's rectangle at `top` in a window `w` wide.
pub fn strip_rect(top: f32, w: f32) -> Rect {
    Rect::new(0.0, top, w, STRIP_H)
}

/// The state that drawing reads.
pub struct Look {
    pub hover: Option<Target>,
    pub pressed: Option<Target>,
    pub terminal: Option<(Rgb, Rgb, Rgb)>,
    pub session_view: Option<bool>,
    pub chat: Option<bool>,
}

/// Elide the launch label, not its ended marker, when the tab's text area is narrow.
fn fit_ended_title(title: &str, width: f32, mut measure: impl FnMut(&str) -> f32) -> Cow<'_, str> {
    if measure(title) <= width {
        return title.into();
    }
    let label = title.strip_suffix(ENDED_SUFFIX).unwrap_or(title);
    let marker = ENDED_SUFFIX.trim_start();
    if measure("… (ended)") > width {
        return marker.into();
    }
    let ends: Vec<_> = label.char_indices().map(|(i, _)| i).collect();
    let n = ends.partition_point(|&i| measure(&format!("{}…{ENDED_SUFFIX}", &label[..i])) <= width);
    let end = n.checked_sub(1).map_or(0, |i| ends[i]);
    format!("{}…{ENDED_SUFFIX}", label[..end].trim_end()).into()
}

/// Draws the strip at `top` and records its hits. The tabs are the toolkit's title-row tabs
/// drawn with the painter moved down; each one's document glyph is covered by the glyph of the
/// Sessions tab or the account's colour.
pub fn draw(
    p: &mut Painter,
    pal: &Palette,
    anim: &mut Animator,
    (tabs, marks): (&Tabs, &claude_switch_core::marks::Marks),
    strip: Rect,
    look: &Look,
    hits: &mut Vec<(Rect, Target)>,
) -> bool {
    let (top, w) = (strip.y, strip.w);
    let mut moving = false;
    p.fill(strip, pal.title);
    p.fill(Rect::new(0.0, strip.bottom() - p.px(), w, p.px()), pal.line);
    let mut labels: Vec<_> = vec![(SESSIONS, Cow::Borrowed("Sessions"), None, false)];
    labels.extend(tabs.list.iter().map(|t| {
        let ended = t.term.ended.is_some();
        let title = match t.rename.as_deref().or_else(|| t.session.as_deref().and_then(|id| marks.name(id))) {
            Some(name) if ended => format!("{name}{ENDED_SUFFIX}").into(),
            Some(name) => Cow::Borrowed(name),
            None => t.title(),
        };
        (t.id, title, Some(t.color), ended)
    }));
    let natural: Vec<f32> = labels.iter().map(|(_, l, _, _)| chrome::tab_natural_width(p, l)).collect();
    let chat_reserve = if look.chat.is_some() { crate::view::chat::STRIP_RESERVE } else { 0.0 };
    let avail = (w - 2.0 * LEFT - chat_reserve - if look.session_view.is_some() { 112.0 } else { 0.0 }).max(0.0);
    let widths = chrome::tab_widths(&natural, avail);
    let active = tabs.active.unwrap_or(SESSIONS);
    let hovered = |id: u64| matches!(look.hover, Some(Target::Tab(t) | Target::TabClose(t)) if t == id);
    let origin = p.offset(0.0, top);
    let clip = p.clip(Rect::new(0.0, 0.0, w, STRIP_H));
    let mut x = LEFT;
    for (k, (id, label, color, ended)) in labels.iter().enumerate() {
        let tw = widths[k];
        let next_quiet = labels.get(k + 1).is_some_and(|(n, _, _, _)| *n != active && !hovered(*n));
        let s = TabState {
            active: *id == active,
            hovered: hovered(*id),
            dragging: tabs.drag.is_some_and(|drag| drag.id == *id && drag.started),
            missing: false,
            next_quiet: next_quiet && *id != active && !hovered(*id),
            close_hovered: *id != SESSIONS && look.hover == Some(Target::TabClose(*id)),
            close_pressed: *id != SESSIONS && look.pressed == Some(Target::TabClose(*id)),
            pinned: false,
        };
        let active_pal = look
            .terminal
            .filter(|_| s.active && *id != SESSIONS)
            .map(|colors| crate::termtheme::active_tab_palette(pal, colors));
        let pal = active_pal.as_ref().unwrap_or(pal);
        let fitted;
        let label = if *ended && tw >= 64.0 {
            // Match the toolkit's text face and space reserved for the icon and close button.
            let face = if s.active { Face::Semibold } else { Face::Ui };
            let width = tw - if s.active || s.hovered { 64.0 } else { 46.0 };
            fitted = fit_ended_title(label, width, |text| p.measure(face, 12.5, text));
            &fitted
        } else {
            label
        };
        let hover_id = Id::new(("terminal-tab", id));
        let drawn = chrome::draw_tab_animated(p, pal, anim, hover_id, x, tw, w, label, &s);
        let fade = anim.get::<f32>(hover_id.with("hover")).unwrap_or(f32::from(u8::from(s.hovered)));
        moving |= !s.active && fade != f32::from(u8::from(s.hovered));
        let under = if s.active { pal.bar } else { mix(pal.title, pal.hover, fade) };
        // The glyph: the list's for the Sessions tab, the account's colour for a terminal.
        if tw >= 64.0 {
            let (gx, gy) = (x + 19.0, 23.0);
            p.fill(Rect::new(gx - 8.0, gy - 8.0, 16.0, 16.0), under);
            match color {
                None => p.icon(glyph::OUTLINE, 13.0, gx, gy, if s.active { pal.accent } else { pal.muted }),
                Some(c) => p.rbox(Rect::new(gx - 4.5, gy - 4.5, 9.0, 9.0), 4.5, *c),
            }
        }
        let mark = anim.toggle(("tab-mark", id), s.active, document::HOVER);
        moving |= mark != f32::from(u8::from(s.active));
        if mark > 0.0 && tw >= 30.0 {
            let color = if s.active { pal.accent } else { mdshared_ui::tokens::legible(pal.accent, under, 3.0) };
            p.rbox_a(Rect::new(x + 12.0, TITLE_H - 5.0, 16.0, 2.0), 1.0, color, mark);
        }
        if tw >= 28.0 && tabs.get(*id).is_some_and(|t| t.attention.marked) {
            let warn = mdshared_ui::tokens::legible(pal.warn, under, 3.0);
            p.rbox(Rect::new(x + 8.0, 7.0, 7.0, 7.0), 3.5, warn);
        }
        let tab = Rect::new(drawn.tab.x, drawn.tab.y + top, drawn.tab.w, drawn.tab.h);
        hits.push((tab, Target::Tab(*id)));
        match drawn.close {
            // The Sessions tab cannot be closed: its close button is covered again.
            Some(c) if *id == SESSIONS => p.fill(c, under),
            Some(c) => hits.push((Rect::new(c.x, c.y + top, c.w, c.h), Target::TabClose(*id))),
            None => {}
        }
        x += tw;
    }
    p.restore(clip);
    p.restore_offset(origin);
    if let Some(open) = look.session_view {
        let r = Rect::new(strip.right() - 117.0 - chat_reserve, strip.y + 7.0, 105.0, strip.h - 14.0);
        let target = Target::SessionToggle;
        let bg = if look.pressed == Some(target) || look.hover == Some(target) {
            pal.hover
        } else if open {
            pal.on
        } else {
            pal.bar
        };
        p.rbox(r, 5.0, bg);
        p.text_center(Face::Semibold, 12.0, r, "Session view", pal.text);
        hits.push((r, target));
    }
    if let Some(chat) = look.chat {
        crate::view::chat::strip_button(p, pal, strip, chat, look.hover, hits);
    }
    moving
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(label: &str, session: Option<&str>) -> NewTab {
        NewTab {
            label: label.into(),
            color: [1, 2, 3],
            session: session.map(str::to_string),
            account: Some("main".into()),
        }
    }

    fn three() -> (Tabs, [u64; 3]) {
        let mut t = Tabs::default();
        let a = t.open(new("a", Some("s-a")), Term::empty((10, 3)));
        let b = t.open(new("b", None), Term::empty((10, 3)));
        let c = t.open(new("c", Some("s-c")), Term::empty((10, 3)));
        (t, [a, b, c])
    }

    #[test]
    fn a_handover_tab_moves_beside_its_source_and_stays_shown() {
        let (mut t, [a, b, c]) = three();
        assert!(t.place_after(c, a));
        assert_eq!(t.list.iter().map(|x| x.id).collect::<Vec<_>>(), [a, c, b]);
        assert_eq!(t.active, Some(c));
        assert!(t.place_after(a, b));
        assert_eq!(t.list.iter().map(|x| x.id).collect::<Vec<_>>(), [c, b, a]);
        assert!(!t.place_after(a, a) && !t.place_after(a, 99) && !t.place_after(99, a));
    }

    #[test]
    fn opening_shows_the_new_tab_and_closing_shows_the_one_before() {
        let (mut t, [a, b, c]) = three();
        assert_eq!(t.active, Some(c));
        assert_eq!(t.close(b).map(|x| x.label), Some("b".into()));
        assert_eq!(t.active, Some(c), "closing a tab not shown leaves the shown one");
        t.close(c);
        assert_eq!(t.active, Some(a));
        t.close(a);
        assert_eq!(t.active, None, "the first tab gives way to Sessions");
        assert!(t.is_empty());
        assert!(t.close(a).is_none());
    }

    #[test]
    fn stepping_goes_round_through_the_sessions_tab() {
        let (mut t, [a, b, c]) = three();
        t.step(true);
        assert_eq!(t.active, None);
        t.step(true);
        assert_eq!(t.active, Some(a));
        t.step(false);
        t.step(false);
        assert_eq!(t.active, Some(c));
        t.step(false);
        assert_eq!(t.active, Some(b));
        assert!(t.activate(SESSIONS));
        assert_eq!(t.active, None);
        assert!(!t.activate(99));
        assert!(t.activate(a));
        assert_eq!(t.shown().map(|x| x.label.as_str()), Some("a"));
    }

    #[test]
    fn a_session_is_found_by_its_tab_until_the_tab_closes() {
        let (mut t, [a, _, c]) = three();
        assert_eq!(t.by_session("s-c", |_| true).map(|x| x.id), Some(c));
        assert_eq!(t.by_session("s-a", |_| true).map(|x| x.id), Some(a));
        assert!(t.by_session("other", |_| true).is_none());
        t.close(c);
        assert!(t.by_session("s-c", |_| true).is_none());
        assert_eq!(t.running(), 0, "terminals without a program do not run");
    }

    #[test]
    fn the_program_title_replaces_the_label_while_it_has_one() {
        let (mut t, [a, ..]) = three();
        let tab = t.get_mut(a).unwrap();
        assert_eq!(tab.title(), "a");
        tab.term.feed(b"\x1b]0;vim notes.md\x07");
        let set = tab.term.screen.take_title();
        assert_eq!(set.as_deref(), Some("vim notes.md"));
        tab.program_title = set.unwrap();
        assert_eq!(tab.title(), "vim notes.md");
        tab.program_title = "  ".into();
        assert_eq!(tab.title(), "a");
    }

    #[test]
    fn an_ended_tab_shows_the_launch_label_even_when_the_program_set_a_title() {
        let (mut t, [a, ..]) = three();
        let tab = t.get_mut(a).unwrap();
        tab.program_title = "a different title".into();
        tab.term.ended = Some(mdshared_pty::Exit { code: Some(0), signal: None });
        assert_eq!(tab.title(), "a (ended)");
    }

    #[test]
    fn a_long_ended_title_keeps_its_marker_as_the_tab_narrows() {
        let measure = |s: &str| s.chars().count() as f32;
        let title = "Çok uzun bir etiket (ended)";
        assert_eq!(fit_ended_title(title, 30.0, measure), title);
        assert_eq!(fit_ended_title(title, 12.0, measure), "Çok… (ended)");
        assert_eq!(fit_ended_title(title, 8.0, measure), "(ended)");
    }
}
