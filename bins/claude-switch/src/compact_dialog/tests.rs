use super::*;
use crate::dialogs;

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
