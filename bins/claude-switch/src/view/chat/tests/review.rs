use super::*;
use crate::{state::AppState, tabs::SESSIONS};

#[test]
fn command_uses_separate_writes_and_times_out_on_wrong_or_unfresh_echo() {
    for echoed in [false, true] {
        let mut app = demo();
        let tab = app.tabs.shown_mut().unwrap();
        let now = Instant::now();
        let command = "/model sonnet";
        AppState::submit_slash(tab, now, command, false);
        tab.term.feed("\r\n❯ /model son".as_bytes());
        assert!(!AppState::advance_submission(tab, now, true));
        tab.term.feed(b"net");
        // Exact bytes from an old snapshot are not confirmation.
        let pane = tab.pane.as_mut().unwrap();
        pane.chat.output_epoch = pane.chat.output_epoch.wrapping_sub(1);
        assert!(!AppState::advance_submission(tab, now, true));
        if echoed {
            tab.pane.as_mut().unwrap().chat.output(now, true, false);
        }
        let when = if echoed { now } else { now + crate::view::chat::submission::ECHO_TIMEOUT };
        assert!(AppState::advance_submission(tab, when, true));
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert_eq!(chat.writes[0], command.as_bytes());
        if echoed {
            assert_eq!(chat.writes[1..], [b"\r"]);
        } else {
            assert_eq!(chat.writes.len(), 1);
            assert!(chat.error.unwrap().contains("did not appear"));
        }
    }
}

#[test]
fn timed_out_image_blocks_plain_overrides_and_slashes_until_a_fresh_clean_input_row() {
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    image_draft(tab, "caption");
    tab.pane.as_mut().unwrap().chat.fresh = true;
    AppState::chat_send(tab, now, true);
    assert!(!tab.pane.as_ref().unwrap().chat.fresh);
    let later = now + crate::view::chat::submission::IMAGE_TIMEOUT;
    assert!(AppState::advance_submission(tab, later, true));
    tab.pane.as_mut().unwrap().chat.chips.clear();
    for force in [false, true] {
        AppState::chat_send(tab, later, force);
        assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [b"\x1bv"]);
    }
    AppState::submit_slash(tab, later, "/context", false);
    assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [b"\x1bv"]);
    assert!(tab.pane.as_ref().unwrap().chat.notice(&Default::default()).unwrap().contains("may hold a picture"));
    tab.term.feed("\r\n❯ ".as_bytes());
    assert!(!AppState::advance_submission(tab, later, true), "a cached empty row cannot remove the guard");
    tab.pane.as_mut().unwrap().chat.output(later, true, false);
    assert!(AppState::advance_submission(tab, later + QUIET, true));
    AppState::chat_send(tab, later + QUIET, true);
    assert_eq!(tab.pane.as_ref().unwrap().chat.writes[1..], [b"\x1b[200~caption\x1b[201~".as_slice(), b"\r"]);
}

#[test]
fn direct_terminal_input_cancels_image_wait_but_cannot_clear_the_staged_picture_guard() {
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    image_draft(tab, "caption");
    AppState::chat_send(tab, now, true);
    tab.pane.as_mut().unwrap().note_terminal_input();
    assert!(!tab.pane.as_ref().unwrap().chat.locked());
    tab.pane.as_mut().unwrap().chat.chips.clear();
    AppState::chat_send(tab, now + QUIET, true);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert_eq!(chat.writes, [b"\x1bv"]);
    assert_eq!(chat.editor.text, "caption");
    assert!(chat.error.unwrap().contains("may hold a picture"));
}

#[test]
fn failed_enter_uses_only_enter_and_never_repeats_the_paste_or_command() {
    for (text, picture, command) in [("caption", false, false), ("caption", true, false), ("/model opus", false, true)]
    {
        let mut app = demo();
        let tab = app.tabs.shown_mut().unwrap();
        let now = Instant::now();
        if picture {
            image_draft(tab, text);
        } else {
            tab.pane.as_mut().unwrap().chat.editor.insert(text);
        }
        tab.pane.as_mut().unwrap().chat.fail_enter_once = true;
        if command {
            AppState::submit_slash(tab, now, text, true);
        } else {
            AppState::chat_send(tab, now, true);
        }
        if picture || command {
            assert!(AppState::advance_submission(tab, now, true));
        }
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert!(chat.enter_only());
        assert_eq!(chat.editor.text, text);
        let original = chat.writes.clone();
        AppState::chat_send(tab, now, false);
        AppState::chat_send(tab, now, true);
        assert_eq!(tab.pane.as_ref().unwrap().chat.writes, original);
        tab.term.feed(b"\r\nDo you want to proceed?\r\n1. Yes\r\n2. No\r\nEsc to cancel");
        AppState::retry_enter(tab, now);
        assert_eq!(tab.pane.as_ref().unwrap().chat.writes, original);
        tab.term = crate::term::Term::empty((80, 20));
        let line = if picture { format!("❯ [Image #1] {text}") } else { format!("❯ {text}") };
        tab.term.feed(line.as_bytes());
        AppState::retry_enter(tab, now);
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert_eq!(&chat.writes[..original.len()], original);
        assert_eq!(chat.writes[original.len()..], [b"\r"]);
        assert!(chat.editor.text.is_empty() && !chat.enter_only());
    }
}

