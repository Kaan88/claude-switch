//! Mouse selection and the right-click menu for the app's single-line text fields: the session
//! search box, the name dialogs (account, tab, handover agent, the handover's new session name),
//! the new-session folder box and the quick switcher's and rename dialog's boxes.
//!
//! The toolkit's `textfield::Pointer` does the hit testing, the double click (a word), triple
//! click (everything), Shift+click and drag, with horizontal edge scrolling. This module finds the
//! field under the pointer, keeps the one drag that can be held at a time, and turns the toolkit's
//! context actions into the app's own popup menu. Each field records its [`FieldHit`] (its text
//! rectangle as drawn) every frame, so a press uses the geometry the user sees.
//!
//! The Settings form owns its fields inside the toolkit. Its mouse selection is the form's own;
//! this module adds the I-beam, the triple click and the same menu through the form's keys.

use std::cell::RefCell;
use std::time::Instant;

use mdshared_ui::gfx::{Face, Gfx, Painter, Rect};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::menu::{Item, Menu};
use mdshared_ui::textfield::{ContextAction, FieldHit, Pointer, TextField};

use crate::dialogs::{self, Dialog};
use crate::state::{AppState, Focus, MenuKind, Target};

/// A text field the pointer can select in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldId {
    Search,
    /// The name dialog (add or rename an account, rename a tab, the handover agent).
    Name,
    NewSession,
    /// The quick switcher's search, or the rename, topic and review dialogs' first box.
    SessionName,
    SessionTopic,
    /// The handover dialog's new session name.
    CompactName,
    /// A text row of the Settings form, by its setting id.
    Settings(u32),
}

/// The menu's items in the order the toolkit lists them; a menu id is an index here.
const ACTIONS: [ContextAction; 4] =
    [ContextAction::Cut, ContextAction::Copy, ContextAction::Paste, ContextAction::SelectAll];

/// The hit geometry of a field drawn with the UI face at `size` into `area` (its text rectangle,
/// already inset from the box).
pub fn hit(p: &Painter, area: Rect, size: f32) -> Option<FieldHit> {
    Some(FieldHit { area, face: Face::Ui, size, scale: p.scale })
}

thread_local! {
    /// Text layout for hit testing between frames. Events and painting share the UI thread.
    static GFX: RefCell<Option<Gfx>> = const { RefCell::new(None) };
}

fn with_gfx<R>(f: impl FnOnce(&mut Gfx) -> R) -> R {
    GFX.with(|cell| f(cell.borrow_mut().get_or_insert_with(Gfx::new)))
}

/// A press anywhere in the field's box counts: one in the padding goes to the nearest text edge.
fn into_area(area: Rect, (x, y): (f32, f32)) -> (f32, f32) {
    let right = (area.right() - 0.5).max(area.x);
    (x.clamp(area.x, right), if area.contains(x, y) { y } else { area.cy() })
}

/// The drag in progress, if any, and the click counter that makes double and triple clicks.
#[derive(Default)]
pub struct FieldMouse {
    pointer: Option<(f32, Pointer)>,
    held: Option<FieldId>,
}

impl FieldMouse {
    fn pointer(&mut self, scale: f32) -> &mut Pointer {
        if self.pointer.as_ref().is_none_or(|(s, _)| *s != scale) {
            self.pointer = Some((scale, Pointer::system(scale)));
        }
        &mut self.pointer.as_mut().unwrap().1
    }

    /// A primary press on field `id`. True when the toolkit placed the caret or selected.
    pub fn press(
        &mut self,
        id: FieldId,
        field: &mut TextField,
        hit: FieldHit,
        at: (f32, f32),
        mods: Mods,
        now: Instant,
    ) -> bool {
        let at = into_area(hit.area, at);
        let pointer = self.pointer(hit.scale);
        let pressed = with_gfx(|gfx| pointer.press(field, gfx, hit, at, mods, now));
        self.held = pressed.then_some(id);
        pressed
    }

