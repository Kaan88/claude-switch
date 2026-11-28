use super::*;
use mdshared_term::Color;

/// The cells of command output in tests: a fixed size, so no font is measured.
pub(in crate::view) fn cells() -> Metrics {
    Metrics::fixed(FONT_SIZE, 1.0, 7.0, 15.0)
}

/// A made-up output shaped like `/context`'s: a heading, a grid of ten by ten cells in several
/// colours (256-colour, true colour, the sixteen, dim), and a legend in a column beside the grid,
/// then a short list under it. Colour codes as Claude Code writes them; no real session's text.
pub(in crate::view) fn context_fixture() -> String {
    const GREY: &str = "\u{1b}[38;5;246m";
    const PURPLE: &str = "\u{1b}[38;2;150;110;220m";
    const YELLOW: &str = "\u{1b}[33m";
    const BLUE: &str = "\u{1b}[38;2;90;160;240m";
    const FREE: &str = "\u{1b}[90m";
    const DIM: &str = "\u{1b}[2m";
    const OFF: &str = "\u{1b}[39m";
    const BOLD: &str = "\u{1b}[1m";
    const UNBOLD: &str = "\u{1b}[22m";
    let cell = |i: usize| match i {
        0..4 => format!("{GREY}⛁{OFF}"),
        4..12 => format!("{PURPLE}⛁{OFF}"),
        12..14 => format!("{YELLOW}⛁{OFF}"),
        14..29 => format!("{BLUE}⛁{OFF}"),
        29 => format!("{BLUE}⛀{OFF}"),
        30..90 => format!("{FREE}⛶{OFF}"),
        _ => format!("{DIM}⛝{UNBOLD}"),
    };
    let legend = [
        format!("{BOLD}model-name-here{UNBOLD} · 48k/200k tokens (24%)"),
        String::new(),
        format!("{GREY}⛁{OFF} System prompt: 3.1k tokens (1.6%)"),
        format!("{PURPLE}⛁{OFF} System tools: 14.2k tokens (7.1%)"),
        format!("{YELLOW}⛁{OFF} Memory files: 2.4k tokens (1.2%)"),
        format!("{BLUE}⛁{OFF} Messages: 28.3k tokens (14.2%)"),
        format!("{FREE}⛶{OFF} Free space: 129k (64.5%)"),
        format!("{DIM}⛝ Autocompact buffer: 22.5k tokens (11.2%){UNBOLD}"),
        String::new(),
        String::new(),
    ];
    let mut out = format!(" {BOLD}Context Usage{UNBOLD}\n");
    for (row, legend) in legend.iter().enumerate() {
        let cells: Vec<String> = (0..10).map(|col| cell(row * 10 + col)).collect();
        out.push_str(&format!("   {}", cells.join(" ")));
        if !legend.is_empty() {
            out.push_str(&format!("   {legend}"));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "\n {BOLD}Memory files{UNBOLD} · /memory\n └ Project (C:\\example\\CLAUDE.md): {DIM}2.4k tokens{UNBOLD}"
    ));
    out
}

/// The text without its escape sequences.
fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let len = if c == '\u{1b}' { escape_len(rest) } else { c.len_utf8() };
        if c != '\u{1b}' {
            out.push(c);
        }
        rest = &rest[len..];
    }
    out
}

fn grid(source: &str, cols: usize) -> Grid {
    Grid::new(source, cells(), cols as f32 * 7.0 + 3.0)
}

/// The first column of `row` that holds an ASCII letter.
fn first_letter(g: &Grid, row: usize) -> Option<usize> {
    (0..g.cols()).find(|&c| g.screen.cell(row, c).ch.is_ascii_alphabetic())
}

