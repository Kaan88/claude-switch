use super::*;
use crate::dialogs::{self, OK};
use claude_switch_core::delete::FolderBin;
use claude_switch_core::marks::Marks;
use claude_switch_core::meaning::{Cache, Digest};
use claude_switch_core::{Roots, Session};
use std::path::{Path, PathBuf};

const ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const OTHER: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const NEW: &str = "cccccccc-0000-4000-8000-000000000003";

/// A window over temporary folders: one session (`ID`) with its own folder, a neighbour
/// (`OTHER`), and a test bin; the real Recycle Bin is never reached.
struct Real {
    st: AppState,
    root: PathBuf,
    roots: Roots,
    bin: Arc<FolderBin>,
    transcript: PathBuf,
    folder: PathBuf,
    neighbour: PathBuf,
}

impl Drop for Real {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn session(st: &AppState, id: &str, file: &Path) -> Session {
    let mut s = crate::demo::scan(&st.config).sessions[0].clone();
    s.id = id.into();
    s.file = file.to_path_buf();
    s.cwd = PathBuf::from(r"C:\work\delete-test");
    s.title = format!("Title of {}", &id[..8]);
    s.desktop_id = None;
    s.desktop_account = None;
    s
}

fn real(tag: &str) -> Real {
    let root = crate::testing::unique(&format!("cs-delete-{tag}"));
    let _ = std::fs::remove_dir_all(&root);
    let roots = Roots { home: root.join("home"), appdata: root.join("appdata") };
    let project = roots.primary_cli_dir().join("projects").join("C--work-delete-test");
    let transcript = project.join(format!("{ID}.jsonl"));
    let folder = project.join(ID);
    let neighbour = project.join(format!("{OTHER}.jsonl"));
    put(&transcript, "{\"type\":\"user\"}\n");
    put(&folder.join("subagents").join("agent-1.jsonl"), "{}\n");
    put(&neighbour, "keep");
    let mut st = AppState::new(Source::Real(roots.clone()), None, Default::default());
    st.sessions = vec![session(&st, ID, &transcript), session(&st, OTHER, &neighbour)];
    st.by_id = st.sessions.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
    st.rebuild_view();
    st.selected = Some(ID.into());
    let bin = Arc::new(FolderBin::new(root.join("bin"), root.clone()));
    st.recycle = bin.clone();
    Real { st, root, roots, bin, transcript, folder, neighbour }
}

impl Real {
    fn kept(&self) {
        assert!(self.bin.moved().is_empty(), "{:?}", self.bin.moved());
        assert!(self.transcript.is_file() && self.folder.is_dir());
        assert!(self.st.by_id.contains_key(ID));
    }

    fn deleted(&self) {
        assert_eq!(self.bin.moved().len(), 2, "the transcript and its folder");
        assert!(!self.transcript.exists() && !self.folder.exists());
        assert!(!self.st.by_id.contains_key(ID) && self.st.by_id.contains_key(OTHER));
        assert_eq!(std::fs::read_to_string(&self.neighbour).unwrap(), "keep");
    }

    fn info(&self) -> (String, String) {
        self.st.info.as_ref().map(|i| (i.title().to_owned(), i.message().to_owned())).unwrap_or_default()
    }
}

fn digest(summary: &str) -> Digest {
    Digest { basis: 1, active_ms: 1, engine: "claude".into(), summary: summary.into(), keywords: vec![], subject: None }
}

#[test]
fn the_row_menu_offers_delete_after_archive_and_copy_then_delete_after_open_a_copy() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    let free = st
        .sessions
        .iter()
        .find(|s| st.delete_block(&s.id).is_none() && st.entry_of(&s.id).is_some())
        .unwrap()
        .id
        .clone();
    st.row_menu(st.entry_of(&free).unwrap(), mdshared_ui::gfx::Rect::default());
    let (_, menu) = st.menu.take().unwrap();
    let ids: Vec<u32> = menu.items.iter().map(|i| i.id).collect();
    let at = |id| ids.iter().position(|&x| x == id).unwrap();
    assert_eq!(at(DELETE_ROW), at(crate::archive::ARCHIVE_ROW) + 1);
    assert_eq!(at(COPY_DELETE), at(crate::actions::OPEN_COPY) + 1);
    let item = &menu.items[at(DELETE_ROW)];
    assert_eq!((item.label.as_str(), item.enabled, item.hint.as_str()), ("Delete session…", true, ""));
    let hide = &menu.items[at(crate::actions::HIDE)];
    assert_eq!((hide.label.as_str(), hide.hint.as_str()), ("Session hidden", "Delete"), "the Delete key still hides");
}