    /// The pointer moved with the button held; true when the selection changed.
    pub fn drag(&mut self, field: &mut TextField, hit: FieldHit, at: (f32, f32), now: Instant) -> bool {
        let Some((_, pointer)) = &mut self.pointer else { return false };
        with_gfx(|gfx| pointer.pointer_move(field, gfx, hit, at, now))
    }

    /// Edge scrolling while the pointer rests beyond the field's end.
    pub fn tick(&mut self, field: &mut TextField, hit: FieldHit, now: Instant) -> bool {
        let Some((_, pointer)) = &mut self.pointer else { return false };
        with_gfx(|gfx| pointer.tick(field, gfx, hit, now))
    }

    pub fn held(&self) -> Option<FieldId> {
        self.held
    }

    /// The button went up: the drag ends, the click run goes on.
    pub fn release(&mut self) -> bool {
        if let Some((_, pointer)) = &mut self.pointer {
            pointer.release();
        }
        self.held.take().is_some()
    }

    /// Another button, a key or lost focus: the drag ends and the next click is a first one.
    pub fn cancel(&mut self) {
        if let Some((_, pointer)) = &mut self.pointer {
            pointer.cancel();
        }
        self.held = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.held?;
        self.pointer.as_ref()?.1.deadline()
    }
}

/// The toolkit's menu for `field` with the app's menu ids.
pub fn menu_items(field: &TextField) -> Vec<Item<u32>> {
    field.context_menu().into_iter().map(|item| relabel(item.id, item)).collect()
}

fn relabel(action: ContextAction, item: Item<ContextAction>) -> Item<u32> {
    let index = ACTIONS.iter().position(|a| *a == action).unwrap_or(0) as u32;
    let mut out = Item::new(index, item.label).enabled(item.enabled);
    out.separator_before = item.separator_before;
    out
}

/// The Settings form's version: the form keeps its selection to itself, so Cut and Copy are
/// offered whenever the row has text and do nothing on an empty selection.
fn settings_items(has_text: bool) -> Vec<Item<u32>> {
    ["Cut", "Copy", "Paste", "Select all"]
        .into_iter()
        .enumerate()
        .map(|(i, label)| Item::new(i as u32, label).enabled(i == 2 || has_text))
        .collect()
}

/// The key that performs a menu action in a field that only takes keys.
fn shortcut(index: u32) -> Option<KeyName> {
    Some(KeyName::Char(['x', 'c', 'v', 'a'].get(index as usize).copied()?))
}

const CTRL: Mods = ctrl();

const fn ctrl() -> Mods {
    let mut mods = Mods { ctrl: false, shift: false, alt: false, control: false };
    mods.ctrl = true;
    mods
}

