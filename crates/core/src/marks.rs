//! App-owned metadata. A damaged file is never replaced with empty defaults.

use crate::util::{io_refusal, json_bytes, lock_beside, write_atomic};
use crate::{Refusal, Roots};
use serde_json::{Value, json};
use std::path::Path;

const RETAIN_MS: u64 = 90 * 86_400_000;

#[derive(Clone, Debug)]
pub struct Marks {
    data: Value,
    original: Option<Vec<u8>>,
}

impl Default for Marks {
    fn default() -> Self {
        Self { data: json!({}), original: None }
    }
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, Refusal> {
    crate::util::regular_target(path)?;
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_refusal("read", path, &e)),
    }
}

fn path(roots: &Roots) -> std::path::PathBuf {
    roots.config_file().with_file_name("marks.json")
}

impl Marks {
    pub fn unchanged(&self, roots: &Roots) -> bool {
        read(&path(roots)).is_ok_and(|bytes| bytes == self.original)
    }

    pub fn load(roots: &Roots) -> Result<Self, Refusal> {
        let path = path(roots);
        let Some(bytes) = read(&path)? else { return Ok(Self::default()) };
        let data: Value = serde_json::from_slice(&bytes).map_err(|e| io_refusal("decode", &path, &e))?;
        if !data.is_object() || ["sessions", "folders"].iter().any(|k| data.get(k).is_some_and(|v| !v.is_object())) {
            return Err(Refusal::Io(format!("The structure of {} is not recognized.", path.display())));
        }
        Ok(Self { data, original: Some(bytes) })
    }

    pub fn save(&mut self, roots: &Roots) -> Result<(), Refusal> {
        let path = path(roots);
        let _lock = lock_beside(&path)?;
        if read(&path)? != self.original {
            return Err(Refusal::Io(format!("{} changed on disk. Restart before saving marks.", path.display())));
        }
        let written = json_bytes(&path, &self.data)?;
        write_atomic(&path, &written)?;
        self.original = Some(written);
        Ok(())
    }

    pub fn name(&self, id: &str) -> Option<&str> {
        self.data["sessions"][id]["name"].as_str().filter(|name| !name.is_empty())
    }

    pub fn handover_from(&self, id: &str) -> Option<&str> {
        self.data["sessions"][id]["handoverFrom"].as_str()
    }

    pub fn parent(&self, id: &str) -> Option<&str> {
        self.data["sessions"][id]["parentSessionId"]
            .as_str()
            .filter(|p| crate::launch::validate_session_id(p).is_ok() && !p.eq_ignore_ascii_case(id))
    }

    pub fn set_parent(&mut self, id: &str, parent: &str) -> bool {
        crate::launch::validate_session_id(id).is_ok()
            && crate::launch::validate_session_id(parent).is_ok()
            && !id.eq_ignore_ascii_case(parent)
            && self.parent(id).is_none()
            && self.set("sessions", id, "parentSessionId", json!(parent.to_ascii_lowercase()))
    }

    pub fn clear_parent(&mut self, id: &str) {
        self.clear_session_key(id, "parentSessionId");
    }

    pub fn topic(&self, id: &str) -> Option<&str> {
        crate::grouping::valid_topic(self.data["sessions"][id]["topic"].as_str()?)
    }

    pub fn set_topic(&mut self, id: &str, file: &Path, topic: &str) -> bool {
        if !topic.trim().is_empty() && crate::grouping::valid_topic(topic).is_none() {
            return false;
        }
        let topic = crate::grouping::valid_topic(topic).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "));
        self.session(id, file, "topic", json!(topic))
    }

    pub fn set_handover_from(&mut self, id: &str, name: &str) -> bool {
        self.set("sessions", id, "handoverFrom", json!(name))
    }

    pub fn clear_handover_from(&mut self, id: &str) {
        self.clear_session_key(id, "handoverFrom");
    }

    fn clear_session_key(&mut self, id: &str, key: &str) {
        if let Some(sessions) = self.data["sessions"].as_object_mut()
            && let Some(entry) = sessions.get_mut(id).and_then(Value::as_object_mut)
        {
            entry.remove(key);
            if entry.is_empty() {
                sessions.remove(id);
            }
        }
    }

