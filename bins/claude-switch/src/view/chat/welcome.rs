//! A new session before its first message: the mark, one line that names the folder, what
//! Claude Code's own banner says it runs (the model and plan, the folder), the message box in the
//! middle of the window, and a few starts under it. Everything shown is read: the folder is the
//! tab's, the model and plan are the banner's rows on the terminal's screen (left out when they
//! are not there), and the starts are the user's starred commands or commands of the local list.
//! A start fills the message box; it sends nothing.
use std::path::Path;

use mdshared_term::Screen;
use mdshared_ui::gfx::{Face, Painter, Rect};
use mdshared_ui::theme::Palette;

use super::{Chat, Control};
use crate::state::Target;
use crate::view::look;

/// The greeting over the message box: the mark, the line, the facts, and the air under them.
pub(super) const GREETING: f32 = 146.0;
/// The starts under the box, with the air over them.
pub(super) const STARTS: f32 = 48.0;
/// The message box's placeholder in a new session.
pub(super) const HINT: &str = "Describe a task, / for commands, @ for files";
/// The commands offered when none is starred, where the local list has them.
const DEFAULT_STARTS: [&str; 3] = ["init", "review", "resume"];
/// The most starts shown.
const MAX_STARTS: usize = 5;

/// What a start does when clicked. Nothing is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Fill the message box with this command of the list (by its place in the list).
    Command(usize),
    /// Attach the clipboard's picture, as "Paste picture" in the Attach menu does.
    Paste,
}

/// What Claude Code's banner says: its model and plan row, and its folder row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Banner {
    pub model: String,
    pub folder: String,
}

/// The text of a banner row after the logo: the row must hold nothing else before it.
fn after_logo(row: &str) -> Option<&str> {
    let row = row.trim_end();
    let logo = |c: char| c.is_whitespace() || ('\u{2580}'..='\u{259f}').contains(&c);
    let start = row.char_indices().find(|(_, c)| !logo(*c)).map(|(at, _)| at)?;
    // A logo is drawn before the text, and at least two spaces part them.
    let before = &row[..start];
    (before.chars().any(|c| !c.is_whitespace()) && before.ends_with("  ")).then(|| row[start..].trim())
}

/// Reads the banner Claude Code draws when it starts: "Claude Code v…" beside its logo, then the
/// model and plan, then the folder. `None` when the screen does not show exactly that.
pub(super) fn banner(screen: &Screen) -> Option<Banner> {
    let rows: Vec<String> = (0..screen.rows()).map(|row| screen.row_text(row)).collect();
    let at = rows.iter().position(|row| after_logo(row).is_some_and(|text| text.starts_with("Claude Code v")))?;
    let model = after_logo(rows.get(at + 1)?)?;
    let folder = after_logo(rows.get(at + 2)?)?;
    let path = folder.starts_with('~') || folder.contains(['\\', '/']);
    (!model.is_empty() && model.chars().count() <= 80 && path && folder.chars().count() <= 200)
        .then(|| Banner { model: model.to_owned(), folder: folder.to_owned() })
}

/// A session with no message yet, for the greeting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Welcome {
    /// The folder's name, for the line.
    pub name: String,
    /// The folder and, when the banner is on the screen, the model and plan.
    pub facts: String,
    /// The starts: their words and what a click does.
    pub starts: Vec<(String, Start)>,
}

impl Welcome {
    pub(super) fn read(chat: &Chat, cwd: &Path, banner: Option<&Banner>, pins: &[String]) -> Self {
        // The folder as Claude Code names it, else the tab's own.
        let folder = banner.map_or_else(|| cwd.display().to_string(), |b| b.folder.clone());
        let name = folder.rsplit(['\\', '/']).find(|part| !part.is_empty()).unwrap_or("this folder").to_owned();
        let mut facts = folder.clone();
        if let Some(banner) = banner {
            facts.push_str("  ·  ");
            facts.push_str(&banner.model);
        }
        // The starred commands, in their order; without any, a few of the local list.
        let listed = |name: &str| chat.commands.catalog.iter().position(|cmd| cmd.name == name);
        let mut starts: Vec<(String, Start)> =
            pins.iter().filter_map(|pin| Some((format!("/{pin}"), Start::Command(listed(pin)?)))).collect();
        if starts.is_empty() {
            starts = DEFAULT_STARTS
                .iter()
                .filter_map(|name| Some((format!("/{name}"), Start::Command(listed(name)?))))
                .collect();
        }
        starts.truncate(MAX_STARTS - 1);
        starts.push(("Paste a picture".into(), Start::Paste));
        Self { name, facts, starts }
    }

