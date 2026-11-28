//! The review of the Chat redesign (docs/TODO.md): what its fixes keep.
use std::time::{Duration, Instant};

use mdshared_ui::gfx::Rect;
use mdshared_ui::keys::{KeyName, Mods};

use super::super::input::{Docked, docked_key};
use super::super::question::Hit;
use super::super::readme_shots::{self as shots, Size};
use super::super::shots::{WORKING, WORKING_LINE};
use super::*;
use crate::state::Target;

const BENCH: (&str, &str, &str) = (
    "Measure eviction before and after the change.",
    "I’ll run the eviction benchmark on this version, then on the one before it.",
    "cargo bench -p tiles --bench eviction",
);

fn running_window(size: Size, panel: Option<bool>) -> (crate::demo::Transcript, crate::state::AppState, u64) {
    let demo = crate::demo::Transcript::new_showcase(Some(BENCH)).unwrap();
    let (mut st, id) = shots::chat_window(true, &demo);
    shots::running(&mut st, id, WORKING_LINE);
    let later = shots::show(&mut st, id, WORKING, Instant::now() + Duration::from_secs(1));
    shots::running(&mut st, id, WORKING_LINE);
    shots::tab(&mut st, id).pane.as_mut().unwrap().chat.side_choice = panel;
    let mut anim = mdshared_ui::anim::Animator::new();
    shots::frame_at(&mut st, &mut anim, later, size);
    shots::frame_at(&mut st, &mut anim, later, size);
    (demo, st, id)
}

fn panel_hits(st: &crate::state::AppState) -> usize {
    st.hits.iter().filter(|(_, t)| *t == Target::Chat(Control::Panel)).count()
}

/// "2 of 3" on the working line opens the panel; below 900 px the panel cannot show, so the click
/// did nothing (and was kept as the user's choice). There it is not drawn, and no click there is
/// taken as a choice.
#[test]
fn the_task_progress_on_the_working_line_shows_only_where_the_panel_can_open() {
    let wide = Size { w: 1280.0, h: 900.0, scale: 1.0 };
    let narrow = Size { w: 820.0, h: 900.0, scale: 1.0 };
    // Closed by the user in a wide window: the progress opens it.
    let (_demo, st, id) = running_window(wide, Some(false));
    assert!(panel_hits(&st) >= 1);
    drop((st, id));
    let (_demo, mut st, id) = running_window(narrow, Some(false));
    assert_eq!(panel_hits(&st), 0, "no progress pill and no Details button where the panel cannot open");
    let chat = &mut shots::tab(&mut st, id).pane.as_mut().unwrap().chat;
    assert!(!chat.layout.panel_room);
    assert!(Layout::panel_room(Rect::new(0.0, 0.0, 900.0, 600.0), false));
    assert!(Layout::panel_room(Rect::new(0.0, 0.0, 700.0, 600.0), true), "a section that reveals it");
}

/// Escape under a card does what the card's own "Esc to …" does, and only while the card draws
/// that button; the keys that would have edited or sent the hidden draft say where typing goes.
#[test]
fn escape_under_a_card_is_the_cards_esc_and_other_keys_say_where_typing_goes() {
    let escape = (Rect::new(0.0, 0.0, 24.0, 24.0), Target::Chat(Control::Question(Hit::Escape)));
    let open = (Rect::new(30.0, 0.0, 24.0, 24.0), Target::Chat(Control::Question(Hit::Open)));
    let none = Mods::default();
    assert_eq!(docked_key(&[open, escape], Some(&KeyName::Escape), none), Docked::Escape);
    assert_eq!(docked_key(&[open], Some(&KeyName::Escape), none), Docked::Nothing, "no Esc offered");
    let shift = Mods { shift: true, ..Mods::default() };
    assert_eq!(docked_key(&[escape], Some(&KeyName::Escape), shift), Docked::Nothing);
    for key in [KeyName::Enter, KeyName::Backspace, KeyName::Delete] {
        assert_eq!(docked_key(&[escape], Some(&key), none), Docked::TypedAway);
    }
    assert_eq!(docked_key(&[escape], Some(&KeyName::Up), none), Docked::Nothing);
}

/// The composer's Stop stops the turn it was drawn for: a click that lands after another turn
/// started stops nothing (the working line's Stop already worked that way).
#[test]
fn the_composers_stop_remembers_the_turn_it_was_drawn_for() {
    let wide = Size { w: 1280.0, h: 900.0, scale: 1.0 };
    let (_demo, mut st, id) = running_window(wide, None);
    // The host's working line knows the turn it can stop (its checks passed).
    let pane = shots::tab(&mut st, id).pane.as_mut().unwrap();
    let (generation, prompt) = (pane.model.chat.generation, pane.model.chat.prompt.unwrap());
    let stop = Some(crate::view::working::Turn(generation, prompt));
    pane.ui.set_working(Some(crate::view::working::Footer { label: WORKING_LINE.into(), stop }));
    let later = Instant::now() + Duration::from_secs(3);
    shots::frame_at(&mut st, &mut mdshared_ui::anim::Animator::new(), later, wide);
    let drawn = st.hits.iter().any(|(_, t)| *t == Target::Chat(Control::Stop));
    let pane = shots::tab(&mut st, id).pane.as_ref().unwrap();
    let info = &pane.model.chat;
    let turn = pane.chat.stop_turn.expect("Stop is drawn for the running turn");
    assert!(drawn);
    assert_eq!((turn.0, Some(turn.1)), (info.generation, info.prompt));
}

/// Chat's 28 px side padding gives way to the window's own 12 px under a 560 px column.
#[test]
fn chats_side_padding_narrows_in_a_narrow_window() {
    use crate::view::ui::chat_pad;
    assert_eq!(chat_pad(1400.0), 28.0);
    assert_eq!(chat_pad(560.0 + 56.0), 28.0);
    assert_eq!(chat_pad(560.0 + 55.0), 12.0);
    assert_eq!(chat_pad(600.0), 12.0);
}