    pub fn pinned(&self, id: &str) -> bool {
        self.data["sessions"][id]["pinned"] == true
    }

    pub fn hidden(&self, id: &str) -> bool {
        self.data["sessions"][id]["hidden"] == true
    }

    pub fn folder_hidden(&self, key: &str) -> bool {
        self.data["folders"][key]["hidden"] == true
    }

    pub fn archived_at(&self, id: &str) -> Option<u64> {
        self.data["sessions"][id]["archivedMs"].as_u64()
    }

    pub fn archive(&mut self, id: &str, file: &Path, at: Option<u64>) -> bool {
        if at.is_some() && (self.pinned(id) || self.archived_at(id).is_some()) {
            return false;
        }
        self.session(id, file, "archivedMs", json!(at))
    }

    pub fn clear_archive(&mut self) -> usize {
        let Some(entries) = self.data["sessions"].as_object_mut() else { return 0 };
        entries
            .values_mut()
            .filter_map(Value::as_object_mut)
            .map(|entry| {
                if entry.get("archivedMs").is_some_and(Value::is_u64) {
                    entry.remove("archivedMs");
                    1
                } else {
                    0
                }
            })
            .sum()
    }

    pub fn folder_default(&self, key: &str) -> Option<(&str, crate::App)> {
        crate::folder_defaults::decode(&self.data["folders"][key]["openDefault"])
    }

    pub fn set_folder_default(&mut self, key: &str, value: Option<(&str, crate::App)>) -> bool {
        !key.is_empty() && self.set("folders", key, "openDefault", crate::folder_defaults::encode(value))
    }

    pub fn unseen(&self, id: &str, modified: u64) -> bool {
        modified > self.data["sessions"][id]["seenMs"].as_u64().unwrap_or(0)
    }

    fn set(&mut self, group: &str, key: &str, field: &str, value: Value) -> bool {
        if self.data[group][key][field] == value || self.data[group].get(key).is_some_and(|v| !v.is_object()) {
            return false;
        }
        self.data[group][key][field] = value;
        true
    }

    fn session(&mut self, id: &str, file: &Path, field: &str, value: Value) -> bool {
        if !self.set("sessions", id, field, value) {
            return false;
        }
        if let Some(file) = file.to_str().filter(|_| file.is_absolute()) {
            self.set("sessions", id, "file", json!(file));
        }
        true
    }

    pub fn rename(&mut self, id: &str, file: &Path, name: &str) -> bool {
        self.session(id, file, "name", json!(name.trim()))
    }

    pub fn pin(&mut self, id: &str, file: &Path, pinned: bool) -> bool {
        self.session(id, file, "pinned", json!(pinned))
    }

    pub fn hide(&mut self, id: &str, file: &Path, hidden: bool) -> bool {
        self.session(id, file, "hidden", json!(hidden))
    }

    pub fn hide_folder(&mut self, key: &str, hidden: bool) -> bool {
        self.set("folders", key, "hidden", json!(hidden))
    }

    pub fn see(&mut self, id: &str, file: &Path, modified: u64) -> bool {
        self.unseen(id, modified) && self.session(id, file, "seenMs", json!(modified))
    }

