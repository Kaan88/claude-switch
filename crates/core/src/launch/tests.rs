use super::*;
use crate::util::testing::{TempDir, config_with, log_in, put};
const ID: &str = "01234567-89ab-cdef-0123-456789abcdef";
const COPY: &str = "11111111-2222-4333-8444-555555555555";

fn session(cwd: &Path) -> Session {
    Session {
        id: ID.into(),
        title: "T".into(),
        last_prompt: String::new(),
        grouping: Default::default(),
        turn_ended: false,
        transcript_ms: 0,
        cwd: cwd.to_path_buf(),
        branch: None,
        last_active_ms: 0,
        bytes: 3 * 1024 * 1024,
        last_app: App::Cli,
        desktop_id: None,
        desktop_account: None,
        archived: false,
        worker: false,
        file: PathBuf::new(),
    }
}

fn setup(t: &TempDir) -> (Roots, Config) {
    let roots = t.roots();
    let config = config_with(&roots, &[("work", "Work")]);
    for a in &config.accounts {
        log_in(&roots, a);
    }
    (roots, config)
}

#[test]
fn first_secondary_sign_in_seeds_mcp_and_creates_shared_projects_before_launch() {
    let t = TempDir::new("launch-first-secondary");
    let roots = t.roots();
    put(&roots.home.join(".claude.json"), r#"{"mcpServers":{"one":{}},"oauthAccount":{"private":true}}"#);
    let mut config = crate::config::load(&roots);
    let work = crate::accounts::add(&roots, &mut config, "Work").unwrap();
    let main_projects = roots.primary_cli_dir().join("projects");
    assert!(!main_projects.exists());
    assert_eq!(login(&roots, &config, &work.slug).unwrap().args, ["auth", "login"]);
    assert_eq!(read_json(&work.cli_dir.join(".claude.json")), Some(json!({"mcpServers":{"one":{}}})));
    assert_eq!(
        std::fs::canonicalize(main_projects).unwrap(),
        std::fs::canonicalize(work.cli_dir.join("projects")).unwrap()
    );
}

#[test]
fn separate_secondary_projects_allow_only_reachable_transcripts_and_are_never_replaced() {
    let t = TempDir::new("launch-separate-projects");
    let (roots, config) = setup(&t);
    let mut s = session(&t.0);
    let work_projects = config.accounts[1].cli_dir.join("projects");
    s.file = roots.primary_cli_dir().join("projects/project").join(format!("{ID}.jsonl"));
    put(&s.file, "main data");
    let work_file = work_projects.join("project").join(format!("{ID}.jsonl"));
    put(&work_file, "work data");
    assert!(resume(&roots, &config, &s, "work", App::Cli, &[]).is_err());
    assert!(new_session(&roots, &config, "work", App::Cli, &t.0).is_err());
    s.file = work_file.clone();
    assert!(resume(&roots, &config, &s, "work", App::Cli, &[]).is_ok());
    assert!(resume(&roots, &config, &s, "main", App::Cli, &[]).is_err());
    assert_eq!(std::fs::read_to_string(work_file).unwrap(), "work data");
}

#[test]
fn a_copy_forks_the_session_also_while_it_runs_but_needs_a_login() {
    let t = TempDir::new("lau-fork");
    let (roots, config) = setup(&t);
    let folder = t.0.join("kweb-mail");
    std::fs::create_dir_all(&folder).unwrap();
    let s = session(&folder);
    // A running original and a start a moment ago do not stop a copy.
    note_launch(&roots, &s.id, "main");
    let l = fork(&roots, &config, &s, "main", COPY).unwrap();
    assert_eq!(l.program, PathBuf::from("claude"));
    assert_eq!(l.args, ["--resume", ID, "--fork-session", "--session-id", COPY]);
    assert!(fork(&roots, &config, &s, "main", ID).is_err());
    assert!(fork(&roots, &config, &s, "main", "--bad").is_err());
    assert_eq!(l.cwd.as_deref(), Some(folder.as_path()));
    assert_eq!(l.place, Place::Terminal);
    assert_eq!(l.label, "Main · kweb-mail · Copy");
    assert_eq!(l.env_remove[0], "CLAUDE_CONFIG_DIR");
    let w = fork(&roots, &config, &s, "work", COPY).unwrap();
    assert_eq!(w.env.get("CLAUDE_CONFIG_DIR"), Some(&config.accounts[1].cli_dir.to_string_lossy().into_owned()));
    assert_eq!(w.label, "Work · kweb-mail · Copy");
    // The copy is not noted as a start of the original.
    let history = std::fs::read_to_string(roots.appdata.join("claude-switch").join("history.json")).unwrap();
    assert_eq!(history.matches(ID).count(), 1);
    std::fs::remove_file(config.accounts[1].cli_dir.join(".credentials.json")).unwrap();
    assert_eq!(fork(&roots, &config, &s, "work", COPY), Err(Refusal::NotLoggedIn("Work".into())));
    assert!(matches!(fork(&roots, &config, &s, "nobody", COPY), Err(Refusal::NoSuchAccount(_))));
}

#[test]
fn a_shell_gets_no_session_identity_and_no_config_folder() {
    let t = TempDir::new("lau-shell");
    let l = shell(&t.0);
    assert_eq!(l.env_remove[0], "CLAUDE_CONFIG_DIR");
    assert_eq!(l.env_remove[1..], SESSION_VARS);
    assert!(l.env.is_empty());
    assert_eq!(l.place, Place::Terminal);
}

#[test]
fn a_session_running_anywhere_is_refused() {
    let t = TempDir::new("lau-running");
    let (roots, config) = setup(&t);
    let s = session(&t.0);
    let running =
        [Running { session_id: s.id.clone(), pid: 42, app: App::Desktop, account: "work".into(), cwd: PathBuf::new() }];
    for app in [App::Cli, App::Desktop] {
        for account in ["main", "work"] {
            let r = resume(&roots, &config, &s, account, app, &running);
            assert_eq!(r, Err(Refusal::AlreadyRunning(running[0].clone())));
        }
    }
    let text = Refusal::AlreadyRunning(running[0].clone()).to_string();
    assert!(text.contains("the desktop app") && text.contains("42"));
}

#[test]
fn the_first_account_removes_the_config_variable_and_a_further_one_sets_it() {
    let t = TempDir::new("lau-env");
    let (roots, config) = setup(&t);
    let folder = t.0.join("kweb-mail");
    std::fs::create_dir_all(&folder).unwrap();
    let s = session(&folder);
    let main = resume(&roots, &config, &s, "main", App::Cli, &[]).unwrap();
    assert_eq!(main.program, PathBuf::from("claude"));
    assert_eq!(main.args, ["--resume", ID]);
    assert_eq!(main.cwd.as_deref(), Some(folder.as_path()));
    assert!(main.env.is_empty());
    assert_eq!(main.env_remove[0], "CLAUDE_CONFIG_DIR");
    assert_eq!(main.env_remove[1..], SESSION_VARS);
    assert_eq!(main.place, Place::Terminal);
    assert_eq!(main.label, "Main · kweb-mail · Resume");
    let work = resume(&roots, &config, &s, "work", App::Desktop, &[]).unwrap();
    assert_eq!(work.args, ["--desktop", "--resume", ID]);
    assert_eq!(work.place, Place::Detached);
    assert_eq!(work.env.get("CLAUDE_CONFIG_DIR"), Some(&config.accounts[1].cli_dir.to_string_lossy().into_owned()));
    assert_eq!(work.env_remove, SESSION_VARS);
}

#[test]
fn no_launch_passes_on_a_sessions_identity_or_login() {
    let t = TempDir::new("lau-identity");
    let (roots, config) = setup(&t);
    for account in ["main", "work"] {
        let launches = [
            resume(&roots, &config, &session(&t.0), account, App::Cli, &[]).unwrap(),
            resume(&roots, &config, &session(&t.0), account, App::Desktop, &[]).unwrap(),
            new_session(&roots, &config, account, App::Cli, &t.0).unwrap(),
            login(&roots, &config, account).unwrap(),
            fork(&roots, &config, &session(&t.0), account, COPY).unwrap(),
            shell(&t.0),
            desktop_launch(PathBuf::from("Claude.exe"), find_account(&config, account).unwrap()),
        ];
        for l in launches {
            for var in [
                "CLAUDE_CODE_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDECODE",
                "CLAUDE_CODE_ENTRYPOINT",
                "CLAUDE_CODE_SESSION_ID",
                "CLAUDE_CODE_HOST_SESSION_ID",
                "CLAUDE_CODE_ACCOUNT_UUID",
                "CLAUDE_CODE_ORGANIZATION_UUID",
                "CLAUDE_CODE_MESSAGING_SOCKET",
                "CLAUDE_CODE_MESSAGING_TOKEN",
                "CLAUDE_CODE_CHILD_SESSION",
                "CLAUDE_CODE_EXECPATH",
            ] {
                assert!(l.env_remove.iter().any(|v| v == var), "{} keeps {var}", l.label);
            }
        }
    }
}

#[test]
fn provider_prefix_rules_are_case_insensitive_and_forward_compatible() {
    for name in ["Anthropic_Base_URL", "anthropic_FUTURE", "cLaUdE_code_use_BEDROCK", "CLAUDE_CONFIG_DIR", "claudecode"]
    {
        assert!(scrub_name(OsStr::new(name)), "{name}");
    }
    for name in ["PATH", "X_CLAUDE_CONFIG_DIR", "ANTHROPICS", "CLAUDECODES"] {
        assert!(!scrub_name(OsStr::new(name)), "{name}");
    }
}

#[test]
fn a_session_opened_a_moment_ago_is_refused() {
    let t = TempDir::new("lau-moment");
    let (roots, config) = setup(&t);
    let s = session(&t.0);
    let again = |account: &str, app: App| resume(&roots, &config, &s, account, app, &[]);
    let noted_at = |at: u64| {
        write_json(&history_file(&roots), &json!({(ID): {"account": "main", "at": at}})).unwrap();
    };
    let refusal = Err(Refusal::Invalid("This session was opened a moment ago. Give it a few seconds to start.".into()));
    noted_at(now_ms() - 5_000);
    assert_eq!(again("main", App::Cli), refusal);
    assert_eq!(again("work", App::Desktop), refusal);
    noted_at(now_ms() - 21_000);
    assert!(again("main", App::Cli).is_ok());
    noted_at(now_ms() + 3_600_000);
    assert!(again("main", App::Cli).is_ok(), "a time far ahead does not lock the session");
    note_starting(&roots, &s.id, "work");
    assert_eq!(again("main", App::Cli), refusal);
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"));
    noted_at(0);
    note_launch(&roots, &s.id, "main");
    assert_eq!(again("main", App::Cli), refusal, "the same account again still records the new time");
}

#[test]
fn the_old_history_format_is_still_read() {
    let t = TempDir::new("lau-old-history");
    let (roots, config) = setup(&t);
    let s = session(&t.0);
    put(&history_file(&roots), &json!({(ID): "work", "other": "main"}).to_string());
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"));
    assert!(resume(&roots, &config, &s, "work", App::Cli, &[]).is_ok());
    note_starting(&roots, "new-one", "main");
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"), "older entries stay as they were");
}

#[test]
fn non_uuid_transcript_stems_never_reach_resume_or_fork() {
    let t = TempDir::new("lau-stem");
    let (roots, config) = setup(&t);
    let mut s = session(&t.0);
    s.id = "not-a-uuid".into();
    assert!(matches!(resume(&roots, &config, &s, "main", App::Cli, &[]), Err(Refusal::Invalid(_))));
    assert!(matches!(fork(&roots, &config, &s, "main", COPY), Err(Refusal::Invalid(_))));
}

#[test]
fn a_gone_folder_resumes_in_the_home_folder_and_a_missing_login_is_refused() {
    let t = TempDir::new("lau-home");
    let (roots, mut config) = setup(&t);
    let s = session(&t.0.join("gone"));
    assert_eq!(resume(&roots, &config, &s, "main", App::Cli, &[]).unwrap().cwd, Some(roots.home.clone()));
    assert!(matches!(resume(&roots, &config, &s, "nobody", App::Cli, &[]), Err(Refusal::NoSuchAccount(_))));
    config.accounts.push(crate::util::testing::further(&roots, "new", "New"));
    assert_eq!(resume(&roots, &config, &s, "new", App::Cli, &[]), Err(Refusal::NotLoggedIn("New".into())));
}

#[test]
fn resuming_as_a_further_account_brings_its_links_and_mcp_up_to_date() {
    let t = TempDir::new("lau-share");
    let (roots, config) = setup(&t);
    std::fs::create_dir_all(roots.primary_cli_dir().join("skills")).unwrap();
    put(&roots.home.join(".claude.json"), r#"{"oauthAccount":{},"mcpServers":{"m":{"command":"x"}}}"#);
    resume(&roots, &config, &session(&t.0), "work", App::Cli, &[]).unwrap();
    let work = &config.accounts[1];
    assert!(work.cli_dir.join("skills").is_dir());
    let v = read_json(&work.cli_dir.join(".claude.json")).unwrap();
    assert!(v.get("mcpServers").is_none(), "existing Claude-owned settings are not rewritten");
    assert!(v.get("oauthAccount").is_some(), "the account's own login stays");
}

#[test]
fn new_sessions_and_logins_are_terminal_launches() {
    let t = TempDir::new("lau-new");
    let (roots, config) = setup(&t);
    let n = new_session(&roots, &config, "work", App::Cli, &t.0.join("proj")).unwrap();
    assert!(n.args.is_empty());
    assert_eq!(n.cwd, Some(t.0.join("proj")));
    assert_eq!(n.label, "Work · proj · New");
    assert!(n.env.contains_key("CLAUDE_CONFIG_DIR"));
    let l = login(&roots, &config, "main").unwrap();
    assert_eq!(l.args, ["auth", "login"]);
    assert_eq!(l.place, Place::Terminal);
    assert_eq!(l.env_remove[0], "CLAUDE_CONFIG_DIR");
    assert!(is_login(&l));
    let mut other = l;
    other.args[1] = "status".into();
    assert!(!is_login(&other));
}

#[test]
fn the_desktop_app_gets_its_own_data_folder_for_a_further_account() {
    let t = TempDir::new("lau-desktop");
    let (roots, config) = setup(&t);
    let exe = PathBuf::from("C:\\Apps\\Claude.exe");
    let main = desktop_launch(exe.clone(), &config.accounts[0]);
    assert!(main.args.is_empty());
    assert_eq!(main.place, Place::Detached);
    let work = desktop_launch(exe, &config.accounts[1]);
    assert_eq!(work.args, [format!("--user-data-dir={}", config.accounts[1].desktop_dir.display())]);
    for launch in [main, work] {
        assert!(launch.env.is_empty());
        assert_eq!(launch.env_remove[0], "CLAUDE_CONFIG_DIR");
        assert_eq!(launch.env_remove[1..], SESSION_VARS);
    }
    let _ = roots;
}

#[cfg(windows)]
#[test]
fn desktop_open_plans_arguments_without_verifying_an_installation() {
    let t = TempDir::new("desktop-plan");
    let (_, config) = setup(&t);
    let work = open_desktop(&config, "work").unwrap();
    assert!(is_desktop_request(&work));
    assert_eq!(work.args, [format!("--user-data-dir={}", config.accounts[1].desktop_dir.display())]);
    assert!(work.env.is_empty());
    assert!(work.env_remove.iter().any(|name| name == "CLAUDE_CONFIG_DIR"));
}

#[test]
fn the_shell_is_the_first_one_on_the_path() {
    let t = TempDir::new("lau-shell");
    put(&t.0.join("b").join("powershell.exe"), "");
    put(&t.0.join("c").join("cmd.exe"), "");
    let path = std::env::join_paths([t.0.join("a"), t.0.join("b"), t.0.join("c")]).unwrap();
    let names = ["pwsh.exe", "powershell.exe", "cmd.exe"];
    assert_eq!(first_on_path(Some(&path), &names), Some("powershell.exe"));
    assert_eq!(first_on_path(None, &names), None);
    let s = shell(&t.0);
    assert_eq!(s.place, Place::Terminal);
    assert_eq!(s.cwd.as_deref(), Some(t.0.as_path()));
}

#[test]
fn a_carry_to_another_account_is_warned_about() {
    let t = TempDir::new("lau-carry");
    let (roots, config) = setup(&t);
    let mut s = session(&t.0);
    assert_eq!(last_account(&roots, &config, &s), None);
    assert!(carry_warning(&roots, &config, &s, "main").unwrap().contains("unknown"));
    let w = carry_warning(&roots, &config, &s, "work").unwrap();
    assert!(w.contains("unknown") && w.contains("Work") && w.contains("3.0 MB"));
    s.desktop_account = Some("work".into());
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"));
    note_launch(&roots, &s.id, "main");
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("main"));
    note_launch(&roots, &s.id, "nobody");
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"), "an unknown account is not believed");
}

#[test]
fn verified_ownership_is_persisted_without_creating_a_launch_cooldown() {
    let t = TempDir::new("verified-owner");
    let (roots, config) = setup(&t);
    let s = session(&t.0);
    assert!(last_account(&roots, &config, &s).is_none());
    record_verified_owner(&roots, &s.id, "work").unwrap();
    assert_eq!(last_account(&roots, &config, &s).as_deref(), Some("work"));
    assert!(!launched_a_moment_ago(&roots, &s.id));
}
