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

#[test]
fn handover_launch_keeps_the_isolated_working_directory_and_config() {
    let temp = Scratch::new().unwrap();
    let cwd = temp.0.clone();
    assert!(fs::read_dir(&cwd).unwrap().next().is_none());
    let cmd = command(Path::new("fake-opencode"), &cwd, true);
    assert_eq!(cmd.get_current_dir(), Some(cwd.as_path()));
    assert!(cmd.get_envs().any(|(key, value)| key == "PWD" && value.is_none()));
    assert!(cmd.get_envs().any(|(key, value)| key == "OPENCODE_DISABLE_PROJECT_CONFIG" && value.unwrap() == "1"));
    assert!(cmd.get_envs().any(|(key, value)| key == "OPENCODE_CONFIG" && value.is_none()));
}
