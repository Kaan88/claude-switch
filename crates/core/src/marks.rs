//! App-owned metadata. A damaged file is never replaced with empty defaults.

use crate::util::{io_refusal, json_bytes, lock_beside, write_atomic};
use crate::{Refusal, Roots};
use serde_json::{Value, json};
use std::path::Path;

const RETAIN_MS: u64 = 90 * 86_400_000;

#[derive(Clone, Debug)]
pub struct Marks {
    data: Value,
    /// The file's bytes when last read or written, and what they said: the base this window's
    /// own changes are taken from when another window wrote in between.
    original: Option<Vec<u8>>,
    base: Value,
}

impl Default for Marks {
    fn default() -> Self {
        Self { data: json!({}), original: None, base: json!({}) }
    }
}

/// Sessions and folders, each a map of entries, each a map of fields.
const DEPTH: usize = 3;

fn decode(path: &Path, bytes: &[u8]) -> Result<Value, Refusal> {
    let mut data: Value = serde_json::from_slice(bytes).map_err(|e| io_refusal("decode", path, &e))?;
    if !data.is_object() || ["sessions", "folders"].iter().any(|k| data.get(k).is_some_and(|v| !v.is_object())) {
        return Err(Refusal::Io(format!("The structure of {} is not recognized.", path.display())));
    }
    // An earlier version's merge could leave `"id": null`, which refused every mark for that
    // entry; it holds nothing, so it is dropped (other unrecognized entries are kept as they are).
    for group in ["sessions", "folders"] {
        if let Some(map) = data.get_mut(group).and_then(Value::as_object_mut) {
            map.retain(|_, entry| !entry.is_null());
        }
    }
    Ok(data)
}

/// Fields each window keeps up by itself are no conflict: the later `seenMs`, the earlier
/// `missingSinceMs`, and the transcript `file` as on disk. True when `path` was one of them.
/// An entry the other window removed (pruned) stays removed: bookkeeping alone never brings it
/// back, and never leaves a null in its place; the next scan that finds it starts a new one.
fn bookkeeping(merged: &mut Value, ours: &Value, path: &[String]) -> bool {
    let [group, key, field] = path else { return false };
    if !matches!(field.as_str(), "file" | "seenMs" | "missingSinceMs") {
        return false;
    }
    let target = merged.get_mut(group.as_str()).and_then(|g| g.get_mut(key.as_str())).and_then(Value::as_object_mut);
    let (Some(target), "seenMs" | "missingSinceMs") = (target, field.as_str()) else { return true };
    let pick = |a: u64, b: u64| if field == "seenMs" { a.max(b) } else { a.min(b) };
    let mine = ours[group.as_str()][key.as_str()][field.as_str()].as_u64();
    let theirs = target.get(field.as_str()).and_then(Value::as_u64);
    match (mine, theirs) {
        (Some(a), Some(b)) => _ = target.insert(field.clone(), json!(pick(a, b))),
        (Some(a), None) if field == "seenMs" => _ = target.insert(field.clone(), json!(a)),
        _ => {}
    }
    true
}

