use super::*;
use crate::tabs::NewTab;
use mdshared_ui::host::{Button, KeyPress};
use mdshared_ui::keys::{KeyName, Mods};

fn term(text: &str) -> Term {
    let mut t = Term::empty((40, 10));
    t.feed(text.as_bytes());
    t
}

#[test]
fn the_tree_splits_the_focused_pane_lays_out_without_overlap_and_holds_four() {
    let mut p = Panes::default();
    assert_eq!(p.split(Axis::Right, term("b")), Some(1));
    assert_eq!(p.focus, 1);
    assert_eq!(p.split(Axis::Down, term("c")), Some(2));
    assert_eq!(p.ids(), [MAIN, 1, 2]);
    let area = Rect::new(0.0, 0.0, 606.0, 406.0);
    let rects = p.layout(area);
    assert_eq!(rects[0].1, Rect::new(0.0, 0.0, 300.0, 406.0));
    assert_eq!(rects[1].1, Rect::new(306.0, 0.0, 300.0, 200.0));
    assert_eq!(rects[2].1, Rect::new(306.0, 206.0, 300.0, 200.0));
    assert_eq!(p.dividers.len(), 2);
    assert_eq!(p.at(310.0, 300.0), Some(2));
    assert_eq!(p.at(303.0, 300.0), None, "the gap");
    assert_eq!(p.divider_at(303.0, 100.0), Some(0));
    assert!(p.split(Axis::Right, term("d")).is_some());
    assert_eq!(p.split(Axis::Right, term("e")), None, "four at most");
    assert_eq!(p.count(), MAX_PANES);
}

#[test]
fn removing_a_pane_gives_its_place_to_its_sibling_and_the_tabs_own_stays() {
    let mut p = Panes::default();
    p.split(Axis::Right, term("b"));
    p.split(Axis::Down, term("c"));
    assert!(p.remove(MAIN).is_none(), "the tab's own pane is not taken out here");
    assert!(p.remove(2).is_some());
    assert_eq!(p.ids(), [MAIN, 1]);
    assert_eq!(p.focus, 1, "the pane before it");
    assert!(p.remove(1).is_some());
    assert_eq!(p.tree, Node::Leaf(MAIN));
    assert!(!p.is_split());
    assert_eq!(p.focus, MAIN);
}

#[test]
fn dragging_a_divider_moves_it_and_keeps_both_panes_usable() {
    let mut p = Panes::default();
    p.split(Axis::Right, term("b"));
    let area = Rect::new(0.0, 0.0, 606.0, 400.0);
    p.layout(area);
    p.drag = p.divider_at(303.0, 50.0);
    assert!(p.drag_to(153.0, 50.0));
    let rects = p.layout(area);
    assert_eq!(rects[0].1.w, 150.0);
    assert!(p.drag_to(-500.0, 50.0));
    assert_eq!(p.layout(area)[0].1.w, MIN_SIDE);
    assert!(p.drag_to(5_000.0, 50.0));
    assert_eq!(p.layout(area)[1].1.w, MIN_SIDE);
}

fn app() -> (AppState, u64, Rect) {
    app_themed(true)
}

fn app_themed(dark: bool) -> (AppState, u64, Rect) {
    let mut st = AppState::new(crate::source::Source::Demo, Some(dark), Default::default());
    st.usage.demo_echo = true;
    st.window_focused = true;
    let id =
        st.tabs.open(NewTab { label: "Shell".into(), color: [0; 3], session: None, account: None }, term("$ main"));
    st.tabs.get_mut(id).unwrap().folder = Some(std::env::temp_dir());
    let window = Rect::new(0.0, 0.0, 800.0, 400.0);
    st.layout_terminals(window, window, 1.0);
    (st, id, window)
}

fn typed(st: &mut AppState, c: char) {
    let k = KeyPress { name: Some(KeyName::Char(c)), mods: Mods::default(), text: Some(c.to_string()), repeat: false };
    st.term_key(Instant::now(), &k);
}

