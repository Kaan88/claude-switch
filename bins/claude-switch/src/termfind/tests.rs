use super::*;

fn screen(cols: usize, rows: usize, text: &[u8]) -> Screen {
    let mut s = Screen::new(cols, rows, 100);
    s.feed(text);
    s
}

fn spans(hits: &[Hit]) -> Vec<((u64, usize), (u64, usize))> {
    hits.iter().map(|h| ((h.start.line, h.start.col), (h.end.line, h.end.col))).collect()
}

#[test]
fn finds_in_the_scrollback_and_the_screen_ignoring_case_unless_asked() {
    let s = screen(20, 3, b"Error one\r\nfine\r\nerror two\r\nok\r\nERROR three");
    assert!(s.scrollback_len() > 0);
    let all = search(&s, "error", false);
    assert_eq!(spans(&all), [((0, 0), (0, 4)), ((2, 0), (2, 4)), ((4, 0), (4, 4))]);
    assert_eq!(spans(&search(&s, "error", true)), [((2, 0), (2, 4))]);
    assert!(search(&s, "", false).is_empty());
    assert!(search(&s, "missing", false).is_empty());
}

#[test]
fn a_match_across_a_wrap_and_wide_characters_cover_their_cells() {
    // 10 columns: "abcdefghij" wraps onto the next row, so "ijkl" spans both.
    let s = screen(10, 4, "abcdefghijkl\r\n日本語 x".as_bytes());
    assert_eq!(spans(&search(&s, "ijkl", false)), [((0, 8), (1, 1))]);
    // Each wide character takes two cells; the match ends on the second cell of 語.
    assert_eq!(spans(&search(&s, "本語", false)), [((2, 2), (2, 5))]);
    assert_eq!(spans(&search(&s, "語 x", false)), [((2, 4), (2, 7))]);
}

#[test]
fn matches_do_not_overlap_and_trailing_blanks_do_not_match() {
    let s = screen(12, 2, b"aaaa\r\nb");
    assert_eq!(search(&s, "aa", false).len(), 2);
    assert!(search(&s, "a ", false).is_empty());
}

#[test]
fn the_current_match_starts_above_the_view_and_steps_round_with_a_count() {
    let mut s = screen(20, 3, b"x1\r\nx2\r\nx3\r\nx4\r\nx5");
    let mut f = Find::new();
    f.field.set_text("X");
    // The view shows lines 2..5; the search goes upwards from its bottom.
    let hit = f.update(&s, 1, 5).unwrap();
    assert_eq!(hit.start.line, 4);
    assert_eq!(f.count(), "5 of 5");
    assert_eq!(f.step(true).unwrap().start.line, 3);
    assert_eq!(f.count(), "4 of 5");
    assert_eq!(f.step(false).unwrap().start.line, 4);
    assert_eq!(f.step(false).unwrap().start.line, 0, "round from the bottom");
    assert_eq!(f.step(true).unwrap().start.line, 4, "round from the top");
    // New output keeps the current match and does not ask to scroll.
    f.step(true);
    s.feed(b"\r\nx6");
    assert_eq!(f.update(&s, 2, 6), None);
    assert_eq!(f.current().unwrap().start.line, 3);
    assert_eq!(f.count(), "4 of 6");
    // Nothing changed: nothing found again.
    assert_eq!(f.update(&s, 2, 6), None);
    f.field.set_text("nothing");
    assert_eq!(f.update(&s, 2, 6), None);
    assert_eq!(f.count(), "No results");
    assert_eq!(f.step(true), None);
    f.field.set_text("");
    f.update(&s, 2, 6);
    assert_eq!(f.count(), "");
}

#[test]
fn reveal_scrolls_a_line_into_the_middle_only_when_it_is_not_shown() {
    let mut t = Term::empty((10, 4));
    for i in 0..30 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    let start = t.screen.screen_start();
    t.reveal(start + 1);
    assert_eq!(t.scroll, 0);
    t.reveal(5);
    assert_eq!(t.view(false).top_line(&t.screen), 3);
    t.reveal(4);
    assert_eq!(t.view(false).top_line(&t.screen), 3, "already shown");
    t.reveal(0);
    assert_eq!(t.scroll, t.screen.scrollback_len());
}

fn key(name: KeyName, mods: Mods, text: Option<&str>) -> mdshared_ui::host::KeyPress {
    mdshared_ui::host::KeyPress { name: Some(name), mods, text: text.map(str::to_string), repeat: false }
}

const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, alt: false, control: false };

fn app_with_output(dark: bool) -> AppState {
    use crate::tabs::NewTab;
    let mut st = AppState::new(crate::source::Source::Demo, Some(dark), Default::default());
    st.usage.demo_echo = true;
    st.window_focused = true;
    let mut t = Term::empty((60, 8));
    t.feed(b"$ cargo test\r\nerror: one failed\r\n   Compiling claude-switch\r\nwarning: unused\r\n");
    t.feed(b"\x1b[31merror\x1b[0m[E0308]: mismatched types\r\n  --> src/main.rs:4\r\n");
    t.feed(b"test result: ERROR. 3 passed\r\n$ ");
    st.tabs.open(NewTab { label: "Shell".into(), color: [0; 3], session: None, account: None }, t);
    st
}

