use super::*;
use crate::term::Term;
use mdshared_term::Color;

/// A terminal of `cols` by `rows` that shows `bytes` (rows end with `\r\n`).
fn term(cols: u16, rows: u16, bytes: &str) -> Term {
    let mut t = Term::empty((cols, rows));
    t.feed(bytes.as_bytes());
    t
}

/// A made-up screen: some earlier output, a tool call that waits for an answer the CLI has no
/// card for, in colour, and the input under it.
const WAITING: &str = concat!(
    "\u{1b}[2mearlier output, not part of the call\u{1b}[0m\r\n\r\n",
    "\u{1b}[32m●\u{1b}[0m \u{1b}[1mBash\u{1b}[0m(./setup.sh)\r\n",
    "  ⎿  Replace the old settings file? \u{1b}[36m[y/N]\u{1b}[0m\r\n\r\n",
    "\u{1b}[38;5;244m────────────────────────────────────────────────────────────\u{1b}[0m\r\n",
    "❯ \r\n",
    "\u{1b}[38;5;244m────────────────────────────────────────────────────────────\u{1b}[0m\r\n",
    "  \u{1b}[2m? for shortcuts\u{1b}[0m",
);

/// The text of the card's rows, without their colour codes.
fn shown(chat: &Chat) -> Vec<String> {
    let metrics = mdshared_ui::termview::Metrics::fixed(12.0, 1.0, 7.0, 15.0);
    let grid = Grid::with_cols(chat.terminal_card.source(), metrics, 200);
    grid.text().lines().map(str::to_owned).collect()
}

fn chat_for(reason: Reason) -> Chat {
    Chat { reason: Some(reason), ..Chat::default() }
}

#[test]
fn which_refusals_show_the_card() {
    let all = [
        Reason::Tool,
        Reason::Bell,
        Reason::Slash,
        Reason::Permission,
        Reason::Identity,
        Reason::Ended,
        Reason::Question,
        Reason::Approval,
        Reason::Menu,
    ];
    for reason in all {
        let shows = matches!(reason, Reason::Tool | Reason::Bell | Reason::Permission);
        assert_eq!(cause(&chat_for(reason)), shows.then_some(Cause::Reason(reason)), "{reason:?}");
        // The terminal in view shows it already.
        let mut open = chat_for(reason);
        open.open_drawer();
        assert_eq!(cause(&open), None, "{reason:?}");
    }
    // Sends refused because the screen cannot be read: while the screen is not ready.
    for words in REFUSALS {
        let mut chat = Chat { error: Some(words), last_gate: Some(Gate::Check), ..Chat::default() };
        assert_eq!(cause(&chat), Some(Cause::Refused(words)), "{words}");
        chat.last_gate = Some(Gate::Running);
        assert_eq!(cause(&chat), Some(Cause::Refused(words)), "{words}");
        chat.last_gate = Some(Gate::Ready);
        assert_eq!(cause(&chat), None, "readable again: {words}");
    }
    // Refusals about the draft or a card's own prompt are not about the screen.
    for words in [submission::BAD_TEXT, submission::REMOVE_ATTACHMENTS, submission::BAD_COMMAND, Reason::Menu.label()] {
        let chat = Chat { error: Some(words), last_gate: Some(Gate::Check), ..Chat::default() };
        assert_eq!(cause(&chat), None, "{words}");
    }
}

#[test]
fn the_rows_come_from_the_live_screen_and_follow_it() {
    let mut t = term(70, 20, WAITING);
    let mut chat = chat_for(Reason::Tool);
    assert!(observe(&mut chat, &t.screen));
    let rows = shown(&chat);
    assert_eq!(rows[0], "● Bash(./setup.sh)", "from the call that waits, not the output before it");
    assert_eq!(rows[1], "  ⎿  Replace the old settings file? [y/N]");
    assert_eq!(rows.last().unwrap(), "  ? for shortcuts", "down to the screen's last row with text");
    assert!(chat.terminal_card.source().contains("\u{1b}[0;38;5;2m●"), "with its colours");
    assert!(!observe(&mut chat, &t.screen), "nothing changed");
    // The screen moves on: the card follows it.
    t.feed("\r\n  typed in the terminal".as_bytes());
    assert!(observe(&mut chat, &t.screen));
    assert_eq!(shown(&chat).last().unwrap(), "  typed in the terminal");
    // Chat can read the screen again: the card goes.
    chat.reason = None;
    assert!(observe(&mut chat, &t.screen));
    assert_eq!(chat.terminal_card.cause, None);
    assert_eq!(chat.terminal_card.source(), "");
}

