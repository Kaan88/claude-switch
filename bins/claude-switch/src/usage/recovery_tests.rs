use super::*;
use crate::winfile::WindowFile;

/// The app with the `work` account in a scratch folder of its own, with its fake credentials:
/// the folder goes when the test ends.
fn fixture(name: &str) -> (AppState, crate::testing::Scratch) {
    let folder = crate::testing::Scratch::created(&format!("cs-usage-{name}"));
    let roots = Roots { home: folder.to_path_buf(), appdata: folder.to_path_buf() };
    let mut app = AppState::new(Source::Demo, Some(true), WindowFile::default());
    app.source = Source::Real(roots);
    app.config.accounts.iter_mut().find(|a| a.slug == "work").unwrap().cli_dir = folder.to_path_buf();
    app.usage = Usage::default();
    (app, folder)
}

fn read_fixture(_: &Roots, dir: &Path, _: &dyn Fn() -> bool) -> Result<Reading, Error> {
    let token = claude_switch_core::usage::read_token(dir, 0)?;
    token.reading(br#"{"five_hour":{"utilization":10}}"#, 42)
}

fn finish(app: &mut AppState, now: Instant) {
    let until = Instant::now() + Duration::from_secs(5);
    while app.usage.jobs["work"].running() {
        assert!(Instant::now() < until, "fixture worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.take_usage_jobs_with(now, read_fixture));
}

fn credentials(folder: &Path) {
    std::fs::write(
        folder.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fixture-only","expiresAt":100}}"#,
    )
    .unwrap();
}

#[test]
fn refresh_reads_new_credentials_right_after_not_signed_in() {
    let (mut app, folder) = fixture("refresh");
    let now = Instant::now();
    app.request_usage_with(now, Demand::Automatic, Some("work"), read_fixture);
    finish(&mut app, now);
    assert_eq!(app.usage.attempt("work").unwrap().error(), Some(Error::NoFile));
    assert!(app.usage.can_refresh("work", now));

    credentials(&folder);
    app.request_usage_with(now, Demand::Refresh, Some("work"), read_fixture);
    finish(&mut app, now);
    assert_eq!(app.usage.reading("work").unwrap().limits[0].used, 10.0);
    assert!(app.usage.attempt("work").unwrap().error().is_none());
    assert!(app.usage.reading("main").is_none());
}

#[test]
fn sign_in_end_is_not_lost_when_a_pre_login_read_finishes_late() {
    let (mut app, folder) = fixture("sign-in");
    let now = Instant::now();
    let (release, wait) = std::sync::mpsc::channel();
    let old = Job::start(
        || {},
        move |_, _| {
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Err(Error::NotLoggedIn)
        },
    )
    .unwrap();
    app.usage.attempts.entry("work".into()).or_default().start(now);
    app.usage.jobs.insert("work".into(), old);

    credentials(&folder);
    app.request_usage_with(now, Demand::SignIn, Some("work"), read_fixture);
    release.send(()).unwrap();
    finish(&mut app, now);
    assert!(app.usage.attempt("work").unwrap().busy);
    finish(&mut app, now);
    assert_eq!(app.usage.reading("work").unwrap().limits[0].used, 10.0);
    assert!(app.usage.jobs.is_empty());
    assert!(app.usage.sign_in_pending.is_empty());
}

#[test]
fn the_fixture_folder_and_its_fake_credentials_go_when_the_test_ends() {
    let (app, folder) = fixture("cleanup");
    credentials(&folder);
    let path = folder.to_path_buf();
    assert!(path.join(".credentials.json").is_file());
    drop(app);
    drop(folder);
    assert!(!path.exists(), "nothing is left in %TEMP%");
}
