use super::*;
use std::fs;

mod fixture {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

const TIME: Duration = Duration::from_secs(5);

fn fake(mode: &str) -> Command {
    let mut cmd = Command::new(fixture::fake_program());
    cmd.env("FAKE_MODE", mode).env_remove("FAKE_LOG");
    cmd
}

#[test]
fn discovery_prefers_absolute_override_and_native_npm_target() {
    let exe = format!("opencode{}", std::env::consts::EXE_SUFFIX);
    let sandbox = fixture::fake_program().with_file_name("discover");
    let npm = sandbox.join("node_modules/@opencode/cli/bin").join(&exe);
    fs::create_dir_all(npm.parent().unwrap()).unwrap();
    fs::write(&npm, "").unwrap();
    fs::write(sandbox.join(&exe), "").unwrap();
    let path = std::env::join_paths([&sandbox]).unwrap();
    assert_eq!(discover(None, Some(&path), None, None, None), Some(npm));
    let custom = fixture::fake_program();
    assert_eq!(discover(Some(custom.clone()), Some(&path), None, None, None), Some(custom));
    assert_eq!(discover(Some(PathBuf::from(exe)), Some(&path), None, None, None), None);
}

#[test]
fn run_captures_trimmed_output_and_diagnostic() {
    assert_eq!(run(fake(""), &["reload"], TIME).unwrap(), "reloaded");
    assert_eq!(run(fake("error"), &["reload"], TIME).unwrap_err(), "opencode reload failed: reload failure");
}

#[test]
fn status_counts_only_running_and_distinguishes_absent_and_bad_responses() {
    assert_eq!(status(|| None), Status::default());
    let online = status(|| Some(fake("")));
    assert_eq!(online.path, Some(fixture::fake_program()));
    assert_eq!(online.version.as_deref(), Some("2.0.5"));
    assert_eq!(online.running, Some(2));
    assert_eq!(active_sessions(fake("idle")), Some(0));
    assert_eq!(status(|| Some(fake("malformed"))).running, None);
    assert_eq!(active_sessions(fake("unknown")), None);
    let log = fixture::fake_program().with_file_name("down.log");
    assert_eq!(
        status(|| {
            let mut cmd = fake("down");
            cmd.env("FAKE_LOG", &log);
            Some(cmd)
        })
        .running,
        None
    );
    assert_eq!(fs::read_to_string(&log).unwrap(), "--version\nservice status\n");
}

#[test]
fn restart_uses_fallback_only_on_failure_and_proves_response() {
    for (mode, expected) in [
        ("normal", "service restart\napi session.active\n"),
        ("fallback", "service restart\nservice stop\nservice start\napi session.active\n"),
        ("failed-stop", "service restart\nservice stop\n"),
        ("malformed", "service restart\napi session.active\n"),
    ] {
        let log = fixture::fake_program().with_file_name(format!("{mode}.log"));
        let result = restart_service(|| {
            let mut cmd = fake(mode);
            cmd.env("FAKE_LOG", &log);
            cmd
        });
        if mode == "failed-stop" {
            assert!(result.unwrap_err().contains("Could not stop"));
        } else if mode == "malformed" {
            assert!(result.unwrap_err().contains("does not answer"));
        } else {
            assert_eq!(result.unwrap(), 2);
        }
        assert_eq!(fs::read_to_string(&log).unwrap(), expected);
    }
}

#[test]
fn model_listing_and_private_stdin_round_trip() {
    let models = models(fake(""), TIME, || false).unwrap();
    assert_eq!(models.iter().map(Model::id).collect::<Vec<_>>(), ["p/first", "p/group/model#high", "z/a"]);
    assert_eq!(models[1].provider, "p");
    assert_eq!(models[1].model, "group/model#high");
    let input = "private marker 😺\nsecond line\r\n".repeat(5_000);
    assert_eq!(run_prompt(fake(""), "p/model", &["--archived"], &input, TIME, || false).unwrap(), input);
    assert!(
        !run_prompt(fake("private_error"), "p/model", &[], "", TIME, || false)
            .unwrap_err()
            .contains("private diagnostic")
    );
}

#[test]
fn timeout_and_cancel_kill_children_instead_of_leaving_them_running() {
    use std::time::Instant;
    let done_timeout = fixture::fake_program().with_file_name("timeout-finished");
    let done_cancel = fixture::fake_program().with_file_name("cancel-finished");
    let mut timeout = fake("");
    timeout.arg("sleep").env("FAKE_DONE", &done_timeout);
    let ready = done_timeout.with_extension("started");
    let started = Instant::now();
    let budget = Duration::from_millis(700);
    assert_eq!(
        process::capture_with_clock(
            timeout,
            None,
            false,
            || false,
            budget,
            || { if ready.is_file() || started.elapsed() >= TIME { started + budget } else { started } }
        )
        .unwrap_err(),
        "OpenCode timed out."
    );
    assert!(ready.is_file(), "test child did not signal readiness before the watchdog");

    let mut cancel = fake("");
    cancel.env("FAKE_DONE", &done_cancel);
    let started = done_cancel.with_extension("started");
    assert_eq!(
        run_prompt(cancel, "p/model", &["--sleep"], "private marker", TIME, || started.is_file()).unwrap_err(),
        "Cancelled."
    );
    std::thread::sleep(Duration::from_millis(2100));
    assert!(!done_timeout.exists() && !done_cancel.exists(), "a timed-out or cancelled process was not killed");
}