#[test]
fn running_open_in_a_tab_or_pinned_disables_the_item_with_the_reason() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    let id = st.sessions.iter().find(|s| st.delete_block(&s.id).is_none()).unwrap().id.clone();
    let file = st.sessions[st.by_id[&id]].file.clone();
    assert!(row_item(st.delete_block(&id)).enabled);

    st.running_ids.insert(id.clone());
    assert_eq!(st.delete_block(&id), Some("Running"));
    st.running_ids.remove(&id);

    let tab = st.tabs.open(
        crate::tabs::NewTab { label: "t".into(), color: [0; 3], session: Some(id.clone()), account: None },
        crate::term::Term::empty((8, 4)),
    );
    assert_eq!(st.delete_block(&id), Some("Open in a tab"));
    st.tabs.list.retain(|t| t.id != tab);

    st.marks.pin(&id, &file, true);
    assert_eq!(st.delete_block(&id), Some("Pinned"));
    for (block, item) in [("Pinned", row_item(Some("Pinned"))), ("Running", copy_item(Some("Running")))] {
        assert!(!item.enabled);
        assert_eq!(item.hint, block);
    }
    st.selected = Some(id.clone());
    st.delete_action(DELETE_ROW);
    assert!(st.dialog.is_none(), "no confirmation for a blocked session");
    assert!(st.info.as_ref().is_some_and(|i| i.message().contains("pinned")));
}

