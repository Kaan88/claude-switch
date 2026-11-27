use super::*;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

const TOKEN: &str = "synthetic-loopback-token";

struct Fixture(PathBuf);

impl Fixture {
    fn new(expires: u64) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "claude-switch-usage-test-{}-{}-{}",
            std::process::id(),
            now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join(".credentials.json"),
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{TOKEN}","expiresAt":{expires}}}}}"#),
        )
        .unwrap();
        Self(dir)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn listener() -> TcpListener {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()
}

fn base(listener: &TcpListener) -> String {
    format!("http://{}", listener.local_addr().unwrap())
}

fn server(listener: TcpListener, respond: impl FnOnce(TcpStream) + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let (sock, _) = loop {
            match listener.accept() {
                Ok(conn) => break conn,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("test server accept: {e}"),
            }
        };
        sock.set_nonblocking(false).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut reader = BufReader::new(sock);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        respond(reader.into_inner());
        headers
    })
}

fn response(sock: &mut TcpStream, status: &str, extra: &str, body: &str) {
    write!(sock, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len())
        .unwrap();
}

#[test]
fn expiry_is_rechecked_inside_the_final_settings_permit_before_any_connection() {
    use std::cell::Cell;
    let fixture = Fixture::new(100);
    let roots = Roots { home: fixture.0.clone(), appdata: fixture.0.clone() };
    let dir = roots.primary_cli_dir();
    fs::create_dir(&dir).unwrap();
    fs::rename(fixture.0.join(".credentials.json"), dir.join(".credentials.json")).unwrap();
    claude_switch_core::config::save(&roots, &claude_switch_core::config::load(&roots)).unwrap();
    let socket = listener();
    socket.set_nonblocking(true).unwrap();
    let endpoint = Endpoint { host: "127.0.0.1", port: socket.local_addr().unwrap().port(), path: "test", flags: 0 };
    let count = Cell::new(0);
    let clock = || {
        count.set(count.get() + 1);
        if count.get() == 3 {
            assert!(claude_switch_core::config::usage_permit(&roots, &dir).is_none(), "final permit must be held");
            100
        } else {
            99
        }
    };
    assert_eq!(request(&dir, endpoint, TIMEOUT, || true, reading, Some(&roots), clock), Err(Error::Expired));
    assert_eq!(count.get(), 3);
    assert_eq!(socket.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    fs::remove_dir_all(roots.config_file().parent().unwrap()).unwrap();
    fs::remove_file(dir.join(".credentials.json")).unwrap();
    fs::remove_dir(dir).unwrap();
}

#[test]
fn request_context_is_dropped_once_on_success_pre_send_failure_and_pending_read_cancellation() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    assert_ne!(std::hint::black_box(CALLBACK_FLAGS) & WINHTTP_CALLBACK_STATUS_HANDLE_CREATED, 0);
    assert_ne!(std::hint::black_box(CALLBACK_FLAGS) & WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING, 0);
    let track_drop = || {
        let (tx, rx) = mpsc::channel();
        ON_DROP.with(|cell| *cell.borrow_mut() = Some(tx));
        rx
    };
    let verify = |rx: Receiver<()>| {
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Disconnected));
    };
    let good = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let url = base(&sock);
    let job = server(sock, |mut s| response(&mut s, "200 OK", "", "{}"));
    let drops = track_drop();
    assert!(loopback(&good.0, &url, TIMEOUT).is_ok());
    verify(drops);
    job.join().unwrap();

    let expired = Fixture::new(0);
    let sock = listener();
    let drops = track_drop();
    assert_eq!(loopback(&expired.0, &base(&sock), TIMEOUT), Err(Error::Expired));
    verify(drops);

    let sock = listener();
    let port = sock.local_addr().unwrap().port();
    let (hold, wait) = mpsc::channel::<()>();
    let job = server(sock, move |mut s| {
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n").unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(4));
    });
    let (tx, active) = mpsc::channel();
    ON_READ.with(|cell| *cell.borrow_mut() = Some(tx));
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    let stop = thread::spawn(move || {
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        signal.store(true, Ordering::SeqCst);
    });
    let drops = track_drop();
    let endpoint = Endpoint { host: "127.0.0.1", port, path: "api/oauth/usage", flags: 0 };
    assert_eq!(
        request(&good.0, endpoint, TIMEOUT, || !cancelled.load(Ordering::SeqCst), reading, None, now_ms),
        Err(Error::Network)
    );
    stop.join().unwrap();
    verify(drops);
    drop(hold);
    job.join().unwrap();
}

