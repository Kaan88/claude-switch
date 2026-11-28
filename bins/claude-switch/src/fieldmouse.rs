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
//! The Settings form owns its fields inside the toolkit, which also does their mouse editing and
//! builds their menu from the real selection (`Form::pointer_press`, `context_menu`). This
//! module only routes to it: it holds the Settings drag like any other, so moves, edge-scroll
//! wake-ups, the release and a cancel reach the form.

use std::cell::RefCell;
use std::time::Instant;

use mdshared_ui::gfx::{Face, Gfx, Painter, Rect};
use mdshared_ui::keys::Mods;
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
    /// The find bar's box over the terminal shown.
    TermFind,
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
    /// A Settings drag was cancelled here; the form's own click run is reset on its next press.
    settings_cancelled: bool,
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
        self.settings_cancelled |= matches!(self.held, Some(FieldId::Settings(_)));
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

impl AppState {
    /// The text field under the pointer in the front layer, if one takes the mouse now.
    pub(crate) fn field_under(&self, x: f32, y: f32) -> Option<FieldId> {
        if let Some(Dialog::Settings(form)) = &self.dialog {
            return (self.palette.is_none() && self.menu.is_none())
                .then(|| form.text_row_at(x, y))
                .flatten()
                .map(FieldId::Settings);
        }
        // The front layer, as for hover: nothing under the palette, a dropdown, a menu, the
        // picture overlay or the usage popup reaches a field.
        let target = self.front_target_at(x, y)?;
        match (&self.dialog, target) {
            (None, Target::Search) => Some(FieldId::Search),
            (None, Target::Find(crate::termfind::FindControl::Text)) => Some(FieldId::TermFind),
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
            (FieldId::TermFind, None) => self.find_field().map(|(field, hit)| f(field, hit)),
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
            (FieldId::TermFind, None) => self.find_press(crate::termfind::FindControl::Text),
            (FieldId::NewSession, Some(Dialog::NewSession(picker))) => picker.focus_folder(),
            (FieldId::SessionName | FieldId::SessionTopic, Some(Dialog::Session(panel))) => {
                panel.focus_topic(id == FieldId::SessionTopic);
            }
            (FieldId::CompactName, Some(Dialog::Compact(compact))) => compact.focus_name(),
            _ => {} // The Settings form focuses its own rows.
        }
    }