#[test]
fn ctrl_shift_f_opens_the_bar_and_keys_edit_it_not_the_program() {
    let now = std::time::Instant::now();
    let mut st = app_with_output(true);
    let window = Rect::new(0.0, 0.0, 600.0, 200.0);
    st.layout_terminals(window, window, 1.0);
    st.term_key(now, &key(KeyName::Char('f'), CTRL_SHIFT, Some("\u{6}")));
    assert!(st.find_focused());
    assert!(!st.terminal_focused(), "the cursor shows the terminal without the keyboard");
    for c in ["e", "R", "r"] {
        assert!(st.find_key(&KeyName::Char(c.chars().next().unwrap()), Mods::default(), Some(c)));
    }
    assert!(st.usage.demo_keys.is_empty(), "typed into the box, not the program");
    let find =
        |st: &AppState| st.tabs.shown().unwrap().term.find.as_ref().map(|f| (f.field.text().to_string(), f.count()));
    assert_eq!(find(&st), Some(("eRr".into(), "3 of 3".into())));
    st.find_key(&KeyName::Enter, Mods::default(), Some("\r"));
    assert_eq!(find(&st).unwrap().1, "2 of 3");
    st.find_key(&KeyName::Enter, Mods { shift: true, ..Mods::default() }, Some("\r"));
    assert_eq!(find(&st).unwrap().1, "3 of 3");
    st.find_key(&KeyName::Char('c'), Mods { alt: true, ..Mods::default() }, None);
    assert_eq!(find(&st).unwrap().1, "No results", "match case");
    st.find_press(FindControl::Case);
    // A window shortcut goes on to the window.
    assert!(!st.find_key(&KeyName::Tab, Mods { ctrl: true, ..Mods::default() }, None));
    st.find_key(&KeyName::Escape, Mods::default(), None);
    assert_eq!(find(&st), None);
    assert!(st.terminal_focused());
}

#[test]
fn a_press_on_the_grid_gives_the_keyboard_back_and_the_bar_stays() {
    let mut st = app_with_output(false);
    let window = Rect::new(0.0, 0.0, 600.0, 200.0);
    st.layout_terminals(window, window, 1.0);
    st.open_find();
    st.term_press(40.0, 150.0, mdshared_ui::host::Button::Left, 1, Mods::default());
    assert!(!st.find_focused());
    assert!(st.tabs.shown().unwrap().term.find.is_some());
    st.find_press(FindControl::Field);
    assert!(st.find_focused());
}

/// The bar and the marks over a terminal, in both themes. `CS_SHOT_DIR` keeps the pictures.
#[test]
fn the_bar_and_the_marks_are_drawn_over_the_grid_in_both_themes() {
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    for dark in [true, false] {
        let mut st = app_with_output(dark);
        let (w, h) = (640.0, 220.0);
        let window = Rect::new(0.0, 0.0, w, h);
        st.layout_terminals(window, window, 1.0);
        let draw = |st: &mut AppState, name: &str| {
            let mut bmp = Bitmap::new((w * 2.0) as u32, (h * 2.0) as u32);
            let mut gfx = Gfx::new();
            st.hits.clear();
            {
                let mut p = Painter::new(&mut bmp, &mut gfx, 2.0);
                st.layout_terminals(window, window, 2.0);
                st.draw_terminal_still(&mut p, window);
            }
            if let Ok(dir) = std::env::var("CS_SHOT_DIR") {
                let theme = if dark { "dark" } else { "light" };
                bmp.write_png(&std::path::Path::new(&dir).join(format!("cs-term-find-{name}-{theme}.png"))).unwrap();
            }
            bmp
        };
        let plain = draw(&mut st, "closed");
        st.open_find();
        for c in "error".chars() {
            st.find_key(&KeyName::Char(c), Mods::default(), Some(&c.to_string()));
        }
        let open = draw(&mut st, "open");
        assert!(st.hits.iter().any(|(_, t)| *t == Target::Find(FindControl::Close)));
        let pal = *st.pal();
        let (m, inner) = (st.metrics.unwrap(), st.term_area);
        let px = |b: &Bitmap, r: Rect| {
            let (x, y) = ((r.x * 2.0 + 2.0) as usize, (r.cy() * 2.0) as usize);
            let i = (y * b.width as usize + x) * 4;
            [b.pixels[i + 2], b.pixels[i + 1], b.pixels[i]]
        };
        let find = st.tabs.shown().unwrap().term.find.as_ref().unwrap();
        assert_eq!(find.count(), "3 of 3");
        let current = find.current().unwrap();
        let other = find.matches[0];
        let cell = |hit: Hit| {
            m.cell_rect(
                inner,
                hit.start.line as usize - st.tabs.shown().unwrap().term.screen.first_line() as usize,
                hit.start.col,
            )
        };
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) < 90);
        let (c, o) = (cell(current), cell(other));
        assert_ne!(px(&open, c), px(&plain, c), "current match marked");
        assert_ne!(px(&open, o), px(&plain, o), "other match marked");
        assert_ne!(px(&open, c), px(&open, o), "the current match stands apart");
        assert!(near(px(&open, c), pal.find_current), "{:?} vs {:?}", px(&open, c), pal.find_current);
        // A cell that is no match keeps its pixels.
        let free = m.cell_rect(inner, 2, 40);
        assert_eq!(px(&open, free), px(&plain, free));
    }
}