impl AppState {
    fn front_target(&self, x: f32, y: f32) -> Option<Target> {
        if self.palette.is_some() || self.accounts_dd.is_open() || self.menu.is_some() {
            return None;
        }
        let modal = self.dialog.is_some();
        self.hits
            .iter()
            .rev()
            .filter(|(_, t)| !modal || matches!(t, Target::Dialog(_)))
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| *t)
    }

    /// The text field under the pointer in the front layer, if one takes the mouse now.
    pub(crate) fn field_under(&self, x: f32, y: f32) -> Option<FieldId> {
        if let Some(Dialog::Settings(form)) = &self.dialog {
            return (self.palette.is_none() && self.menu.is_none())
                .then(|| form.text_row_at(x, y))
                .flatten()
                .map(FieldId::Settings);
        }
        let target = self.front_target(x, y)?;
        match (&self.dialog, target) {
            (None, Target::Search) => Some(FieldId::Search),
            (Some(Dialog::Name { .. }), Target::Dialog(dialogs::FIELD)) => Some(FieldId::Name),
            (Some(Dialog::NewSession(_)), Target::Dialog(dialogs::FIELD)) => Some(FieldId::NewSession),
            (Some(Dialog::Session(panel)), Target::Dialog(id)) => panel.field_of(id),
            (Some(Dialog::Compact(compact)), Target::Dialog(crate::compact_dialog::NAME_FIELD))
                if compact.name_editable() =>
            {
                Some(FieldId::CompactName)
            }
            _ => None,
        }
    }

    fn with_field<R>(&mut self, id: FieldId, f: impl FnOnce(&mut TextField, Option<FieldHit>) -> R) -> Option<R> {
        match (id, &mut self.dialog) {
            (FieldId::Search, None) => Some(f(&mut self.search, self.search_hit)),
            (FieldId::Name, Some(Dialog::Name { field, hit, .. })) => Some(f(field, *hit)),
            (FieldId::NewSession, Some(Dialog::NewSession(picker))) => {
                let (field, hit) = picker.field_parts();
                Some(f(field, hit))
            }
            (FieldId::SessionName | FieldId::SessionTopic, Some(Dialog::Session(panel))) => {
                let (field, hit) = panel.field_parts(id == FieldId::SessionTopic);
                Some(f(field, hit))
            }
            (FieldId::CompactName, Some(Dialog::Compact(compact))) => {
                let (field, hit) = compact.name_parts();
                Some(f(field, hit))
            }
            _ => None,
        }
    }

    /// Puts the keyboard on field `id`, as its owner does for a click.
    fn focus_field(&mut self, id: FieldId) {
        match (id, &mut self.dialog) {
            (FieldId::Search, None) => self.focus = Focus::Search,
            (FieldId::NewSession, Some(Dialog::NewSession(picker))) => picker.focus_folder(),
            (FieldId::SessionName | FieldId::SessionTopic, Some(Dialog::Session(panel))) => {
                panel.focus_topic(id == FieldId::SessionTopic);
            }
            (FieldId::CompactName, Some(Dialog::Compact(compact))) => compact.focus_name(),
            (FieldId::Settings(row), Some(Dialog::Settings(form))) => {
                form.focus_text_row(row);
            }
            _ => {}
        }
    }

    /// The text of field `id` changed through the menu: what typing would have done next.
    fn field_edited(&mut self, id: FieldId) {
        match id {
            FieldId::Search => self.after_search(),
            FieldId::NewSession | FieldId::SessionName | FieldId::SessionTopic => {
                let Some(mut dialog) = self.dialog.take() else { return };
                match &mut dialog {
                    Dialog::NewSession(picker) => picker.edited(self),
                    Dialog::Session(panel) => panel.edited(self),
                    _ => {}
                }
                self.dialog = Some(dialog);
            }
            FieldId::Name | FieldId::CompactName | FieldId::Settings(_) => {}
        }
    }

    /// A primary press. True when it landed on a single-line field, which then has the keyboard.
    pub(crate) fn field_press(&mut self, x: f32, y: f32, mods: Mods, now: Instant) -> bool {
        let Some(id) = self.field_under(x, y) else { return false };
        if matches!(id, FieldId::Settings(_)) {
            return false; // The form takes its own presses.
        }
        self.focus_field(id);
        let mut mouse = std::mem::take(&mut self.field_mouse);
        self.with_field(id, |field, hit| hit.is_some_and(|hit| mouse.press(id, field, hit, (x, y), mods, now)));
        self.field_mouse = mouse;
        true
    }

    /// The pointer moved. `Some` while a field drag is held, with whether to redraw.
    pub(crate) fn field_drag(&mut self, x: f32, y: f32, now: Instant) -> Option<bool> {
        let id = self.field_mouse.held()?;
        let mut mouse = std::mem::take(&mut self.field_mouse);
        let changed = self.with_field(id, |field, hit| hit.is_some_and(|hit| mouse.drag(field, hit, (x, y), now)));
        self.field_mouse = mouse;
        if changed.is_none() {
            self.field_mouse.cancel(); // The field went away under the drag.
        }
        Some(changed.unwrap_or(false))
    }

    /// The edge-scroll wake-up. True when the field scrolled.
    pub(crate) fn field_tick(&mut self, now: Instant) -> bool {
        let Some(id) = self.field_mouse.held() else { return false };
        if self.field_mouse.deadline().is_none_or(|at| at > now) {
            return false;
        }
        let mut mouse = std::mem::take(&mut self.field_mouse);
        let changed = self.with_field(id, |field, hit| hit.is_some_and(|hit| mouse.tick(field, hit, now)));
        self.field_mouse = mouse;
        changed.unwrap_or(false)
    }

    /// The primary button went up. True when it ended a field drag.
    pub(crate) fn field_release(&mut self) -> bool {
        self.field_mouse.release()
    }

    /// A Settings press: the form's own, plus a triple click selecting the whole row.
    pub(crate) fn settings_press(&mut self, x: f32, y: f32, mods: Mods, clicks: u32) {
        let Some(Dialog::Settings(form)) = &mut self.dialog else { return };
        let o = form.press(x, y, mods, clicks.min(3) as u8);
        let triple = clicks >= 3 && form.text_row_at(x, y).is_some();
        dialogs::settings_event(self, o);
        if triple && let Some(Dialog::Settings(form)) = &mut self.dialog {
            let o = form.key(&KeyName::Char('a'), CTRL, None);
            dialogs::settings_event(self, o);
        }
    }

    /// A right press on a field opens Cut, Copy, Paste, Select all. True when it did.
    pub(crate) fn field_menu(&mut self, x: f32, y: f32, now: Instant) -> bool {
        let Some(id) = self.field_under(x, y) else { return false };
        self.mouse = (x, y);
        self.field_mouse.cancel();
        self.focus_field(id);
        let items = if let FieldId::Settings(row) = id {
            let Some(Dialog::Settings(form)) = &self.dialog else { return false };
            settings_items(form.text_of(row).is_some_and(|t| !t.is_empty()))
        } else {
            let mut mouse = std::mem::take(&mut self.field_mouse);
            let items = self.with_field(id, |field, hit| {
                // Outside the selection, the caret moves to the pointer first, as in Windows.
                if !field.has_selection()
                    && let Some(hit) = hit
                {
                    mouse.press(id, field, hit, (x, y), Mods::default(), now);
                    mouse.cancel();
                }
                menu_items(field)
            });
            self.field_mouse = mouse;
            let Some(items) = items else { return false };
            items
        };
        self.menu = Some((MenuKind::Field(id), Menu::new(items, x, y, false)));
        true
    }

    /// An item of a field's menu was chosen.
    pub(crate) fn field_menu_chosen(&mut self, id: FieldId, index: u32) {
        if let FieldId::Settings(row) = id {
            let Some(Dialog::Settings(form)) = &mut self.dialog else { return };
            let Some(key) = shortcut(index) else { return };
            form.focus_text_row(row);
            let o = form.key(&key, CTRL, None);
            dialogs::settings_event(self, o);
            return;
        }
        let Some(&action) = ACTIONS.get(index as usize) else { return };
        let edited = self.with_field(id, |field, _| {
            let before = field.text().to_owned();
            field.apply_context_action(action);
            field.text() != before
        });
        if edited == Some(true) {
            self.field_edited(id);
        }
    }

    /// The I-beam: over every field that takes the mouse now.
    pub(crate) fn over_field(&self, x: f32, y: f32) -> bool {
        self.field_under(x, y).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use std::time::Duration;

    fn search_app() -> (AppState, FieldHit) {
        let mut app = AppState::new(Source::Demo, None, Default::default());
        let area = Rect::new(100.0, 10.0, 200.0, 18.0);
        let hit = FieldHit { area, face: Face::Ui, size: 13.0, scale: 1.0 };
        app.search_hit = Some(hit);
        app.hits = vec![(area.inset(-8.0), Target::Search)];
        app.search.set_text("alpha beta gamma");
        app.field_mouse.pointer = Some((1.0, Pointer::new(Duration::from_millis(500), (4.0, 4.0))));
        (app, hit)
    }

    #[test]
    fn clicks_in_the_search_box_place_select_a_word_and_then_everything() {
        let (mut app, hit) = search_app();
        let now = Instant::now();
        let y = hit.area.cy();
        app.focus = Focus::List;
        assert!(app.field_press(hit.area.right() - 1.0, y, Mods::default(), now));
        assert_eq!(app.focus, Focus::Search);
        assert_eq!(app.search.caret(), (16, 16));
        assert!(app.field_release());
        // A word, then the whole field, at the same place in quick succession.
        let x = hit.area.x + 2.0;
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(600));
        app.field_release();
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(700));
        app.field_release();
        assert_eq!(app.search.selected_text(), "alpha");
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(800));
        app.field_release();
        assert_eq!(app.search.selected_text(), "alpha beta gamma");
    }

    #[test]
    fn drag_and_shift_click_extend_and_a_press_in_the_padding_still_counts() {
        let (mut app, hit) = search_app();
        let now = Instant::now();
        let y = hit.area.cy();
        // The box's left padding is outside the text rectangle but places the caret at its start.
        assert!(app.field_press(hit.area.x - 6.0, y, Mods::default(), now));
        assert_eq!(app.search.caret(), (0, 0));
        assert_eq!(app.field_drag(hit.area.right() - 1.0, y, now), Some(true));
        assert!(app.field_release());
        assert_eq!(app.search.selected_text(), "alpha beta gamma");
        assert_eq!(app.field_drag(hit.area.x, y, now), None, "no drag after the release");
        app.field_press(hit.area.x, y, Mods::default(), now + Duration::from_secs(2));
        app.field_release();
        let shift = Mods { shift: true, ..Mods::default() };
        app.field_press(hit.area.right() - 1.0, y, shift, now + Duration::from_secs(3));
        assert_eq!(app.search.selected_text(), "alpha beta gamma");
    }

    #[test]
    fn the_menu_opens_only_on_a_field_and_select_all_then_cut_edits_through_the_owner() {
        let (mut app, hit) = search_app();
        let now = Instant::now();
        assert!(!app.field_menu(5.0, 5.0, now), "not over the field");
        assert!(app.field_menu(hit.area.x + 3.0, hit.area.cy(), now));
        assert!(matches!(app.menu, Some((MenuKind::Field(FieldId::Search), _))));
        assert!(app.field_under(hit.area.x + 3.0, hit.area.cy()).is_none(), "the menu is in front now");
        app.menu = None;
        app.field_menu_chosen(FieldId::Search, 3);
        assert_eq!(app.search.selected_text(), "alpha beta gamma");
        let labels: Vec<_> = menu_items(&app.search).into_iter().map(|i| (i.id, i.label, i.enabled)).collect();
        assert_eq!(labels[0], (0, "Cut".into(), true));
        assert_eq!(labels[3], (3, "Select all".into(), true));
    }

    #[test]
    fn only_fields_get_the_i_beam_and_a_modal_dialog_hides_the_search_box() {
        let (mut app, hit) = search_app();
        let (x, y) = (hit.area.x + 3.0, hit.area.cy());
        assert!(app.over_field(x, y));
        app.hits.push((Rect::new(0.0, 0.0, 400.0, 40.0), Target::Dialog(dialogs::OK)));
        app.dialog = Some(Dialog::About);
        assert!(!app.over_field(x, y));
        app.dialog = Some(dialogs::name_dialog("Rename", "old", dialogs::NameFor::Tab(1)));
        app.hits.push((hit.area, Target::Dialog(dialogs::FIELD)));
        assert_eq!(app.field_under(x, y), Some(FieldId::Name));
    }

    #[test]
    fn padding_presses_move_to_the_nearest_text_edge() {
        let area = Rect::new(10.0, 10.0, 100.0, 20.0);
        assert_eq!(into_area(area, (4.0, 2.0)), (10.0, 20.0));
        assert_eq!(into_area(area, (200.0, 15.0)), (109.5, 20.0));
        assert_eq!(into_area(area, (50.0, 15.0)), (50.0, 15.0));
        assert_eq!(shortcut(1), Some(KeyName::Char('c')));
        assert_eq!(shortcut(4), None);
        assert!(!settings_items(false)[0].enabled && settings_items(false)[2].enabled);
    }
}
