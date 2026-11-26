//! Subscription usage: request-local credential, sanitized figures, and demand-only scheduling.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoFile,
    NotLoggedIn,
    Expired,
    Network,
    SettingsBusy,
    SettingsUnavailable,
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
            Self::SettingsBusy => f.write_str("settings busy; try again"),
            Self::SettingsUnavailable => f.write_str("usage settings unavailable or off"),
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
    pub fn reading(&self, body: &[u8], read_at: u64) -> Result<Reading, Error> {
        decode(body, self.value(), read_at)
    }

    /// A diagnostic tree, not the response: only member names, types and array lengths.
    pub fn shape(&self, body: &[u8], labels: bool) -> Result<Value, Error> {
        let raw = serde_json::from_slice(body).map_err(|_| Error::NotUnderstood)?;
        Ok(shape(raw, self.value(), labels))
    }
}

fn shape(raw: Value, bearer: &str, labels: bool) -> Value {
    fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
        Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
    }
    match raw {
        Value::Object(raw) => {
            let members = raw.into_iter().map(|(name, value)| {
                let expose = labels && matches!(name.as_str(), "kind" | "group" | "severity" | "key" | "display_name");
                let name = if name.contains(bearer) { "<redacted>".into() } else { name };
                let shown = value.as_str().filter(|s| {
                    expose
                        && !s.contains(bearer)
                        && s.len() <= 40
                        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b" .-_".contains(&b))
                });
                let detail = if let Some(label) = shown {
                    object([("type", "string".into()), ("value", label.into())])
                } else {
                    shape(value, bearer, labels)
                };
                object([("name", Value::String(name)), ("shape", detail)])
            });
            object([("type", "object".into()), ("members", Value::Array(members.collect()))])
        }
        Value::Array(raw) => {
            let len = raw.len();
            let items = raw.into_iter().map(|value| shape(value, bearer, labels)).collect();
            object([("type", "array".into()), ("length", json!(len)), ("items", Value::Array(items))])
        }
        Value::Number(_) => json!({"type": "number"}),
        Value::String(_) => json!({"type": "string"}),
        Value::Bool(_) => json!({"type": "bool"}),
        Value::Null => json!({"type": "null"}),
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
    let parsed =
        if read.is_ok() && bytes.len() as u64 <= MAX { parse_token(&bytes, now) } else { Err(Error::NotUnderstood) };
    bytes.fill(0);
    std::hint::black_box(&bytes);
    parsed
}

fn parse_token(bytes: &[u8], now: u64) -> Result<Token, Error> {
    // Values outside these fields remain borrowed JSON spans, not decoded heap strings.
    type Fields<'a> = BTreeMap<String, &'a serde_json::value::RawValue>;
    let fields: Fields<'_> = serde_json::from_slice(bytes).map_err(|_| Error::NotUnderstood)?;
    let oauth: Fields<'_> = serde_json::from_str(fields.get("claudeAiOauth").ok_or(Error::NotLoggedIn)?.get())
        .map_err(|_| Error::NotUnderstood)?;
    let expires_at = serde_json::from_str(oauth.get("expiresAt").ok_or(Error::NotUnderstood)?.get())
        .map_err(|_| Error::NotUnderstood)?;
    let value: String = serde_json::from_str(oauth.get("accessToken").ok_or(Error::NotLoggedIn)?.get())
        .map_err(|_| Error::NotLoggedIn)?;
    let token = Token { value: value.into_bytes(), expires_at };
    if token.value().is_empty() {
        return Err(Error::NotLoggedIn);
    }
    // RFC 6750 bearer alphabet. No control characters or non-ASCII can reach the header API.
    let base = token.value().trim_end_matches('=');
    if base.is_empty() || !base.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b)) {
        return Err(Error::NotUnderstood);
    }
    token.bearer(now)?;
    Ok(token)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    Session,
    Week,
    Model,
    Other,
}

