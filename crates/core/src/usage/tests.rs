use super::*;
use crate::util::testing::{TempDir, put};

#[test]
fn credentials_are_read_fresh_and_never_changed_or_debugged() {
    let t = TempDir::new("usage-token");
    let dir = t.roots().primary_cli_dir();
    let p = dir.join(".credentials.json");
    assert_eq!(read_token(&dir, 0).unwrap_err(), Error::NoFile);
    put(&p, r#"{"other":123}"#);
    assert_eq!(read_token(&dir, 0).unwrap_err(), Error::NotLoggedIn);
    put(&p, r#"{"claudeAiOauth":{"accessToken":"secret\r\nbad","expiresAt":100}}"#);
    assert_eq!(read_token(&dir, 0).unwrap_err(), Error::NotUnderstood);
    let one = r#"{"claudeAiOauth":{"accessToken":"synthetic-first","expiresAt":100,"ignored":"anything"}}"#;
    put(&p, one);
    let token = read_token(&dir, 99).unwrap();
    assert_eq!(format!("{token:?}"), "<token>");
    assert_eq!(token.bearer(99), Ok("synthetic-first"));
    assert_eq!(token.bearer(100), Err(Error::Expired));
    assert_eq!(read_token(&dir, 100).unwrap_err(), Error::Expired);
    assert_eq!(fs::read_to_string(&p).unwrap(), one);
    let two = one.replace("first", "second");
    put(&p, &two);
    assert_eq!(read_token(&dir, 99).unwrap().bearer(99), Ok("synthetic-second"));
    assert_eq!(fs::read_to_string(&p).unwrap(), two);
    put(&p, r#"{"claudeAiOauth":{"accessToken":"must-not-leak","expiresAt":"invalid"}}"#);
    assert_eq!(read_token(&dir, 0).unwrap_err(), Error::NotUnderstood);
}

#[test]
fn credentials_extract_only_access_token_and_expiry_and_usage_labels_share_the_same_limits() {
    let token = parse_token(
        br#"{"private":{"tokens":["unrelated"]},"claudeAiOauth":{"refreshToken":"fake","accessToken":"tok\u0065n","expiresAt":100}}"#,
        0,
    ).unwrap();
    assert_eq!(token.bearer(99), Ok("token"));
    let legacy = json!({
        "seven_day": {"utilization": 12},
        "new\nlimit": {"utilization": 13},
        "seven_day_\tbad": {"utilization": 15},
        ("x".repeat(257)): {"utilization": 14},
    });
    let modern =
        json!({"limits":[{"kind":"seven_day_long","scope":{"model":{"display_name":"z".repeat(256)}},"percent":88}]});
    let reading = |v: Value| token.reading(v.to_string().as_bytes(), 0).unwrap().limits;
    assert_eq!(reading(legacy), [Limit::legacy("seven_day", 12.0, None)]);
    assert!(reading(modern).is_empty());
}

#[test]
fn local_settings_contention_does_not_back_off_manual_refresh() {
    let now = Instant::now();
    let mut attempt = Attempt::default();
    attempt.start(now);
    attempt.finish(now, Err(Error::SettingsBusy));
    assert!(attempt.allowed(Demand::Refresh, now, None));
    assert_eq!(attempt.error(), Some(Error::SettingsBusy));
}

#[test]
fn tolerant_limits_remain_sorted_and_only_normalized_data_survives() {
    let token = Token { value: b"synthetic-token".to_vec(), expires_at: 100 };
    let limits = token
        .reading(
            br#"{
        "z_future":{"utilization":1e400},
        "extra_usage":{"utilization":42.5,"resets_at":"garbage","extra":"synthetic-token"},
        "seven_day_sonnet":{"utilization":23,"resets_at":"1970-01-02T04:30:00.123+04:30"},
        "seven_day":{"utilization":79.9,"resets_at":null},
        "five_hour":{"utilization":-1,"resets_at":"2024-02-29T00:00:00Z"},
        "seven_day_opus":{"utilization":80},
        "bad":null,"other":[],"f":{"utilization":"12"},"g":{"foo":2},"large":1e990,
        "\u0073ynthetic-token":{"utilization":53}
     }"#,
            0,
        )
        .unwrap()
        .limits;
    assert_eq!(
        limits.iter().map(|l| l.label.as_str()).collect::<Vec<_>>(),
        ["Session", "Week", "Week, Opus", "Week, Sonnet", "extra usage", "z future"]
    );
    assert_eq!(limits.iter().map(|l| l.used).collect::<Vec<_>>(), [0.0, 79.9, 80.0, 23.0, 42.5, 100.0]);
    assert_eq!(limits[0].reset, Some(1_709_164_800));
    assert_eq!(limits[3].reset, Some(86_400));
    assert_eq!(limits[4].reset, None);
    assert_eq!(timestamp("2023-02-29T00:00:00Z"), None);
    assert_eq!(timestamp("1970-01-01T00:00:00-01:30"), Some(5400));
    assert_eq!(timestamp("1970-01-01T00:00:00.++01:30"), None);
    assert_eq!(token.reading(b"[]", 0), Err(Error::NotUnderstood));
    assert_eq!(token.reading(b"no", 0), Err(Error::NotUnderstood));
    assert!(token.reading(b"{}", 0).unwrap().limits.is_empty());
}

#[test]
fn cache_atomically_replaces_the_file_with_just_measurements() {
    let t = TempDir::new("usage-cache");
    let path = t.roots().config_file().with_file_name(CACHE_NAME);
    assert!(load_cache(&path).is_empty());
    let token = Token { value: b"synthetic-token".to_vec(), expires_at: 100 };
    let reading = token.reading(br#"{"five_hour":{"utilization":42.5,"resets_at":"1970-01-01T00:00:34Z","other":"synthetic-token"},"synthetic-token":{"utilization":99}}"#, 7).unwrap();
    let readings = Cache::from([("main".into(), reading)]);
    save_cache(&path, &Cache::new()).unwrap();
    put(&path, r#"{"main":{"read_at":7,"limits":{"five_hour":{"used":42.5,"reset":34}}}}"#);
    assert_eq!(load_cache(&path), readings);
    save_cache(&path, &readings).unwrap();
    assert_eq!(load_cache(&path), readings);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"main":{"read_at":7,"limits":[{"label":"Session","slot":"session","used":42.5,"reset":34,"severity":""}],"breakdown":[]}})
    );
    assert!(
        !fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "tmp"))
    );
}

