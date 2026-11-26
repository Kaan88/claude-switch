use super::submission::send_bytes;
use super::*;
use crate::view::model;
use mdshared_term::{Key, Mods, Screen, input};
use mdshared_ui::gfx::Rect;

fn ready() -> Signals<'static> {
    Signals {
        phase: Some(ChatPhase::EndTurn),
        fresh: true,
        pending_for: None,
        quiet_for: QUIET,
        title: "",
        bell: false,
        slash: false,
        ended: false,
        bracketed: true,
        alternate: false,
    }
}

#[test]
fn readiness_requires_explicit_fresh_completion_and_paste_mode() {
    let base = ready();
    assert_eq!(assess(base), (Gate::Ready, None));
    for changed in [
        Signals { fresh: false, ..base },
        Signals { phase: None, title: "Claude Code — Idle", ..base },
        Signals { phase: Some(ChatPhase::Unknown), ..base },
        Signals { quiet_for: QUIET - Duration::from_millis(1), ..base },
        Signals { bracketed: false, ..base },
        Signals { alternate: true, ..base },
    ] {
        assert_eq!(assess(changed), (Gate::Check, None));
    }
    assert_eq!(assess(Signals { phase: Some(ChatPhase::Active), ..base }), (Gate::Running, None));
}

#[test]
fn forced_send_keeps_the_draft_while_an_interaction_is_pending() {
    use crate::{source::Source, state::AppState};
    let mut app = AppState::new(Source::Demo, Some(false), Default::default());
    app.start_demo_session();
    let tab = app.tabs.shown_mut().unwrap();
    for bell in [true, false] {
        let chat = &mut tab.pane.as_mut().unwrap().chat;
        chat.editor.set("Keep this draft".into());
        chat.bell = bell;
        chat.slash = !bell;
        for force in [false, true] {
            AppState::chat_send(tab, Instant::now(), force);
            assert_eq!(tab.pane.as_ref().unwrap().chat.editor.text, "Keep this draft");
        }
    }
}

#[test]
fn a_visible_permission_question_vetoes_fresh_completion() {
    use crate::{source::Source, state::AppState};
    let mut app = AppState::new(Source::Demo, Some(false), Default::default());
    app.start_demo_session();
    let tab = app.tabs.shown_mut().unwrap();
    tab.term = crate::term::Term::empty((60, 20));
    tab.term.feed(b"\x1b[?2004hDo you want to proceed?\r\n1. Yes\r\n2. No\r\nEsc to cancel");
    let now = Instant::now();
    let pane = tab.pane.as_mut().unwrap();
    pane.chat.fresh = true;
    pane.chat.last_output = Some(now - QUIET);
    pane.chat.editor.set("Keep this draft".into());
    pane.model.chat.latest = Some((100, ChatPhase::EndTurn));
    assert_eq!(pane.chat.decision(&pane.model.chat, &tab.term, "", now), (Gate::Check, Some(Reason::Permission)));
    for force in [false, true] {
        AppState::chat_send(tab, now, force);
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert_eq!(chat.error, Some(Reason::Permission.label()));
        assert_eq!(chat.editor.text, "Keep this draft");
    }
    tab.term.feed("\r\n```\r\n\r\n❯ ".as_bytes());
    let pane = tab.pane.as_ref().unwrap();
    assert_eq!(pane.chat.decision(&pane.model.chat, &tab.term, "", now), (Gate::Ready, None));
}

