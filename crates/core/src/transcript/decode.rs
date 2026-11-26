use super::*;

pub(super) fn agent_line(info: &mut Subagent, bytes: &[u8]) {
    let Some((kind, line)) = message_line(bytes) else { return };
    if let Some(time) = line["timestamp"].as_str() {
        info.started_at.get_or_insert_with(|| time.into());
        info.last_activity_at = Some(time.into());
    }
    if kind == LineKind::Assistant {
        for (_, name, input) in blocks(&line["message"]["content"]).filter_map(tool_use) {
            info.tool_calls += 1;
            info.last_tool_summary = Some(tool_summary(name, input));
        }
    }
}

pub(super) enum Plan {
    Snapshot(Vec<Task>),
    Start,
    Advance,
    Done,
}

impl Plan {
    pub(super) fn parse(name: &str, input: &Value) -> Option<Self> {
        let key = match name {
            "TodoWrite" => "todos",
            "mcp__ccd_turn__update_plan" => "items",
            _ => return None,
        };
        if let Some(items) = input[key].as_array() {
            let tasks: Vec<_> = items
                .iter()
                .filter_map(|item| {
                    let status = match item["status"].as_str()? {
                        "pending" => TaskStatus::Pending,
                        "in_progress" => TaskStatus::InProgress,
                        "completed" => TaskStatus::Completed,
                        _ => return None,
                    };
                    Some(Task { content: item["content"].as_str()?.into(), status })
                })
                .collect();
            return (items.is_empty() || !tasks.is_empty()).then_some(Self::Snapshot(tasks));
        }
        if key == "items" {
            for (key, action) in [("start", Self::Start), ("advance", Self::Advance), ("done", Self::Done)] {
                if input[key].as_bool() == Some(true) || input["action"].as_str() == Some(key) {
                    return Some(action);
                }
            }
        }
        None
    }

    pub(super) fn apply(&self, tasks: &mut [Task]) {
        if matches!(self, Self::Advance | Self::Done)
            && let Some(active) = tasks.iter_mut().find(|task| task.status == TaskStatus::InProgress)
        {
            active.status = TaskStatus::Completed;
        }
        if matches!(self, Self::Start | Self::Advance)
            && !tasks.iter().any(|task| task.status == TaskStatus::InProgress)
            && let Some(next) = tasks.iter_mut().find(|task| task.status == TaskStatus::Pending)
        {
            next.status = TaskStatus::InProgress;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    User,
    Assistant,
    System,
}

pub(super) fn message_line(line: &[u8]) -> Option<(LineKind, Value)> {
    message_line_with_meta(line).filter(|(_, value)| !flag(value, "isMeta"))
}

pub(super) fn message_line_with_meta(line: &[u8]) -> Option<(LineKind, Value)> {
    let kind = line_kind(line)?;
    let value: Value = serde_json::from_slice(line).ok()?;
    // In case of duplicate JSON keys, do not expose content classified by a different type.
    let expected = match kind {
        LineKind::User => "user",
        LineKind::Assistant => "assistant",
        LineKind::System => "system",
    };
    (value["type"] == expected).then_some((kind, value))
}

/// Scan top-level keys without building a JSON tree or allocating for skipped values.
pub(crate) fn line_kind(mut bytes: &[u8]) -> Option<LineKind> {
    bytes = bytes.trim_ascii_start().strip_prefix(b"{")?;
    loop {
        bytes = bytes.trim_ascii_start();
        let key_end = string_end(bytes)?;
        let is_type = json_string_eq(&bytes[..key_end], "type");
        bytes = bytes[key_end..].trim_ascii_start().strip_prefix(b":")?.trim_ascii_start();
        if is_type {
            let value = &bytes[..string_end(bytes)?];
            return [("user", LineKind::User), ("assistant", LineKind::Assistant), ("system", LineKind::System)]
                .into_iter()
                .find_map(|(text, kind)| json_string_eq(value, text).then_some(kind));
        }
        let mut i = 0;
        let mut depth = 0usize;
        loop {
            match *bytes.get(i)? {
                b'"' => {
                    i += string_end(&bytes[i..])?;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth = depth.checked_sub(1)?,
                b',' if depth == 0 => break,
                _ => (),
            }
            i += 1;
        }
        bytes = &bytes[i + 1..];
    }
}

fn json_string_eq(bytes: &[u8], expected: &str) -> bool {
    let inner = &bytes[1..bytes.len() - 1];
    inner == expected.as_bytes()
        || (inner.contains(&b'\\') && serde_json::from_slice::<String>(bytes).is_ok_and(|s| s == expected))
}

fn string_end(bytes: &[u8]) -> Option<usize> {
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    loop {
        match *bytes.get(i)? {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
}

pub(super) fn image_source(block: &Value) -> Option<(&str, &str)> {
    let source = &block["source"];
    (block["type"] == "image" && source["type"] == "base64").then_some(())?;
    Some((source["media_type"].as_str()?, source["data"].as_str()?))
}

pub(super) fn invalid_picture(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn decode_base64(bytes: &[u8], limit: usize) -> io::Result<Vec<u8>> {
    let invalid = || invalid_picture("invalid base64");
    let digits = bytes.strip_suffix(b"==").or_else(|| bytes.strip_suffix(b"=")).unwrap_or(bytes);
    let tail = digits.len() % 4;
    let padding = bytes.len() - digits.len();
    if tail == 1 || (padding > 0 && (tail == 0 || tail + padding != 4)) {
        return Err(invalid());
    }
    let len = digits.len() / 4 * 3 + tail.saturating_sub(1);
    if len > limit {
        return Err(invalid_picture("picture exceeds 32 MiB"));
    }
    let mut out = Vec::with_capacity(len);
    let mut bits = 0u8;
    let mut acc = 0u16;
    for &digit in digits {
        let digit = match digit {
            b'A'..=b'Z' => digit - b'A',
            b'a'..=b'z' => digit - b'a' + 26,
            b'0'..=b'9' => digit - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(invalid()),
        };
        acc = (acc << 6) | u16::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if acc != 0 {
        return Err(invalid());
    }
    Ok(out)
}
