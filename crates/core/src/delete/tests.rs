use super::*;
use crate::util::testing::{TempDir, config_with, put};
use serde_json::json;

const ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const OTHER: &str = "bbbbbbbb-0000-4000-8000-000000000002";

/// A projects folder with the session, its own folder, a neighbour and things that are not it.
struct Setup {
    t: TempDir,
    roots: Roots,
    config: Config,
    transcript: PathBuf,
    folder: PathBuf,
    untouched: Vec<PathBuf>,
}

fn setup(tag: &str) -> Setup {
    let t = TempDir::new(tag);
    let roots = t.roots();
    let config = config_with(&roots, &[]);
    let project = roots.primary_cli_dir().join("projects").join("C--work");
    let transcript = project.join(format!("{ID}.jsonl"));
    let folder = project.join(ID);
    put(&transcript, "{\"type\":\"user\"}\n");
    put(&folder.join("subagents").join("agent-1.jsonl"), "{}\n");
    put(&folder.join("tool-results").join("r.txt"), "result");
    let untouched = vec![
        project.join(format!("{OTHER}.jsonl")),
        project.join(OTHER).join("subagents").join("agent-2.jsonl"),
        project.join(format!("{ID}-notes.txt")),
        roots.primary_cli_dir().join("projects").join("C--other").join(format!("{OTHER}.jsonl")),
        t.0.join("appdata").join("Claude").join("claude-code-sessions").join("a").join("o").join("local_1.json"),
    ];
    for path in &untouched {
        put(path, "keep");
    }
    Setup { t, roots, config, transcript, folder, untouched }
}

impl Setup {
    fn bin(&self) -> FolderBin {
        FolderBin::new(self.t.0.join("bin"), self.roots.primary_cli_dir())
    }

    fn untouched(&self) {
        for path in &self.untouched {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "keep", "{}", path.display());
        }
    }
}

#[test]
fn the_transcript_and_its_own_folder_go_to_the_bin_and_nothing_else_does() {
    let s = setup("delete-moves");
    let bin = s.bin();
    let real = std::fs::canonicalize(&s.transcript).unwrap();
    let out = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap();
    assert!(out.folder_error.is_none());
    assert_eq!(out.plan.transcript, real);
    assert_eq!(bin.moved(), vec![out.plan.transcript.clone(), out.plan.folder.clone().unwrap()]);
    assert!(!s.transcript.exists() && !s.folder.exists());
    let kept = std::fs::read_dir(s.t.0.join("bin")).unwrap().count();
    assert_eq!(kept, 2, "both are recoverable from the bin");
    assert!(s.t.0.join("bin").join(format!("1-{ID}")).join("subagents").join("agent-1.jsonl").is_file());
    s.untouched();
}

#[test]
fn a_running_session_a_window_refusal_or_an_unfinished_check_moves_nothing() {
    let s = setup("delete-refuse");
    let bin = s.bin();
    let sessions = s.roots.primary_cli_dir().join("sessions");
    let me = std::process::id();
    put(&sessions.join(format!("{me}.json")), &json!({"pid": me, "sessionId": ID.to_ascii_uppercase()}).to_string());
    let e = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || panic!("checked after the running check"));
    assert!(matches!(e, Err(Refusal::AlreadyRunning(_))), "{e:?}");
    std::fs::remove_dir_all(&sessions).unwrap();

    let e = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Err("It is pinned.".into())).unwrap_err();
    assert_eq!(e.to_string(), "It is pinned.");

    put(&sessions, "not a directory");
    let e = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap_err();
    assert!(e.to_string().contains("Nothing was deleted"), "{e}");
    std::fs::remove_file(&sessions).unwrap();

    assert!(bin.moved().is_empty());
    assert!(s.transcript.is_file() && s.folder.is_dir());
    s.untouched();
}

#[test]
fn a_session_started_a_moment_ago_is_not_deleted() {
    let s = setup("delete-reserved");
    let bin = s.bin();
    crate::launch::reserve(&s.roots, &s.config, ID, "main").unwrap().finish(true).unwrap();
    let e = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap_err();
    assert!(e.to_string().contains("a moment ago"), "{e}");
    assert!(bin.moved().is_empty() && s.transcript.is_file());
}

#[test]
fn a_folder_that_does_not_move_is_reported_and_the_transcript_still_went() {
    let s = setup("delete-partial");
    let mut bin = s.bin();
    bin.refuse = Some(ID.into());
    let out = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap();
    assert!(out.folder_error.as_deref().is_some_and(|e| e.contains("refused")), "{:?}", out.folder_error);
    assert!(!s.transcript.exists() && s.folder.is_dir());
    s.untouched();

    // The other way round: the transcript does not move, so the folder is not tried.
    let s = setup("delete-transcript-refused");
    let mut bin = s.bin();
    bin.refuse = Some(format!("{ID}.jsonl"));
    let e = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap_err();
    assert!(e.to_string().contains("Nothing was deleted"), "{e}");
    assert!(bin.moved().is_empty() && s.transcript.is_file() && s.folder.is_dir());
}