#[test]
fn at_most_the_bottom_rows_without_a_call_to_start_from() {
    let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    let t = term(40, 40, &lines.join("\r\n"));
    let r = rows(&t.screen);
    assert_eq!(r.len(), MAX_ROWS);
    assert_eq!(t.screen.row_text(r.end - 1), "line 30");
    assert_eq!(t.screen.row_text(r.start), format!("line {}", 30 - MAX_ROWS + 1));
    assert_eq!(rows(&term(40, 10, "").screen), 0..0, "an empty screen has no rows");
}

#[test]
fn the_screens_cells_are_drawn_with_their_colours_and_attributes() {
    let bytes = "\u{1b}[1;31mbold red\u{1b}[0m \u{1b}[44;38;2;10;20;30mrgb on blue\u{1b}[0m \u{1b}[7minverse\u{1b}[0m 界x\r\n\
                 \u{1b}[3;4;9mitalic\u{1b}[0m \u{1b}[38;5;200m256\u{1b}[0m";
    let t = term(60, 5, bytes);
    let (source, rows, cols) = encode(&t.screen, 0..2);
    assert_eq!(rows, 2);
    let metrics = mdshared_ui::termview::Metrics::fixed(12.0, 1.0, 7.0, 15.0);
    let grid = Grid::with_cols(&source, metrics, cols);
    let (copy, original) = (grid.screen(), &t.screen);
    for r in 0..2 {
        for c in 0..cols {
            assert_eq!(copy.cell(r, c), original.cell(r, c), "row {r} column {c}");
        }
    }
    assert_eq!(original.cell(0, 0).fg, Color::Indexed(1));
}

#[test]
fn dismiss_holds_until_the_cause_is_gone_and_answer_opens_the_terminal_with_the_keyboard() {
    let t = term(70, 20, WAITING);
    let mut chat = chat_for(Reason::Tool);
    observe(&mut chat, &t.screen);
    press(&mut chat, Hit::Dismiss);
    assert_eq!(chat.terminal_card.cause, None);
    observe(&mut chat, &t.screen);
    assert_eq!(chat.terminal_card.cause, None, "dismissed for this wait");
    chat.reason = None;
    observe(&mut chat, &t.screen);
    chat.reason = Some(Reason::Tool);
    observe(&mut chat, &t.screen);
    assert_eq!(chat.terminal_card.cause, Some(Cause::Reason(Reason::Tool)), "a new wait shows it again");
    press(&mut chat, Hit::Answer);
    assert!(chat.drawer == Drawer::Manual && chat.drawer_focus, "the drawer, with the keyboard");
    observe(&mut chat, &t.screen);
    assert_eq!(chat.terminal_card.cause, None, "the terminal is in view");
}

#[test]
fn the_card_never_writes_to_the_terminal() {
    let mut t = term(70, 20, WAITING);
    let before: Vec<String> = (0..20).map(|r| t.screen.row_text(r)).collect();
    let mut chat = chat_for(Reason::Tool);
    chat.layout.conversation = Rect::new(0.0, 0.0, 700.0, 600.0);
    chat.layout.box_frame = Rect::new(20.0, 620.0, 660.0, 100.0);
    let (mut gfx, mut bmp) = (mdshared_ui::gfx::Gfx::new(), mdshared_text::Bitmap::new(700, 740));
    for hit in [None, Some(Hit::Dismiss), None, Some(Hit::Answer)] {
        chat.reason = Some(Reason::Tool);
        observe(&mut chat, &t.screen);
        place(&mut chat, 1.0);
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        draw(
            &mut chat,
            &mut p,
            &mdshared_ui::theme::DARK,
            Scheme::from_palette(&mdshared_ui::theme::DARK),
            None,
            &mut Vec::new(),
        );
        if let Some(hit) = hit {
            press(&mut chat, hit);
        }
        t.feed(b"");
    }
    assert!(chat.writes.is_empty(), "{:?}", chat.writes);
    assert_eq!((0..20).map(|r| t.screen.row_text(r)).collect::<Vec<_>>(), before);
}