/// "the name of session 1a2b3c4d", for a conflict notice.
fn describe(path: &[String]) -> String {
    let short = |key: &str| key.chars().take(8).collect::<String>();
    match path {
        [group, key, field, ..] => {
            let what = if group == "folders" { "folder" } else { "session" };
            let key = if group == "folders" { key.clone() } else { short(key) };
            format!("the {field} of {what} {key}")
        }
        [group, key] => format!("{group} entry {}", short(key)),
        other => other.join("/"),
    }
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, Refusal> {
    crate::util::regular_target(path)?;
    match crate::read_bounded(path, crate::APP_FILE_LIMIT) {
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
        ["sessions", "folders"].iter().all(|k| {
            self.data.get(k).is_none_or(|v| {
                v.as_object().is_some_and(|map| {
                    map.values().all(|entry| entry.is_object() && entry.get("hidden").is_none_or(Value::is_boolean))
                })
            })
        }) && read(&path(roots)).is_ok_and(|bytes| bytes == self.original)
    }

    pub fn load(roots: &Roots) -> Result<Self, Refusal> {
        let path = path(roots);
        let Some(bytes) = read(&path)? else { return Ok(Self::default()) };
        let data = decode(&path, &bytes)?;
        Ok(Self { base: data.clone(), data, original: Some(bytes) })
    }

    /// Writes this window's changes. When another window wrote in between, only this window's
    /// own changes (per entry and field) are applied onto the file as it is now, and this
    /// window then also has the other one's; a field both changed differently refuses the save
    /// and takes the other window's value.
    pub fn save(&mut self, roots: &Roots) -> Result<(), Refusal> {
        let path = path(roots);
        let _lock = lock_beside(&path)?;
        let disk = read(&path)?;
        if disk != self.original {
            let theirs = match &disk {
                Some(bytes) => decode(&path, bytes).map_err(|_| {
                    Refusal::Io(format!(
                        "Another window left {} damaged or unrecognized; nothing was saved.",
                        path.display()
                    ))
                })?,
                None => json!({}),
            };
            let merged = crate::merge::three_way(Some(&self.base), Some(&self.data), Some(&theirs), DEPTH);
            let mut value = merged.value.unwrap_or_else(|| json!({}));
            let conflicts: Vec<_> =
                merged.conflicts.into_iter().filter(|p| !bookkeeping(&mut value, &self.data, p)).collect();
            if let Some(first) = conflicts.first() {
                let more = conflicts.len() - 1;
                let more = if more > 0 { format!(" and {more} more") } else { String::new() };
                self.data = value;
                self.base = theirs;
                self.original = disk;
                return Err(Refusal::Io(format!(
                    "Another Claude Switch window changed {}{more} at the same time. That window's change is kept; nothing else was saved, so try again.",
                    describe(first)
                )));
            }
            self.data = value;
        }
        let written = json_bytes(&path, &self.data)?;
        write_atomic(&path, &written)?;
        self.original = Some(written);
        self.base = self.data.clone();
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

    /// The folder session `id` was moved to with "Continue in another folder". Claude Code names
    /// a new folder only in the records written there, and the list reads a session's folder
    /// from its first records, so the move is kept here.
    pub fn folder(&self, id: &str) -> Option<&str> {
        self.data["sessions"][id]["folder"].as_str().filter(|s| !s.is_empty())
    }

    pub fn set_folder(&mut self, id: &str, file: &Path, folder: &Path) -> bool {
        self.session(id, file, "folder", json!(folder.to_string_lossy()))
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

    /// The session a handover continued this one in, once it was handed over.
    pub fn handed_over_to(&self, id: &str) -> Option<&str> {
        self.data["sessions"][id]["handedOverTo"]
            .as_str()
            .filter(|to| crate::launch::validate_session_id(to).is_ok() && !to.eq_ignore_ascii_case(id))
    }

    pub fn set_handed_over_to(&mut self, id: &str, to: &str) -> bool {
        crate::launch::validate_session_id(id).is_ok()
            && crate::launch::validate_session_id(to).is_ok()
            && !id.eq_ignore_ascii_case(to)
            && self.set("sessions", id, "handedOverTo", json!(to.to_ascii_lowercase()))
    }

    pub fn clear_handed_over_to(&mut self, id: &str) {
        self.clear_session_key(id, "handedOverTo");
    }

    /// Names reserved for new handovers not yet visible in the index. Never overrides a native title.
    pub fn handover_launch_names(&self, now_ms: u64) -> impl Iterator<Item = (&str, &str)> {
        self.data["sessions"].as_object().into_iter().flatten().filter_map(move |(id, entry)| {
            let reservation = &entry["handoverLaunchName"];
            let at = reservation["at"].as_u64()?;
            (now_ms.saturating_sub(at) < 120_000).then_some((id.as_str(), reservation["name"].as_str()?))
        })
    }

    pub fn set_handover_launch_name(&mut self, id: &str, name: &str, now_ms: u64) -> bool {
        crate::launch::validate_session_id(id).is_ok()
            && self.data["sessions"][id].get("handoverLaunchName").is_none()
            && self.set("sessions", id, "handoverLaunchName", json!({"name": name, "at": now_ms}))
    }

    pub fn clear_handover_launch_name(&mut self, id: &str) {
        self.clear_session_key(id, "handoverLaunchName");
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

    /// Forgets session `id`: its own entry (name, topic, pin, hide, archive, seen time,
    /// handover links) goes. Other sessions' entries are kept, even those that name it. True
    /// when there was an entry.
    pub fn forget(&mut self, id: &str) -> bool {
        let Some(sessions) = self.data["sessions"].as_object_mut() else { return false };
        let before = sessions.len();
        sessions.retain(|key, _| !key.eq_ignore_ascii_case(id));
        sessions.len() != before
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
    fn unchanged_but_damaged_hidden_marks_do_not_authorize_batch_naming() {
        let t = TempDir::new("marks-privacy");
        let roots = t.roots();
        put(&path(&roots), r#"{"sessions":{"s":{"hidden":"yes"}}}"#);
        assert!(!Marks::load(&roots).unwrap().unchanged(&roots));
    }
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
    fn a_session_pruned_in_one_window_and_seen_in_the_other_never_becomes_null() {
        // The reviewer's case: one window prunes a session and saves; the other, loaded before,
        // sees it and saves. The merge wrote "a": null, which then refused every mark for it.
        let t = TempDir::new("marks-null");
        let roots = t.roots();
        let gone = t.0.join("gone.jsonl");
        let old = 1_000;
        put(&path(&roots), &json!({"sessions": {"a": {"file": gone, "seenMs": 1, "missingSinceMs": old}}}).to_string());
        let mut one = Marks::load(&roots).unwrap();
        let mut two = Marks::load(&roots).unwrap();
        assert!(one.prune([], old + RETAIN_MS));
        one.save(&roots).unwrap();
        assert!(two.see("a", &gone, 7));
        two.save(&roots).unwrap();
        let disk: Value = serde_json::from_slice(&std::fs::read(path(&roots)).unwrap()).unwrap();
        assert!(disk["sessions"].get("a").is_none_or(Value::is_object), "{disk}");
        for mut m in [two, Marks::load(&roots).unwrap()] {
            assert!(m.pin("a", &gone, true) && m.rename("a", &gone, "Back"), "marks work again");
            assert!(m.data["sessions"]["a"].is_object());
        }
    }

    #[test]
    fn a_null_entry_left_by_an_earlier_version_is_repaired_on_load() {
        let t = TempDir::new("marks-null-load");
        let roots = t.roots();
        let file = t.0.join("a.jsonl");
        put(&path(&roots), r#"{"sessions":{"a":null,"b":{"name":"B"},"odd":5},"folders":{"C:/w":null}}"#);
        let mut m = Marks::load(&roots).unwrap();
        assert!(m.pin("a", &file, true) && m.hide_folder("C:/w", true));
        assert_eq!((m.name("b"), &m.data["sessions"]["odd"]), (Some("B"), &json!(5)), "the rest is kept");
        m.save(&roots).unwrap();
        assert!(Marks::load(&roots).unwrap().pinned("a"));
    }

    #[test]
    fn two_windows_keep_each_others_changes_and_refuse_only_the_same_field() {
        let t = TempDir::new("marks-two");
        let roots = t.roots();
        let file = t.0.join("a.jsonl");
        let mut one = Marks::load(&roots).unwrap();
        let mut two = Marks::load(&roots).unwrap();
        one.rename("a", &file, "From one");
        one.save(&roots).unwrap();
        two.pin("a", &file, true);
        two.hide_folder("C:/w", true);
        two.see("a", &file, 5);
        two.save(&roots).expect("a different field of the same session saves");
        assert_eq!(two.name("a"), Some("From one"), "and this window now has the other's change");
        let back = Marks::load(&roots).unwrap();
        assert_eq!((back.name("a"), back.pinned("a"), back.folder_hidden("C:/w")), (Some("From one"), true, true));
        one.see("a", &file, 9);
        one.save(&roots).expect("seen times merge: the later one");
        assert!(!Marks::load(&roots).unwrap().unseen("a", 9));
        assert!(one.pinned("a"));
        two.rename("a", &file, "From two");
        one.rename("a", &file, "Also one");
        one.save(&roots).unwrap();
        let e = two.save(&roots).expect_err("both renamed the same session").to_string();
        assert!(e.contains("the name of session a") && !e.contains("Restart"), "{e}");
        assert_eq!(two.name("a"), Some("Also one"), "the other window's name is kept");
        assert_eq!(Marks::load(&roots).unwrap().name("a"), Some("Also one"));
        two.hide("a", &file, true);
        two.save(&roots).expect("after a conflict the next change saves");
        assert!(Marks::load(&roots).unwrap().hidden("a"));
    }

    #[test]
    fn forgetting_a_session_removes_only_its_own_entry_and_saves_over_another_windows_change() {
        let t = TempDir::new("marks-forget");
        let roots = t.roots();
        let (gone, kept) = ("aaaaaaaa-0000-4000-8000-000000000001", "bbbbbbbb-0000-4000-8000-000000000002");
        let file = t.0.join("a.jsonl");
        let mut m = Marks::default();
        m.rename(gone, &file, "Gone");
        m.pin(gone, &file, true);
        m.set_topic(gone, &file, "Build");
        m.set_handed_over_to(gone, kept);
        m.set_parent(kept, gone);
        m.rename(kept, &file, "Kept");
        m.hide_folder("C:/w", true);
        m.save(&roots).unwrap();
        let mut other = Marks::load(&roots).unwrap();
        assert!(m.forget(&gone.to_ascii_uppercase()), "ids compare without case");
        assert!(!m.forget(gone), "nothing left to forget");
        assert!(!m.pinned(gone) && m.name(gone).is_none() && m.topic(gone).is_none());
        assert_eq!((m.name(kept), m.parent(kept)), (Some("Kept"), Some(gone)), "the other session's marks stay");
        other.hide("cccccccc-0000-4000-8000-000000000003", &file, true);
        other.save(&roots).unwrap();
        m.save(&roots).expect("another window's change elsewhere merges");
        let disk = Marks::load(&roots).unwrap();
        assert!(disk.data["sessions"].get(gone).is_none());
        assert!(disk.hidden("cccccccc-0000-4000-8000-000000000003") && disk.folder_hidden("C:/w"));
        assert_eq!(disk.name(kept), Some("Kept"));
    }

    #[test]
    fn a_moved_sessions_folder_is_kept_and_an_empty_one_is_none() {
        let mut m = Marks::default();
        let file = Path::new(r"C:\p\s.jsonl");
        assert_eq!(m.folder("s"), None);
        assert!(m.set_folder(
            "s",
            file,
            Path::new(
                r"C:\work
ew"
            )
        ));
        assert_eq!(
            m.folder("s"),
            Some(
                r"C:\work
ew"
            )
        );
        assert!(m.set_folder("s", file, Path::new("")));
        assert_eq!(m.folder("s"), None);
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
