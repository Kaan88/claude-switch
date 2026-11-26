use super::{CHUNK, contains, flag, parse, prompt_of, str_field};
use crate::grouping::Transcript;
use serde_json::Value;

pub(super) fn head(bytes: &[u8], id: &str) -> Transcript {
    let head = if bytes.len() > CHUNK as usize {
        let bytes = &bytes[..CHUNK as usize];
        &bytes[..bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)]
    } else {
        bytes
    };
    let mut result = Transcript::default();
    let mut parents = std::collections::HashSet::new();
    for (i, line) in head.split(|&b| b == b'\n').enumerate() {
        let ancestry = i < 32 && (contains(line, b"\"forkedFrom\"") || contains(line, b"\"parentSessionId\""));
        let activity = (result.first_active.is_none() || result.first_prompt.is_none())
            && (contains(line, b"\"user\"") || contains(line, b"\"assistant\""));
        if !ancestry && !activity {
            if i >= 32 && result.first_active.is_some() && result.first_prompt.is_some() {
                break;
            }
            continue;
        }
        let Some(v) = parse(line) else { continue };
        if flag(&v, "isSidechain") || flag(&v, "isMeta") || flag(&v, "isCompactSummary") {
            continue;
        }
        if ancestry && str_field(&v, "sessionId").is_none_or(|s| s.eq_ignore_ascii_case(id)) {
            for parent in [str_field(&v, "parentSessionId"), fork_parent(&v)].into_iter().flatten() {
                if crate::launch::validate_session_id(parent).is_ok() && !parent.eq_ignore_ascii_case(id) {
                    parents.insert(parent.to_ascii_lowercase());
                }
            }
        }
        if result.first_active.is_none() && matches!(str_field(&v, "type"), Some("user" | "assistant")) {
            result.first_active = str_field(&v, "timestamp")
                .and_then(crate::usage::timestamp)
                .and_then(|s| u64::try_from(s).ok()?.checked_mul(1000));
        }
        if result.first_prompt.is_none() {
            result.first_prompt = prompt_of(&v, 500);
        }
    }
    if parents.len() == 1 {
        result.parent = parents.into_iter().next();
    }
    result
}

fn fork_parent(v: &Value) -> Option<&str> {
    str_field(&v["forkedFrom"], "sessionId")
}

#[cfg(test)]
mod tests {
    use super::super::read_transcript;
    use crate::util::testing::{TempDir, put};
    use serde_json::json;

    #[test]
    fn reads_optional_session_ancestry_not_message_uuids_and_refuses_conflicting_parents() {
        let t = TempDir::new("thread-head");
        let id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let parent = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let file = t.0.join(format!("{id}.jsonl"));
        let prompt = json!({"type":"user","sessionId":id,"parentUuid":parent,"timestamp":"1970-01-01T00:00:01Z",
                           "message":{"content":"First actual prompt"}});
        put(&file, &prompt.to_string());
        assert_eq!(read_transcript(&file).unwrap().grouping.parent, None);
        put(&file, &format!("{}\n{prompt}", json!({"forkedFrom":{"sessionId":parent}})));
        let data = read_transcript(&file).unwrap().grouping;
        assert_eq!(data.parent.as_deref(), Some(parent));
        assert_eq!(data.first_active, Some(1000));
        assert_eq!(data.first_prompt.as_deref(), Some("First actual prompt"));
        put(&file, &format!("{}\n{prompt}", json!({"parentSessionId":parent})));
        assert_eq!(read_transcript(&file).unwrap().grouping.parent.as_deref(), Some(parent));
        put(
            &file,
            &format!(
                "{}\n{}\n{prompt}",
                json!({"parentSessionId":parent}),
                json!({"forkedFrom":{"sessionId":"cccccccc-cccc-cccc-cccc-cccccccccccc"}})
            ),
        );
        assert_eq!(read_transcript(&file).unwrap().grouping.parent, None);
    }
}
