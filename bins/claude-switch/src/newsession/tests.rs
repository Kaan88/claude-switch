use std::time::Duration;

use mdshared_text::Bitmap;
use mdshared_ui::gfx::Gfx;
use mdshared_ui::theme;
use mdshared_ui::tooltip::Over;

use super::list::{ROW_H, ROWS};
use super::*;
use crate::source::{Scan, Source};
use crate::winfile::WindowFile;
use crate::{actions, demo, dialogs};

fn demo_state() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), WindowFile::default());
    st.apply_scan(demo::scan(&st.config));
    st
}

/// A demo state with one session in each folder, the first the most recent, the dialog open.
fn state_with(folders: &[String]) -> AppState {
    let mut st = demo_state();
    let template = st.sessions[0].clone();
    let now = crate::list::now_ms();
    let sessions = folders
        .iter()
        .enumerate()
        .map(|(i, cwd)| {
            let mut s = template.clone();
            s.id = format!("picker-test-{i}");
            s.cwd = PathBuf::from(cwd);
            s.last_active_ms = now - (i as u64 * i as u64 * 20 + 3) * 60_000;
            s.archived = false;
            s
        })
        .collect();
    st.apply_scan(Scan { sessions, ..Scan::default() });
    st.selected = None;
    st.ask_new_session();
    st
}

fn numbered(n: usize) -> Vec<String> {
    (0..n).map(|i| format!(r"C:\work\project-{i:02}")).collect()
}

fn picker(st: &AppState) -> &NewSession {
    match &st.dialog {
        Some(Dialog::NewSession(picker)) => picker,
        _ => panic!("expected the New session dialog"),
    }
}

fn listed(st: &AppState) -> Vec<PathBuf> {
    picker(st).list.folders().map(|f| f.path.clone()).collect()
}

fn highlighted(st: &AppState) -> Option<usize> {
    picker(st).list.highlight
}

fn press(st: &mut AppState, key: KeyName) {
    dialogs::key(st, Instant::now(), &key, Mods::default(), None);
}

/// Draws the open dialog as the window does and gives the bitmap and its hits.
fn paint(st: &mut AppState, (w, h): (f32, f32), scale: f32) -> (Bitmap, Vec<(Rect, Target)>) {
    let mut bitmap = Bitmap::new((w * scale) as u32, (h * scale) as u32);
    let mut gfx = Gfx::new();
    let mut p = Painter::new(&mut bitmap, &mut gfx, scale);
    p.fill(Rect::new(0.0, 0.0, w, h), theme::DARK.page);
    let mut hits = Vec::new();
    let (hover, tip) = (st.hover, st.tip.shown().cloned());
    let mut d =
        super::super::Draw { hits: &mut hits, hover, pressed: None, pal: theme::DARK, radius: 9.0, shadow: 0.0 };
    match &mut st.dialog {
        Some(Dialog::NewSession(picker)) => picker.draw(&mut d, &mut p, w, h),
        _ => panic!("expected the New session dialog"),
    }
    if let Some(tip) = tip {
        let bubble = mdshared_ui::overlay::Bubble::tip(&mut p, &tip.text, tip.anchor, w, h);
        bubble.draw(&mut p, &theme::DARK);
    }
    st.hits = hits.clone();
    (bitmap, hits)
}