#[test]
fn request_headers_and_usage_are_verified_without_logging_the_token() {
    let dir = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let url = base(&sock);
    let thread = server(sock, |mut stream| {
        response(
            &mut stream,
            "200 OK",
            "",
            &format!(r#"{{"five_hour":{{"utilization":42}},"{TOKEN}":{{"utilization":100}}}}"#),
        );
    });
    let result = loopback(&dir.0, &url, TIMEOUT).unwrap();
    let headers = thread.join().unwrap();
    assert!(headers.starts_with("GET /api/oauth/usage HTTP/1.1\r\n"));
    assert!(headers.lines().any(|s| s.eq_ignore_ascii_case(&format!("User-Agent: {AGENT}"))));
    assert!(headers.lines().any(|s| s.eq_ignore_ascii_case("anthropic-beta: oauth-2025-04-20")));
    assert!(headers.lines().any(|s| s.eq_ignore_ascii_case("Accept: application/json")));
    assert!(
        headers
            .lines()
            .filter_map(|s| s.split_once(": "))
            .any(|(k, v)| k.eq_ignore_ascii_case("Authorization") && v == format!("Bearer {TOKEN}"))
    );
    assert_eq!(result.limits.len(), 1);
    assert_eq!(result.limits[0].used, 42.0);
    assert!(!format!("{result:?}").contains(TOKEN));
}

#[test]
fn one_shot_stdout_contains_only_the_nested_shape_from_loopback() {
    let dir = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let extra = sock.try_clone().unwrap();
    let url = base(&sock);
    let job = server(sock, |mut stream| {
        response(
            &mut stream,
            "200 OK",
            "X-Test-Secret: test-secret-header\r\n",
            r#"{"limits":[{"model":"sk-ant-secret-body","pct":87.12345,"enabled":true},null,["Bearer not-for-stdout"],[]],
             "\u0073ynthetic-loopback-token":{"other":"synthetic-loopback-token"}}"#,
        )
    });
    let mut cfg = crate::demo::config();
    cfg.accounts[0].cli_dir = dir.0.clone();
    let mut out = Vec::new();
    crate::usage_shape::probe(&cfg, Some("main"), &mut out, |dir| {
        loopback_with(dir, &url, TIMEOUT, |token, body| token.shape(body, false))
    })
    .unwrap();
    job.join().unwrap();
    extra.set_nonblocking(true).unwrap();
    assert_eq!(extra.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    let stdout = String::from_utf8(out).unwrap();
    for secret in [TOKEN, "sk-ant-secret-body", "87.12345", "true", "Bearer", "not-for-stdout", "test-secret-header"] {
        assert!(!stdout.contains(secret));
    }
    let actual: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "type": "object",
            "members": [
                {"name": "limits", "shape": {"type": "array", "length": 4, "items": [
                    {"type": "object", "members": [
                        {"name": "enabled", "shape": {"type": "bool"}},
                        {"name": "model", "shape": {"type": "string"}},
                        {"name": "pct", "shape": {"type": "number"}}
                    ]},
                    {"type": "null"},
                    {"type": "array", "length": 1, "items": [{"type": "string"}]},
                    {"type": "array", "length": 0, "items": []}
                ]}},
                {"name": "<redacted>", "shape": {"type": "object", "members": [
                    {"name": "other", "shape": {"type": "string"}}
                ]}}
            ]
        })
    );
}