#[test]
fn a_session_without_a_folder_moves_just_its_transcript() {
    let s = setup("delete-no-folder");
    std::fs::remove_dir_all(&s.folder).unwrap();
    let bin = s.bin();
    let out = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap();
    assert_eq!((out.plan.folder, out.folder_error), (None, None));
    assert_eq!(bin.moved().len(), 1);
    s.untouched();
}

#[test]
fn only_a_session_transcript_in_a_projects_folder_is_planned() {
    let s = setup("delete-plan");
    assert!(plan(&s.transcript, OTHER).is_err(), "another id");
    assert!(plan(&s.transcript, "not-a-uuid").is_err());
    assert!(plan(&s.transcript.with_extension("json"), ID).is_err(), "not a transcript");
    let stray = s.t.0.join("elsewhere").join(format!("{ID}.jsonl"));
    put(&stray, "{}");
    assert!(plan(&stray, ID).unwrap_err().to_string().contains("projects folder"));
    let dir = s.roots.primary_cli_dir().join("projects").join("C--dir").join(format!("{ID}.jsonl"));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(plan(&dir, ID).is_err(), "a folder named like a transcript");
    let missing = s.transcript.with_file_name(format!("{}.jsonl", "cccccccc-0000-4000-8000-000000000003"));
    assert!(plan(&missing, "cccccccc-0000-4000-8000-000000000003").is_err());
    assert!(plan(&s.transcript, &ID.to_ascii_uppercase()).is_ok(), "ids compare without case");
}

#[test]
fn the_test_bin_moves_nothing_outside_its_folder() {
    let s = setup("delete-fake-bin");
    let bin = FolderBin::new(s.t.0.join("bin"), s.t.0.join("home").join("nowhere"));
    std::fs::create_dir_all(s.t.0.join("home").join("nowhere")).unwrap();
    assert!(bin.recycle(&s.transcript).is_err());
    assert!(s.transcript.is_file() && bin.moved().is_empty());
}

#[test]
fn the_shell_gets_paths_without_the_long_path_prefix() {
    assert_eq!(shell_path(Path::new(r"\\?\C:\a\b.jsonl")), PathBuf::from(r"C:\a\b.jsonl"));
    assert_eq!(shell_path(Path::new(r"\\?\UNC\srv\share\b")), PathBuf::from(r"\\srv\share\b"));
    assert_eq!(shell_path(Path::new(r"C:\a")), PathBuf::from(r"C:\a"));
}

#[cfg(windows)]
#[test]
fn a_transcript_reached_through_another_accounts_shared_projects_is_moved_once() {
    let s = setup("delete-shared");
    let config = config_with(&s.roots, &[("work", "Work")]);
    let work = config.accounts[1].cli_dir.clone();
    std::fs::create_dir_all(&work).unwrap();
    crate::share::junction::create(&work.join("projects"), &s.roots.primary_cli_dir().join("projects")).unwrap();
    let through = work.join("projects").join("C--work").join(format!("{ID}.jsonl"));
    assert!(through.is_file());
    let bin = FolderBin::new(s.t.0.join("bin"), s.t.0.clone());
    let out = delete(&s.roots, &config, ID, &through, &bin, || Ok(())).unwrap();
    assert_eq!(
        out.plan.transcript,
        std::fs::canonicalize(s.transcript.parent().unwrap()).unwrap().join(format!("{ID}.jsonl"))
    );
    assert_eq!(bin.moved().len(), 2, "the one real transcript and its folder, once");
    assert!(!through.exists() && !s.transcript.exists() && !s.folder.exists());
    assert!(work.join("projects").is_dir(), "the shared link itself stays");
    s.untouched();
}

#[cfg(windows)]
#[test]
fn a_linked_session_folder_is_left_in_place_and_not_followed() {
    let s = setup("delete-linked-folder");
    std::fs::remove_dir_all(&s.folder).unwrap();
    let elsewhere = s.t.0.join("elsewhere");
    put(&elsewhere.join("precious.txt"), "keep");
    crate::share::junction::create(&s.folder, &elsewhere).unwrap();
    let bin = FolderBin::new(s.t.0.join("bin"), s.t.0.clone());
    let out = delete(&s.roots, &s.config, ID, &s.transcript, &bin, || Ok(())).unwrap();
    assert_eq!(out.plan.folder, None);
    assert!(out.plan.linked_folder.as_ref().is_some_and(|p| p.ends_with(ID)), "{:?}", out.plan.linked_folder);
    assert_eq!(bin.moved().len(), 1);
    assert_eq!(std::fs::read_to_string(elsewhere.join("precious.txt")).unwrap(), "keep");
    s.untouched();
}