    /// The greeting in `area` (the room over the message box) and the starts in `row` under it.
    pub(super) fn draw(
        &self,
        p: &mut Painter,
        pal: &Palette,
        area: Rect,
        row: Rect,
        hover: Option<Target>,
        hits: &mut Vec<(Rect, Target)>,
    ) {
        let clip = p.clip(Rect::new(area.x, area.y, area.w, row.bottom() - area.y));
        let mut y = area.bottom() - GREETING;
        let line = |y: f32, h: f32| Rect::new(area.x, y, area.w, h);
        p.text_center(Face::Semibold, 34.0, line(y, 40.0), look::MARK, pal.accent);
        y += 40.0 + 16.0;
        let question = format!("What should Claude do in {}?", self.name);
        // A long folder name is shortened by the painter; the facts name it whole.
        let size = if p.measure(Face::Semibold, 24.0, &question) <= area.w { 24.0 } else { 19.0 };
        let w = p.measure(Face::Semibold, size, &question).min(area.w);
        p.text(Face::Semibold, size, area.x + (area.w - w) / 2.0, y + 16.0, &question, pal.text, w + 1.0);
        y += 32.0 + 8.0;
        let w = p.measure(Face::Ui, 13.0, &self.facts).min(area.w);
        p.text(Face::Ui, 13.0, area.x + (area.w - w) / 2.0, y + 10.0, &self.facts, pal.muted, w + 1.0);
        // The starts, centred under the box; those that do not fit the row are left out.
        let widths: Vec<f32> = self.starts.iter().map(|(label, _)| p.measure(Face::Ui, 12.5, label) + 26.0).collect();
        let mut shown = widths.len();
        let total = |n: usize| widths[..n].iter().sum::<f32>() + 8.0 * n.saturating_sub(1) as f32;
        while shown > 0 && total(shown) > row.w {
            shown -= 1;
        }
        let mut x = row.x + (row.w - total(shown)) / 2.0;
        for ((label, start), w) in self.starts.iter().zip(&widths).take(shown) {
            let r = Rect::new(x, row.bottom() - 30.0, *w, 30.0);
            let target = Target::Chat(Control::Start(*start));
            let hot = hover == Some(target);
            p.frame(r, 15.0, if hot { pal.hover } else { pal.page }, look::edge(pal));
            p.text_center(Face::Ui, 12.5, r, label, if hot { pal.text } else { pal.muted });
            hits.push((r, target));
            x += w + 8.0;
        }
        p.restore(clip);
    }
}

