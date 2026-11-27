use std::time::{Duration, Instant};

use super::*;
use crate::demo;
use crate::source::{Scan, Source};
use crate::winfile::WindowFile;

/// A demo state whose sessions are one per `(folder, minutes ago)`.
fn state(folders: &[(&str, u64)]) -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), WindowFile::default());
    st.apply_scan(demo::scan(&st.config));
    let template = st.sessions[0].clone();
    let sessions = folders
        .iter()
        .enumerate()
        .map(|(i, (cwd, age))| {
            let mut s = template.clone();
            s.id = format!("folder-test-{i}");
            s.cwd = PathBuf::from(cwd);
            s.last_active_ms = 1_000_000_000_000 - age * 60_000;
            s.archived = false;
            s
        })
        .collect();
    st.apply_scan(Scan { sessions, ..Scan::default() });
    st
}

fn paths(st: &AppState) -> Vec<String> {
    st.recent_folders().iter().map(|f| f.path.to_string_lossy().into_owned()).collect()
}

#[test]
fn one_row_per_folder_ordered_by_its_most_recent_session() {
    let st = state(&[
        (r"C:\w\old", 50),
        (r"C:\w\busy", 90),
        (r"C:\w\new", 10),
        (r"c:/W/Busy/", 5),
        (r"C:\w\\busy\.", 70),
        (r"\\?\C:\w\new\", 60),
        ("", 1),
    ]);
    let found = st.recent_folders();
    assert_eq!(paths(&st), [r"c:\W\Busy", r"C:\w\new", r"C:\w\old"], "the newest session's spelling, cleaned");
    assert_eq!(found[0].last_active_ms, 1_000_000_000_000 - 5 * 60_000);
    assert_eq!((found[1].name.as_str(), found[1].parent.as_str()), ("new", r"C:\w"));
}

#[test]
fn roots_and_shares_keep_their_meaning() {
    assert_eq!(clean(Path::new("C:/")), PathBuf::from(r"C:\"));
    assert_eq!(clean(Path::new(r"\\?\UNC\server\share\x\")), PathBuf::from(r"\\server\share\x"));
    assert_eq!(clean(Path::new(r"\\server\share\x")), PathBuf::from(r"\\server\share\x"));
    let root = &state(&[("e:/", 1)]).recent_folders()[0];
    assert_eq!((root.name.as_str(), root.parent.as_str()), (r"e:\", ""));
}

#[test]
fn archived_and_hidden_sessions_and_hidden_folders_do_not_count() {
    let mut st = state(&[
        (r"C:\w\here-archived", 1),
        (r"C:\w\app-archived", 2),
        (r"C:\w\hidden-session", 3),
        (r"C:\w\hidden-folder", 4),
        (r"c:/w/Hidden-Folder/", 5),
        (r"C:\w\kept", 6),
        (r"C:\w\half", 7),
        (r"C:\w\half", 8),
    ]);
    let id = |st: &AppState, cwd: &str| st.sessions.iter().find(|s| s.cwd == Path::new(cwd)).unwrap().id.clone();
    let archived = id(&st, r"C:\w\here-archived");
    let file = st.sessions[st.by_id[&archived]].file.clone();
    st.marks.archive(&archived, &file, Some(1));
    let desktop = id(&st, r"C:\w\app-archived");
    let i = st.by_id[&desktop];
    st.sessions[i].archived = true;
    let hidden = id(&st, r"C:\w\hidden-session");
    st.marks.hide(&hidden, &file, true);
    st.marks.hide_folder(&folder_key(Path::new(r"C:\w\hidden-folder")), true);
    let newest_half = id(&st, r"C:\w\half");
    st.marks.archive(&newest_half, &file, Some(1));

    st.config.show_archived = false;
    st.config.show_hidden = true;
    assert_eq!(paths(&st), [r"C:\w\kept", r"C:\w\half"], "showing hidden rows does not offer hidden folders");
    assert!(st.recent_folders()[1].last_active_ms < st.recent_folders()[0].last_active_ms);
    st.config.show_archived = true;
    assert_eq!(paths(&st), [r"C:\w\app-archived", r"C:\w\kept", r"C:\w\half"]);
}

/// Runs the folder check and waits for its answer.
fn look(st: &mut AppState) {
    st.start_folder_check();
    let end = Instant::now() + Duration::from_secs(10);
    while st.folders.job.is_some() {
        st.take_folder_check();
        assert!(Instant::now() < end, "the check did not finish");
        std::thread::yield_now();
    }
}

/// `git` in `dir`, untouched by this machine's configuration.
fn git(dir: &Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    let output = command
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "core.hooksPath=NUL"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn a_real_git_worktree_and_its_subfolders_are_left_out_once_the_worker_has_looked() {
    let scratch = crate::testing::Scratch::new("cs-recent-worktree");
    let root = scratch.to_path_buf();
    let (main, linked) = (root.join("main"), root.join("linked"));
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "--quiet"]);
    git(&main, &["commit", "--quiet", "--allow-empty", "-m", "first"]);
    git(&main, &["worktree", "add", "--quiet", "-b", "side", "../linked"]);
    assert!(linked.join(".git").is_file());
    let folders = [main.clone(), linked.clone(), main.join("src"), linked.join("src").join("deep")];
    for folder in &folders {
        std::fs::create_dir_all(folder).unwrap();
    }
    let text: Vec<String> = folders.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut st = state(&text.iter().enumerate().map(|(i, f)| (f.as_str(), i as u64)).collect::<Vec<_>>());
    st.folders.temp.clear();
    assert_eq!(paths(&st).len(), 4, "building the list reads no disk");
    look(&mut st);
    assert_eq!(paths(&st), [text[0].clone(), text[2].clone()]);
    assert_eq!(paths(&st).len(), 2, "the answer is kept");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn temporary_folders_are_left_out_by_name() {
    let env = |name: &str| {
        let value = match name {
            "TEMP" => r"C:\Users\LONGNA~1\AppData\Local\Temp\",
            "TMP" => "D:/scratch",
            "LOCALAPPDATA" => r"C:\Users\longname\AppData\Local",
            "SystemRoot" => r"C:\Windows",
            _ => return None,
        };
        Some(OsString::from(value))
    };
    let temp = temp_roots(env, PathBuf::from(r"E:\system-temp"));
    let mut st = state(&[
        (r"C:\Users\LongNa~1\AppData\Local\Temp\x", 1),
        (r"d:\Scratch", 2),
        (r"C:\Users\longname\AppData\Local\Temp\claude\C--work\1234\scratchpad", 3),
        (r"c:/windows/temp/job", 4),
        (r"E:\system-temp\play", 5),
        (r"D:\scratch-notes", 6),
        (r"C:\work\Temp", 7),
    ]);
    st.folders.temp = temp;
    assert_eq!(paths(&st), [r"D:\scratch-notes", r"C:\work\Temp"]);
    // A variable that is unset, relative or a whole drive rules nothing out.
    let odd = |name: &str| (name == "TEMP").then(|| OsString::from(r"C:\"));
    assert!(temp_roots(odd, PathBuf::from("relative")).is_empty());
}

#[test]
fn this_computer_s_temporary_folder_is_one() {
    let inside = std::env::temp_dir().join("claude").join("scratchpad");
    let st = state(&[(&inside.to_string_lossy(), 1), (r"C:\w\kept", 2)]);
    assert_eq!(paths(&st), [r"C:\w\kept"]);
}

#[test]
fn worker_sessions_count_only_while_the_list_shows_them() {
    let mut st = state(&[(r"C:\w\worker-only", 1), (r"C:\w\mine", 5), (r"C:\w\both", 9), (r"C:\w\both", 2)]);
    for s in &mut st.sessions {
        s.worker = s.cwd == Path::new(r"C:\w\worker-only") || s.last_active_ms == 1_000_000_000_000 - 2 * 60_000;
    }
    st.config.show_workers = false;
    assert_eq!(paths(&st), [r"C:\w\mine", r"C:\w\both"], "a worker's activity does not move a folder up either");
    st.config.show_workers = true;
    assert_eq!(paths(&st), [r"C:\w\worker-only", r"C:\w\both", r"C:\w\mine"]);
}

#[test]
fn a_folder_that_is_not_on_disk_is_dropped_once_the_worker_has_looked() {
    let scratch = crate::testing::Scratch::new("cs-recent-folders");
    let root = scratch.to_path_buf();
    let (kept, gone) = (root.join("kept"), root.join("gone"));
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::create_dir_all(&gone).unwrap();
    let file = root.join("file");
    std::fs::write(&file, "").unwrap();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let mut st = state(&[(&text(&gone), 1), (&text(&kept), 2), (&text(&file), 3)]);
    st.folders.temp.clear();
    look(&mut st);
    assert_eq!(paths(&st), [text(&gone), text(&kept)], "a file is not a folder");
    std::fs::remove_dir(&gone).unwrap();
    assert_eq!(paths(&st).len(), 2, "building the list reads no disk");
    look(&mut st);
    assert_eq!(paths(&st), [text(&kept)]);
    std::fs::create_dir(&gone).unwrap();
    look(&mut st);
    assert_eq!(paths(&st).len(), 2, "a folder that came back is offered again");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_filter_takes_every_word_then_falls_back_to_a_fuzzy_match() {
    let st = state(&[
        (r"C:\Users\d\Desktop\kweb-mail", 1),
        (r"C:\Users\d\Documents\Sfera", 2),
        (r"C:\Users\d\Desktop\claude-switch\bins", 3),
        (r"C:\Users\d\Desktop\mail-notes", 4),
    ]);
    let all = st.recent_folders();
    let names = |q: &str| matching(&all, q).into_iter().map(|i| all[i].name.clone()).collect::<Vec<_>>();
    assert_eq!(names(" "), ["kweb-mail", "Sfera", "bins", "mail-notes"]);
    assert_eq!(names("MAIL"), ["kweb-mail", "mail-notes"], "newest first, whatever the case");
    assert_eq!(names("desktop mail"), ["kweb-mail", "mail-notes"]);
    assert_eq!(names("switch/bins/"), ["bins"], "either separator");
    assert_eq!(names("sfr"), ["Sfera"], "letters in order");
    assert!(names("zzzz").is_empty());
}

#[test]
fn the_list_stops_at_the_limit_and_the_filter_reaches_past_it() {
    let folders: Vec<String> = (0..LIMIT + 20).map(|i| format!(r"C:\w\project-{i:03}")).collect();
    let st = state(&folders.iter().enumerate().map(|(i, f)| (f.as_str(), i as u64)).collect::<Vec<_>>());
    let all = st.recent_folders();
    assert_eq!(matching(&all, "").len(), LIMIT);
    assert_eq!(matching(&all, "project-215"), [215]);
}
