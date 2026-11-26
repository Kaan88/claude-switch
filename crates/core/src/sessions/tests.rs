use super::*;

#[test]
fn prompt_excerpt_removes_inline_runtime_reminders_before_truncation() {
    let value = serde_json::json!({"type":"user","message":{"content":[
        {"type":"text","text":"Before <system-reminder>PRIVATE_RUNTIME_TEXT</system-reminder>after"},
        {"type":"text","text":"<system-reminder>SECOND_REMINDER</system-reminder>"}
    ]}});
    assert_eq!(prompt_of(&value, 30).as_deref(), Some("Before after"));
}
use crate::util::testing::{TempDir, config_with, put};
use serde_json::json;

fn user(text: &str) -> String {
    json!({"type":"user","isSidechain":false,"cwd":"C:\\work\\kweb-mail","sessionId":"s","entrypoint":"cli",
           "gitBranch":"main","message":{"role":"user","content":text}})
    .to_string()
}

fn assistant(text: &str) -> String {
    json!({"type":"assistant","cwd":"C:\\work\\kweb-mail","entrypoint":"cli","message":{"role":"assistant",
           "content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":text},
                      {"type":"tool_use","id":"t","name":"Read","input":{}}]}})
    .to_string()
}

fn transcript(dir: &Path, id: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(format!("{id}.jsonl"));
    put(&path, &(lines.join("\n") + "\n"));
    path
}

#[test]
fn the_custom_title_beats_the_ai_title_and_the_last_one_wins() {
    let t = TempDir::new("ses-title");
    let lines = [
        r#"{"type":"custom-title","customTitle":"First name","sessionId":"s"}"#.to_string(),
        user("hello there"),
        r#"{"type":"ai-title","aiTitle":"Generated","sessionId":"s"}"#.to_string(),
        r#"{"type":"custom-title","customTitle":"Second name","sessionId":"s"}"#.to_string(),
        r#"{"type":"ai-title","aiTitle":"Generated later","sessionId":"s"}"#.to_string(),
    ];
    let s = read_transcript(&transcript(&t.0, "a", &lines)).unwrap();
    assert_eq!(s.title, "Second name");
    let lines = [
        user("hello"),
        r#"{"type":"ai-title","aiTitle":"One"}"#.into(),
        r#"{"type":"ai-title","aiTitle":"Two"}"#.into(),
    ];
    assert_eq!(read_transcript(&transcript(&t.0, "b", &lines)).unwrap().title, "Two");
}

#[test]
fn the_fallback_title_is_the_first_real_prompt() {
    let t = TempDir::new("ses-prompt");
    let meta = json!({"type":"user","isMeta":true,"message":{"role":"user","content":"meta text"}}).to_string();
    let command = user("<command-name>/clear</command-name>");
    let blocks = json!({"type":"user","cwd":"C:\\x","message":{"role":"user","content":[
        {"type":"text","text":"  Fix   the\nbug in the parser please"}]}})
    .to_string();
    let lines = ["not json at all".to_string(), meta, command, blocks, user("a later prompt")];
    let s = read_transcript(&transcript(&t.0, "0123456789abcdef", &lines)).unwrap();
    assert_eq!(s.title, "Fix the bug in the parser please");
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!(s.cwd, PathBuf::from("C:\\work\\kweb-mail"));
    assert_eq!(s.last_app, App::Cli);
    assert_eq!(s.id, "0123456789abcdef");
    let long = "abcdefghij ".repeat(20);
    let s = read_transcript(&transcript(&t.0, "long", &[user(&long)])).unwrap();
    assert_eq!(s.title.chars().count(), 80);
    let s = read_transcript(&transcript(&t.0, "0123456789", &[assistant("only an answer")])).unwrap();
    assert_eq!(s.title, "01234567");
}

