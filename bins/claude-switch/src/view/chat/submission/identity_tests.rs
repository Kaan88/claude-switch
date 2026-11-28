//! Items 23, 24 and 36: a new chat's first message, a session the same process moves to, and
//! the full-screen refusal.
use super::*;
use crate::source::Source;
use crate::tabs::NewTab;
use crate::term::Term;
use crate::view::chat::QUIET;
use crate::view::{handover_transcript, model};

const A: &str = "11111111-0000-0000-0000-000000000000";
const B: &str = "22222222-0000-0000-0000-000000000000";
const C: &str = "33333333-0000-0000-0000-000000000000";

struct Fixture {
    app: AppState,
    root: std::path::PathBuf,
    tab: u64,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(name: &str, source: impl FnOnce(&std::path::Path) -> model::Source) -> Self {
        let root = crate::testing::unique(&format!("cs-identity-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = AppState::new(Source::Demo, Some(false), Default::default());
        let mut pane = app.new_session_pane(source(&root), root.clone());
        // Demo writes go to the screen and are recorded; the gate is the real one.
        pane.chat.demo = true;
        pane.chat.enabled = true;
        let mut term = Term::empty((60, 12));
        term.feed("\x1b[?2004h❯ ".as_bytes());
        let tab = app.tabs.open(NewTab { label: "Test".into(), color: [0; 3], session: None, account: None }, term);
        app.tabs.get_mut(tab).unwrap().pane = Some(pane);
        Self { app, root, tab }
    }

    fn tab(&mut self) -> &mut Tab {
        self.app.tabs.get_mut(self.tab).unwrap()
    }

    fn chat(&mut self) -> &mut Chat {
        &mut self.tab().pane.as_mut().unwrap().chat
    }

    fn transcript(&self, id: &str, cwd: &std::path::Path) -> std::path::PathBuf {
        let file = handover_transcript(&self.root, &self.root, id);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let record = serde_json::json!({ "type": "user", "cwd": cwd }).to_string();
        std::fs::write(&file, format!("{record}\n")).unwrap();
        file
    }

    fn send(&mut self, text: &str, now: Instant) -> Option<&'static str> {
        let chat = self.chat();
        chat.editor.set(text.into());
        chat.last_output = Some(now - QUIET);
        chat.writes.clear();
        AppState::chat_send(self.tab(), now, false);
        let chat = self.chat();
        // A draft that only has to wait is held, not refused.
        let queued = chat.queue.take().is_some();
        assert_eq!(chat.writes.is_empty(), chat.error.is_some() || queued, "{:?}", chat.error);
        if queued { Some(QUEUED) } else { chat.error }
    }
}

/// What `Fixture::send` returns for a draft that was queued.
const QUEUED: &str = "queued";

fn discover(root: &std::path::Path) -> model::Source {
    model::Source::Discover {
        config: root.to_owned(),
        cwd: root.to_owned(),
        started: std::time::UNIX_EPOCH,
        verified: Default::default(),
    }
}

#[test]
fn a_new_chat_sends_its_first_message_before_its_transcript_exists() {
    let mut f = Fixture::new("first", discover);
    let now = Instant::now();
    assert_eq!(f.send("hello", now), Some(Reason::Identity.label()), "no live record yet");
    assert!(f.tab().apply_live_identity(Some(A)));
    let tab = f.tab();
    assert!(!tab.pane.as_ref().unwrap().chat.identity_unverified);
    assert!(tab.pane.as_ref().unwrap().chat.awaiting_transcript);
    assert_eq!(tab.pane.as_ref().unwrap().pinned_id(), Some(A));
    assert!(tab.needs_identity(), "the record does not change when the file appears");
    assert_eq!(tab.resume_id, None, "nothing to resume yet");
    assert_eq!(f.send("hello", now), None, "the first message goes with an ordinary Send");
    assert!(f.chat().writes.last().is_some_and(|w| w == b"\r"));
    assert_eq!(f.send("again", now), Some(QUEUED), "later sends wait for the transcript");
    assert_eq!(blocked(f.tab(), now, Intent::Draft), Some(NO_TRANSCRIPT));
    assert_eq!(f.chat().editor.text, "again");

    // Another session named before the first message's transcript appeared is refused.
    f.tab().apply_live_identity(Some(B));
    assert!(f.chat().identity_unverified);
    assert_eq!(f.tab().pane.as_ref().unwrap().pinned_id(), Some(A));
    assert_eq!(f.send("again", now), Some(Reason::Identity.label()));

    let file = f.transcript(A, &f.root.clone());
    assert!(f.tab().apply_live_identity(Some(A)));
    let tab = f.tab();
    let pane = tab.pane.as_ref().unwrap();
    assert!(!pane.chat.identity_unverified && !pane.chat.awaiting_transcript);
    assert_eq!(pane.verified_transcript(), Some(file.as_path()));
    assert!(pane.model.error.is_none());
    assert_eq!(tab.resume_id.as_deref(), Some(A));
    assert!(!tab.needs_identity());
}

#[test]
fn a_new_chat_whose_record_is_missing_or_names_a_foreign_transcript_stays_refused() {
    let mut f = Fixture::new("foreign", discover);
    let now = Instant::now();
    f.tab().apply_live_identity(None);
    assert_eq!(f.send("hello", now), Some(Reason::Identity.label()));
    f.transcript(A, std::path::Path::new(r"C:\somewhere\else"));
    f.tab().apply_live_identity(Some(A));
    assert!(f.chat().identity_unverified && !f.chat().awaiting_transcript);
    assert_eq!(f.send("hello", now), Some(Reason::Identity.label()));
    f.tab().apply_live_identity(Some("not-an-id"));
    assert_eq!(f.send("hello", now), Some(Reason::Identity.label()));
}

#[test]
fn the_same_process_moving_to_another_session_is_followed() {
    let mut f = Fixture::new("follow", |root| model::Source::File(handover_transcript(root, root, A)));
    let root = f.root.clone();
    f.transcript(A, &root);
    assert!(f.tab().apply_live_identity(Some(A)));
    let before = f.tab().pane.as_ref().unwrap().model.generation;
    f.chat().editor.set("keep this draft".into());

    // `/clear`: a new ID whose transcript the CLI writes with the next prompt.
    assert!(f.tab().apply_live_identity(Some(B)));
    let tab = f.tab();
    let pane = tab.pane.as_ref().unwrap();
    assert_eq!(pane.pinned_id(), Some(B));
    assert!(!pane.chat.identity_unverified && pane.chat.awaiting_transcript);
    assert!(pane.model.error.is_none());
    assert!(pane.model.generation >= before);
    assert_eq!(pane.chat.editor.text, "keep this draft");
    assert_eq!(pane.continued, Some((B.to_owned(), A.to_owned())), "a new session continues the old one");
    assert_eq!(tab.resume_id, None);
    let file = f.transcript(B, &root);
    f.tab().apply_live_identity(Some(B));
    assert_eq!(f.tab().pane.as_ref().unwrap().verified_transcript(), Some(file.as_path()));
    assert_eq!(f.tab().resume_id.as_deref(), Some(B));

    // `/resume`: an older transcript of this folder is attached as it is.
    f.transcript(A, &root);
    f.tab().pane.as_mut().unwrap().continued = None;
    f.tab().apply_live_identity(Some(A));
    assert_eq!(f.tab().pane.as_ref().unwrap().pinned_id(), Some(A));
    assert!(!f.chat().identity_unverified);
    assert_eq!(f.tab().pane.as_ref().unwrap().continued, None, "an older transcript is another conversation");

    // A transcript that records another folder is a change Switch cannot explain.
    f.transcript(C, std::path::Path::new(r"C:\somewhere\else"));
    f.tab().apply_live_identity(Some(C));
    let pane = f.tab().pane.as_ref().unwrap();
    assert!(pane.chat.identity_unverified);
    assert_eq!(pane.pinned_id(), Some(A));
    assert_eq!(pane.model.error.as_deref(), Some(crate::view::SESSION_CHANGED));
}

#[test]
fn a_followed_transcript_needs_a_prompt_written_after_the_follow() {
    let mut chat = Chat::default();
    let mut old = ChatInfo::default();
    old.latest = Some((900, ChatPhase::EndTurn));
    old.prompt = Some(800);
    chat.observe(&old, Instant::now());
    chat.followed(&ChatInfo::default(), 500);
    chat.observe(&ChatInfo::default(), Instant::now());
    let mut info = ChatInfo::default();
    info.latest = Some((400, ChatPhase::EndTurn));
    info.prompt = Some(300);
    chat.observe(&info, Instant::now());
    assert!(!chat.fresh, "an old completion of the resumed transcript is not proof");
    info.latest = Some((700, ChatPhase::EndTurn));
    info.prompt = Some(600);
    chat.observe(&info, Instant::now());
    assert!(chat.fresh);
}

#[test]
fn the_first_prompt_of_a_new_transcript_rearms_chat() {
    let mut chat = Chat { awaiting_transcript: true, ..Chat::default() };
    chat.observe(&ChatInfo::default(), Instant::now());
    chat.watermark_input(&ChatInfo::default(), Some(0));
    let mut info = ChatInfo::default();
    info.latest = Some((300, ChatPhase::Active));
    info.prompt = Some(0);
    chat.observe(&info, Instant::now());
    assert!(chat.fresh);
}

#[test]
fn the_alternate_screen_refusal_names_the_fix_for_a_claude_tab() {
    let mut f = Fixture::new("alternate", discover);
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    let now = Instant::now();
    f.tab().term.feed("\x1b[?1049h\x1b[?2004h❯ ".as_bytes());
    assert_eq!(f.send("hello", now), Some(FULLSCREEN));
    assert!(FULLSCREEN.contains("/tui default") && FULLSCREEN.contains("Classic terminal view"));
    f.chat().classic_renderer = true;
    assert_eq!(f.send("hello", now), Some(ALTERNATE), "another full-screen program");
}

#[test]
fn a_message_of_several_lines_keeps_its_breaks_and_waits_for_the_input_line() {
    let mut f = Fixture::new("lines", discover);
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    f.chat().demo_progress = true;
    let now = Instant::now();
    assert_eq!(f.send("first line\nsecond line", now), None);
    let writes = std::mem::take(&mut f.chat().writes);
    assert_eq!(writes, [b"\x1b[200~first line\rsecond line\x1b[201~".to_vec()], "no Enter outside the paste yet");
    assert!(f.chat().waiting().is_some());
    assert!(AppState::advance_submission(f.tab(), now, true));
    assert_eq!(f.chat().writes, [b"\r".to_vec()], "Enter once the input line shows both lines");
    assert!(f.chat().waiting().is_none());

    // The screen never shows it: no Enter, the draft is kept and the terminal is to be checked.
    let mut f = Fixture::new("lines-timeout", discover);
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    f.chat().demo_progress = true;
    f.send("one\ntwo", now);
    f.tab().term.feed("\x1b[2J\x1b[H\x1b[?2004h❯ ".as_bytes());
    f.chat().writes.clear();
    AppState::advance_submission(f.tab(), now + ECHO_TIMEOUT, true);
    assert!(f.chat().writes.is_empty());
    assert_eq!(f.chat().error, Some(LINES_TIMEOUT));
    assert!(f.chat().recovery_notice().is_some());
    assert_eq!(f.chat().editor.text, "one\ntwo");
}

#[test]
fn a_message_of_numbered_lines_is_entered_once_the_input_shows_it() {
    // `❯ 1. fix a` under the input box's rule is the input, not a menu's highlighted choice: read
    // as a menu row, the echo never showed, and the text was left on the CLI's input.
    let mut f = Fixture::new("numbered-lines", discover);
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    f.chat().demo_progress = true;
    let now = Instant::now();
    f.send("1. fix a\n2. fix b", now);
    let rule = "─".repeat(20);
    let screen = format!("\x1b[2J\x1b[H\x1b[?2004h{rule}\r\n❯ 1. fix a\r\n  2. fix b\r\n{rule}\x1b[3;12H");
    f.tab().term.feed(screen.as_bytes());
    f.chat().writes.clear();
    assert!(AppState::advance_submission(f.tab(), now, true));
    assert_eq!(f.chat().writes, [b"\r".to_vec()], "Enter once the input shows both lines");
    assert!(f.chat().error.is_none() && f.chat().waiting().is_none());
}

#[test]
fn a_followed_session_links_only_to_a_verified_one() {
    // A new chat whose transcript was never written: `/clear` moves on, but no parent is recorded.
    let mut f = Fixture::new("link-awaiting", discover);
    assert!(f.tab().apply_live_identity(Some(A)));
    assert!(f.chat().awaiting_transcript);
    f.tab().apply_live_identity(Some(B));
    let pane = f.tab().pane.as_ref().unwrap();
    assert_eq!(pane.pinned_id(), Some(B));
    assert_eq!(pane.continued, None, "A never existed");

    // A tab refused for its session: the next one is not linked to the session it had left.
    let mut f = Fixture::new("link-refused", |root| model::Source::File(handover_transcript(root, root, A)));
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(None);
    assert!(f.chat().identity_unverified);
    f.tab().apply_live_identity(Some(B));
    assert_eq!(f.tab().pane.as_ref().unwrap().continued, None);

    // Attached and verified: linked, as before.
    let mut f = Fixture::new("link-verified", |root| model::Source::File(handover_transcript(root, root, A)));
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    f.tab().apply_live_identity(Some(B));
    assert_eq!(f.tab().pane.as_ref().unwrap().continued, Some((B.to_owned(), A.to_owned())));
}

#[test]
fn a_command_that_may_start_a_turn_counts_as_a_new_chats_first_send() {
    for (line, starts) in [("/effort high", false), ("/review", true)] {
        let mut f = Fixture::new(if starts { "first-cmd-turn" } else { "first-cmd-local" }, discover);
        assert!(f.tab().apply_live_identity(Some(A)));
        let now = Instant::now();
        f.chat().last_output = Some(now - QUIET);
        AppState::submit_slash(f.tab(), now, line, false);
        assert!(AppState::advance_submission(f.tab(), now, true), "{:?}", f.chat().error);
        assert!(f.chat().writes.last().is_some_and(|w| w == b"\r"), "{line} was entered");
        assert_eq!(f.chat().first_sent, starts, "{line}");
        let expected = if starts { Some(QUEUED) } else { None };
        assert_eq!(f.send("hello", now), expected, "{line}");
    }
}

#[test]
fn a_confirmed_command_rechecks_the_session_right_before_writing() {
    let mut f = Fixture::new("confirm-recheck", |root| model::Source::File(handover_transcript(root, root, A)));
    let root = f.root.clone();
    f.transcript(A, &root);
    f.tab().apply_live_identity(Some(A));
    f.tab().session = Some(A.to_owned());
    let now = Instant::now();
    f.chat().last_output = Some(now - QUIET);
    f.app.tabs.active = Some(f.tab);

    // The session records now name another session (a /clear typed meanwhile), while the tab's
    // last poll still says A: the command is refused.
    f.tab().resume_id = Some(B.to_owned());
    f.app.chat_confirm_slash(now, f.tab, Some(A.to_owned()), "/rename Parser work", false);
    assert!(f.chat().writes.is_empty());
    assert!(f.chat().error.is_some_and(|e| e.contains("session changed")));

    f.tab().resume_id = Some(A.to_owned());
    f.chat().error = None;
    f.app.chat_confirm_slash(now, f.tab, Some(A.to_owned()), "/rename Parser work", false);
    assert!(!f.chat().writes.is_empty(), "{:?}", f.chat().error);
}