#[test]
fn context_output_keeps_its_colours_and_its_columns() {
    let g = grid(&context_fixture(), 100);
    assert_eq!(g.cols(), 100);
    assert_eq!(g.text(), strip(&context_fixture()), "the plain text is the output without its codes");
    // Row 1 is the grid's first row: its first cell in the 256-colour grey, a true colour cell,
    // one of the sixteen; a free cell in bright black.
    let first = (0..g.cols()).find(|&c| g.screen.cell(1, c).ch == '⛁').unwrap();
    assert_eq!(g.screen.cell(1, first).fg, Color::Indexed(246));
    let step = (first + 1..g.cols()).find(|&c| g.screen.cell(1, c).ch == '⛁').unwrap() - first;
    assert_eq!(g.screen.cell(1, first + 4 * step).fg, Color::Rgb(150, 110, 220));
    assert_eq!(g.screen.cell(2, first + 2 * step).fg, Color::Indexed(3));
    assert_eq!(g.screen.cell(4, first).fg, Color::Indexed(8));
    assert!(g.screen.cell(1, 4).attrs.is_empty() && g.screen.cell(0, 1).attrs.contains(Attrs::BOLD));
    // The legend starts in the same column on every row beside the grid: the grid's cells are
    // one cell each.
    let legend: Vec<_> = (3..=8).map(|row| first_letter(&g, row)).collect();
    assert!(legend.iter().all(|c| c.is_some() && *c == legend[0]), "{legend:?}");
    assert_eq!(g.height(), 14.0 * 15.0, "a heading, ten grid rows, a blank line and two lines");
}

#[test]
fn a_narrow_box_wraps_its_lines_and_copies_them_whole() {
    let wide = grid(&context_fixture(), 100);
    let narrow = grid(&context_fixture(), 30);
    assert_eq!(narrow.text(), wide.text(), "a wrapped row continues its line");
    assert!(narrow.height() > wide.height());
    assert!(narrow.width() <= 30.0 * 7.0);
    // A width with as many columns lays out alike.
    assert_eq!(Grid::cols_for(cells(), 30.0 * 7.0 + 6.0), 30);
    assert_eq!(Grid::cols_for(cells(), 0.0), 1, "never no columns");
}

#[test]
fn output_with_no_colour_codes_is_the_same_grid() {
    let plain = strip(&context_fixture());
    let g = grid(&plain, 100);
    assert_eq!(g.text(), plain);
    let coloured = grid(&context_fixture(), 100);
    assert_eq!(
        (3..=8).map(|r| first_letter(&g, r)).collect::<Vec<_>>(),
        (3..=8).map(|r| first_letter(&coloured, r)).collect::<Vec<_>>()
    );
    assert!((0..g.used).all(|r| (0..g.cols()).all(|c| g.screen.cell(r, c).fg == Color::Default)));
}

#[test]
fn only_colour_codes_reach_the_screen() {
    let source = "one\u{1b}[2J\u{1b}[H\u{1b}[?1049htwo\u{1b}[1A\rthree\u{8}\u{7}\u{1b}]0;title\u{7}\u{1b}(0q\u{1b}[?25l\u{1b}[31mred\u{1b}[0m\u{9b}x";
    let g = grid(source, 40);
    assert_eq!(g.text(), "onetwothreeqredx", "nothing cleared, moved, overwritten or switched");
    assert_eq!(g.used, 1);
    assert_eq!(g.screen.cell(0, 12).fg, Color::Indexed(1));
    assert!(!g.screen.modes().alt_screen && g.screen.title().is_empty());
    assert_eq!(
        pieces("a\u{1b}[1;38:2::1:2:3mb\u{1b}[>4;1mc"),
        [Piece::Text("a"), Piece::Sgr("\u{1b}[1;38:2::1:2:3m"), Piece::Text("b"), Piece::Text("c")]
    );
}

#[test]
fn trailing_blank_lines_take_no_room() {
    let g = grid("top\n\u{1b}[0m\n  \n", 20);
    assert_eq!((g.used, g.text()), (1, "top"));
    // A blank line with a background colour is shown.
    let g = grid("top\n\u{1b}[44m    \u{1b}[0m", 20);
    assert_eq!(g.used, 2);
    assert_eq!(grid("", 20).text(), "");
    assert_eq!(grid("", 20).height(), 15.0);
}