#[test]
fn real_pane_vetoes_both_send_paths_until_its_identity_is_verified() {
    use crate::{source::Source, state::AppState, tabs::NewTab, term::Term};
    let root = std::env::temp_dir().join(format!("cs-real-chat-identity-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let roots = claude_switch_core::Roots { home: root.clone(), appdata: root.clone() };
    let mut app = AppState::new(Source::Real(roots), Some(false), Default::default());
    let id = "11111111-0000-0000-0000-000000000000";
    let mut pane = app.new_session_pane(model::Source::File(root.join(format!("{id}.jsonl"))), root.clone());
    pane.chat.editor.set("Do not send yet".into());
    let mut term = Term::empty((60, 20));
    term.feed(b"\x1b[?2004h");
    let tab_id =
        app.tabs.open(NewTab { label: "Test".into(), color: [0; 3], session: Some(id.into()), account: None }, term);
    let tab = app.tabs.get_mut(tab_id).unwrap();
    tab.pane = Some(pane);
    for found in [None, Some("22222222-0000-0000-0000-000000000000")] {
        for force in [false, true] {
            AppState::chat_send(tab, Instant::now(), force);
            let chat = &tab.pane.as_ref().unwrap().chat;
            assert_eq!(chat.error, Some(Reason::Identity.label()));
            assert_eq!(chat.editor.text, "Do not send yet");
        }
        tab.apply_live_identity(found);
    }
    assert!(tab.needs_identity());
    assert!(tab.apply_live_identity(Some(id)));
    assert!(!tab.needs_identity());
    assert_eq!(tab.pane.as_ref().unwrap().chat.editor.text, "Do not send yet");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_input_watermark_uses_cached_discovery_path_without_rereading_head() {
    use crate::{source::Source, state::AppState};
    let id = "11111111-0000-0000-0000-000000000000";
    let root = std::env::temp_dir().join(format!("cs-chat-watermark-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let file = crate::view::handover_transcript(&root, &root, id);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{}\n").unwrap();
    let source = model::Source::Discover {
        config: root.clone(),
        cwd: root.clone(),
        started: std::time::UNIX_EPOCH,
        verified: Default::default(),
    };
    let app = AppState::new(Source::Demo, None, Default::default());
    let mut pane = app.new_session_pane(source, root.clone());
    assert!(pane.update_identity(Some(id)));
    let conflicting = b"{\"cwd\":\"different\"}\n";
    std::fs::write(&file, conflicting).unwrap();
    assert!(pane.transcript_source.find_file().is_none(), "fresh discovery would reread and reject the changed head");
    pane.note_terminal_input();
    assert_eq!(pane.chat.input_floor, Some(conflicting.len() as u64));
    pane.update_identity(None);
    pane.note_terminal_input();
    assert_eq!(pane.chat.input_floor, Some(u64::MAX));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_tool_needs_both_age_and_quiet_to_open_the_drawer() {
    let base = ready();
    let age = Some(TOOL_WAIT);
    assert_eq!(assess(Signals { pending_for: age, ..base }), (Gate::Check, Some(Reason::Tool)));
    for changed in [
        Signals { pending_for: Some(TOOL_WAIT - Duration::from_millis(1)), ..base },
        Signals { pending_for: age, quiet_for: Duration::ZERO, ..base },
    ] {
        assert_eq!(assess(changed), (Gate::Running, None));
    }
    assert_eq!(assess(Signals { slash: true, ..base }), (Gate::Check, Some(Reason::Slash)));
    assert_eq!(assess(Signals { ended: true, bell: true, ..base }), (Gate::Ended, Some(Reason::Ended)));
}

#[test]
fn toolkit_reads_recorded_osc_bel_and_modes_without_a_process() {
    let mut screen = Screen::new(20, 4, 0);
    screen.feed(b"\x1b]2;Claude Code \xe2\x80\x94 Working\x07\x1b[?2004h");
    let title = screen.take_title().unwrap();
    assert_eq!(title, "Claude Code — Working");
    assert!(!screen.take_bell(), "OSC's terminator is not a standalone bell");
    assert_eq!(
        assess(Signals { title: &title, bracketed: screen.modes().bracketed_paste, ..ready() }),
        (Gate::Running, None)
    );
    screen.feed(b"\x07");
    assert_eq!(assess(Signals { bell: screen.take_bell(), ..ready() }), (Gate::Check, Some(Reason::Bell)));
    assert!(!screen.take_bell());
}

#[test]
fn mode_shortcut_is_scoped_to_the_palette_and_f6_still_has_terminal_bytes() {
    use mdshared_ui::keys::{KeyName, Mods as KeyMods};
    assert!(layout_shortcut(&KeyName::F(6), KeyMods::default(), true, true));
    assert!(!layout_shortcut(&KeyName::F(6), KeyMods::default(), false, true));
    assert!(!layout_shortcut(&KeyName::F(6), KeyMods::default(), true, false));
    assert!(!input::encode_key(Key::F(6), Mods::NONE, &mdshared_term::Modes::default()).is_empty());
}

#[test]
fn drawer_only_closes_automatically_if_it_opened_automatically() {
    let mut chat = Chat::default();
    assert!(chat.react(Some(Reason::Tool)));
    assert_eq!(chat.drawer, Drawer::Auto(Reason::Tool));
    assert!(chat.react(None));
    assert_eq!(chat.drawer, Drawer::Closed);
    chat.open_drawer();
    chat.react(Some(Reason::Bell));
    chat.react(None);
    assert_eq!(chat.drawer, Drawer::Manual);
    chat.react(Some(Reason::Tool));
    chat.toggle_drawer();
    chat.react(Some(Reason::Tool));
    assert_eq!(chat.drawer, Drawer::Closed);
    chat.react(Some(Reason::Ended));
    assert_eq!(chat.drawer, Drawer::Auto(Reason::Ended));
}

#[test]
fn a_send_or_manual_input_cannot_reuse_the_previous_completion() {
    let now = Instant::now();
    let mut chat = Chat::default();
    let mut info = ChatInfo::default();
    info.latest = Some((40, ChatPhase::EndTurn));
    info.prompt = Some(10);
    chat.observe(&info, now);
    assert!(!chat.fresh, "old completion after resume");
    info.latest = Some((55, ChatPhase::EndTurn));
    chat.observe(&info, now);
    assert!(!chat.fresh, "a new completion without a new prompt does not arm a resumed tab");
    info.latest = Some((80, ChatPhase::EndTurn));
    info.prompt = Some(60);
    chat.observe(&info, now);
    assert!(chat.fresh);
    chat.invalidate(&info);
    info.latest = Some((90, ChatPhase::EndTurn));
    chat.observe(&info, now);
    assert!(!chat.fresh, "a late old-turn completion cannot rearm");
    info.prompt = Some(92);
    info.latest = Some((100, ChatPhase::EndTurn));
    chat.observe(&info, now);
    assert!(chat.fresh);
    info.generation += 1;
    chat.observe(&info, now);
    assert!(!chat.fresh, "transcript replacement cannot rearm");
}

#[test]
fn paste_bytes_put_paths_before_control_free_prompt_without_an_image_key() {
    let chips = [
        Chip::Path(PathBuf::from(r"C:\my photo.png")),
        Chip::Clipboard(clipboard::Picture { seq: 1, thumbnail: None }),
    ];
    assert_eq!(
        send_bytes(&chips[..1], "line 1\nline 2\tword").unwrap(),
        b"\x1b[200~\"C:\\my photo.png\" line 1\rline 2 word\x1b[201~"
    );
    assert_eq!(send_bytes(&chips[1..], "").unwrap(), b"");
    assert_eq!(send_bytes(&[], "just text").unwrap(), b"\x1b[200~just text\x1b[201~");
    assert!(send_bytes(&[], "line\x1b[201~\rmenu").is_none());
    for control in ['\0', '\x07', '\x1b', '\x7f', '\u{009b}', '\r'] {
        assert!(send_bytes(&[], &format!("prompt{control}[201~")).is_none());
    }
    assert!(send_bytes(&[], "").is_none());
}

#[test]
fn direct_input_discards_unsampled_old_prompt_and_stop_cannot_reuse_a_turn() {
    let now = Instant::now();
    let mut chat = Chat::default();
    let mut info = ChatInfo::default();
    info.latest = Some((50, ChatPhase::Active));
    info.prompt = Some(20);
    chat.observe(&info, now);
    assert!(!chat.can_stop(&info));
    chat.invalidate_input(&info, Some(100));
    info.latest = Some((95, ChatPhase::EndTurn));
    info.prompt = Some(75);
    chat.observe(&info, now);
    assert!(!chat.fresh);
    info.latest = Some((130, ChatPhase::Active));
    info.prompt = Some(110);
    chat.observe(&info, now);
    assert!(chat.can_stop(&info));
    chat.stopped = Some((info.generation, 110));
    assert!(!chat.can_stop(&info));
}

#[test]
fn layout_keeps_composer_and_drawer_disjoint_and_centers_the_reading_measure() {
    for (w, h, panel) in [(760.0, 480.0, false), (1100.0, 720.0, true), (1920.0, 1080.0, true)] {
        let area = Rect::new(0.0, 80.0, w, h - 80.0);
        for chips in [false, true] {
            let l = Layout::new(area, true, true, None, chips, false);
            assert!(l.conversation.h >= 80.0 && l.conversation.w <= 770.0);
            assert_eq!(l.side.w > 0.0, panel);
            assert_eq!(l.conversation.x * 2.0 + l.conversation.w + l.side.w, w);
            assert_eq!(l.conversation.bottom(), l.composer.y);
            assert_eq!(l.composer.bottom(), l.grip.y);
            assert_eq!(l.grip.bottom(), l.drawer.y);
            assert_eq!(l.drawer.bottom(), h);
            assert!((l.drawer.h / area.h - 0.4).abs() < 0.04);
        }
        let closed = Layout::new(area, false, false, None, false, false);
        assert_eq!(closed.drawer.h, 0.0);
        assert_eq!(closed.side.w, 0.0);
        let resized = Layout::new(area, true, true, Some(65.0), false, false);
        assert_eq!(resized.drawer.h, 65.0);
    }
}

#[test]
fn drawer_drag_applies_the_release_position_even_if_motion_was_coalesced() {
    let mut chat = Chat { resizing: Some((460.0, 240.0)), ..Default::default() };
    assert!(chat.end_drag(Some((0.0, 535.0))));
    assert_eq!(chat.drawer_height, Some(165.0));
    assert!(!chat.end_drag(Some((0.0, 600.0))));
    assert_eq!(chat.drawer_height, Some(165.0));
}

#[test]
fn demo_refuses_an_uncertain_draft_then_sends_only_into_its_fake_screen() {
    use crate::{source::Source, state::AppState};
    let mut app = AppState::new(Source::Demo, Some(false), Default::default());
    app.start_demo_session();
    let tab = app.tabs.shown_mut().unwrap();
    let pane = tab.pane.as_mut().unwrap();
    assert!(tab.term.pty.is_none());
    let path = pane.transcript_source.find_file().unwrap();
    let before = std::fs::read(&path).unwrap();
    pane.chat.editor.insert("Synthetic one\nSynthetic two");
    let now = Instant::now();

    AppState::chat_send(tab, now, false);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert_eq!(chat.editor.text, "Synthetic one\nSynthetic two");
    assert_eq!(chat.error, Some("Open the terminal to check"));
    assert!(!chat.demo_progress);

    AppState::chat_send(tab, now, true);
    // Two lines: Enter follows once the fake input line shows them.
    assert!(tab.pane.as_ref().unwrap().chat.waiting().is_some());
    AppState::advance_submission(tab, now, true);
    assert!(tab.pane.as_ref().unwrap().chat.editor.text.is_empty());
    let screen = (0..tab.term.screen.rows()).map(|row| tab.term.screen.row_text(row)).collect::<Vec<_>>().join("\n");
    assert!(screen.contains("Synthetic one"));
    assert!(screen.contains("Synthetic two"));
    assert!(screen.contains("Demo input"));
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(tab.term.pty.is_none());
}

pub(super) fn demo() -> crate::state::AppState {
    let mut app = crate::state::AppState::new(crate::source::Source::Demo, Some(false), Default::default());
    app.start_demo_session();
    app
}

#[test]
fn uncertain_notice_survives_editing_and_offers_the_explicit_override() {
    use crate::state::AppState;
    let mut app = demo();
    let now = Instant::now();
    let tab = app.tabs.shown_mut().unwrap();
    tab.pane.as_mut().unwrap().chat.editor.insert("First");
    AppState::chat_send(tab, now, false);
    tab.pane.as_mut().unwrap().chat.editor.insert(" and second");
    app.chat_editor_changed();
    let tab = app.tabs.shown_mut().unwrap();
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert_eq!(chat.error, Some(submission::CHECK));
    assert_eq!(chat.editor.text, "First and second");
    assert!(chat.writes.is_empty());
    AppState::chat_send(tab, now, true);
    assert_eq!(
        tab.pane.as_ref().unwrap().chat.writes,
        [b"\x1b[200~First and second\x1b[201~".to_vec(), b"\r".to_vec()]
    );
}

fn image_draft(tab: &mut crate::tabs::Tab, text: &str) {
    tab.term = crate::term::Term::empty((80, 20));
    tab.term.feed("\x1b[?2004h❯ ".as_bytes());
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    chat.chips.push(Chip::Clipboard(clipboard::Picture { seq: 42, thumbnail: None }));
    chat.editor.insert(text);
}

#[test]
fn picture_pastes_only_after_a_new_cursor_row_marker_then_enters_exactly_once() {
    use crate::state::AppState;
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    image_draft(tab, "caption");
    AppState::chat_send(tab, now, true);
    assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [b"\x1bv".to_vec()]);
    // Replace the demo marker with an old echo/assistant line and an empty real input row.
    tab.term.feed("\r\nAssistant quoted [Image #2]\r\n❯ ".as_bytes());
    assert!(!AppState::advance_submission(tab, now, true));
    tab.term.feed(b"[Image #2]");
    assert!(!AppState::advance_submission(tab, now, false));
    assert!(AppState::advance_submission(tab, now, true));
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert_eq!(chat.writes, [b"\x1bv".to_vec(), b"\x1b[200~caption\x1b[201~".to_vec(), b"\r".to_vec()]);
    assert!(chat.editor.text.is_empty() && chat.chips.is_empty());
    assert!(!AppState::advance_submission(tab, now, true));
}

#[test]
fn picture_timeout_and_permission_never_press_enter_and_keep_the_draft() {
    use crate::state::AppState;
    for permission in [false, true] {
        let mut app = demo();
        let tab = app.tabs.shown_mut().unwrap();
        let now = Instant::now();
        image_draft(tab, "keep this");
        AppState::chat_send(tab, now, true);
        let when = if permission {
            tab.term.feed(b"\r\nDo you want to proceed?\r\n1. Yes\r\n2. No\r\nEsc to cancel");
            now
        } else {
            // Even a now-visible marker cannot authorize an expired attempt.
            now + Duration::from_secs(4)
        };
        assert!(AppState::advance_submission(tab, when, true));
        let chat = &tab.pane.as_ref().unwrap().chat;
        let err = chat.error.unwrap();
        assert!(err.contains(if permission { "interrupted" } else { "No image marker" }));
        assert_eq!(chat.writes, [b"\x1bv".to_vec()]);
        assert_eq!(chat.editor.text, "keep this");
        assert_eq!(chat.clipboard_sequence(), Some(42));
        app.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat.editor.insert(" typed later");
        app.chat_editor_changed();
        assert_eq!(app.tabs.shown().unwrap().pane.as_ref().unwrap().chat.error, Some(err));
    }
}

pub(super) fn running(tab: &mut crate::tabs::Tab, now: Instant) {
    let pane = tab.pane.as_mut().unwrap();
    tab.term = crate::term::Term::empty((80, 20));
    tab.term.feed("\x1b[?2004h❯ ".as_bytes());
    pane.chat.fresh = true;
    pane.chat.last_output = Some(now - QUIET);
    pane.chat.last_gate = Some(Gate::Running);
    pane.model.chat.latest = Some((100, ChatPhase::Active));
    pane.model.chat.prompt = Some(50);
    pane.chat.editor.insert("next");
}

#[test]
fn queue_waits_for_a_newer_completion_then_sends_once_and_cancel_keeps_the_draft() {
    use crate::state::AppState;
    for cancel in [false, true] {
        let mut app = demo();
        let tab = app.tabs.shown_mut().unwrap();
        let now = Instant::now();
        running(tab, now);
        AppState::chat_send(tab, now, false);
        let pane = tab.pane.as_mut().unwrap();
        assert!(pane.chat.locked() && pane.chat.writes.is_empty());
        assert_eq!(pane.chat.notice(&pane.model.chat), Some("A turn is running. This will be sent when it ends."));
        if cancel {
            pane.chat.stop_waiting();
            assert!(!pane.chat.locked());
        }
        pane.model.chat.latest = Some((100, ChatPhase::EndTurn));
        assert!(!AppState::advance_submission(tab, now, true));
        tab.pane.as_mut().unwrap().model.chat.latest = Some((120, ChatPhase::EndTurn));
        assert!(!AppState::advance_submission(tab, now, false));
        assert_eq!(AppState::advance_submission(tab, now, true), !cancel);
        assert!(!AppState::advance_submission(tab, now, true));
        let chat = &tab.pane.as_ref().unwrap().chat;
        assert_eq!(chat.editor.text, if cancel { "next" } else { "" });
        assert_eq!(chat.writes.iter().filter(|w| w.as_slice() == b"\r").count(), usize::from(!cancel));
    }
}

#[test]
fn send_now_never_overrides_a_live_permission_question() {
    use crate::state::AppState;
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    running(tab, now);
    AppState::chat_send(tab, now, false);
    tab.term.feed(b"\r\nDo you want to proceed?\r\n1. Yes\r\n2. No\r\nEsc to cancel");
    AppState::chat_send(tab, now, true);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert!(chat.locked() && chat.writes.is_empty());
    assert_eq!(chat.error, Some(Reason::Permission.label()));
    tab.term.feed("\r\n\r\n\r\n\r\n❯ ".as_bytes());
    AppState::chat_send(tab, now, true);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert!(!chat.locked());
    assert_eq!(chat.writes, [b"\x1b[200~next\x1b[201~".to_vec(), b"\r".to_vec()]);
}

#[test]
fn send_anyway_from_a_stale_check_does_not_send_into_a_running_turn() {
    use crate::state::AppState;
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    running(tab, now);
    AppState::chat_send(tab, now, true);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert!(chat.error.unwrap().contains("Use Queue"));
    assert!(chat.writes.is_empty());
    assert_eq!(chat.editor.text, "next");
    assert!(!chat.locked());
}

#[test]
fn native_model_command_writes_or_names_the_gate_without_stealing_the_draft() {
    use crate::state::AppState;
    let mut app = demo();
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    let pane = tab.pane.as_mut().unwrap();
    pane.chat.editor.insert("Unrelated draft");
    pane.chat.bell = true;
    AppState::submit_slash(tab, now, "/model", false);
    let pane = tab.pane.as_mut().unwrap();
    assert!(pane.chat.writes.is_empty());
    assert_eq!(pane.chat.error, Some(Reason::Bell.label()));
    pane.chat.bell = false;
    AppState::submit_slash(tab, now, "/model", false);
    assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [b"/model".to_vec()]);
    assert!(AppState::advance_submission(tab, now, true));
    let pane = tab.pane.as_ref().unwrap();
    assert_eq!(pane.chat.writes, [b"/model".to_vec(), b"\r".to_vec()]);
    assert_eq!(pane.chat.editor.text, "Unrelated draft");
    assert_ne!(pane.chat.drawer, Drawer::Closed);
}

#[test]
fn compact_and_clear_only_write_after_confirmation() {
    use crate::dialogs::{self, Dialog};
    use crate::state::AppState;
    for (command, from_editor) in [("/compact", false), ("/clear", false), ("/reset named", true)] {
        let mut app = demo();
        let now = Instant::now();
        app.dialog = AppState::prepare_slash(app.tabs.shown_mut().unwrap(), now, command, from_editor);
        assert!(matches!(app.dialog, Some(Dialog::Confirm { ref then, .. }) if !then.ok_is_primary()));
        dialogs::click(&mut app, now, dialogs::CANCEL, 0.0);
        assert!(app.tabs.shown().unwrap().pane.as_ref().unwrap().chat.writes.is_empty());
        app.dialog = AppState::prepare_slash(app.tabs.shown_mut().unwrap(), now, command, from_editor);
        dialogs::click(&mut app, now, dialogs::OK, 0.0);
        let tab = app.tabs.shown_mut().unwrap();
        assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [command.as_bytes()]);
        assert!(AppState::advance_submission(tab, now, true));
        assert_eq!(tab.pane.as_ref().unwrap().chat.writes, [command.as_bytes(), b"\r"]);
    }
}

mod review;