#[test]
fn stale_windows_cannot_drop_another_accounts_reading_or_roll_back_a_newer_one() {
    let t = TempDir::new("usage-cache-merge");
    let path = t.roots().config_file().with_file_name(CACHE_NAME);
    let row = |read_at| Reading { read_at, limits: Vec::new(), breakdown: Vec::new() };
    save_cache(&path, &Cache::from([("main".into(), row(9))])).unwrap();
    save_cache(&path, &Cache::from([("main".into(), row(4)), ("work".into(), row(7))])).unwrap();
    assert_eq!(load_cache(&path), Cache::from([("main".into(), row(9)), ("work".into(), row(7))]));
}

#[test]
fn modern_limits_win_and_breakdown_and_severity_survive_the_cache() {
    let token = Token { value: b"synthetic-token".to_vec(), expires_at: 100 };
    let reading = token.reading(br#"{
        "five_hour":{"utilization":99},
        "limits":[
            {"kind":"main_session","percent":22,"severity":"normal","scope":null},
            {"kind":"rolling_week","percent":31,"severity":"ok","scope":null},
            {"group":"seven_day","kind":"quota","percent":44,"severity":"close","resets_at":"1970-01-02T00:00:00Z","scope":{"model":{"display_name":"Model X"}}},
            {"kind":"burst_limit","percent":19,"scope":{"model":{"display_name":"Model Y"}}},
            {"kind":"other_limit","percent":88,"severity":"none","is_active":false}
        ],
        "seven_day_breakdown":{"rows":[
            {"display_name":"First","percent":100,"percent_of_limit":10},
            {"display_name":"Second","percent":5,"percent_of_limit":30},
            {"display_name":"synthetic-token","percent_of_limit":90},
            {"display_name":"Three","percent_of_limit":3},{"display_name":"Four","percent_of_limit":4},
            {"display_name":"Five","percent_of_limit":5},{"display_name":"Six","percent_of_limit":6},
            {"display_name":"Seven","percent_of_limit":1}
        ]}
    }"#, 12).unwrap();
    assert_eq!(
        reading.limits.iter().map(|l| (l.label.as_str(), l.slot, l.used, l.warning())).collect::<Vec<_>>(),
        [
            ("Session", Slot::Session, 22.0, false),
            ("Week", Slot::Week, 31.0, false),
            ("Burst limit, Model Y", Slot::Model, 19.0, false),
            ("Week, Model X", Slot::Model, 44.0, true),
            ("Other limit", Slot::Other, 88.0, true),
        ]
    );
    assert_eq!((reading.limits[3].reset, reading.limits[3].severity.as_str()), (Some(86400), "close"));
    assert_eq!(reading.breakdown.iter().map(|r| r.percent).collect::<Vec<_>>(), [30.0, 10.0, 6.0, 5.0, 4.0, 3.0]);
    let dir = TempDir::new("modern-usage-cache");
    let path = dir.roots().config_file().with_file_name(CACHE_NAME);
    let cache = Cache::from([("main".into(), reading)]);
    save_cache(&path, &cache).unwrap();
    assert_eq!(load_cache(&path), cache);
    assert!(!fs::read_to_string(&path).unwrap().contains("synthetic-token"));
}