fn text_of(st: &AppState, id: u64, pane: PaneId) -> String {
    let t = st.tabs.get(id).unwrap().term_of(pane).unwrap();
    // Rows joined as they are, so a path the narrow pane wrapped still reads whole.
    (0..t.screen.rows()).map(|r| t.screen.row_text(r)).collect::<String>()
}

#[test]
fn a_new_pane_starts_in_the_tabs_folder_takes_the_keys_and_a_press_gives_them_back() {
    let (mut st, id, window) = app();
    st.split_pane(Instant::now(), id, Axis::Right);
    let tab = st.tabs.get(id).unwrap();
    assert_eq!(tab.panes.count(), 2);
    assert_eq!(tab.focused_pane(), 1);
    st.layout_terminals(window, window, 1.0);
    // Each pane got its own size, about half the width.
    let (main, right) = (&st.tabs.get(id).unwrap().term, st.tabs.get(id).unwrap().panes.get(1).unwrap());
    assert!(main.screen.cols() < 70 && right.screen.cols() < 70, "{} {}", main.screen.cols(), right.screen.cols());
    let shown = text_of(&st, id, 1);
    assert!(shown.contains(&std::env::temp_dir().display().to_string()), "{shown:?} {:?}", std::env::temp_dir());
    typed(&mut st, 'x');
    assert!(text_of(&st, id, 1).contains('x'));
    assert!(!text_of(&st, id, MAIN).contains('x'));
    // The mouse's grid is the focused pane's.
    let rects = st.tabs.get(id).unwrap().panes.rects.clone();
    assert_eq!(st.term_area, crate::termapp::inner(rects[1].1));
    let left = rects[0].1;
    st.term_press(left.x + 20.0, left.y + 20.0, Button::Left, 1, Mods::default());
    assert_eq!(st.tabs.get(id).unwrap().focused_pane(), MAIN);
    assert_eq!(st.term_area, crate::termapp::inner(left));
    typed(&mut st, 'y');
    assert!(text_of(&st, id, MAIN).contains('y'));
    st.focus_next_pane();
    assert_eq!(st.tabs.get(id).unwrap().focused_pane(), 1);
    // Find works in the focused pane.
    st.open_find();
    assert!(st.tabs.get(id).unwrap().panes.get(1).unwrap().find.is_some());
    assert!(st.tabs.get(id).unwrap().term.find.is_none());
}

#[test]
fn closing_panes_the_tabs_own_last_and_the_last_closes_the_tab() {
    let now = Instant::now();
    let (mut st, id, _) = app();
    st.split_pane(now, id, Axis::Down);
    st.focus_pane(MAIN);
    st.ask_close_pane(id);
    // The tab's own pane never goes alone: the question is about the whole tab.
    let Some(crate::dialogs::Dialog::Confirm { title, then, .. }) = st.dialog.take() else { panic!("no question") };
    assert_eq!(title, "Close the whole tab?");
    assert!(matches!(then, crate::dialogs::Pending::CloseTab(t) if t == id));
    assert_eq!(st.tabs.get(id).unwrap().panes.count(), 2, "nothing closed before the answer");
    st.focus_pane(1);
    st.ask_close_pane(id);
    assert!(!st.tabs.get(id).unwrap().panes.is_split());
    st.ask_close_pane(id);
    assert!(st.tabs.get(id).is_none(), "the last pane closes the tab");
}

#[test]
fn closing_the_tabs_own_pane_under_others_closes_the_whole_tab_once_confirmed() {
    let now = Instant::now();
    let (mut st, id, _) = app();
    st.split_pane(now, id, Axis::Right);
    st.split_pane(now, id, Axis::Down);
    st.focus_pane(MAIN);
    st.ask_close_pane(id);
    let Some(crate::dialogs::Dialog::Confirm { text, .. }) = &st.dialog else { panic!("no question") };
    assert!(text.contains("the 2 other panes"), "{text}");
    // Cancel keeps everything; OK closes the tab, the other panes with it.
    crate::dialogs::click(&mut st, now, crate::dialogs::CANCEL, 0.0);
    assert_eq!(st.tabs.get(id).unwrap().panes.count(), 3);
    st.ask_close_pane(id);
    crate::dialogs::click(&mut st, now, crate::dialogs::OK, 0.0);
    assert!(st.tabs.get(id).is_none());
}