    /// The text of field `id` changed through the menu: what typing would have done next.
    fn field_edited(&mut self, id: FieldId) {
        match id {
            FieldId::Search => self.after_search(),
            FieldId::TermFind => self.find_edited(),
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
        if let FieldId::Settings(_) = id {
            let Some(Dialog::Settings(form)) = &mut self.dialog else {
                self.field_mouse.cancel();
                return Some(false);
            };
            return Some(with_gfx(|gfx| form.text_motion(x, y, now, gfx)));
        }
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
        if self.field_deadline().is_none_or(|at| at > now) {
            return false;
        }
        if let (FieldId::Settings(_), Some(Dialog::Settings(form))) = (id, &mut self.dialog) {
            return with_gfx(|gfx| form.text_tick(now, gfx));
        }
        let mut mouse = std::mem::take(&mut self.field_mouse);
        let changed = self.with_field(id, |field, hit| hit.is_some_and(|hit| mouse.tick(field, hit, now)));
        self.field_mouse = mouse;
        changed.unwrap_or(false)
    }

    /// The next edge-scroll wake-up of the held drag, the Settings form's included.
    pub(crate) fn field_deadline(&self) -> Option<Instant> {
        match (self.field_mouse.held()?, &self.dialog) {
            (FieldId::Settings(_), Some(Dialog::Settings(form))) => form.text_deadline(),
            (FieldId::Settings(_), _) => None,
            _ => self.field_mouse.deadline(),
        }
    }

    /// The primary button went up at `(x, y)`. True when it ended a field drag.
    pub(crate) fn field_release(&mut self, x: f32, y: f32, now: Instant) -> bool {
        if let Some(FieldId::Settings(_)) = self.field_mouse.held() {
            self.field_mouse.held = None;
            if let Some(Dialog::Settings(form)) = &mut self.dialog {
                let o = with_gfx(|gfx| form.text_release(x, y, now, gfx));
                dialogs::settings_event(self, o);
            }
            return true;
        }
        self.field_mouse.release()
    }

    /// A Settings press. In a text row the toolkit places the caret, extends with Shift and
    /// takes a word or the whole row on a double or triple click; the drag is then held here.
    pub(crate) fn settings_press(&mut self, x: f32, y: f32, mods: Mods, clicks: u32, now: Instant, scale: f32) {
        let cancelled = std::mem::take(&mut self.field_mouse.settings_cancelled);
        self.field_mouse.held = None;
        let Some(Dialog::Settings(form)) = &mut self.dialog else { return };
        if cancelled {
            form.cancel_text();
        }
        let row = form.text_row_at(x, y);
        let o = with_gfx(|gfx| form.pointer_press(x, y, mods, clicks.min(3) as u8, now, gfx, scale));
        self.field_mouse.held = row.map(FieldId::Settings);
        dialogs::settings_event(self, o);
    }

    /// A right press on a field opens Cut, Copy, Paste, Select all. True when it did.
    pub(crate) fn field_menu(&mut self, x: f32, y: f32, now: Instant) -> bool {
        let Some(id) = self.field_under(x, y) else { return false };
        self.mouse = (x, y);
        self.field_mouse.cancel();
        self.focus_field(id);
        let items = if let FieldId::Settings(_) = id {
            let Some(Dialog::Settings(form)) = &mut self.dialog else { return false };
            let Some(items) = form.text_menu(x, y) else { return false };
            items.into_iter().map(|item| relabel(item.id, item)).collect()
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
        let Some(&action) = ACTIONS.get(index as usize) else { return };
        if let FieldId::Settings(_) = id {
            let Some(Dialog::Settings(form)) = &mut self.dialog else { return };
            let o = form.text_action(action);
            dialogs::settings_event(self, o);
            return;
        }
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
        assert!(app.field_release(0.0, 0.0, now));
        // A word, then the whole field, at the same place in quick succession.
        let x = hit.area.x + 2.0;
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(600));
        app.field_release(0.0, 0.0, now);
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(700));
        app.field_release(0.0, 0.0, now);
        assert_eq!(app.search.selected_text(), "alpha");
        app.field_press(x, y, Mods::default(), now + Duration::from_millis(800));
        app.field_release(0.0, 0.0, now);
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
        assert!(app.field_release(0.0, 0.0, now));
        assert_eq!(app.search.selected_text(), "alpha beta gamma");
        assert_eq!(app.field_drag(hit.area.x, y, now), None, "no drag after the release");
        app.field_press(hit.area.x, y, Mods::default(), now + Duration::from_secs(2));
        app.field_release(0.0, 0.0, now);
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
    fn an_open_usage_popup_keeps_the_field_menu_and_i_beam_from_the_field_under_it() {
        let (mut app, hit) = search_app();
        let (x, y) = (hit.area.x + 3.0, hit.area.cy());
        app.usage.popup.open = true;
        assert!(!app.over_field(x, y));
        assert!(!app.field_menu(x, y, Instant::now()));
        assert!(app.menu.is_none());
        app.usage.popup.open = false;
        assert!(app.field_menu(x, y, Instant::now()));
    }

    #[test]
    fn padding_presses_move_to_the_nearest_text_edge() {
        let area = Rect::new(10.0, 10.0, 100.0, 20.0);
        assert_eq!(into_area(area, (4.0, 2.0)), (10.0, 20.0));
        assert_eq!(into_area(area, (200.0, 15.0)), (109.5, 20.0));
        assert_eq!(into_area(area, (50.0, 15.0)), (50.0, 15.0));
    }
}