impl Slot {
    pub fn id(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Week => "week",
            Self::Model => "model",
            Self::Other => "other",
        }
    }

    fn from_id(id: Option<&str>) -> Self {
        match id {
            Some("session") => Self::Session,
            Some("week") => Self::Week,
            Some("model") => Self::Model,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub label: String,
    pub slot: Slot,
    pub used: f64,
    /// Reset as UNIX seconds, never arbitrary response text.
    pub reset: Option<i64>,
    pub severity: String,
}

impl Limit {
    pub fn legacy(key: &str, used: f64, reset: Option<i64>) -> Self {
        let (slot, label) = match key {
            "five_hour" => (Slot::Session, "Session".into()),
            "seven_day" => (Slot::Week, "Week".into()),
            key => match key.strip_prefix("seven_day_").filter(|s| !s.is_empty()) {
                Some(model) => (Slot::Model, format!("Week, {}", humanise(model))),
                None => (Slot::Other, key.replace('_', " ")),
            },
        };
        Self { slot, label, used, reset, severity: String::new() }
    }

    pub fn warning(&self) -> bool {
        self.used >= 80.0
            || !matches!(self.severity.trim().to_ascii_lowercase().as_str(), "" | "normal" | "ok" | "none")
    }
}

fn humanise(name: &str) -> String {
    let name = name.replace('_', " ");
    let mut chars = name.chars();
    chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect()
}

fn report_text<'a>(value: &'a Value, bearer: &str) -> Option<&'a str> {
    let text = value.as_str()?;
    valid_label(text, bearer).then(|| text.trim())
}

fn valid_label(text: &str, bearer: &str) -> bool {
    !text.trim().is_empty() && text.len() <= 256 && !text.contains(bearer) && !text.chars().any(char::is_control)
}

fn modern_limit(item: &Value, bearer: &str) -> Option<Limit> {
    // `is_active` is not a filter: the real answer marks only the limit that binds right now, and
    // the others (session, week) are still limits to show.
    let used = percent(&item["percent"])?;
    let kind = report_text(&item["kind"], bearer).or_else(|| report_text(&item["group"], bearer)).unwrap_or("Limit");
    let lower = kind.to_ascii_lowercase();
    let group = report_text(&item["group"], bearer).unwrap_or("").to_ascii_lowercase();
    let (slot, label) = if let Some(model) = report_text(&item["scope"]["model"]["display_name"], bearer) {
        let weekly = [&lower, &group].into_iter().any(|name| name.contains("week") || name.contains("seven"));
        let prefix = if weekly { "Week".into() } else { humanise(kind) };
        (Slot::Model, format!("{prefix}, {model}"))
    } else if [&lower, &group].into_iter().any(|name| {
        let name = name.replace(['-', ' '], "_");
        name.contains("five_hour") || name.contains("session")
    }) {
        (Slot::Session, "Session".into())
    } else if lower.contains("seven_day") || lower.contains("week") {
        (Slot::Week, "Week".into())
    } else {
        (Slot::Other, humanise(kind))
    };
    if !valid_label(&label, bearer) {
        return None;
    }
    let reset = item["resets_at"].as_str().and_then(timestamp);
    let severity = report_text(&item["severity"], bearer).unwrap_or("").into();
    Some(Limit { label, slot, used, reset, severity })
}

fn decode(body: &[u8], bearer: &str, read_at: u64) -> Result<Reading, Error> {
    let raw: Value = serde_json::from_slice(body).map_err(|_| Error::NotUnderstood)?;
    let Some(object) = raw.as_object() else {
        return Err(Error::NotUnderstood);
    };
    let mut limits: Vec<_> = if let Some(items) = raw["limits"].as_array().filter(|items| !items.is_empty()) {
        let mut limits: Vec<_> = items.iter().filter_map(|item| modern_limit(item, bearer)).collect();
        // Some responses still carry the five-hour window only in the legacy field.
        if !limits.iter().any(|limit| limit.slot == Slot::Session)
            && let Some(used) = percent(&raw["five_hour"]["utilization"])
        {
            let reset = raw["five_hour"]["resets_at"].as_str().and_then(timestamp);
            let limit = Limit::legacy("five_hour", used, reset);
            if valid_label(&limit.label, bearer) {
                limits.push(limit);
            }
        }
        limits
    } else {
        object
            .iter()
            .filter_map(|(key, member)| {
                let reset = member["resets_at"].as_str().and_then(timestamp);
                let limit = Limit::legacy(key, percent(&member["utilization"])?, reset);
                (valid_label(key, bearer) && valid_label(&limit.label, bearer)).then_some(limit)
            })
            .collect()
    };
    limits.sort_by(|a, b| (a.slot, &a.label).cmp(&(b.slot, &b.label)));
    let breakdown = breakdown(&raw["seven_day_breakdown"]["rows"], Some(bearer), "display_name", "percent_of_limit");
    Ok(Reading { read_at, limits, breakdown })
}

