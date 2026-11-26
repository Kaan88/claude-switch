//! Subscription usage: request-local credential, sanitized figures, and demand-only scheduling.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoFile,
    NotLoggedIn,
    Expired,
    Network,
    Http(u16),
    NotUnderstood,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFile => f.write_str("no file"),
            Self::NotLoggedIn => f.write_str("not logged in"),
            Self::Expired => f.write_str("expired"),
            Self::Network => f.write_str("network"),
            Self::Http(status) => write!(f, "HTTP {status}"),
            Self::NotUnderstood => f.write_str("not understood"),
        }
    }
}

/// Deliberately not Clone, Display, or serializable.
pub struct Token {
    value: Vec<u8>,
    expires_at: u64,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<token>")
    }
}

impl Token {
    fn value(&self) -> &str {
        // Stored only from a validated ASCII bearer string.
        std::str::from_utf8(&self.value).unwrap_or_default()
    }

    /// The transport borrows this to make the Authorization header, and never returns it.
    pub fn bearer(&self, now: u64) -> Result<&str, Error> {
        if now >= self.expires_at { Err(Error::Expired) } else { Ok(self.value()) }
    }

    /// Do not retain even a credential reflected in the name of a response field.
    pub fn limits(&self, body: &[u8]) -> Result<Vec<Limit>, Error> {
        decode(body, self.value())
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        self.value.fill(0);
        std::hint::black_box(&self.value);
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn read_token(cli_dir: &Path, now: u64) -> Result<Token, Error> {
    const MAX: u64 = 1024 * 1024;
    let file = File::open(cli_dir.join(".credentials.json")).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::NoFile,
        _ => Error::NotUnderstood,
    })?;
    let mut bytes = Vec::new();
    let read = file.take(MAX + 1).read_to_end(&mut bytes);
    let parsed = if read.is_ok() && bytes.len() as u64 <= MAX { serde_json::from_slice(&bytes).ok() } else { None };
    bytes.fill(0);
    std::hint::black_box(&bytes);
    let mut value: Value = parsed.ok_or(Error::NotUnderstood)?;
    let oauth = value.get_mut("claudeAiOauth").ok_or(Error::NotLoggedIn)?;
    let expires_at = oauth["expiresAt"].as_u64().ok_or(Error::NotUnderstood)?;
    let Value::String(value) = oauth.get_mut("accessToken").map(Value::take).ok_or(Error::NotLoggedIn)? else {
        return Err(Error::NotLoggedIn);
    };
    if value.is_empty() {
        return Err(Error::NotLoggedIn);
    }
    // RFC 6750 bearer alphabet. No control characters or non-ASCII can reach the header API.
    let base = value.trim_end_matches('=');
    if base.is_empty() || !base.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b)) {
        return Err(Error::NotUnderstood);
    }
    let token = Token { value: value.into_bytes(), expires_at };
    token.bearer(now)?;
    Ok(token)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub key: String,
    pub used: f64,
    /// Reset as UNIX seconds, never arbitrary response text.
    pub reset: Option<i64>,
}

impl Limit {
    pub fn label(&self) -> String {
        match self.key.as_str() {
            "five_hour" => "Session".into(),
            "seven_day" => "Week".into(),
            key => {
                if let Some(model) = key.strip_prefix("seven_day_").filter(|s| !s.is_empty()) {
                    let name = model.replace('_', " ");
                    let mut chars = name.chars();
                    let first = chars.next().into_iter().flat_map(char::to_uppercase);
                    format!("Week, {}", first.chain(chars).collect::<String>())
                } else {
                    key.replace('_', " ")
                }
            }
        }
    }

    pub fn model(&self) -> bool {
        self.key.strip_prefix("seven_day_").is_some_and(|s| !s.is_empty())
    }

    fn rank(&self) -> u8 {
        match self.key.as_str() {
            "five_hour" => 0,
            "seven_day" => 1,
            _ if self.model() => 2,
            _ => 3,
        }
    }
}

fn sort(limits: &mut [Limit]) {
    limits.sort_by(|a, b| (a.rank(), &a.key).cmp(&(b.rank(), &b.key)));
}

fn decode(body: &[u8], token: &str) -> Result<Vec<Limit>, Error> {
    let Value::Object(object) = serde_json::from_slice(body).map_err(|_| Error::NotUnderstood)? else {
        return Err(Error::NotUnderstood);
    };
    let mut limits = Vec::new();
    for (key, member) in object {
        let Some(used) = member.get("utilization").and_then(percent) else { continue };
        let reset = member.get("resets_at").and_then(Value::as_str).and_then(timestamp);
        let limit = Limit { key, used, reset };
        if !limit.key.contains(token) && !limit.label().contains(token) {
            limits.push(limit);
        }
    }
    sort(&mut limits);
    Ok(limits)
}

fn percent(value: &Value) -> Option<f64> {
    let n = value.as_number()?;
    // Large exponents must not make valid JSON (including unknown members) fail wholesale.
    n.as_f64().or_else(|| n.to_string().parse::<f64>().ok()).map(|n| n.clamp(0.0, 100.0))
}