#[test]
fn an_ended_pane_closes_on_enter_without_closing_the_tab() {
    let (mut st, id, _) = app();
    st.split_pane(Instant::now(), id, Axis::Right);
    st.tabs.get_mut(id).unwrap().panes.get_mut(1).unwrap().ended =
        Some(mdshared_pty::Exit { code: Some(0), signal: None });
    let enter = KeyPress { name: Some(KeyName::Enter), mods: Mods::default(), text: Some("\r".into()), repeat: false };
    st.term_key(Instant::now(), &enter);
    assert!(st.tabs.get(id).is_some_and(|t| !t.panes.is_split()));
}

#[test]
fn splits_wait_in_chat_and_are_not_offered_past_four() {
    let now = Instant::now();
    let (mut st, id, _) = app();
    for _ in 0..3 {
        st.split_pane(now, id, Axis::Right);
    }
    assert_eq!(st.tabs.get(id).unwrap().panes.count(), 4);
    assert!(!st.can_split(id));
    st.split_pane(now, id, Axis::Down);
    assert_eq!(st.tabs.get(id).unwrap().panes.count(), 4);
}

/// Three panes, the focused one edged, in both themes. `CS_SHOT_DIR` keeps the pictures.
#[test]
fn split_panes_are_drawn_with_dividers_and_an_edge_round_the_focused_one() {
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    for dark in [true, false] {
        let now = Instant::now();
        let (mut st, id, _) = app_themed(dark);
        let tab = st.tabs.get_mut(id).unwrap();
        tab.term.feed(b"\r\n$ claude --resume\r\nClaude Code is ready");
        let (w, h) = (900.0, 360.0);
        let window = Rect::new(0.0, 0.0, w, h);
        st.layout_terminals(window, window, 1.0);
        st.split_pane(now, id, Axis::Right);
        st.split_pane(now, id, Axis::Down);
        st.tabs.get_mut(id).unwrap().panes.get_mut(2).unwrap().feed(b"cargo test\r\ntest result: ok");
        let mut bmp = Bitmap::new((w * 2.0) as u32, (h * 2.0) as u32);
        let mut gfx = Gfx::new();
        {
            let mut p = Painter::new(&mut bmp, &mut gfx, 2.0);
            st.layout_terminals(window, window, 2.0);
            st.draw_terminal_still(&mut p, window);
        }
        if let Ok(dir) = std::env::var("CS_SHOT_DIR") {
            let theme = if dark { "dark" } else { "light" };
            bmp.write_png(&std::path::Path::new(&dir).join(format!("cs-term-splits-{theme}.png"))).unwrap();
        }
        let pal = *st.pal();
        let px = |x: f32, y: f32| {
            let i = ((y * 2.0) as usize * bmp.width as usize + (x * 2.0) as usize) * 4;
            [bmp.pixels[i + 2], bmp.pixels[i + 1], bmp.pixels[i]]
        };
        let panes = &st.tabs.get(id).unwrap().panes;
        let d = panes.dividers[0].rect;
        assert_eq!(px(d.cx(), d.cy()), pal.line, "the divider's line");
        let focused = panes.rects.iter().find(|(p, _)| *p == 2).unwrap().1;
        let other = panes.rects.iter().find(|(p, _)| *p == MAIN).unwrap().1;
        assert_ne!(px(focused.x + 40.0, focused.y + 0.4), px(other.x + 40.0, other.y + 0.4), "the focused pane's edge");
        assert!(st.hits.iter().any(|(_, t)| matches!(t, crate::state::Target::PaneDivider(1))));
    }
}

