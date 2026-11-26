use super::*;
use crate::dialogs;

#[test]
fn preview_names_the_continuation_without_editing_markdown_and_refuses_a_blank_name() {
    let mut st = app();
    let now = Instant::now();
    let source = st.selected_session().unwrap().clone();
    assert!(st.marks.rename(&source.id, &source.file, "Custom source"));
    let mut dialog = Compact::new(&st, source, None);
    dialog.apply(&mut st, Ok(Update::Preview("## Goal\n\nContinue the work.".into())));
    assert_eq!(dialog.new_name.text(), "Custom source · continued");
    dialog.key(&KeyName::Tab, Mods::default(), None);
    dialog.text_input("My edited name 😺");
    assert_eq!(dialog.new_name.text(), "My edited name 😺");
    assert_eq!(dialog.preview.text, "## Goal\n\nContinue the work.");
    dialog.new_name.select_all();
    dialog.key(&KeyName::Backspace, Mods::default(), None);
    assert!(dialog.click(&mut st, now, OK, 0.0), "invalid names leave the editor open");
    assert!(dialog.message.as_ref().unwrap().1);
    dialog.text_input("A valid name");
    assert!(!dialog.click(&mut st, now, OK, 0.0));
    assert!(st.tabs.list.is_empty(), "demo does not launch Claude");
}
#[test]
fn saved_agent_is_preselected_and_can_be_overridden_with_plain_claude() {
    let mut st = app();
    st.config.compact.set_default_agent("main-reviewer").unwrap();
    let mut dialog = Compact::new(&st, st.selected_session().unwrap().clone(), Some("main"));
    dialog.apply(&mut st, Ok(Update::Preview("handover".into())));
    assert_eq!(dialog.agent_name(), Some("main-reviewer"), "even before discovery");
    let now = Instant::now();
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 2, "the discovered entry replaces the settings-only entry");
    assert_eq!(dialog.key(&KeyName::Enter, Mods::default(), None), Some(OPTION + 1));
    assert!(dialog.agents[0].description.contains("Synthetic"));
    dialog.click(&mut st, now, OPTION, 0.0);
    assert_eq!(dialog.agent_name(), None, "Default (Claude) explicitly adds no flag");
    dialog.click(&mut st, now, ACCOUNT, 0.0);
    dialog.click(&mut st, now, OPTION + 1, 0.0);
    assert_eq!(dialog.agent_name(), Some("main-reviewer"), "an account change resets to Settings");
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 3);
    assert!(dialog.agents[0].description.contains("not found"));
}

#[test]
fn creating_a_handover_dialog_reads_another_windows_saved_default() {
    let root = std::env::temp_dir().join(format!("cs-agent-default-{}", crate::compact_open::session_id().unwrap()));
    let roots = claude_switch_core::Roots { home: root.join("home"), appdata: root.join("appdata") };
    let mut st = app();
    let session = st.selected_session().unwrap().clone();
    st.source = Source::Real(roots.clone());
    assert!(st.config.compact.default_agent.is_none());
    claude_switch_core::config::update_compact(&roots, |p| p.set_default_agent("reviewer")).unwrap();
    // The history route constructs the same picker without finding or running OpenCode.
    st.ask_compact_for(session, None, true);
    assert_eq!(compact(&st).agent_name(), Some("reviewer"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn preview_offers_default_first_and_changing_account_clears_the_explicit_agent() {
    let mut st = app();
    let now = Instant::now();
    let Some(Dialog::Compact(mut dialog)) = st.dialog.take() else { panic!("expected handover dialog") };
    dialog.apply(&mut st, Ok(Update::Preview("An editable handover".into())));
    assert_eq!(dialog.agent_label(), "Default (Claude)");
    assert_eq!(dialog.agent_name(), None);
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 2);
    assert_eq!(dialog.key(&KeyName::Enter, Mods::default(), None), Some(OPTION));
    dialog.key(&KeyName::Down, Mods::default(), None);
    let chosen = dialog.key(&KeyName::Enter, Mods::default(), None).unwrap();
    dialog.click(&mut st, now, chosen, 0.0);
    assert_eq!(dialog.agent_name(), Some("main-reviewer"));
    dialog.click(&mut st, now, ACCOUNT, 0.0);
    dialog.click(&mut st, now, OPTION + 1, 0.0);
    assert_eq!(dialog.agent_name(), None);
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.agents[0].name, "work-reviewer");
    assert_eq!(dialog.preview.text, "An editable handover");
}

fn app() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.select_first();
    st.command(Instant::now(), COMMAND, None);
    st
}

fn compact(st: &AppState) -> &Compact {
    let Some(Dialog::Compact(dialog)) = &st.dialog else { panic!("expected handover dialog") };
    dialog
}

#[test]
fn real_source_preview_and_cancel_leave_no_handover_history() {
    let root = std::env::temp_dir().join(format!("cs-preview-cancel-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let roots = claude_switch_core::Roots { home: root.clone(), appdata: root.clone() };
    let mut st = app();
    let Some(Dialog::Compact(mut dialog)) = st.dialog.take() else { panic!("expected handover dialog") };
    let id = dialog.session.id.clone();
    st.source = Source::Real(roots.clone());
    dialog.apply(&mut st, Ok(Update::Preview("private unedited preview".into())));
    assert!(dialog.saved.is_none());
    dialog.preview.set("redacted".into());
    st.dialog = Some(Dialog::Compact(dialog));
    dialogs::click(&mut st, Instant::now(), CANCEL, 0.0);
    assert!(st.dialog.is_none());
    assert!(compact::list(&roots, &id).unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn demo_requires_provider_consent_and_preview_before_the_open_action() {
    let mut st = app();
    let now = Instant::now();
    let click = |st: &mut AppState, id| dialogs::click(st, now, id, 0.0);
    assert!(compact(&st).phase == Phase::Setup);
    assert!(compact(&st).focus.text.contains("remaining verification"));
    click(&mut st, MODEL);
    click(&mut st, OPTION + 1);
    assert_eq!(st.config.compact.model, "demo/summary-thorough");
    assert!(st.config.compact.consents.is_empty());
    click(&mut st, ACCOUNT);
    click(&mut st, OPTION + 1);
    click(&mut st, OK);
    assert!(compact(&st).phase == Phase::Consent);
    assert!(st.config.compact.consents.is_empty());
    assert!(st.tabs.list.is_empty());
    click(&mut st, OK);
    assert!(compact(&st).phase == Phase::Preview);
    assert_eq!(compact(&st).accounts[compact(&st).account].0, "work");
    assert!(st.config.compact.consents.contains("demo"));
    assert!(compact(&st).job.is_none());
    let before = claude_switch_core::compact::size(&compact(&st).preview.text);
    dialogs::text_input(&mut st, "edit ");
    assert_eq!(claude_switch_core::compact::size(&compact(&st).preview.text).0, before.0 + 5);
    assert!(st.tabs.list.is_empty());
    click(&mut st, OK);
    assert!(st.dialog.is_none());
    assert!(st.tabs.list.is_empty(), "demo never starts Claude");
    st.forget_handover_consents();
    assert!(st.config.compact.consents.is_empty());
}
