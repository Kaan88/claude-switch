use super::*;

#[test]
fn summarizer_config_has_one_step_no_tool_permissions_and_no_warming() {
    let spec = spec_config();
    assert_eq!(spec["agents"][AGENT]["steps"], 1);
    assert_eq!(spec["agents"][AGENT]["permissions"][0], json!({"action":"*","resource":"*","effect":"deny"}));
    assert_eq!(spec["warming"]["enabled"], false);
    assert!(spec["policies"].as_array().unwrap().iter().any(|p| p["action"] == "tool.use" && p["effect"] == "deny"));
}

#[test]
fn machine_output_accepts_only_text_and_refuses_unexpected_tools_without_leaking_output() {
    let lines = concat!(
        "{\"type\":\"reasoning\",\"part\":{\"text\":\"private\"}}\n",
        "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"first\"}}\n",
        "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\" second\"}}\n"
    );
    assert_eq!(answer(lines).unwrap(), "first second");
    let with_tool = format!("{lines}{{\"type\":\"tool_use\",\"part\":{{\"output\":\"private\"}}}}");
    assert!(!answer(&with_tool).unwrap_err().contains("private"));
    assert!(answer("{\"type\":\"error\",\"error\":{\"message\":\"private\"}}").is_err());
}

#[cfg(windows)]
fn fake(cwd: &Path, script: &str) -> Command {
    let program =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    command(&program, cwd, &["-NoProfile", "-NonInteractive", "-Command", script])
}

#[cfg(windows)]
#[test]
fn fake_reads_multiline_payload_only_from_stdin_and_stderr_is_not_disclosed() {
    let temp = Scratch::new().unwrap();
    let cwd = temp.0.clone();
    assert!(fs::read_dir(&cwd).unwrap().next().is_none());
    let input = "private marker\nline 2 ".repeat(5_000);
    let cmd = fake(&cwd, "[Console]::Out.Write([Console]::In.ReadToEnd())");
    assert!(!cmd.get_args().any(|a| a.to_string_lossy().contains("private marker")));
    assert!(cmd.get_envs().any(|(key, value)| key == "PWD" && value.is_none()));
    assert_eq!(invoke(cmd, Some(&input), || false, Duration::from_secs(15)).unwrap(), input);
    let cmd = fake(&cwd, "[Console]::Error.Write('private diagnostic'); exit 3");
    let error = invoke(cmd, None, || false, Duration::from_secs(15)).unwrap_err();
    assert!(!error.contains("private diagnostic"));
}

#[cfg(windows)]
#[test]
fn fake_is_killed_on_timeout_and_cancel() {
    let temp = Scratch::new().unwrap();
    let cwd = temp.0.clone();
    let cmd = || fake(&cwd, "Start-Sleep -Seconds 30");
    assert_eq!(invoke(cmd(), None, || false, Duration::from_millis(100)).unwrap_err(), "OpenCode timed out.");
    let start = Instant::now();
    let cancelled = || start.elapsed() >= Duration::from_millis(100);
    assert_eq!(invoke(cmd(), None, cancelled, Duration::from_secs(5)).unwrap_err(), "Cancelled.");
    assert!(start.elapsed() < Duration::from_secs(5));
}