#[test]
fn a_pane_that_loses_the_keyboard_takes_its_find_bar_out_of_it() {
    let (mut st, id, _) = app();
    st.split_pane(Instant::now(), id, Axis::Right);
    st.open_find();
    assert!(st.find_focused());
    st.focus_next_pane();
    assert!(!st.find_focused());
    st.focus_next_pane();
    assert!(!st.find_focused(), "coming back, keys go to the program");
    assert!(st.tabs.get(id).unwrap().panes.get(1).unwrap().find.is_some(), "the bar stays open");
}

#[test]
fn enter_on_the_ended_tabs_own_pane_closes_the_tab_through_close() {
    let (mut st, id, _) = app();
    st.split_pane(Instant::now(), id, Axis::Right);
    st.focus_pane(MAIN);
    st.tabs.get_mut(id).unwrap().term.ended = Some(mdshared_pty::Exit { code: Some(0), signal: None });
    let enter = KeyPress { name: Some(KeyName::Enter), mods: Mods::default(), text: Some("\r".into()), repeat: false };
    st.term_key(Instant::now(), &enter);
    // Nothing runs in the demo's panes, so Close closes at once; with a running pane it asks.
    assert!(st.tabs.get(id).is_none());
}

#[test]
fn a_drag_selection_stays_in_the_pane_it_started_in() {
    use mdshared_term::Point;
    let (mut st, id, window) = app();
    st.tabs.get_mut(id).unwrap().term.feed(b"left pane text\r\nsecond line");
    st.split_pane(Instant::now(), id, Axis::Right);
    st.tabs.get_mut(id).unwrap().panes.get_mut(1).unwrap().feed(b"\r\nright pane text");
    st.layout_terminals(window, window, 1.0);
    let rects = st.tabs.get(id).unwrap().panes.rects.clone();
    let (left, right) = (rects[0].1, rects[1].1);
    // From the left pane across the divider, deep into the right one and below the window.
    st.term_press(left.x + 10.0, left.y + 10.0, Button::Left, 1, Mods::default());
    assert_eq!(st.tabs.get(id).unwrap().focused_pane(), MAIN);
    st.term_move(right.x + 200.0, right.bottom() + 50.0, Mods::default());
    st.term_release(right.x + 200.0, right.bottom() + 50.0, Button::Left, Mods::default());
    let tab = st.tabs.get(id).unwrap();
    assert_eq!(tab.focused_pane(), MAIN, "a drag does not move the keyboard");
    let main = &tab.term;
    let (from, to) = main.selection.unwrap().range();
    assert!(to.col < main.screen.cols(), "clipped to its own columns: {to:?}");
    assert!(to.line < main.screen.end_line(), "and its own rows: {to:?}");
    assert_eq!(from, Point::new(main.screen.screen_start(), 0));
    assert!(tab.panes.get(1).unwrap().selection.is_none(), "nothing selected in the other pane");
    // The other way round: from the right pane leftwards past the window's edge.
    st.term_press(right.x + 30.0, right.y + 30.0, Button::Left, 1, Mods::default());
    st.term_move(-40.0, right.y + 30.0, Mods::default());
    st.term_release(-40.0, right.y + 30.0, Button::Left, Mods::default());
    let tab = st.tabs.get(id).unwrap();
    assert_eq!(tab.focused_pane(), 1);
    let (a, b) = tab.panes.get(1).unwrap().selection.unwrap().range();
    assert_eq!((a.col, b.line), (0, a.line), "to its own first column, on the same line");
    let text = st.tabs.get(id).unwrap().focused_term().selected_text().unwrap();
    assert!(!text.contains("left pane"), "no text of the other pane: {text:?}");
    assert!(st.tabs.get(id).unwrap().term.selection.is_none(), "one selection in the tab, the focused pane's");
}