    /// Only forget a known transcript after at least 90 days of observed absence.
    /// `found` includes unfiltered sessions; an inaccessible file isn't evidence of deletion.
    pub fn prune<'a>(&mut self, found: impl IntoIterator<Item = (&'a str, &'a Path)>, now_ms: u64) -> bool {
        let Some(marks) = self.data.get_mut("sessions").and_then(Value::as_object_mut) else { return false };
        let mut changed = false;
        let mut present = std::collections::HashSet::new();
        for (id, file) in found {
            let Some(entry) = marks.get_mut(id).and_then(Value::as_object_mut) else { continue };
            present.insert(id);
            if let Some(path) = file.to_str().filter(|_| file.is_absolute())
                && entry.get("file").and_then(Value::as_str) != Some(path)
            {
                entry.insert("file".into(), json!(path));
                changed = true;
            }
        }
        marks.retain(|id, entry| {
            let Some(entry) = entry.as_object_mut() else { return true };
            let Some(file) = entry.get("file").and_then(Value::as_str).map(Path::new) else { return true };
            let missing = !present.contains(id.as_str())
                && file.is_absolute()
                && std::fs::metadata(file).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            if !missing {
                changed |= entry.remove("missingSinceMs").is_some();
                return true;
            }
            if let Some(since) = entry.get("missingSinceMs").and_then(Value::as_u64) {
                let keep = now_ms.saturating_sub(since) < RETAIN_MS;
                changed |= !keep;
                keep
            } else {
                entry.insert("missingSinceMs".into(), json!(now_ms));
                changed = true;
                true
            }
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn marks_compare_and_replace_refuses_when_another_handle_holds_the_lock() {
        let t = crate::util::testing::TempDir::new("marks-locked");
        let roots = t.roots();
        let lock = lock_beside(&path(&roots)).unwrap();
        let mut marks = Marks::default();
        assert!(marks.save(&roots).is_err());
        assert!(!path(&roots).exists());
        drop(lock);
        marks.save(&roots).unwrap();
        assert_eq!(marks.original.as_deref(), Some(std::fs::read(path(&roots)).unwrap().as_slice()));
    }
    use super::*;
    use crate::util::testing::{TempDir, put};

    #[test]
    fn round_trip_preserves_unknowns_and_never_touches_transcripts() {
        let t = TempDir::new("marks");
        let roots = t.roots();
        let file = t.0.join("transcript.jsonl");
        put(&file, "private transcript");
        put(&path(&roots), r#"{"extra":42,"sessions":{"s":{"future":true},"broken":5}}"#);
        let mut m = Marks::load(&roots).unwrap();
        assert!(m.rename("s", &file, "  New name  "));
        assert!(m.pin("s", &file, true));
        assert!(m.hide("s", &file, true));
        assert!(m.hide_folder("folder", true));
        assert!(m.see("s", &file, 10));
        assert!(!m.rename("broken", &file, "don't overwrite"));
        m.save(&roots).unwrap();
        let mut m = Marks::load(&roots).unwrap();
        assert_eq!(m.name("s"), Some("New name"));
        assert!(m.pinned("s") && m.hidden("s") && m.folder_hidden("folder"));
        assert!(!m.unseen("s", 10) && m.unseen("s", 11));
        assert_eq!(m.data["extra"], 42);
        assert_eq!(m.data["sessions"]["s"]["future"], true);
        assert_eq!(m.data["sessions"]["broken"], 5);
        assert!(m.rename("s", &file, ""));
        assert_eq!(m.name("s"), None);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "private transcript");
    }

    #[test]
    fn damage_and_concurrent_replacement_do_not_get_overwritten() {
        let t = TempDir::new("marks-broken");
        let roots = t.roots();
        let mut m = Marks::load(&roots).unwrap();
        put(&path(&roots), r#"{"sessions":1,"folders":{"keep":true}}"#);
        assert!(Marks::load(&roots).is_err());
        assert!(m.save(&roots).is_err());
        assert_eq!(std::fs::read_to_string(path(&roots)).unwrap(), r#"{"sessions":1,"folders":{"keep":true}}"#);
    }

    #[test]
    fn a_missing_transcript_has_a_full_grace_period() {
        let t = TempDir::new("marks-expire");
        let file = t.0.join("transcript.jsonl");
        let mut m = Marks::default();
        m.pin("s", &file, true);
        assert!(m.prune([], 10));
        assert!(!m.prune([], 10 + RETAIN_MS - 1));
        put(&file, "returned");
        assert!(m.prune([], 10 + RETAIN_MS));
        assert!(m.pinned("s"));
        std::fs::remove_file(file).unwrap();
        assert!(m.prune([], 20 + RETAIN_MS));
        assert!(m.prune([], 20 + 2 * RETAIN_MS));
        assert!(!m.pinned("s"));
    }
}