fn breakdown(rows: &Value, bearer: Option<&str>, name: &str, measure: &str) -> Vec<Breakdown> {
    let mut rows: Vec<_> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let label =
                if let Some(bearer) = bearer { report_text(&row[name], bearer)? } else { row[name].as_str()? };
            let percent = row[measure].as_f64().filter(|p| p.is_finite() && *p >= 0.0);
            Some(Breakdown { label: label.into(), percent })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.percent.unwrap_or(-1.0).total_cmp(&a.percent.unwrap_or(-1.0)).then_with(|| a.label.cmp(&b.label))
    });
    rows.truncate(6);
    rows
}

fn percent(value: &Value) -> Option<f64> {
    let n = value.as_number()?;
    // Large exponents must not make valid JSON (including unknown members) fail wholesale.
    n.as_f64().or_else(|| n.to_string().parse::<f64>().ok()).map(|n| n.clamp(0.0, 100.0))
}

/// Parse an extended ISO timestamp. Fractions are not displayed.
pub(crate) fn timestamp(text: &str) -> Option<i64> {
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
    pub breakdown: Vec<Breakdown>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Breakdown {
    pub label: String,
    pub percent: Option<f64>,
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
            let reset = |limit: &Value| limit["reset"].as_i64().filter(|r| (0..253_402_300_800).contains(r));
            let mut limits: Vec<_> = if let Some(limits) = record["limits"].as_array() {
                limits
                    .iter()
                    .filter_map(|limit| {
                        Some(Limit {
                            label: limit["label"].as_str()?.into(),
                            slot: Slot::from_id(limit["slot"].as_str()),
                            used: percent(&limit["used"])?,
                            reset: reset(limit),
                            severity: limit["severity"].as_str().unwrap_or("").into(),
                        })
                    })
                    .collect()
            } else {
                record["limits"]
                    .as_object()?
                    .iter()
                    .filter_map(|(key, limit)| Some(Limit::legacy(key, percent(&limit["used"])?, reset(limit))))
                    .collect()
            };
            limits.sort_by(|a, b| (a.slot, &a.label).cmp(&(b.slot, &b.label)));
            let breakdown = breakdown(&record["breakdown"], None, "label", "percent");
            Some((slug, Reading { read_at, limits, breakdown }))
        })
        .collect()
}

pub fn save_cache(path: &Path, readings: &Cache) -> std::io::Result<()> {
    let _lock = crate::util::lock_beside(path).map_err(std::io::Error::other)?;
    crate::util::regular_target(path).map_err(std::io::Error::other)?;
    let mut merged = load_cache(path);
    for (slug, reading) in readings {
        if merged.get(slug).is_none_or(|prior| prior.read_at < reading.read_at) {
            merged.insert(slug.clone(), reading.clone());
        }
    }
    let accounts: serde_json::Map<String, Value> = merged
        .iter()
        .map(|(slug, reading)| {
            let limits: Vec<Value> = reading
                .limits
                .iter()
                .map(|l| json!({"label": l.label, "slot": l.slot.id(), "used": l.used, "reset": l.reset, "severity": l.severity}))
                .collect();
            let breakdown: Vec<_> = reading.breakdown.iter().map(|r| json!({"label": r.label, "percent": r.percent})).collect();
            (slug.clone(), json!({"read_at": reading.read_at, "limits": limits, "breakdown": breakdown}))
        })
        .collect();
    let mut bytes = serde_json::to_vec(&accounts).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    crate::util::write_atomic(path, &bytes).map_err(std::io::Error::other)
}

#[derive(Clone, Copy)]
pub enum Demand {
    Automatic,
    Refresh,
    SignIn,
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
        if self.busy {
            return false;
        }
        // These errors occur before HTTP submission. An explicit Refresh or a completed local
        // sign-in may have new credentials now, so the previous attempt is not a rate limit.
        if !auto && matches!(self.error(), Some(Error::NoFile | Error::NotLoggedIn | Error::Expired)) {
            return true;
        }
        if (auto || self.started.is_none()) && cached_age.is_some_and(|age| age < floor) {
            return false;
        }
        if let Some((finished, Err(error))) = self.finished
            && !matches!(error, Error::SettingsBusy | Error::SettingsUnavailable)
        {
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
        if matches!(result, Err(Error::SettingsBusy | Error::SettingsUnavailable)) {
            self.started = None;
        }
        self.finished = Some((now, result));
    }

    pub fn error(&self) -> Option<Error> {
        self.finished.and_then(|(_, result)| result.err())
    }
}

#[cfg(test)]
mod tests;