/// Parse the extended ISO timestamp returned by the service. Fractions are not displayed.
fn timestamp(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(['T', 't'])?;
    let [y, m, d] = <[&str; 3]>::try_from(date.split('-').collect::<Vec<_>>()).ok()?;
    let digits =
        |s: &str, n| (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<i64>().ok()).flatten();
    let (year, month, day) = (digits(y, 4)?, digits(m, 2)?, digits(d, 2)?);
    let leap = |y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = [31, if leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mi = usize::try_from(month - 1).ok()?;
    if year == 0 || !(1..=*months.get(mi)?).contains(&day) {
        return None;
    }
    let before = |y: i64| 365 * y + y / 4 - y / 100 + y / 400;
    let days = before(year - 1) - before(1969) + months[..mi].iter().sum::<i64>() + day - 1;
    let zone_i = time.find(['Z', 'z', '+', '-'])?;
    let (clock, zone) = time.split_at(zone_i);
    let offset = match zone {
        "Z" | "z" => 0,
        _ => {
            let (h, m) = zone.get(1..)?.split_once(':')?;
            let (h, m) = (digits(h, 2)?, digits(m, 2)?);
            if h > 23 || m > 59 {
                return None;
            }
            (h * 3600 + m * 60) * if zone.starts_with('-') { -1 } else { 1 }
        }
    };
    let (clock, fraction) = clock.split_once(['.', ',']).unwrap_or((clock, "0"));
    if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let [h, m, s] = <[&str; 3]>::try_from(clock.split(':').collect::<Vec<_>>()).ok()?;
    let (h, m, s) = (digits(h, 2)?, digits(m, 2)?, digits(s, 2)?);
    (h < 24 && m < 60 && s < 60).then_some(days * 86_400 + h * 3600 + m * 60 + s - offset)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub read_at: u64,
    pub limits: Vec<Limit>,
}

pub const CACHE_NAME: &str = "usage.json";
pub type Cache = BTreeMap<String, Reading>;

pub fn load_cache(path: &Path) -> Cache {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() <= 8 * 1024 * 1024) {
        return Cache::new();
    }
    let Some(Value::Object(accounts)) = crate::util::read_json(path) else { return Cache::new() };
    accounts
        .into_iter()
        .filter_map(|(slug, record)| {
            let read_at = record["read_at"].as_u64()?;
            let mut limits: Vec<_> = record["limits"]
                .as_object()?
                .iter()
                .filter_map(|(key, limit)| {
                    let used = percent(&limit["used"])?;
                    let reset = limit["reset"].as_i64().filter(|r| (0..253_402_300_800).contains(r));
                    Some(Limit { key: key.clone(), used, reset })
                })
                .collect();
            sort(&mut limits);
            Some((slug, Reading { read_at, limits }))
        })
        .collect()
}

pub fn save_cache(path: &Path, readings: &Cache) -> std::io::Result<()> {
    let accounts: serde_json::Map<String, Value> = readings
        .iter()
        .map(|(slug, reading)| {
            let limits: serde_json::Map<String, Value> = reading
                .limits
                .iter()
                .map(|limit| (limit.key.clone(), json!({"used": limit.used, "reset": limit.reset})))
                .collect();
            (slug.clone(), json!({"read_at": reading.read_at, "limits": limits}))
        })
        .collect();
    // No symlink resolution of the destination; never write through an existing file or link.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let temp = Path::new(&name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = File::create_new(temp)?;
    let written = serde_json::to_writer(&mut file, &accounts)
        .map_err(std::io::Error::other)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    drop(file);
    written.and_then(|()| fs::rename(temp, path)).inspect_err(|_| {
        let _ = fs::remove_file(temp);
    })
}

#[derive(Clone, Copy)]
pub enum Demand {
    Automatic,
    Refresh,
}

#[derive(Default)]
pub struct Attempt {
    started: Option<Instant>,
    finished: Option<(Instant, Result<(), Error>)>,
    pub busy: bool,
}

impl Attempt {
    pub fn allowed(&self, demand: Demand, now: Instant, cached_age: Option<Duration>) -> bool {
        let auto = matches!(demand, Demand::Automatic);
        let floor = Duration::from_secs(if auto { 300 } else { 30 });
        if self.busy || ((auto || self.started.is_none()) && cached_age.is_some_and(|age| age < floor)) {
            return false;
        }
        if let Some((finished, Err(error))) = self.finished {
            let seconds = if error == Error::Http(429) { 1800 } else { 300 };
            if now.saturating_duration_since(finished) < Duration::from_secs(seconds) {
                return false;
            }
        }
        self.started.is_none_or(|t| now.saturating_duration_since(t) >= floor)
    }

    pub fn start(&mut self, now: Instant) {
        self.started = Some(now);
        self.busy = true;
    }

    pub fn finish(&mut self, now: Instant, result: Result<(), Error>) {
        self.busy = false;
        self.finished = Some((now, result));
    }

    pub fn error(&self) -> Option<Error> {
        self.finished.and_then(|(_, result)| result.err())
    }
}

#[cfg(test)]
mod tests;