#[test]
fn points_and_selections_map_to_the_plain_text() {
    // a b 界 (two cells) c, then x y: text "ab界c\nxy".
    let g = grid("ab界c\nxy", 10);
    assert_eq!(g.text(), "ab界c\nxy");
    assert_eq!(g.at(0.0, 2.0), 0);
    assert_eq!(g.at(7.0 * 2.6, 2.0), 2, "the second cell of a wide character is its start");
    assert_eq!(g.at(7.0 * 4.0, 2.0), 5);
    assert_eq!(g.at(7.0 * 9.0, 2.0), 6, "past the row's text: its end");
    assert_eq!(g.at(7.0, 15.5), 8);
    assert_eq!(g.at(-5.0, 900.0), 7, "outside: clamped to the grid");
    assert_eq!(selected_runs(&g.rows[0], 2..6), vec![2..5]);
    assert_eq!(selected_runs(&g.rows[0], 0..1), vec![0..1]);
    assert_eq!(selected_runs(&g.rows[1], 0..7), Vec::<Range<usize>>::new());
    assert_eq!(selected_runs(&g.rows[1], 0..100), vec![0..2]);
}

#[test]
fn any_bytes_lay_out_without_a_panic() {
    let alphabet = [
        '\u{1b}', '[', ']', '?', '1', '0', '2', '8', ';', ':', 'm', 'H', 'J', '\n', '\r', '\t', 'a', ' ', '⛁', '界',
        '\u{7}', '\\', '\u{8}', '\u{301}', '\u{9b}',
    ];
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for _ in 0..400 {
        let len = next(200);
        let source: String = (0..len).map(|_| alphabet[next(alphabet.len())]).collect();
        let g = grid(&source, 1 + next(60));
        assert!(g.text().chars().all(|c| c == '\n' || !c.is_control()), "{source:?}");
        for (x, y) in [(0.0, 0.0), (1e6, 1e6), (-1.0, 3.0), (33.0, 16.0)] {
            let at = g.at(x, y);
            assert!(at <= g.text().len() && g.text().is_char_boundary(at), "{source:?}");
        }
    }
}

#[test]
fn very_large_output_is_bounded() {
    let g = grid(&"x".repeat(100_000), 10);
    assert!(g.used <= MAX_ROWS && g.screen.rows() <= MAX_ROWS);
    assert!(g.text().starts_with("xxxxxxxxxx"));
    let g = grid(&"\u{1b}[31mline\n".repeat(5000), 80);
    assert_eq!(g.used, MAX_ROWS, "the lines that fit, from the top");
    assert!(g.text().starts_with("line\nline"));
}

#[test]
fn cells_are_painted_in_the_terminal_themes_colours() {
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    let pal = mdshared_ui::theme::LIGHT;
    let mut scheme = Scheme::from_palette(&pal);
    scheme.ansi[1] = [200, 10, 20];
    scheme.background = [1, 2, 3];
    let mut look = Look { font: CellFont::new(), scheme: Some(scheme) };
    // A red background (colour 1 of the theme) on the first four cells, the default after.
    let g = grid("\u{1b}[41m    \u{1b}[0m    ", 8);
    let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(8 * 7, 15));
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    g.draw(&mut p, (0.0, 0.0), Rect::new(0.0, 0.0, 56.0, 15.0), None, &pal, &mut look);
    let px = |x: usize, y: usize| {
        let i = (y * 8 * 7 + x) * 4;
        [bmp.pixels[i + 2], bmp.pixels[i + 1], bmp.pixels[i]]
    };
    assert_eq!(px(3, 7), [200, 10, 20]);
    assert_eq!(px(7 * 6, 7), [1, 2, 3], "the theme's background, not the palette's");
    // Rows outside the clip are not painted.
    bmp.clear([9, 9, 9]);
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    g.draw(&mut p, (0.0, 0.0), Rect::new(0.0, 15.0, 56.0, 30.0), None, &pal, &mut look);
    assert!(bmp.pixels.chunks(4).all(|px| px[..3] == [9, 9, 9]));
}