#[test]
fn stop_releases_the_queue_only_after_working_disappears_and_the_input_is_clean() {
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    running(tab, now);
    tab.term.feed("\r\n✻ Working… (esc to interrupt)\r\n❯ ".as_bytes());
    AppState::chat_send(tab, now, false);
    AppState::chat_stop(tab, now);
    tab.pane.as_mut().unwrap().chat.output(now, true, false);
    assert!(!AppState::advance_submission(tab, now + QUIET, true));
    tab.term = crate::term::Term::empty((80, 20));
    tab.term.feed("\x1b[?2004h❯ unsubmitted".as_bytes());
    tab.pane.as_mut().unwrap().chat.output(now + QUIET, true, false);
    assert!(!AppState::advance_submission(tab, now + QUIET * 2, true));
    tab.term.feed("\r\x1b[K❯ ".as_bytes());
    let final_output = now + QUIET * 2;
    tab.pane.as_mut().unwrap().chat.output(final_output, true, false);
    assert!(!AppState::advance_submission(tab, final_output, true));
    assert!(AppState::advance_submission(tab, final_output + QUIET, true));
    let pane = tab.pane.as_ref().unwrap();
    assert_eq!(pane.model.chat.latest, Some((100, ChatPhase::Active)));
    assert_eq!(pane.chat.writes, [b"\x1b".as_slice(), b"\x1b[200~next\x1b[201~", b"\r"]);
}

#[test]
fn background_queue_and_image_respect_window_vetoes_but_still_progress_or_expire() {
    for picture in [false, true] {
        for timeout in [false, true].into_iter().filter(|timeout| picture || !timeout) {
            let mut app = demo();
            let tab = app.tabs.shown_mut().unwrap();
            let id = tab.id;
            let now = Instant::now();
            // Keep the demo transcript worker from replacing the hand-built ChatInfo fixture.
            let pane = tab.pane.as_mut().unwrap();
            pane.model = crate::view::model::Model::new(pane.transcript_source.clone(), pane.cwd.clone(), || {});
            pane.next = Some(now + Duration::from_secs(60));
            let later = if timeout { now + crate::view::chat::submission::IMAGE_TIMEOUT } else { now };
            if picture {
                image_draft(tab, "background");
                AppState::chat_send(tab, now, true);
            } else {
                running(tab, now);
                AppState::chat_send(tab, now, false);
                let pane = tab.pane.as_mut().unwrap();
                pane.model.chat.latest = Some((120, ChatPhase::EndTurn));
                pane.model.chat.prompt = Some(80);
            }
            assert!(app.tabs.activate(SESSIONS));
            let writes = app.tabs.get(id).unwrap().pane.as_ref().unwrap().chat.writes.clone();
            for minimized in [true, false] {
                if !minimized {
                    app.dialog = Some(crate::dialogs::Dialog::Confirm {
                        title: "Test dialog".into(),
                        text: "This dialog vetoes automatic sends.".into(),
                        ok: "OK".into(),
                        then: crate::dialogs::Pending::ChatSlash {
                            tab: id,
                            session: None,
                            line: "/clear".into(),
                            from_editor: false,
                        },
                    });
                }
                let allowed = app.chat_window_allows_writes(minimized);
                assert!(!allowed);
                assert!(!app.progress_chat_tabs(now, allowed));
                let chat = &app.tabs.get(id).unwrap().pane.as_ref().unwrap().chat;
                assert!(chat.locked());
                assert_eq!(chat.writes, writes);
            }
            if !timeout {
                app.dialog = None;
            }
            let allowed = app.chat_window_allows_writes(false);
            assert_eq!(allowed, !timeout);
            assert!(!app.progress_chat_tabs(later, allowed), "hidden work does not need a visible redraw");
            assert_eq!(app.tabs.active, None);
            let chat = &app.tabs.get(id).unwrap().pane.as_ref().unwrap().chat;
            if timeout {
                assert_eq!(chat.writes, [b"\x1bv"]);
                assert_eq!(chat.editor.text, "background");
                assert!(chat.error.unwrap().contains("No image marker"));
            } else {
                assert!(chat.editor.text.is_empty());
                assert_eq!(chat.writes.last().unwrap(), b"\r");
            }
        }
    }
}

#[test]
fn clipboard_text_wins_without_consulting_the_bitmap_and_picture_can_be_requested() {
    let mut chat = Chat::default();
    let photo = || Some(clipboard::Picture { seq: 1, thumbnail: None });
    chat.paste_from(false, || Some("Excel cell".into()), || panic!("must not open the bitmap"));
    assert_eq!(chat.editor.text, "Excel cell");
    assert!(chat.chips.is_empty());
    chat.paste_from(true, || panic!("explicit image must not read text"), photo);
    assert_eq!(chat.clipboard_sequence(), Some(1));
    chat.chips.clear();
    chat.paste_from(false, || None, photo);
    assert_eq!(chat.clipboard_sequence(), Some(1));
}
