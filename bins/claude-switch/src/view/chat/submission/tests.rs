use super::*;

#[test]
fn image_confirmation_requires_the_cursor_prompt_and_a_whole_numeric_marker() {
    let mut screen = Screen::new(80, 12, 0);
    screen.feed("❯ [Image #7]\r\nAssistant says [Image #9]\r\n❯ ".as_bytes());
    assert!(clean_prompt(&screen));
    screen.feed(b"[Image #0]");
    assert!(!is_image_marker(&prompt(&screen).unwrap()));
    screen.feed("\r\n❯ [Image #12]".as_bytes());
    assert!(is_image_marker(&prompt(&screen).unwrap()));
    screen.feed(b" draft");
    assert!(!is_image_marker(&prompt(&screen).unwrap()));
}

#[test]
fn slash_prompt_evidence_needs_quiet_and_rejects_existing_input_or_busy_footer() {
    let mut screen = Screen::new(80, 12, 0);
    screen.feed("❯ ".as_bytes());
    assert!(idle_slash_prompt(&screen, true));
    assert!(!idle_slash_prompt(&screen, false));
    screen.feed(b"existing");
    assert!(!idle_slash_prompt(&screen, true));
    screen.feed("\r\nesc to interrupt\r\n❯ ".as_bytes());
    assert!(!idle_slash_prompt(&screen, true));
}

#[test]
fn reports_only_parse_bounded_explicit_success_values() {
    assert_eq!(reported_effort("Set effort level to max (this session only)"), Some("max"));
    assert_eq!(reported_effort("Effort level: auto"), Some("auto"));
    assert_eq!(reported_effort("/effort high"), None);
    assert_eq!(reported_model("Set model to Sonnet 4.6 (default)"), Some("Sonnet 4.6".into()));
    assert_eq!(reported_model("Couldn't set model to sonnet"), None);
    assert_eq!(reported_model("Set model to bogus\u{1b}"), None);
}

#[test]
fn unchanged_screen_reports_are_not_confirmation_but_repeated_new_reports_are() {
    let now = Instant::now();
    let mut screen = Screen::new(80, 16, 0);
    screen.feed("  ⎿ Set effort level to low\r\n❯ ".as_bytes());
    let mut chat = Chat { slash: true, ..Default::default() };
    chat.cli_report.effort = Some("high");
    chat.cli_report.baseline = local_reports(&screen);
    chat.cli_report.expects = Some(Choice::Effort);
    chat.cli_report.tracking = true;
    chat.output(now - QUIET, true, false);
    assert!(chat.observe_cli(&ChatInfo::default(), &screen, now));
    assert_eq!(chat.cli_report.effort, Some("high"));
    assert!(chat.error.unwrap().contains("did not report an effort change"));
    chat.error = None;
    chat.cli_report.epoch = chat.output_epoch;
    chat.cli_report.expects = Some(Choice::Effort);
    chat.cli_report.tracking = true;
    screen.feed("\r\n  ⎿ Set effort level to low\r\n❯ ".as_bytes());
    chat.output(now - QUIET, true, false);
    assert!(chat.observe_cli(&ChatInfo::default(), &screen, now));
    assert_eq!(chat.cli_report.effort, Some("low"));
    assert!(chat.error.is_none());
}

#[test]
fn slash_refuses_controls_and_requires_a_command_name() {
    assert_eq!(slash_bytes("/model sonnet").unwrap(), b"/model sonnet\r");
    assert!(slash_bytes("/").is_none());
    assert!(slash_bytes("/ model").is_none());
    assert!(slash_bytes("/effort low\r/model").is_none());
}