#[test]
fn labelled_diagnostic_only_exposes_allowlisted_short_strings() {
    let dir = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let url = base(&sock);
    let job = server(sock, |mut stream| {
        response(
            &mut stream,
            "200 OK",
            "",
            r#"{
        "kind":"weekly","group":"plan_v1","severity":"good-ok","key":"model.x","display_name":"Model X 4.0",
        "secret":"not printed", "rows":[{"display_name":"contains/slash","key":"synthetic-loopback-token",
        "kind":"12345678901234567890123456789012345678901","severity":42,"group":"line\nbreak"}]
    }"#,
        )
    });
    let tree = loopback_with(&dir.0, &url, TIMEOUT, |token, body| token.shape(body, true)).unwrap();
    job.join().unwrap();
    let printed = tree.to_string();
    for allowed in ["weekly", "plan_v1", "good-ok", "model.x", "Model X 4.0"] {
        assert!(printed.contains(allowed));
    }
    for denied in [TOKEN, "not printed", "contains/slash", "12345678901234567890123456789012345678901", "42", "break"] {
        assert!(!printed.contains(denied));
    }
}

#[test]
fn expired_or_disabled_credential_never_reaches_the_socket() {
    let dir = Fixture::new(now_ms() - 1);
    let sock = listener();
    let url = base(&sock);
    assert_eq!(loopback(&dir.0, &url, TIMEOUT), Err(Error::Expired));
    let roots = Roots { home: dir.0.clone(), appdata: dir.0.clone() };
    assert_eq!(production(&roots, &dir.0, || false, reading), Err(Error::Network));
    sock.set_nonblocking(true).unwrap();
    assert_eq!(sock.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn redirects_do_not_reach_another_listener_and_http_errors_have_no_body() {
    let dir = Fixture::new(now_ms() + 60_000);
    let redirect = listener();
    redirect.set_nonblocking(true).unwrap();
    let target = base(&redirect);
    for (status, code) in [("302 Found", 302), ("429 Too Many Requests", 429)] {
        let sock = listener();
        let url = base(&sock);
        let extra = format!("Location: {target}/leak\r\n");
        let job = server(sock, move |mut stream| response(&mut stream, status, &extra, TOKEN));
        let err = loopback(&dir.0, &url, TIMEOUT).unwrap_err();
        assert_eq!(err, Error::Http(code));
        assert!(!format!("{err} {err:?}").contains(TOKEN));
        job.join().unwrap();
        assert_eq!(redirect.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
}

#[test]
fn production_requires_tls_and_plaintext_never_receives_the_bearer() {
    let prod = production_endpoint().unwrap();
    assert_eq!(
        (prod.host, prod.port, prod.path, prod.flags),
        ("api.anthropic.com", 443, "api/oauth/usage", WINHTTP_FLAG_SECURE)
    );
    let dir = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let endpoint =
        Endpoint { host: "127.0.0.1", port: sock.local_addr().unwrap().port(), path: prod.path, flags: prod.flags };
    let server = thread::spawn(move || {
        sock.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match sock.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("test TLS accept: {e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut header = [0; 5];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[..2], [22, 3]); // A TLS handshake record, not an HTTP GET.
        let mut hello = vec![0; usize::from(u16::from_be_bytes([header[3], header[4]]))];
        stream.read_exact(&mut hello).unwrap();
        assert!(!hello.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()));
        response(&mut stream, "200 OK", "", "{}");
    });
    assert_eq!(request(&dir.0, endpoint, Duration::from_secs(4), || true, reading, None, now_ms), Err(Error::Network));
    server.join().unwrap();
}

#[test]
fn a_stalled_response_is_cancelled_at_the_absolute_deadline() {
    let dir = Fixture::new(now_ms() + 60_000);
    let sock = listener();
    let url = base(&sock);
    let job = server(sock, |mut stream| {
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\n{").unwrap();
        thread::sleep(Duration::from_millis(1200));
    });
    let started = Instant::now();
    assert_eq!(loopback(&dir.0, &url, Duration::from_millis(400)), Err(Error::Network));
    assert!((Duration::from_millis(400)..Duration::from_millis(1050)).contains(&started.elapsed()));
    job.join().unwrap();
}