impl Chat {
    /// A start was clicked: the message box is filled, and nothing is sent.
    pub(super) fn start(&mut self, start: Start) {
        if self.locked() {
            return;
        }
        match start {
            Start::Command(at) => {
                let Some(command) = self.commands.catalog.get(at) else { return };
                let line = format!("/{} ", command.name);
                self.recall.end();
                self.editor.set(line);
                let end = self.editor.text.len();
                self.editor.move_to(end, false);
            }
            Start::Paste => self.paste_clipboard(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::chat::question::screens::recorded::recorded;
    use crate::view::chat::readme_shots as shots;

    #[test]
    fn the_banner_is_read_only_as_claude_code_draws_it() {
        let screen = recorded(shots::IDLE);
        assert_eq!(
            banner(&screen),
            Some(Banner { model: "Opus 5.5 · Claude Max".into(), folder: "~\\Desktop\\trail-planner".into() })
        );
        // No banner on the screen (a conversation scrolled it away, or a question is up): nothing.
        assert_eq!(banner(&recorded(shots::QUESTION)), None);
        // Text that only mentions the product is not the banner.
        let prose =
            "cursor 3 2\n@size 100 12\n\n● Claude Code v2 is out\n  Opus 5.5 · Claude Max\n  ~\\Desktop\n\n> \n";
        assert_eq!(banner(&recorded(prose)), None);
        assert_eq!(after_logo(" ▐▛███▜▌   Claude Code v2.1.332"), Some("Claude Code v2.1.332"));
        assert_eq!(after_logo("   Claude Code v2.1.332"), None, "no logo before it");
    }

    #[test]
    fn a_new_session_names_its_folder_and_offers_starred_commands_or_a_few_of_the_list() {
        let chat = Chat::default();
        let cwd = Path::new("work").join("trail-planner");
        let plain = Welcome::read(&chat, &cwd, None, &[]);
        assert_eq!(plain.name, "trail-planner");
        assert_eq!(plain.facts, cwd.display().to_string(), "without the banner, only the tab's folder");
        assert_eq!(
            plain.starts.iter().map(|(label, _)| label.as_str()).collect::<Vec<_>>(),
            ["/init", "/review", "/resume", "Paste a picture"]
        );
        let banner = Banner { model: "Opus 5.5 · Claude Max".into(), folder: "~\\Desktop\\trail-planner".into() };
        let pins = ["compact".to_owned(), "not-a-command-here".to_owned(), "model".to_owned()];
        let starred = Welcome::read(&chat, &cwd, Some(&banner), &pins);
        assert_eq!(starred.facts, "~\\Desktop\\trail-planner  ·  Opus 5.5 · Claude Max");
        // Starred commands, in their order; one the list does not have is left out.
        assert_eq!(
            starred.starts.iter().map(|(label, _)| label.as_str()).collect::<Vec<_>>(),
            ["/compact", "/model", "Paste a picture"]
        );
        // A click fills the box and sends nothing.
        let mut chat = Chat::default();
        let Start::Command(at) = starred.starts[0].1 else { panic!() };
        chat.start(Start::Command(at));
        assert_eq!(chat.editor.text, "/compact ");
        assert!(chat.queue.is_none() && chat.writes.is_empty());
    }
}

#[cfg(test)]
mod frame_tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::view::chat::readme_shots::{self as shots, Size};

    /// A new session greets with its box in the middle and its starts under it; once its first
    /// message is on its way the box is at the foot of the window, as in any session.
    #[test]
    fn a_new_session_greets_until_its_first_message_is_sent() {
        let demo = crate::demo::Transcript::new_empty().unwrap();
        let mut st = shots::window(true);
        let id = shots::claude_tab(&mut st, "Another tab");
        let source = crate::view::model::Source::File(demo.path().into());
        let mut pane = st.new_session_pane(source, demo.cwd().into());
        pane.chat.scripted = true;
        pane.chat.enabled = true;
        pane.chat.identity_unverified = false;
        pane.chat.awaiting_transcript = true;
        pane.ui.set_chat(true);
        shots::tab(&mut st, id).pane = Some(pane);
        let later = shots::show(&mut st, id, shots::IDLE, Instant::now() + Duration::from_secs(1));
        let size = Size { w: 1100.0, h: 820.0, scale: 1.0 };
        let mut anim = mdshared_ui::anim::Animator::new();
        shots::frame_at(&mut st, &mut anim, later, size);
        fn chat(st: &mut crate::state::AppState, id: u64) -> &mut Chat {
            &mut shots::tab(st, id).pane.as_mut().unwrap().chat
        }
        let layout = chat(&mut st, id).layout;
        assert!(layout.greeting.h > 0.0 && layout.side.w == 0.0, "greeting, and no panel with nothing to show");
        assert!(layout.box_frame.bottom() < size.h - 200.0, "the box is in the middle");
        let starts: Vec<Start> = st
            .hits
            .iter()
            .filter_map(|(r, t)| match t {
                Target::Chat(Control::Start(start)) if layout.starts.contains(r.cx(), r.cy()) => Some(*start),
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 4, "{starts:?}");
        assert_eq!(starts[3], Start::Paste);
        assert!(
            st.hits.iter().any(|(r, t)| *t == Target::Chat(Control::Editor) && layout.editor.contains(r.cx(), r.cy()))
        );
        // A start fills the box; nothing is written to the terminal.
        chat(&mut st, id).start(starts[0]);
        assert_eq!(chat(&mut st, id).editor.text, "/init ");
        assert!(chat(&mut st, id).writes.is_empty());
        // The first message is on its way: no greeting, the box at the foot.
        chat(&mut st, id).first_sent = true;
        shots::frame_at(&mut st, &mut anim, later, size);
        let layout = chat(&mut st, id).layout;
        assert_eq!(layout.greeting.h, 0.0);
        assert!(layout.box_frame.bottom() > size.h - 40.0);
        assert!(!st.hits.iter().any(|(_, t)| matches!(t, Target::Chat(Control::Start(_)))));
    }
}