#[test]
fn replacing_cache_does_not_write_through_a_hard_link() {
    let t = TempDir::new("usage-cache-link");
    let path = t.roots().config_file().with_file_name(CACHE_NAME);
    let other = path.with_file_name("generated-original");
    put(&other, "{}");
    fs::hard_link(&other, &path).unwrap();
    save_cache(&path, &Cache::new()).unwrap();
    assert_eq!(fs::read(&other).unwrap(), b"{}");
    assert_eq!(fs::read(&path).unwrap(), b"{}\n");
}

#[test]
fn on_demand_throttle_has_distinct_automatic_manual_and_failure_floors() {
    let now = Instant::now();
    let later = |s| now + Duration::from_secs(s);
    let mut a = Attempt::default();
    let auto = Demand::Automatic;
    let manual = Demand::Refresh;
    assert!(!a.allowed(auto, now, Some(Duration::from_secs(299))));
    assert!(a.allowed(auto, now, Some(Duration::from_secs(300))));
    assert!(!a.allowed(manual, now, Some(Duration::from_secs(29))));
    assert!(a.allowed(manual, now, Some(Duration::from_secs(30))));
    a.start(now);
    assert!(!a.allowed(manual, later(2000), None));
    a.finish(later(1), Ok(()));
    assert!(!a.allowed(manual, later(29), None));
    assert!(a.allowed(manual, later(30), Some(Duration::ZERO)));
    assert!(!a.allowed(auto, later(299), None));
    assert!(a.allowed(auto, later(300), None));
    for (failure, seconds) in [(Error::Expired, 300), (Error::Network, 300), (Error::Http(429), 1800)] {
        let mut b = Attempt::default();
        b.start(now);
        b.finish(later(10), Err(failure));
        for demand in [auto, manual] {
            assert!(!b.allowed(demand, later(seconds + 9), None));
            assert!(b.allowed(demand, later(seconds + 10), None));
        }
    }
}