#[test]
fn a_file_without_session_lines_is_not_a_session() {
    let t = TempDir::new("ses-none");
    let lines = [r#"{"type":"file-history-snapshot","snapshot":{}}"#.to_string(), "garbage".into()];
    assert!(read_transcript(&transcript(&t.0, "x", &lines)).is_none());
    let side = json!({"type":"user","isSidechain":true,"cwd":"C:\\x","message":{"content":"hi"}}).to_string();
    assert!(read_transcript(&transcript(&t.0, "y", &[side])).is_none());
    put(&t.0.join("z.txt"), &user("hi"));
    assert!(read_transcript(&t.0.join("z.txt")).is_none());
}

#[test]
fn the_newest_line_gives_the_folder_branch_and_app() {
    let t = TempDir::new("ses-newest");
    let desk = json!({"type":"assistant","cwd":"C:\\later","gitBranch":"feature","entrypoint":"claude-desktop",
                      "message":{"content":[{"type":"text","text":"ok"}]}})
    .to_string();
    let side = json!({"type":"assistant","isSidechain":true,"cwd":"C:\\side","entrypoint":"cli"}).to_string();
    let s = read_transcript(&transcript(&t.0, "n", &[user("hi"), desk, side])).unwrap();
    assert_eq!(s.cwd, PathBuf::from("C:\\later"));
    assert_eq!(s.branch.as_deref(), Some("feature"));
    assert_eq!(s.last_app, App::Desktop);
    assert!(!s.worker);
}

#[test]
fn a_session_whose_newest_entrypoint_is_an_sdk_is_a_worker() {
    let t = TempDir::new("ses-worker");
    let line = |entry: &str| {
        json!({"type":"user","cwd":"C:\\w","entrypoint":entry,"message":{"role":"user","content":"go"}}).to_string()
    };
    let s = read_transcript(&transcript(&t.0, "w1", &[line("cli"), line("sdk-cli")])).unwrap();
    assert!(s.worker);
    assert_eq!(s.last_app, App::Cli);
    let s = read_transcript(&transcript(&t.0, "w2", &[line("sdk-ts")])).unwrap();
    assert!(s.worker);
    let s = read_transcript(&transcript(&t.0, "w3", &[line("sdk-ts"), line("cli")])).unwrap();
    assert!(!s.worker, "the newest entrypoint decides");
    let s = read_transcript(&transcript(&t.0, "w4", &[line("claude-desktop")])).unwrap();
    assert!(!s.worker);
    let s = read_transcript(&transcript(&t.0, "w5", &[assistant("no entrypoint here")])).unwrap();
    assert!(!s.worker);
}

#[test]
fn a_big_transcript_is_read_at_both_ends_only() {
    let t = TempDir::new("ses-big");
    let mut lines = vec![r#"{"type":"custom-title","customTitle":"Early title"}"#.to_string(), user("first prompt")];
    let filler = assistant(&"x".repeat(1000));
    for _ in 0..1500 {
        lines.push(filler.clone());
    }
    // A title in the middle of the file is not seen: only the head and tail are read.
    lines.push(r#"{"type":"custom-title","customTitle":"Middle title"}"#.to_string());
    for _ in 0..1500 {
        lines.push(filler.clone());
    }
    lines.push(r#"{"type":"ai-title","aiTitle":"Late ai title"}"#.to_string());
    let file = transcript(&t.0, "big", &lines);
    assert!(std::fs::metadata(&file).unwrap().len() > 3 * CHUNK);
    let s = read_transcript(&file).unwrap();
    assert_eq!(s.title, "Early title");
    assert!(s.last_prompt.is_empty(), "the first prompt isn't carried forward from the head");
    let p = preview(&file, 2);
    assert_eq!(p.len(), 2);
    assert!(!p[0].from_user && p[0].text.starts_with("xxx"));
}

#[test]
fn a_line_cut_by_either_chunk_is_not_read() {
    let t = TempDir::new("ses-cut");
    let mut body = String::new();
    let big = r#"{"type":"custom-title","customTitle":"Hidden"}"#.to_string();
    body.push_str(&user("hi"));
    body.push('\n');
    // One huge line crossing the tail's start, whose end alone would parse as a title line.
    body.push_str(&format!(r#"{{"type":"assistant","pad":"{}","inner":{}}}"#, "y".repeat(700 * 1024), big));
    body.push('\n');
    body.push_str(&assistant("done"));
    body.push('\n');
    let file = t.0.join("cut.jsonl");
    put(&file, &body);
    let s = read_transcript(&file).unwrap();
    assert_eq!(s.title, "hi");
}

#[test]
fn a_line_that_starts_where_the_tail_chunk_starts_is_read() {
    let t = TempDir::new("ses-edge");
    let title = r#"{"type":"custom-title","customTitle":"At the edge"}"#;
    // The last `CHUNK` bytes: the title line, then one line that fills the rest.
    let (open, close) = (r#"{"type":"assistant","pad":""#, "\"}\n");
    let pad = CHUNK as usize - title.len() - 1 - open.len() - close.len();
    let end = format!("{title}\n{open}{}{close}", "x".repeat(pad));
    assert_eq!(end.len() as u64, CHUNK);
    let start = format!("{}\n{}\n", user("hi"), assistant(&"y".repeat(2 * CHUNK as usize)));
    let file = t.0.join("edge.jsonl");
    put(&file, &(start + &end));
    assert_eq!(read_transcript(&file).unwrap().title, "At the edge");
}

fn setup_scan(t: &TempDir) -> (Roots, Config) {
    let roots = t.roots();
    let config = config_with(&roots, &[("work", "Work")]);
    (roots, config)
}

#[test]
fn sidechains_and_subagent_folders_are_not_sessions() {
    let t = TempDir::new("ses-sub");
    let (roots, config) = setup_scan(&t);
    let folder = roots.primary_cli_dir().join("projects").join("C--work");
    transcript(&folder, "main-session", &[user("hi")]);
    transcript(&folder.join("main-session").join("subagents"), "agent-1", &[user("sub")]);
    transcript(&roots.primary_cli_dir().join("projects").join("subagents"), "agent-2", &[user("sub")]);
    let side = json!({"type":"user","isSidechain":true,"message":{"content":"x"}}).to_string();
    transcript(&folder, "side-only", &[side]);
    let ids: Vec<String> = scan(&roots, &config).into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["main-session"]);
}

#[test]
fn naming_runs_never_enter_the_index_even_with_tools_shown() {
    let t = TempDir::new("ses-naming");
    let (roots, mut config) = setup_scan(&t);
    config.show_workers = true;
    let projects = roots.primary_cli_dir().join("projects");
    transcript(&projects.join("C--Temp-claude-switch-naming-123-456-0"), "private-project", &[user("hi")]);
    let tool = |folder: &str| {
        json!({"type":"user","cwd":folder,"entrypoint":"sdk-cli","message":{"content":"go"}}).to_string()
    };
    transcript(&projects.join("other-encoding"), "private-cwd", &[tool(r"C:\Temp\claude-switch-naming-123-456-1")]);
    transcript(&projects.join("C--Temp-legitimate"), "normal-tool", &[tool(r"C:\Temp\work")]);
    assert_eq!(scan(&roots, &config).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["normal-tool"]);
}

#[test]
fn the_same_projects_folder_through_a_junction_counts_once() {
    let t = TempDir::new("ses-junction");
    let (roots, config) = setup_scan(&t);
    let folder = roots.primary_cli_dir().join("projects").join("C--work");
    transcript(&folder, "one", &[user("hi")]);
    transcript(&folder, "two", &[user("there")]);
    let work = &config.accounts[1];
    std::fs::create_dir_all(&work.cli_dir).unwrap();
    crate::share::apply(&roots, work).unwrap();
    assert!(work.cli_dir.join("projects").join("C--work").join("one.jsonl").is_file());
    let sessions = scan(&roots, &config);
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|s| s.file.starts_with(roots.primary_cli_dir())));
}

#[test]
fn a_desktop_record_is_joined_by_its_cli_session_id() {
    let t = TempDir::new("ses-desktop");
    let (roots, config) = setup_scan(&t);
    let folder = roots.primary_cli_dir().join("projects").join("C--work");
    transcript(&folder, "cli-now", &[user("prompt text")]);
    transcript(&folder, "cli-before", &[user("older")]);
    transcript(&folder, "plain", &[r#"{"type":"custom-title","customTitle":"Mine"}"#.into(), user("x")]);
    let records = config.accounts[1].desktop_dir.join("claude-code-sessions").join("acc").join("org");
    put(
        &records.join("local_1.json"),
        &json!({"sessionId":"local_1","cliSessionId":"cli-now","cwd":"C:\\work","title":"Desktop title",
                "isArchived":true,"lastActivityAt":4_000_000_000_000u64,"createdAt":1,
                "priorCliSessionIds":["cli-before","cli-now"]})
        .to_string(),
    );
    put(
        &records.join("local_2.json"),
        &json!({"sessionId":"local_2","cliSessionId":"plain","title":"Ignored","lastActivityAt":1}).to_string(),
    );
    put(&records.join("tasks.json"), r#"{"scheduledTasks":[]}"#);
    put(&records.join("list.json"), "[1]");
    let sessions = scan(&roots, &config);
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids[0], "cli-now", "the record's activity makes it the newest");
    assert!(!ids.contains(&"cli-before"), "an earlier transcript of a desktop session is left out");
    let s = &sessions[0];
    assert_eq!(s.title, "Desktop title");
    assert_eq!(s.desktop_id.as_deref(), Some("local_1"));
    assert_eq!(s.desktop_account.as_deref(), Some("work"));
    assert!(s.archived);
    assert_eq!(s.last_active_ms, 4_000_000_000_000);
    let plain = sessions.iter().find(|s| s.id == "plain").unwrap();
    assert_eq!(plain.title, "Mine", "a custom title beats the record's");
    assert_eq!(plain.desktop_id.as_deref(), Some("local_2"));
}

#[test]
fn prior_ids_stay_when_the_current_transcript_is_missing() {
    let t = TempDir::new("ses-prior");
    let (roots, config) = setup_scan(&t);
    let folder = roots.primary_cli_dir().join("projects").join("C--work");
    transcript(&folder, "cli-before", &[user("older")]);
    let records = config.accounts[0].desktop_dir.join("claude-code-sessions").join("acc").join("org");
    put(
        &records.join("local_1.json"),
        &json!({"sessionId":"local_1","cliSessionId":"gone","priorCliSessionIds":["cli-before"]}).to_string(),
    );
    assert_eq!(scan(&roots, &config).len(), 1);
}

fn session(id: &str, cwd: &str, at: u64) -> Session {
    Session {
        id: id.into(),
        title: id.into(),
        last_prompt: String::new(),
        grouping: Default::default(),
        turn_ended: false,
        transcript_ms: at,
        cwd: PathBuf::from(cwd),
        branch: None,
        last_active_ms: at,
        bytes: 0,
        last_app: App::Cli,
        desktop_id: None,
        desktop_account: None,
        archived: false,
        worker: false,
        file: PathBuf::new(),
    }
}

#[test]
fn groups_follow_their_newest_session() {
    let list = [
        session("a", "C:\\Work\\kweb-mail", 10),
        session("b", "c:\\work\\KWEB-MAIL\\", 30),
        session("c", "C:\\other", 20),
        session("d", "", 5),
        session("e", "C:\\", 1),
    ];
    let groups = group_by_folder(&list);
    let summary: Vec<(&str, Vec<usize>)> = groups.iter().map(|g| (g.name.as_str(), g.sessions.clone())).collect();
    assert_eq!(summary, [("KWEB-MAIL", vec![1, 0]), ("other", vec![2]), ("(no folder)", vec![3]), ("C:", vec![4])]);
    assert!(groups[2].cwd.as_os_str().is_empty());
}

#[test]
fn the_preview_leaves_out_tools_meta_and_commands() {
    let t = TempDir::new("ses-preview");
    let tool_result = json!({"type":"user","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"t","content":"file text"}]}})
    .to_string();
    let only_tool = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}).to_string();
    let meta = json!({"type":"user","isMeta":true,"message":{"content":"meta"}}).to_string();
    let side =
        json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"s"}]}}).to_string();
    let lines = [
        user("first"),
        assistant("answer one"),
        user("<command-name>/model</command-name>"),
        tool_result,
        only_tool,
        meta,
        side,
        user("second\n\n  question"),
        assistant("answer two"),
    ];
    let file = transcript(&t.0, "p", &lines);
    let p = preview(&file, 3);
    let got: Vec<(bool, &str)> = p.iter().map(|l| (l.from_user, l.text.as_str())).collect();
    assert_eq!(got, [(false, "answer one"), (true, "second question"), (false, "answer two")]);
    assert!(preview(&file, 0).is_empty());
    assert!(preview(&t.0.join("missing.jsonl"), 5).is_empty());
    let long = transcript(&t.0, "q", &[user(&"z".repeat(1000))]);
    assert_eq!(preview(&long, 1)[0].text.len(), 400);
}

#[test]
#[ignore = "reads the real folders of this computer"]
fn the_real_sessions_are_listed() {
    let roots = Roots::from_env().expect("USERPROFILE and APPDATA");
    let start = std::time::Instant::now();
    let config = crate::config::load(&roots);
    let sessions = scan(&roots, &config);
    let scanned = start.elapsed().as_millis();
    let running = crate::running::scan(&roots, &config);
    let total = start.elapsed().as_millis();
    let groups = group_by_folder(&sessions);
    let desktop = sessions.iter().filter(|s| s.desktop_id.is_some()).count();
    let archived = sessions.iter().filter(|s| s.archived).count();
    let no_cwd = sessions.iter().filter(|s| s.cwd.as_os_str().is_empty()).count();
    let from_id = sessions.iter().filter(|s| s.title.len() == 8 && s.id.starts_with(&s.title)).count();
    println!(
        "accounts {} sessions {} groups {} with desktop record {} archived {} without folder {} \
         titled by id {} running {} desktop app found {} scan {} ms scan+running {} ms",
        config.accounts.len(),
        sessions.len(),
        groups.len(),
        desktop,
        archived,
        no_cwd,
        from_id,
        running.len(),
        crate::launch::desktop_exe().is_some(),
        scanned,
        total
    );
}
