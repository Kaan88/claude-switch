use super::*;
use crate::dialogs::{self, Dialog};
use crate::source::Source;
use crate::winfile::WindowFile;
use std::time::Instant;

fn demo() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(true), WindowFile::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st
}

#[test]
fn the_launch_is_the_owners_normal_opencode_with_options_before_the_folder() {
    let folder = std::env::temp_dir().join("cs-oc folder");
    let program = Path::new("C:/tools/opencode.exe");
    let fresh = launch(program, &folder, &Start::Fresh);
    assert_eq!(fresh.program, program);
    assert_eq!(fresh.args, [folder.to_string_lossy().into_owned()]);
    assert_eq!(fresh.cwd.as_deref(), Some(folder.as_path()));
    assert_eq!(fresh.label, "OpenCode · cs-oc folder");
    assert!(fresh.env_remove.iter().any(|k| k == "PWD"));
    assert!(fresh.env_remove.iter().any(|k| k == "CLAUDE_CONFIG_DIR"), "no Claude account leaks in");
    assert!(
        !fresh.env.keys().any(|k| k.starts_with("OPENCODE_")),
        "no isolated job config: the owner's own config and plugins apply"
    );
    let again = launch(program, &folder, &Start::Continue);
    assert_eq!(again.args[0], "--continue");
    let text = "## Goal\nFix \"push\" & keep going\n";
    let handover = launch(program, &folder, &Start::Prompt(text.into()));
    assert_eq!(handover.args, ["--prompt".to_string(), text.into(), folder.to_string_lossy().into_owned()]);
}

#[test]
fn the_first_prompt_is_the_text_or_a_request_to_read_the_saved_file() {
    let saved = std::env::temp_dir().join("handover.md");
    let short = "## Goal\nMultiline, with \"quotes\" and \\ back\\slashes.";
    assert_eq!(first_prompt(short, &saved).unwrap(), short);
    let long = format!("## Goal\n{}", "x".repeat(16_000));
    assert!(first_prompt(&long, &saved).unwrap().starts_with("Read the handover file at \""));
    let option_like = "--model evil/model";
    let prompt = first_prompt(option_like, &saved).unwrap();
    assert!(prompt.starts_with("Read the handover file") && prompt.contains("handover.md"), "{prompt}");
    assert!(first_prompt("/clear", &saved).is_err());
    assert!(first_prompt("  ", &saved).is_err());
}

#[test]
fn new_opencode_tab_asks_for_a_folder_only_and_opens_a_tab_that_is_no_claude_session() {
    let mut st = demo();
    st.command(Instant::now(), crate::actions::NEW_OPENCODE, None);
    assert!(matches!(st.dialog, Some(Dialog::NewSession(_))));
    let folder = st.sessions[0].cwd.clone();
    dialogs::text_input(&mut st, &folder.to_string_lossy());
    dialogs::click(&mut st, Instant::now(), dialogs::OK, 0.0);
    assert!(st.dialog.is_none(), "{:?}", st.info.as_ref().map(|i| i.message().to_owned()));
    let tab = st.tabs.shown().expect("the new tab is shown");
    assert_eq!(tab.opencode.as_deref(), Some(folder.as_path()));
    assert!(tab.session.is_none() && tab.account.is_none() && tab.pane.is_none() && tab.cwd.is_none());
    assert!(tab.label.starts_with("OpenCode · "));
}

#[test]
fn continue_in_opencode_opens_beside_the_source_tab_and_records_nothing() {
    let mut st = demo();
    let session = st.sessions[3].clone();
    let marks = format!("{:?}", st.marks);
    let claude = st.tabs.open(
        crate::tabs::NewTab {
            label: "source".into(),
            color: [1, 2, 3],
            session: Some(session.id.clone()),
            account: Some("main".into()),
        },
        crate::term::Term::empty(st.grid),
    );
    st.tabs.open(
        crate::tabs::NewTab { label: "other".into(), color: [1, 2, 3], session: None, account: None },
        crate::term::Term::empty(st.grid),
    );
    let tab = st.continue_in_opencode(&session, "## Goal\nCarry on", Path::new("demo-handover.md")).unwrap();
    let ids: Vec<_> = st.tabs.list.iter().map(|t| t.id).collect();
    let at = ids.iter().position(|&id| id == claude).unwrap();
    assert_eq!(ids[at + 1], tab, "next to the source session's tab");
    assert_eq!(st.tabs.get(tab).unwrap().opencode.as_deref(), Some(session.cwd.as_path()));
    assert_eq!(format!("{:?}", st.marks), marks, "no Claude provenance for an OpenCode tab");
    assert!(st.continue_in_opencode(&session, "/compact", Path::new("x.md")).is_err());
}
