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
        idle: false,
        working: false,
        typed: false,
        resting: false,
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
    let scratch = crate::testing::Scratch::new("cs-real-chat-identity");
    let root = scratch.to_path_buf();
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
    let scratch = crate::testing::Scratch::new("cs-chat-watermark");
    let root = scratch.to_path_buf();
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
    assert!(chat.react(Some(Reason::Permission)));
    assert_eq!(chat.drawer, Drawer::Auto(Reason::Permission));
    assert!(chat.react(None));
    assert_eq!(chat.drawer, Drawer::Closed);
    // Claude Code may be waiting, or the bell rang: the screen shows as a card instead.
    for reason in [Reason::Tool, Reason::Bell] {
        assert!(chat.react(Some(reason)));
        assert_eq!(chat.drawer, Drawer::Closed);
        chat.react(None);
    }
    chat.open_drawer();
    chat.react(Some(Reason::Bell));
    chat.react(None);
    assert_eq!(chat.drawer, Drawer::Manual);
    chat.react(Some(Reason::Permission));
    chat.toggle_drawer();
    chat.react(Some(Reason::Permission));
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
fn layout_keeps_composer_and_drawer_disjoint_and_puts_the_box_on_the_reading_column() {
    for (w, h, panel) in [(760.0, 480.0, false), (1100.0, 720.0, true), (1920.0, 1080.0, true)] {
        let area = Rect::new(0.0, 80.0, w, h - 80.0);
        for chips in [false, true] {
            let l = Layout::new(area, true, true, None, chips, false);
            // The box is the medium reading column (720 px at most) and 16 px on each side,
            // centred where the conversation's text is (beside its scrollbar's gutter).
            // With attachments in the smallest window the conversation keeps the layout's floor.
            assert!(l.conversation.h >= 70.0 && l.box_frame.w <= 752.0);
            assert_eq!(l.side.w > 0.0, panel);
            assert_eq!(l.conversation.w + l.side.w, w);
            assert_eq!((l.composer.x, l.composer.w), (l.conversation.x, l.conversation.w));
            assert!(l.box_frame.x >= 12.0 && l.box_frame.right() <= l.conversation.right());
            assert!((l.box_frame.cx() - (l.conversation.w - 13.0) / 2.0).abs() <= 1.0, "{:?}", l.box_frame);
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
    // Not refused: held until the prompt is ready, with nothing written.
    assert!(chat.queue.is_some() && chat.error.is_none() && chat.writes.is_empty());
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
fn an_uncertain_draft_is_held_and_cancel_or_send_now_stay_explicit() {
    use crate::state::AppState;
    let mut app = demo();
    let now = Instant::now();
    let tab = app.tabs.shown_mut().unwrap();
    tab.pane.as_mut().unwrap().chat.editor.insert("First");
    AppState::chat_send(tab, now, false);
    let chat = &mut tab.pane.as_mut().unwrap().chat;
    assert!(chat.queue.is_some() && chat.locked() && chat.error.is_none());
    assert!(chat.writes.is_empty());
    // Cancel unlocks the draft for editing; nothing was written.
    chat.stop_waiting();
    assert!(!chat.locked() && chat.writes.is_empty());
    chat.editor.insert(" and second");
    app.chat_editor_changed();
    let tab = app.tabs.shown_mut().unwrap();
    AppState::chat_send(tab, now, false);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert_eq!(chat.editor.text, "First and second");
    assert!(chat.queue.is_some() && chat.writes.is_empty());
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
            now + submission::IMAGE_TIMEOUT
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

/// Send while the turn runs and Claude Code's input does not read empty, so the draft is held
/// (into an empty input it would go into the turn at once); then the input reads empty again.
pub(super) fn send_held(tab: &mut crate::tabs::Tab, now: Instant) {
    tab.term.feed("\r\x1b[K❯ typed".as_bytes());
    crate::state::AppState::chat_send(tab, now, false);
    tab.term.feed("\r\x1b[K❯ ".as_bytes());
}

#[test]
fn queue_waits_for_a_newer_completion_then_sends_once_and_cancel_keeps_the_draft() {
    use crate::state::AppState;
    for cancel in [false, true] {
        let mut app = demo();
        let tab = app.tabs.shown_mut().unwrap();
        let now = Instant::now();
        running(tab, now);
        send_held(tab, now);
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
    send_held(tab, now);
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

mod redesign;
mod review;

/// Settings › Chat: the Compact chip runs Switch's handover by default, through the same command
/// as "Compact and open…" (the handover dialog for the shown tab); with "Claude Code /compact" it
/// is the CLI's `/compact` with its confirmation, as before.
#[test]
fn the_compact_chip_runs_the_handover_or_claude_codes_compact() {
    use crate::dialogs::{Dialog, Pending};
    use crate::state::AppState;
    use crate::view::chat::composer::chips_of;
    use crate::view::chat::input::chip_slash;
    let mut app = demo();
    assert!(app.config.chat.compact_handover);
    let chips = chips_of(&app.config.chat);
    assert!(chips.contains(&Control::Handover) && !chips.contains(&Control::Compact), "{chips:?}");
    assert_eq!(chip_slash(Control::Handover), None, "the handover chip types nothing into the CLI");
    // The handover's own checks: a session the list has not indexed gets no dialog.
    app.handover_chip(Instant::now());
    assert_eq!(app.compact_source(), None);
    app.dialog = None;
    app.apply_scan(crate::demo::scan(&app.config));
    let session = app.sessions[0].id.clone();
    let tab = app.tabs.shown_mut().unwrap();
    (tab.session, tab.resume_id) = (Some(session.clone()), Some(session.clone()));
    let id = tab.id;
    app.handover_chip(Instant::now());
    assert_eq!(app.compact_source(), Some((session.as_str(), Some(id))), "the handover dialog for this tab");
    assert!(app.tabs.shown().unwrap().pane.as_ref().unwrap().chat.writes.is_empty());
    app.dialog = None;

    app.config.chat.compact_handover = false;
    let chips = chips_of(&app.config.chat);
    assert!(chips.contains(&Control::Compact) && !chips.contains(&Control::Handover), "{chips:?}");
    let line = chip_slash(Control::Compact).unwrap();
    assert_eq!(line, "/compact");
    app.dialog = AppState::prepare_slash(app.tabs.shown_mut().unwrap(), Instant::now(), line, false);
    assert!(
        matches!(&app.dialog, Some(Dialog::Confirm { then: Pending::ChatSlash { line, .. }, .. }) if line == "/compact")
    );
    // A hidden Compact chip stays hidden whichever it runs.
    app.config.chat.set_slot(4, None);
    for handover in [false, true] {
        app.config.chat.compact_handover = handover;
        let chips = chips_of(&app.config.chat);
        assert!(!chips.iter().any(|c| matches!(c, Control::Compact | Control::Handover)), "{chips:?}");
    }
}

#[test]
fn cut_removes_the_selection_once_the_clipboard_took_it() {
    let mut app = demo();
    let chat = &mut app.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
    chat.editor.insert("hello world");
    chat.editor.move_to(6, false);
    chat.editor.move_to(11, true);
    // The right-click menu offers Cut for the selection and keeps it.
    app.chat_editor_menu(0.0, 0.0);
    let (_, menu) = app.menu.as_ref().expect("the editor menu");
    let cut = menu.items.iter().find(|item| item.label == "Cut").unwrap();
    assert!(cut.enabled, "Cut is offered for a selection");
    let chat = &mut app.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
    assert_eq!(chat.editor.selection(), 6..11, "opening the menu keeps the selection");

    let mut copied = String::new();
    assert!(!chat.editor_copy_to(true, |_| false), "a refused clipboard keeps the text");
    assert_eq!(chat.editor.text, "hello world");
    assert!(!chat.editor_copy_to(false, |s| {
        copied = s.to_owned();
        true
    }));
    assert_eq!((copied.as_str(), chat.editor.text.as_str()), ("world", "hello world"), "Copy leaves the draft");
    assert!(chat.editor_copy_to(true, |s| {
        copied = s.to_owned();
        true
    }));
    assert_eq!((copied.as_str(), chat.editor.text.as_str()), ("world", "hello "), "Cut removes what it copied");
    assert_eq!((chat.editor.caret, chat.editor.selection()), (6, 6..6));
    assert!(!chat.editor_copy_to(true, |_| true), "nothing selected, nothing cut");
}

#[test]
fn undo_and_redo_walk_every_kind_of_draft_edit_with_its_caret_and_selection() {
    use claude_switch_core::transcript::{Item, ItemId, ItemKind};
    use mdshared_ui::keys::{KeyName, Mods};
    let mut chat = Chat::default();
    chat.editor.history.clock = Some(Instant::now());
    let none = Mods::default();
    let snapshot = |chat: &Chat| (chat.editor.text.clone(), chat.editor.caret, chat.editor.selection());
    // Typing, two words: two steps.
    for c in "hello world".chars() {
        assert!(chat.edit_key(&KeyName::Char(c), none, Some(&c.to_string())));
    }
    // A paste, then a cut of "world".
    chat.paste_from(false, || Some(" PASTE".into()), || None);
    assert_eq!(chat.editor.text, "hello world PASTE");
    chat.editor.move_to(6, false);
    chat.editor.move_to(11, true);
    assert!(chat.editor_copy_to(true, |_| true));
    assert_eq!(chat.editor.text, "hello  PASTE");
    // Two Backspaces at the end: one step.
    chat.editor.move_to(chat.editor.text.len(), false);
    assert!(chat.edit_key(&KeyName::Backspace, none, None));
    assert!(chat.edit_key(&KeyName::Backspace, none, None));
    assert_eq!(chat.editor.text, "hello  PAS");
    // A completion or a chip fill replaces the draft: one step.
    chat.editor.set("/model".into());
    chat.editor.move_to(6, false);
    // A recalled prompt is a step too, and so is leaving it.
    let mut model =
        crate::view::model::Model::new(crate::view::model::Source::File(Default::default()), Default::default(), || {});
    let kind =
        ItemKind::Prompt { text: "an earlier prompt".into(), pictures: Vec::new(), synthetic: false, queued: false };
    model.items.insert(ItemId { offset: 1, block: 0 }, Item { timestamp: None, kind });
    assert!(chat.recall_key(&model, &KeyName::Up));
    assert_eq!(chat.editor.text, "an earlier prompt");

    let mut seen = vec![snapshot(&chat)];
    while chat.undo_or_redo(false) {
        seen.push(snapshot(&chat));
    }
    let texts = seen.iter().map(|(text, ..)| text.as_str()).collect::<Vec<_>>();
    assert_eq!(
        texts,
        ["an earlier prompt", "/model", "hello  PAS", "hello  PASTE", "hello world PASTE", "hello world", "hello ", ""]
    );
    // The cut comes back selected, as it was cut.
    assert_eq!(seen[4], ("hello world PASTE".into(), 11, 6..11));
    // The recalled prompt's draft came back where its caret was.
    assert_eq!(seen[1].1, 6);
    // Redo walks forward to the end, then stops.
    let mut forward = Vec::new();
    while chat.undo_or_redo(true) {
        forward.push(chat.editor.text.clone());
    }
    assert_eq!(forward.len(), texts.len() - 1);
    assert_eq!(chat.editor.text, "an earlier prompt");
    // A new edit after an undo drops the redo steps.
    assert!(chat.undo_or_redo(false));
    assert!(chat.edit_key(&KeyName::Char('!'), none, Some("!")));
    assert!(!chat.undo_or_redo(true));
}

#[test]
fn the_menu_offers_undo_and_redo_and_a_sent_message_clears_the_history() {
    use crate::state::AppState;
    let mut app = demo();
    let chat = &mut app.tabs.shown_mut().unwrap().pane.as_mut().unwrap().chat;
    assert!(!chat.editor.history.can_undo());
    chat.insert_text("one line");
    // The right-click menu offers Undo (Redo not yet), before the clipboard items.
    app.chat_editor_menu(0.0, 0.0);
    let (_, menu) = app.menu.as_ref().unwrap();
    let labels = menu.items.iter().map(|i| (i.label.as_str(), i.enabled)).collect::<Vec<_>>();
    assert_eq!(
        labels,
        [("Undo", true), ("Redo", false), ("Cut", false), ("Copy", false), ("Paste", true), ("Select all", true)]
    );
    let tab = app.tabs.shown_mut().unwrap();
    let now = Instant::now();
    AppState::chat_send(tab, now, true);
    AppState::advance_submission(tab, now, true);
    let chat = &tab.pane.as_ref().unwrap().chat;
    assert!(chat.editor.text.is_empty(), "sent");
    assert!(!chat.editor.history.can_undo(), "a sent message is not brought back by Ctrl+Z");
}

#[test]
fn caret_keys_move_and_select_by_character_word_line_and_draft_like_a_windows_text_box() {
    use claude_switch_core::transcript::{Item, ItemId, ItemKind};
    use mdshared_ui::keys::{KeyName, Mods};
    let plain = Mods::default();
    let shift = Mods { shift: true, ..plain };
    let ctrl = Mods { ctrl: true, ..plain };
    let both = Mods { ctrl: true, shift: true, ..plain };
    let mut model = model::Model::new(model::Source::File(Default::default()), Default::default(), || {});
    let kind =
        ItemKind::Prompt { text: "an earlier prompt".into(), pictures: Vec::new(), synthetic: false, queued: false };
    model.items.insert(ItemId { offset: 1, block: 0 }, Item { timestamp: None, kind });
    let mut chat = Chat::default();
    chat.editor.history.clock = Some(Instant::now());
    let press = |chat: &mut Chat, key: KeyName, mods: Mods| {
        chat.editor_key(&model, Some(&key), mods, None);
        (chat.editor.caret, chat.editor.selection())
    };
    chat.editor.insert("Fix the bug, then ship");
    let end = chat.editor.text.len();

    // Ctrl+Left and Ctrl+Right jump to word starts; punctuation is a stop of its own.
    assert_eq!(press(&mut chat, KeyName::Left, ctrl), (18, 18..18), "the start of “ship”");
    assert_eq!(press(&mut chat, KeyName::Left, ctrl), (13, 13..13), "“then”");
    assert_eq!(press(&mut chat, KeyName::Left, ctrl), (11, 11..11), "the comma");
    assert_eq!(press(&mut chat, KeyName::Right, ctrl), (13, 13..13));
    // Ctrl+Shift extends by a word, Shift by a character.
    assert_eq!(press(&mut chat, KeyName::Right, both), (18, 13..18));
    assert_eq!(press(&mut chat, KeyName::Right, shift), (19, 13..19));
    assert_eq!(press(&mut chat, KeyName::Left, shift), (18, 13..18));
    assert_eq!(press(&mut chat, KeyName::Left, both), (13, 13..13), "back to where it began");
    assert_eq!(press(&mut chat, KeyName::Left, both), (11, 11..13));
    // A plain arrow collapses the selection to its side.
    assert_eq!(press(&mut chat, KeyName::Right, plain), (13, 13..13));
    // Ctrl+Home and Ctrl+End, selecting with Shift.
    assert_eq!(press(&mut chat, KeyName::Home, both), (0, 0..13));
    assert_eq!(press(&mut chat, KeyName::End, ctrl), (end, end..end));
    assert_eq!(press(&mut chat, KeyName::Home, ctrl), (0, 0..0));
    assert_eq!(press(&mut chat, KeyName::End, both), (end, 0..end));
    // Ctrl+A selects the whole draft.
    chat.editor.move_to(3, false);
    let ctrl_a = chat.editor_key(&model, Some(&KeyName::Char('a')), ctrl, Some("\u{1}"));
    assert!(!ctrl_a && chat.editor.selection() == (0..end) && chat.editor.text == "Fix the bug, then ship");

    // Ctrl+Backspace and Ctrl+Delete remove a word each, one undo step each.
    chat.editor.move_to(end, false);
    assert!(chat.editor_key(&model, Some(&KeyName::Backspace), ctrl, None));
    assert!(chat.editor_key(&model, Some(&KeyName::Backspace), ctrl, None));
    assert_eq!(chat.editor.text, "Fix the bug, ");
    chat.editor.move_to(0, false);
    assert!(chat.editor_key(&model, Some(&KeyName::Delete), ctrl, None));
    assert_eq!(chat.editor.text, "the bug, ");
    assert!(chat.undo_or_redo(false));
    assert_eq!(chat.editor.text, "Fix the bug, ");
    assert!(chat.undo_or_redo(false));
    assert_eq!(chat.editor.text, "Fix the bug, then ");
    assert!(chat.undo_or_redo(false));
    assert_eq!(chat.editor.text, "Fix the bug, then ship");

    // Home and End stop at the wrapped line first, with Shift selecting.
    chat.editor.set("alpha beta gamma".into());
    chat.editor.rows = vec![0..6, 6..11, 11..16];
    chat.editor.move_to(8, false);
    assert_eq!(press(&mut chat, KeyName::Home, shift), (6, 6..8), "to the start of the wrapped line");
    assert_eq!(press(&mut chat, KeyName::End, plain), (11, 11..11));
    assert_eq!(chat.editor.caret_row(), 1, "the end of the wrapped line, not the start of the next");
    assert_eq!(press(&mut chat, KeyName::Home, plain), (6, 6..6));

    // Shift+Up and Shift+Down select lines and never recall; plain Up on the first line recalls.
    chat.editor.set("one\ntwo".into());
    chat.editor.move_to(7, false);
    assert_eq!(press(&mut chat, KeyName::Up, shift), (3, 3..7));
    assert_eq!(chat.editor.text, "one\ntwo");
    assert_eq!(press(&mut chat, KeyName::Up, shift), (3, 3..7), "Shift+Up on the first line does not recall");
    assert_eq!(chat.editor.text, "one\ntwo");
    assert_eq!(press(&mut chat, KeyName::Down, shift), (7, 7..7));
    chat.editor.move_to(1, false);
    press(&mut chat, KeyName::Up, plain);
    assert_eq!(chat.editor.text, "an earlier prompt", "plain Up on the first line recalls as before");
    press(&mut chat, KeyName::Escape, plain);
    assert_eq!(chat.editor.text, "one\ntwo");
}

/// The Details panel opens when it has a first section to show and is closed when it has none,
/// unless the user opened or closed it: that choice holds.
#[test]
fn the_details_panel_follows_what_it_has_to_show_until_the_user_chooses() {
    use super::readme_shots as shots;
    fn chat(st: &mut crate::state::AppState, id: u64) -> &mut Chat {
        &mut shots::tab(st, id).pane.as_mut().unwrap().chat
    }
    let area = Rect::new(0.0, 80.0, 1280.0, 800.0);
    let demo = crate::demo::Transcript::new_showcase(None).unwrap();
    let (mut st, id) = shots::chat_window(true, &demo);
    st.layout_chat(area);
    assert!(chat(&mut st, id).layout.side.w > 0.0, "tasks and a changed file: the panel is open");
    chat(&mut st, id).side_choice = Some(false);
    st.layout_chat(area);
    assert_eq!(chat(&mut st, id).layout.side.w, 0.0, "closed by the user, it stays closed");

    // A new session has nothing to show.
    let empty = crate::demo::Transcript::new_empty().unwrap();
    let mut st = shots::window(true);
    let id = shots::claude_tab(&mut st, "Another tab");
    let source = crate::view::model::Source::File(empty.path().into());
    let mut pane = st.new_session_pane(source, empty.cwd().into());
    pane.chat.scripted = true;
    pane.chat.enabled = true;
    pane.ui.set_chat(true);
    shots::tab(&mut st, id).pane = Some(pane);
    st.layout_chat(area);
    assert_eq!(chat(&mut st, id).layout.side.w, 0.0, "nothing to show: the panel is closed");
    assert!(chat(&mut st, id).side_hidden);
    chat(&mut st, id).side_choice = Some(true);
    st.layout_chat(area);
    assert!(chat(&mut st, id).layout.side.w > 0.0, "opened by the user, it stays open");
}

#[test]
fn ctrl_c_copies_the_conversations_selection_unless_the_message_box_has_one() {
    use mdshared_text::bitmap::Bitmap;
    use mdshared_ui::gfx::{Gfx, Painter};
    use mdshared_ui::keys::{KeyName, Mods};
    let demo = crate::demo::Transcript::new_chat().unwrap();
    let source = model::Source::File(demo.path().into());
    let (wake, events) = std::sync::mpsc::channel();
    let mut model = model::Model::new(source.clone(), demo.cwd().into(), move || {
        let _ = wake.send(());
    });
    model.poll();
    events.recv_timeout(Duration::from_secs(5)).expect("the transcript was read");
    assert!(model.collect());
    let mut app = demo_app_with_pane(source, demo.cwd().into());
    let pane = app.tabs.shown_mut().unwrap().pane.as_mut().unwrap();
    let (mut gfx, mut bmp) = (Gfx::new(), Bitmap::new(700, 500));
    let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
    pane.ui.set_chat(true);
    let area = Rect::new(0.0, 0.0, 700.0, 500.0);
    pane.ui.draw(&mut p, &mdshared_ui::theme::DARK, area, &mut model, (-1.0, -1.0), Instant::now(), false);
    let ctrl = Mods { ctrl: true, ..Mods::default() };
    let copy = |pane: &crate::view::Pane, name: char, mods: Mods| {
        let mut copied = None;
        let done = pane.copy_conversation(&KeyName::Char(name), mods, |text| {
            copied = Some(text.to_owned());
            true
        });
        (done, copied)
    };
    // Nothing selected in the conversation: the key is the box's, as before.
    assert_eq!(copy(pane, 'c', ctrl), (false, None));
    assert!(pane.ui.select_everything());
    let (done, copied) = copy(pane, 'c', ctrl);
    assert!(done && copied.is_some_and(|text| !text.is_empty()));
    assert!(copy(pane, 'C', Mods { shift: true, ..ctrl }).0, "Ctrl+Shift+C too, as in the box");
    assert_eq!(copy(pane, 'c', Mods { alt: true, ..ctrl }), (false, None));
    assert_eq!(copy(pane, 'c', Mods::default()), (false, None));
    assert_eq!(copy(pane, 'v', ctrl), (false, None));
    // The box's own selection comes first.
    pane.chat.editor.set("draft".into());
    pane.chat.editor.select_all();
    assert_eq!(copy(pane, 'c', ctrl), (false, None));
    pane.chat.editor.set("draft".into());
    assert!(pane.chat.editor.selection().is_empty());
    assert!(copy(pane, 'c', ctrl).0, "a caret is no selection");
}

/// The demo window with a Chat pane over `source`.
fn demo_app_with_pane(source: model::Source, cwd: std::path::PathBuf) -> crate::state::AppState {
    let mut app = demo();
    let pane = app.new_session_pane(source, cwd);
    app.tabs.shown_mut().unwrap().pane = Some(pane);
    app
}

#[test]
fn the_terminal_takes_the_keyboard_when_it_opens_for_a_question_chat_cannot_answer() {
    let now = Instant::now();
    let typed = |ago: u64| Some(now - Duration::from_millis(ago));
    for (reason, typed_at, focus) in [
        (Reason::Permission, None, true),
        (Reason::Permission, typed(3000), true),
        // Someone typing in the box keeps it: the next key would land on Claude Code's prompt.
        (Reason::Permission, typed(300), false),
        (Reason::Identity, None, false),
    ] {
        let mut chat = Chat { typed_at, ..Chat::default() };
        chat.react_at(Some(reason), now);
        assert_eq!(chat.drawer, Drawer::Auto(reason));
        assert_eq!(chat.drawer_focus, focus, "{reason:?} {typed_at:?}");
        // Gone again, the drawer closes and the box has the keyboard.
        chat.react_at(None, now);
        assert!(chat.drawer == Drawer::Closed && !chat.drawer_focus);
    }
    // Claude Code may be waiting, or the bell rang: the terminal stays closed and the keyboard
    // here; its card in the conversation opens it, with the keyboard, on a click.
    for reason in [Reason::Tool, Reason::Bell] {
        let mut chat = Chat::default();
        chat.react_at(Some(reason), now);
        assert!(chat.drawer == Drawer::Closed && !chat.drawer_focus, "{reason:?}");
    }
    // Nor while the keyboard is in use here: text in the box, an attachment, text in a card's
    // field, or a selection in the conversation (Ctrl+C is meant for it, never for the CLI).
    let draft = |chat: &mut Chat| chat.editor.set("half a sentence".into());
    let field = |chat: &mut Chat| chat.question.field.set_text("my own answer");
    let selected = |chat: &mut Chat| chat.set_selection(true);
    for (what, keep) in [("draft", draft as fn(&mut Chat)), ("field", field), ("selection", selected)] {
        let mut chat = Chat::default();
        keep(&mut chat);
        assert!(chat.keeps_keyboard(now), "{what}");
        chat.react_at(Some(Reason::Permission), now);
        assert!(chat.drawer == Drawer::Auto(Reason::Permission) && !chat.drawer_focus, "{what}");
    }
    // A drawer the user opened keeps its focus as it was.
    let mut chat = Chat { drawer: Drawer::Manual, ..Chat::default() };
    chat.react_at(Some(Reason::Permission), now);
    assert!(!chat.drawer_focus);
}

#[test]
fn a_branch_is_cut_only_where_a_turn_ended_and_never_in_a_subagents_transcript() {
    use claude_switch_core::branch::Cut;
    use claude_switch_core::transcript::{ChatPhase, Item, ItemId, ItemKind, NoticeKind};
    let mut app = demo();
    let pane = app.tabs.shown_mut().unwrap().pane.as_mut().unwrap();
    let at = |offset| ItemId { offset, block: 0 };
    let prompt =
        |text: &str, queued| ItemKind::Prompt { text: text.into(), pictures: vec![], synthetic: false, queued };
    let answer = |text: &str| ItemKind::Assistant { markdown: text.into() };
    pane.model.items = [
        (at(10), prompt("first", false)),
        (at(20), answer("working on it")),
        (at(30), prompt("also this", true)),
        (at(40), answer("done")),
        (at(50), prompt("second", false)),
        (at(60), answer("thinking aloud")),
        (at(65), answer("the newest")),
        (at(70), ItemKind::Notice { kind: NoticeKind::Info, text: "x".into() }),
    ]
    .into_iter()
    .map(|(id, kind)| (id, Item { timestamp: None, kind }))
    .collect();
    let model = &mut pane.model;
    // The newest turn still runs: nothing in it is a place to branch.
    model.chat.latest = Some((65, ChatPhase::Active));
    // A turn that ended before the next prompt typed at an idle input: cut right before that
    // prompt, from its prompt, from a message sent during it and from its last answer.
    for item in [10, 30, 40] {
        assert_eq!(model.branch_cut(at(item)), Some(Cut::Before(50)), "{item}");
    }
    // An answer the turn went on after is no end; nor a notice; nor the running turn.
    for item in [20, 70, 50, 60, 65] {
        assert_eq!(model.branch_cut(at(item)), None, "{item}");
    }
    // Once it has ended: cut after the record of its end, from its prompt and its last answer.
    model.chat.latest = Some((65, ChatPhase::EndTurn));
    assert_eq!(model.branch_cut(at(50)), Some(Cut::After(65)));
    assert_eq!(model.branch_cut(at(65)), Some(Cut::After(65)));
    assert_eq!(model.branch_cut(at(60)), None, "not mid-turn");
    // A call still pending is no ended turn.
    model.chat.pending = Some(64);
    assert_eq!(model.branch_cut(at(65)), None);
    model.chat.pending = None;
    // A call without its result before the cut: Claude Code could not go on from the copy.
    let running = ItemKind::Tool {
        id: "toolu_lost".into(),
        name: "Bash".into(),
        summary: "ping".into(),
        file_path: None,
        state: claude_switch_core::transcript::ToolState::Running,
        duration_ms: None,
        pictures: vec![],
        subagent: None,
        denial: None,
        card: None,
    };
    model.items.insert(at(15), Item { timestamp: None, kind: running });
    for item in [10, 40, 50, 65] {
        assert_eq!(model.branch_cut(at(item)), None, "{item}");
    }
    model.items.remove(&at(15));
    // In a subagent's transcript the items are the agent's: no branch, no buttons, a refusal.
    model.agent = Some("agent-1".into());
    assert_eq!(model.branch_cut(at(10)), None);
    assert_eq!(
        pane.branch_plan(at(10)).unwrap_err(),
        "Branch works in the session's own conversation, not in a subagent's."
    );
    let tab = app.tabs.shown().unwrap().id;
    app.branch_at(Instant::now(), tab, at(10));
    assert!(app.info.is_some(), "refused, with the reason");
}

/// A Claude tab split with a shell pane beside its own.
fn split_claude_tab() -> (crate::state::AppState, u64) {
    use crate::tabs::NewTab;
    let mut app = crate::state::AppState::new(crate::source::Source::Demo, Some(true), Default::default());
    app.apply_scan(crate::demo::scan(&app.config));
    app.window_focused = true;
    let session = app.sessions.iter().find(|s| s.desktop_id.is_none()).unwrap().clone();
    let tab = app.tabs.open(
        NewTab { label: "tab".into(), color: [0; 3], session: Some(session.id.clone()), account: None },
        crate::term::Term::empty((40, 8)),
    );
    let pane = app.new_session_pane(model::Source::Demo(session.file.clone()), session.cwd.clone());
    app.tabs.get_mut(tab).unwrap().pane = Some(pane);
    let window = Rect::new(0.0, 0.0, 800.0, 300.0);
    app.layout_terminals(window, window, 1.0);
    app.split_pane(std::time::Instant::now(), tab, crate::panes::Axis::Right);
    (app, tab)
}

#[test]
fn keys_for_a_shell_pane_are_not_input_to_claudes_terminal() {
    let (mut app, tab) = split_claude_tab();
    assert_eq!(app.tabs.get(tab).unwrap().focused_pane(), 1);
    let chat = |app: &crate::state::AppState| app.tabs.get(tab).unwrap().pane.as_ref().unwrap().chat.last_input;
    let before = chat(&app);
    app.note_chat_terminal_input();
    app.term_text("ls");
    assert_eq!(chat(&app), before, "typed into the shell pane");
    app.focus_pane(crate::panes::MAIN);
    app.note_chat_terminal_input();
    assert_ne!(chat(&app), before, "typed into Claude's own pane");
}

#[test]
fn the_chat_layout_takes_the_keyboard_to_claudes_pane_and_back() {
    let (mut app, tab) = split_claude_tab();
    app.tabs.get_mut(tab).unwrap().panes.get_mut(1).unwrap().selecting = true;
    assert!(app.set_shown_tab_chat(true));
    let t = app.tabs.get(tab).unwrap();
    assert_eq!(t.focused_pane(), crate::panes::MAIN);
    assert!(!t.panes.get(1).unwrap().selecting, "the shell pane's drag ended");
    assert!(!app.set_shown_tab_chat(true), "no change");
    assert!(app.set_shown_tab_chat(false));
    assert_eq!(app.tabs.get(tab).unwrap().focused_pane(), 1, "the shell pane has it again");
}