#[test]
fn the_confirmation_names_the_session_its_folder_and_the_recycle_bin_and_cancel_is_the_default() {
    let mut st = AppState::new(Source::Demo, None, Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    let s = st.sessions.iter().find(|s| s.desktop_id.is_some() && st.delete_block(&s.id).is_none()).unwrap().clone();
    st.selected = Some(s.id.clone());
    st.delete_action(DELETE_ROW);
    let Some(Dialog::Confirm { title, text, ok, then }) = &st.dialog else { panic!("no confirmation") };
    assert_eq!((title.as_str(), ok.as_str()), ("Delete session?", "Delete"));
    assert!(text.contains(st.title_of(&s)) && text.contains(&s.cwd.display().to_string()), "{text}");
    assert!(text.contains("moved to the Recycle Bin"), "{text}");
    assert!(text.contains("desktop app") && text.contains("may still list it"), "{text}");
    assert!(text.contains("Demo: nothing is deleted"), "{text}");
    assert!(!then.ok_is_primary(), "Delete is not the default button");
    // Enter takes the default: Cancel.
    dialogs::key(&mut st, Instant::now(), &mdshared_ui::keys::KeyName::Enter, Default::default(), None);
    assert!(st.dialog.is_none() && st.by_id.contains_key(&s.id));
    // The demo's Delete deletes nothing and says so.
    st.delete_action(DELETE_ROW);
    dialogs::click(&mut st, Instant::now(), OK, 0.0);
    assert!(st.by_id.contains_key(&s.id));
    assert!(st.toasts.find(|t| t.contains("Demo") && t.contains("not deleted")).is_some());

    st.delete_action(COPY_DELETE);
    let Some(Dialog::Confirm { title, text, then, .. }) = &st.dialog else { panic!("no confirmation") };
    assert!(title.contains("Open a copy, then delete the original"));
    assert!(text.contains("Once Claude Code has written the copy") && text.contains("Recycle Bin"), "{text}");
    assert!(matches!(then, Pending::Copy { delete: true, .. }) && !then.ok_is_primary());
}

#[test]
fn delete_moves_the_session_and_its_folder_and_forgets_only_its_own_marks_and_summary() {
    let mut r = real("moves");
    let (st, roots) = (&mut r.st, r.roots.clone());
    st.marks.rename(ID, &r.transcript, "Gone");
    st.marks.set_topic(ID, &r.transcript, "Build");
    st.marks.hide(ID, &r.transcript, true);
    st.marks.archive(ID, &r.transcript, Some(5));
    st.marks.set_handed_over_to(ID, OTHER);
    st.marks.rename(OTHER, &r.neighbour, "Stays");
    st.marks.set_parent(OTHER, ID);
    st.marks.hide_folder("C:/work", true);
    assert!(st.save_marks());
    let path = Cache::path(&roots);
    let mut cache = Cache::default();
    cache.digests.insert(ID.into(), digest("gone"));
    cache.digests.insert(OTHER.into(), digest("stays"));
    cache.save(&path).unwrap();
    st.meaning.cache = cache;

    st.delete_action(DELETE_ROW);
    dialogs::click(st, Instant::now(), OK, 0.0);
    r.deleted();
    let (st, roots) = (&r.st, &r.roots);
    assert!(st.info.is_none(), "{:?}", r.info());
    assert!(st.toasts.find(|t| t.contains("Recycle Bin")).is_some());
    let disk = Marks::load(roots).unwrap();
    assert!(disk.name(ID).is_none() && disk.topic(ID).is_none() && !disk.hidden(ID) && disk.archived_at(ID).is_none());
    assert!(disk.handed_over_to(ID).is_none());
    assert_eq!((disk.name(OTHER), disk.parent(OTHER)), (Some("Stays"), Some(ID)), "the other session's marks stay");
    assert!(disk.folder_hidden("C:/work"));
    let kept = Cache::load(&path);
    assert!(!kept.digests.contains_key(ID) && kept.digests.contains_key(OTHER));
    assert!(!st.meaning.cache.digests.contains_key(ID));
}

#[test]
fn the_guards_are_checked_again_at_the_moment_of_deletion() {
    let mut r = real("guards");
    r.st.delete_action(DELETE_ROW);
    // Meanwhile another window pins it, then Delete is pressed.
    let mut other = Marks::load(&r.roots).unwrap();
    other.pin(ID, &r.transcript, true);
    other.save(&r.roots).unwrap();
    dialogs::click(&mut r.st, Instant::now(), OK, 0.0);
    r.kept();
    assert_eq!(r.info().0, "Not deleted");
    assert!(r.info().1.contains("pinned"), "{:?}", r.info());
    other.pin(ID, &r.transcript, false);
    other.save(&r.roots).unwrap();

    // It starts in a CLI meanwhile (a live process says it runs it).
    let me = std::process::id();
    let record = r.roots.primary_cli_dir().join("sessions").join(format!("{me}.json"));
    put(&record, &serde_json::json!({"pid": me, "sessionId": ID, "cwd": "C:\\work"}).to_string());
    assert_eq!(r.st.delete_session(ID).to_owned_kept(), Some("already open".into()));
    std::fs::remove_file(&record).unwrap();
    r.kept();

    // This window's own view says it runs now (its state is checked again too).
    r.st.running_ids.insert(ID.into());
    assert!(matches!(r.st.delete_session(ID), Deleted::Kept(why) if why.contains("running")));
    r.st.running_ids.clear();
    r.kept();

    assert_eq!(r.st.delete_session(ID), Deleted::Done);
    r.deleted();
}

impl Deleted {
    /// "already open" for a refusal that says so; for the assertions above.
    fn to_owned_kept(&self) -> Option<String> {
        match self {
            Deleted::Kept(why) if why.contains("already open") => Some("already open".into()),
            Deleted::Kept(why) => Some(why.clone()),
            _ => None,
        }
    }
}

#[test]
fn a_folder_that_does_not_move_is_reported_not_silent() {
    let mut r = real("partial");
    let mut bin = FolderBin::new(r.root.join("bin"), r.root.clone());
    bin.refuse = Some(ID.into());
    r.st.recycle = Arc::new(bin);
    r.st.delete_action(DELETE_ROW);
    dialogs::click(&mut r.st, Instant::now(), OK, 0.0);
    assert!(!r.transcript.exists() && r.folder.is_dir());
    let (title, text) = r.info();
    assert_eq!(title, "Session partly deleted");
    assert!(text.contains("is in the Recycle Bin") && text.contains("was not moved"), "{text}");
}

#[test]
fn a_copy_deletes_its_original_only_once_the_copy_is_written() {
    let mut r = real("after-copy");
    let original = r.st.sessions[0].clone();
    r.st.delete_after_copy(&original, NEW, "main", None);
    let copy = r.st.copy_deletes[0].copy_file.clone();
    assert_eq!(copy, r.transcript.with_file_name(format!("{NEW}.jsonl")));
    assert!(!r.st.check_copy_deletes(Instant::now()), "nothing yet");
    put(&copy, "");
    assert!(!r.st.check_copy_deletes(Instant::now()), "an empty file is not a written copy");
    r.kept();
    put(&copy, &format!("{{\"type\":\"user\",\"sessionId\":\"{NEW}\"}}\n"));
    assert!(r.st.check_copy_deletes(Instant::now()));
    r.deleted();
    assert!(r.st.copy_deletes.is_empty());
    assert!(copy.is_file(), "the copy is left alone");
}

#[test]
fn a_copy_whose_tab_closed_before_it_was_written_keeps_the_original_and_says_why() {
    let mut r = real("copy-closed");
    let original = r.st.sessions[0].clone();
    r.st.delete_after_copy(&original, NEW, "main", Some(9_999));
    assert!(r.st.check_copy_deletes(Instant::now()));
    r.kept();
    let (title, text) = r.info();
    assert_eq!(title, "The original session was kept");
    assert!(text.contains("tab closed"), "{text}");
    assert!(r.st.copy_deletes.is_empty());

    // Written, but the original is open in a tab by then: kept, with the reason.
    r.st.delete_after_copy(&original, NEW, "main", None);
    put(&r.transcript.with_file_name(format!("{NEW}.jsonl")), &format!("{{\"sessionId\":\"{NEW}\"}}\n"));
    r.st.running_ids.insert(ID.into());
    assert!(r.st.check_copy_deletes(Instant::now()));
    r.kept();
    assert!(r.info().1.contains("The copy is open.") && r.info().1.contains("running"), "{:?}", r.info());
}

#[test]
fn a_handover_continued_in_place_deletes_the_session_the_tab_left_not_the_one_it_follows() {
    let mut r = real("in-place");
    let blank = crate::tabs::NewTab { label: "t".into(), color: [0; 3], session: None, account: Some("main".into()) };
    let tab = r.st.tabs.open(blank, crate::term::Term::empty((8, 4)));
    // The tab has not followed yet: kept.
    r.st.tabs.get_mut(tab).unwrap().resume_id = Some(ID.into());
    r.st.delete_after_in_place(tab, ID, "Old", NEW);
    r.kept();
    assert_eq!(r.info().0, "The original session was kept");
    // Asked to delete the one it follows: kept.
    r.st.tabs.get_mut(tab).unwrap().resume_id = Some(NEW.into());
    r.st.delete_after_in_place(tab, NEW, "New", NEW);
    r.kept();
    // Followed: the old one goes.
    r.st.info = None;
    r.st.delete_after_in_place(tab, ID, "Old", NEW);
    r.deleted();
    assert!(r.st.info.is_none(), "{:?}", r.info());
}

#[test]
fn a_window_test_without_its_own_bin_cannot_reach_the_recycle_bin() {
    let mut r = real("no-bin");
    r.st.recycle = default_bin();
    assert!(matches!(r.st.delete_session(ID), Deleted::Kept(why) if why.contains("not moved")));
    assert!(r.transcript.is_file() && r.folder.is_dir());
}
