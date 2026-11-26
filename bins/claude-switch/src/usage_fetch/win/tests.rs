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
        let _ = fs::remove_file(self.0.join(".credentials.json"));
        let _ = fs::remove_dir(&self.0);
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
fn expired_or_disabled_credential_never_reaches_the_socket() {
    let dir = Fixture::new(now_ms() - 1);
    let sock = listener();
    let url = base(&sock);
    assert_eq!(loopback(&dir.0, &url, TIMEOUT), Err(Error::Expired));
    assert_eq!(production(&dir.0, || false), Err(Error::Network)); // Returns before opening WinHTTP or file.
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
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut header = [0; 5];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[..2], [22, 3]); // A TLS handshake record, not an HTTP GET.
        let mut hello = vec![0; usize::from(u16::from_be_bytes([header[3], header[4]]))];
        stream.read_exact(&mut hello).unwrap();
        assert!(!hello.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()));
        response(&mut stream, "200 OK", "", "{}");
    });
    assert_eq!(request(&dir.0, endpoint, Duration::from_secs(4), || true), Err(Error::Network));
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