#[test]
fn the_card_takes_its_room_above_the_box_and_fits_the_terminals_width() {
    let wide = format!("\u{1b}[32m●\u{1b}[0m {}\r\n{}", "x".repeat(150), "─".repeat(200));
    for (text, small) in [(WAITING.to_owned(), false), (wide, true)] {
        let t = term(200, 20, &text);
        let mut chat = chat_for(Reason::Tool);
        chat.layout.conversation = Rect::new(0.0, 0.0, 700.0, 600.0);
        chat.layout.box_frame = Rect::new(20.0, 620.0, 660.0, 100.0);
        observe(&mut chat, &t.screen);
        place(&mut chat, 1.0);
        let card = chat.terminal_card.rect;
        assert!(card.h > 0.0 && card.bottom() <= 600.0 && chat.layout.conversation.bottom() <= card.y);
        let laid = chat.terminal_card.grid.as_ref().unwrap();
        assert_eq!(laid.grid.text().lines().count(), chat.terminal_card.rows, "a terminal row stays one row");
        let size = laid.metrics.font_size();
        if small {
            assert!((MIN_FONT..FONT_SIZE).contains(&size), "{size}");
        } else {
            assert_eq!(size, FONT_SIZE);
            assert!(laid.grid.width() <= laid.width);
        }
    }
    // No room, no card; and none without a cause.
    let t = term(70, 20, WAITING);
    let mut chat = chat_for(Reason::Tool);
    chat.layout.conversation = Rect::new(0.0, 0.0, 700.0, 60.0);
    chat.layout.box_frame = Rect::new(20.0, 80.0, 660.0, 100.0);
    observe(&mut chat, &t.screen);
    place(&mut chat, 1.0);
    assert!(!chat.terminal_card.shown());
    chat.reason = None;
    chat.layout.conversation = Rect::new(0.0, 0.0, 700.0, 600.0);
    observe(&mut chat, &t.screen);
    place(&mut chat, 1.0);
    assert!(!chat.terminal_card.shown() && chat.layout.conversation.h == 600.0);
}

#[test]
fn the_rows_are_painted_in_the_terminal_themes_colours() {
    let t = term(40, 6, "\u{1b}[32m●\u{1b}[0m call\r\n\u{1b}[41m      \u{1b}[0m waits");
    let mut chat = chat_for(Reason::Bell);
    chat.layout.conversation = Rect::new(0.0, 0.0, 500.0, 400.0);
    chat.layout.box_frame = Rect::new(0.0, 420.0, 500.0, 80.0);
    observe(&mut chat, &t.screen);
    place(&mut chat, 1.0);
    let mut scheme = Scheme::from_palette(&mdshared_ui::theme::LIGHT);
    scheme.background = [3, 4, 5];
    scheme.ansi[1] = [210, 20, 30];
    let (mut gfx, mut bmp) = (mdshared_ui::gfx::Gfx::new(), mdshared_text::Bitmap::new(500, 500));
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    let mut hits = Vec::new();
    draw(&mut chat, &mut p, &mdshared_ui::theme::LIGHT, scheme, None, &mut hits);
    let has = |rgb: [u8; 3]| bmp.pixels.chunks(4).any(|px| [px[2], px[1], px[0]] == rgb);
    assert!(has([3, 4, 5]) && has([210, 20, 30]));
    let targets: Vec<_> = hits.iter().map(|(_, t)| *t).collect();
    assert_eq!(targets, [Target::Chat(Control::Screen(Hit::Dismiss)), Target::Chat(Control::Screen(Hit::Answer))]);
}