fn row_hits(hits: &[(Rect, Target)]) -> Vec<(Rect, usize)> {
    hits.iter()
        .filter_map(|(r, t)| match t {
            Target::Dialog(id) if *id >= ROW => Some((*r, (*id - ROW) as usize)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_new_session_command_asks_with_the_selected_folder_or_home_and_current_account() {
    let mut st = demo_state();
    let now = Instant::now();
    let id = st.sessions[0].id.clone();
    st.select(&id);
    st.switch_account("work");
    st.command(now, actions::NEW_SESSION, None);
    let modal = picker(&st);
    assert_eq!(modal.field.text(), st.sessions[0].cwd.to_string_lossy());
    assert_eq!(modal.accounts[modal.account].0, "work");
    assert!(st.toasts.is_empty(), "asking doesn't start anything");
    assert_eq!(listed(&st).len(), st.recent_folders().len(), "the prefilled folder filters nothing");
    dialogs::key(&mut st, now, &KeyName::Escape, Mods::default(), None);
    assert!(st.dialog.is_none() && st.toasts.is_empty());
    st.selected = None;
    st.command(now, actions::NEW_SESSION, None);
    assert_eq!(picker(&st).field.text(), st.source.home().to_string_lossy());
}

#[test]
fn a_picked_folder_fills_the_field_and_keeps_the_dialog_open() {
    let mut st = demo_state();
    let now = Instant::now();
    st.ask_new_session();
    dialogs::click(&mut st, now, BROWSE, 0.0);
    assert!(matches!(st.dialog, Some(Dialog::NewSession(_))), "Browse keeps the dialog (demo opens no picker)");
    let folder = st.sessions[1].cwd.clone();
    st.new_session_folder_picked(&folder);
    let modal = picker(&st);
    assert_eq!(modal.field.text(), folder.to_string_lossy());
    assert!(modal.error.is_none());
    st.dialog = None;
    st.new_session_folder_picked(&folder);
    assert!(st.dialog.is_none(), "a late pick after the dialog closed opens nothing");
}

#[test]
fn choosing_a_folder_loads_its_default_and_successful_start_remembers_an_override() {
    let mut st = demo_state();
    let now = Instant::now();
    st.ask_new_session();
    let cwd = listed(&st)[1].clone();
    st.remember_folder_open(&cwd, "work", App::Desktop);
    dialogs::click(&mut st, now, ROW + 1, 0.0);
    assert_eq!(picker(&st).accounts[picker(&st).account].0, "work");
    assert_eq!(picker(&st).app, App::Desktop);
    dialogs::click(&mut st, now, CLI, 0.0);
    dialogs::click(&mut st, now, OK, 0.0);
    assert_eq!(st.folder_open(&cwd, App::Desktop), ("work", App::Cli));
    assert_eq!(st.config.current, "main");
}

#[test]
fn enter_refuses_an_invalid_folder_then_starts_with_the_chosen_folder_and_account() {
    let mut st = demo_state();
    let now = Instant::now();
    st.ask_new_session();
    dialogs::text_input(&mut st, r"C:\made-up\missing");
    dialogs::key(&mut st, now, &KeyName::Enter, Mods::default(), None);
    assert!(picker(&st).error.is_some());
    assert!(st.toasts.is_empty());
    let mut st = demo_state();
    st.ask_new_session();
    let chosen = listed(&st)[1].clone();
    dialogs::click(&mut st, now, ROW + 1, 0.0);
    assert_eq!(picker(&st).field.text(), chosen.to_string_lossy());
    press(&mut st, KeyName::Tab);
    press(&mut st, KeyName::Right);
    assert_eq!(picker(&st).accounts[picker(&st).account].0, "work");
    press(&mut st, KeyName::Enter);
    assert!(st.dialog.is_none() && !st.toasts.is_empty());
    assert_eq!(st.config.current, "main", "the pick is only for this launch");
}

#[test]
fn real_folder_validation_refuses_blank_missing_and_file_paths() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(folder_from_text(&here.to_string_lossy(), Path::is_dir), Ok(here.to_owned()));
    assert!(folder_from_text(" ", Path::is_dir).is_err());
    let file = here.join("Cargo.toml");
    assert!(folder_from_text(&file.to_string_lossy(), Path::is_dir).is_err());
    assert!(folder_from_text(&file.join("missing").to_string_lossy(), Path::is_dir).is_err());
}

#[test]
fn the_arrows_enter_the_list_and_move_through_it_and_enter_picks_before_it_starts() {
    let mut st = state_with(&numbered(30));
    paint(&mut st, (900.0, 700.0), 1.0);
    assert_eq!(picker(&st).list.page(), ROWS);
    assert_eq!((highlighted(&st), picker(&st).focus), (None, FOLDER));
    press(&mut st, KeyName::Down);
    assert_eq!((highlighted(&st), picker(&st).focus), (Some(0), LIST));
    press(&mut st, KeyName::Down);
    press(&mut st, KeyName::Down);
    press(&mut st, KeyName::Up);
    assert_eq!(highlighted(&st), Some(1));
    assert_eq!(picker(&st).list.scroll(), 0.0);
    press(&mut st, KeyName::PageDown);
    assert_eq!(highlighted(&st), Some(9));
    assert_eq!(picker(&st).list.scroll(), 2.0 * ROW_H, "the highlight is brought into view");
    press(&mut st, KeyName::End);
    assert_eq!(highlighted(&st), Some(29));
    assert_eq!(picker(&st).list.scroll(), (30 - ROWS) as f32 * ROW_H);
    press(&mut st, KeyName::Down);
    assert_eq!(highlighted(&st), Some(29), "no wrapping");
    press(&mut st, KeyName::PageUp);
    assert_eq!(highlighted(&st), Some(21));
    press(&mut st, KeyName::Home);
    assert_eq!((highlighted(&st), picker(&st).list.scroll()), (Some(0), 0.0));
    press(&mut st, KeyName::Down);
    press(&mut st, KeyName::Enter);
    assert_eq!(picker(&st).field.text(), r"C:\work\project-01");
    assert_eq!((highlighted(&st), picker(&st).focus), (None, FOLDER));
    assert!(st.toasts.is_empty(), "the first Enter only picks");
    press(&mut st, KeyName::Enter);
    assert!(st.dialog.is_none() && !st.toasts.is_empty());
}

#[test]
fn tab_keeps_its_three_stops_and_home_and_end_stay_the_field_s_until_the_list_has_the_keyboard() {
    let mut st = state_with(&numbered(5));
    for want in [ACCOUNT, APP, FOLDER] {
        press(&mut st, KeyName::Tab);
        assert_eq!(picker(&st).focus, want);
    }
    press(&mut st, KeyName::Home);
    press(&mut st, KeyName::End);
    assert_eq!((highlighted(&st), picker(&st).focus), (None, FOLDER));
    press(&mut st, KeyName::Down);
    press(&mut st, KeyName::Tab);
    assert_eq!(picker(&st).focus, ACCOUNT);
    press(&mut st, KeyName::Down);
    assert_eq!(highlighted(&st), Some(0), "arrows on the account do not move the list");
}

#[test]
fn the_wheel_and_the_scrollbar_scroll_and_only_rows_in_view_take_a_click() {
    let mut st = state_with(&numbered(30));
    let (_, hits) = paint(&mut st, (900.0, 700.0), 1.0);
    assert_eq!(row_hits(&hits).len(), ROWS);
    let list_top = row_hits(&hits)[0].0.y;
    let max = (30 - ROWS) as f32 * ROW_H;

    dialogs::wheel(&mut st, 0.0, 0.0, -70.0);
    assert_eq!(picker(&st).list.scroll(), 70.0);
    let (_, hits) = paint(&mut st, (900.0, 700.0), 1.0);
    let rows = row_hits(&hits);
    assert_eq!(rows[0].1, 2, "row 2 is half in view");
    assert!(rows.iter().all(|(r, _)| r.y >= list_top && r.bottom() <= list_top + ROWS as f32 * ROW_H));
    dialogs::click(&mut st, Instant::now(), ROW + rows[1].1 as u32, 0.0);
    assert_eq!(picker(&st).field.text(), r"C:\work\project-03");
    assert_eq!(picker(&st).list.scroll(), 70.0, "picking does not move the list");

    dialogs::wheel(&mut st, 0.0, 0.0, -10_000.0);
    assert_eq!(picker(&st).list.scroll(), max);
    dialogs::wheel(&mut st, 0.0, 0.0, 10_000.0);
    assert_eq!(picker(&st).list.scroll(), 0.0);

    paint(&mut st, (900.0, 700.0), 1.0);
    let (track, thumb) = picker(&st).list.track().unwrap();
    assert!(!st.folder_scroll_press(track.x - 20.0, thumb.y), "the rows are not the scrollbar");
    assert!(st.folder_scroll_press(thumb.cx(), thumb.y + 4.0));
    assert_eq!(picker(&st).list.scroll(), 0.0, "taking the thumb does not move it");
    assert!(st.folder_scroll_drag(track.bottom() + 50.0));
    assert_eq!(picker(&st).list.scroll(), max);
    assert!(st.folder_scroll_release());
    assert!(!st.folder_scroll_drag(track.y), "released");
    assert!(st.folder_scroll_press(track.cx(), track.y + 1.0), "a press on the track jumps there");
    assert_eq!(picker(&st).list.scroll(), 0.0);
}

#[test]
fn a_small_window_shows_fewer_rows_and_keeps_the_footer() {
    for (opencode, size) in [(false, (760.0, 480.0)), (true, (760.0, 480.0)), (false, (900.0, 700.0))] {
        let mut st = state_with(&numbered(30));
        if opencode {
            st.dialog = Some(super::super::new_opencode_dialog(&st));
        }
        let (_, hits) = paint(&mut st, size, 1.0);
        let rows = row_hits(&hits);
        assert!((4..=ROWS).contains(&rows.len()), "{} rows", rows.len());
        let ok = hits.iter().find(|(_, t)| *t == Target::Dialog(OK)).unwrap().0;
        let app = hits.iter().filter(|(_, t)| matches!(t, Target::Dialog(CLI))).map(|(r, _)| r.bottom());
        let bottom = app.fold(rows.last().unwrap().0.bottom(), f32::max);
        assert!(bottom <= ok.y && ok.bottom() <= size.1);
    }
}

#[test]
fn typing_narrows_the_list_and_enter_takes_the_first_match() {
    let mut folders = numbered(30);
    folders.push(r"C:\Users\demo\Documents\Sfera".into());
    folders.push(r"C:\Users\demo\Documents\Sfera archive".into());
    let mut st = state_with(&folders);
    dialogs::text_input(&mut st, "sfer");
    assert_eq!(listed(&st), [PathBuf::from(&folders[30]), PathBuf::from(&folders[31])]);
    assert_eq!(highlighted(&st), Some(0));
    assert_eq!(picker(&st).list.count().as_deref(), Some("2 of 32"));
    press(&mut st, KeyName::Down);
    press(&mut st, KeyName::Enter);
    assert_eq!(picker(&st).field.text(), folders[31]);
    assert_eq!(listed(&st).len(), 2, "picking leaves the list as it is");

    st.dialog = None;
    st.ask_new_session();
    dialogs::text_input(&mut st, "nothing like it");
    assert!(listed(&st).is_empty() && highlighted(&st).is_none());
    press(&mut st, KeyName::Down);
    assert_eq!(picker(&st).focus, FOLDER);
    press(&mut st, KeyName::Enter);
    assert!(picker(&st).error.is_some());
    for _ in 0.."nothing like it".len() {
        press(&mut st, KeyName::Backspace);
    }
    assert_eq!(listed(&st).len(), 32, "an empty field filters nothing");
    assert!(highlighted(&st).is_none() && picker(&st).error.is_none());
}

#[test]
fn typing_from_the_list_goes_on_filtering_and_a_typed_path_is_left_for_enter_to_start() {
    let mut st = state_with(&[r"C:\work\app\sub".to_string(), r"C:\work\app".to_string()]);
    dialogs::text_input(&mut st, "ap");
    press(&mut st, KeyName::Down);
    assert_eq!(picker(&st).focus, LIST);
    dialogs::text_input(&mut st, "p");
    assert_eq!((picker(&st).field.text(), picker(&st).focus), ("app", FOLDER));

    st.dialog = None;
    st.ask_new_session();
    dialogs::text_input(&mut st, r"C:\work\app");
    assert_eq!(listed(&st).len(), 2);
    assert!(highlighted(&st).is_none());
    press(&mut st, KeyName::Enter);
    assert!(st.dialog.is_none(), "Enter started in the typed folder, not in the first match");
}

#[test]
fn a_new_scan_updates_the_open_list_and_keeps_the_filter_and_the_highlight() {
    let mut st = state_with(&numbered(12));
    dialogs::text_input(&mut st, "project-1");
    press(&mut st, KeyName::Down);
    assert_eq!(highlighted(&st), Some(1));
    let on = listed(&st)[1].clone();
    let mut sessions = st.sessions.clone();
    let mut newer = sessions[0].clone();
    newer.id = "newer".into();
    newer.cwd = PathBuf::from(r"C:\work\project-100");
    newer.last_active_ms += 1_000;
    sessions.insert(0, newer.clone());
    st.apply_scan(Scan { sessions, ..Scan::default() });
    assert_eq!(listed(&st)[0], newer.cwd);
    assert_eq!(listed(&st)[highlighted(&st).unwrap()], on);
}

#[test]
fn a_row_tooltip_gives_the_full_path() {
    let mut st = state_with(&numbered(3));
    assert_eq!(st.dialog_tip(Target::Dialog(ROW + 2)).as_deref(), Some(r"C:\work\project-02"));
    assert_eq!(st.dialog_tip(Target::Dialog(ROW + 3)), None);
    assert_eq!(st.dialog_tip(Target::Dialog(OK)), None);
    st.dialog = None;
    assert_eq!(st.dialog_tip(Target::Dialog(ROW)), None);
}

/// Set `CS_DIALOG_TEST_SHOTS` to a folder to keep the pictures.
#[test]
fn the_dialog_draws_with_a_long_list() {
    let mut folders: Vec<String> = [
        r"C:\Users\demo\Desktop\kweb-mail-for-stalwart",
        r"C:\Users\demo\Documents\Sfera",
        r"C:\Users\demo\Desktop\cs-wt\handover\bins\claude-switch\src",
        r"C:\Users\demo\Desktop\claude-switch",
        r"C:\Users\demo\Desktop\markdown-shared-rs",
        r"D:\media\subtitles\season 2\a folder with a rather long name that goes on and on and on",
        r"C:\Users\demo\AppData\Local\Temp\cs-rec\play4",
        r"C:\Users\demo\Desktop\very\deeply\nested\projects\kept\for\many\years\homelab\ansible\roles",
        r"E:\",
    ]
    .map(String::from)
    .to_vec();
    folders.extend((0..40).map(|i| format!(r"C:\Users\demo\Desktop\project-{i:02}")));
    let shots = std::env::var_os("CS_DIALOG_TEST_SHOTS").map(PathBuf::from);
    let save = |bitmap: &Bitmap, name: &str| {
        if let Some(folder) = &shots {
            bitmap.write_png(&folder.join(name)).unwrap();
        }
    };

    let mut st = state_with(&folders);
    // The pointer rests on the third row, long enough for its tooltip.
    let (_, hits) = paint(&mut st, (900.0, 700.0), 1.5);
    let (anchor, _) = row_hits(&hits)[2];
    let key = Target::Dialog(ROW + 2);
    let now = Instant::now();
    st.hover = Some(key);
    st.tip.hover(now, Some(Over { key, text: st.dialog_tip(key).unwrap(), anchor }));
    st.tip.tick(now + Duration::from_secs(5));
    let (bitmap, hits) = paint(&mut st, (900.0, 700.0), 1.5);
    assert_eq!(row_hits(&hits).len(), ROWS);
    save(&bitmap, "new-session.png");

    st.hover = None;
    st.tip.hide();
    for _ in 0..11 {
        press(&mut st, KeyName::Down);
    }
    save(&paint(&mut st, (900.0, 700.0), 1.5).0, "new-session-keyboard.png");
    st.dialog = None;
    st.ask_new_session();
    dialogs::text_input(&mut st, "desktop s");
    save(&paint(&mut st, (900.0, 700.0), 1.5).0, "new-session-filter.png");
    st.dialog = None;
    st.ask_new_session();
    save(&paint(&mut st, (760.0, 480.0), 1.0).0, "new-session-small.png");
}
