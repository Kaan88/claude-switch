//! Three-way merge of app-owned JSON for two writers (two Claude Switch windows): apply only this
//! writer's own changes onto what is on disk now, field by field, and name the fields both
//! changed differently. Arrays and values below `depth` are compared whole.
use serde_json::{Map, Value};

/// The merge, with the paths both writers changed differently. Conflicting fields keep the
/// other writer's value (`theirs`), so a refused save can be retried without them.
pub(crate) struct Merged {
    pub value: Option<Value>,
    pub conflicts: Vec<Vec<String>>,
}

/// `base` is what this writer last read, `ours` what it would write, `theirs` what is on disk.
pub(crate) fn three_way(base: Option<&Value>, ours: Option<&Value>, theirs: Option<&Value>, depth: usize) -> Merged {
    let mut conflicts = Vec::new();
    let value = merge(base, ours, theirs, depth, &mut Vec::new(), &mut conflicts);
    Merged { value, conflicts }
}

fn merge(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    depth: usize,
    path: &mut Vec<String>,
    conflicts: &mut Vec<Vec<String>>,
) -> Option<Value> {
    if ours == base || ours == theirs {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }
    let object = |v: Option<&Value>| v.is_none_or(Value::is_object);
    if depth == 0 || !(object(base) && object(ours) && object(theirs)) {
        conflicts.push(path.clone());
        return theirs.cloned();
    }
    let empty = Map::new();
    let (b, o, t) = (fields(base, &empty), fields(ours, &empty), fields(theirs, &empty));
    let mut out = Map::new();
    let keys: std::collections::BTreeSet<&String> = b.keys().chain(o.keys()).chain(t.keys()).collect();
    // Keep the on-disk order for the keys it has; new keys follow.
    let ordered = t.keys().chain(keys.into_iter().filter(|k| !t.contains_key(*k)));
    for key in ordered {
        path.push(key.clone());
        if let Some(v) = merge(b.get(key), o.get(key), t.get(key), depth - 1, path, conflicts) {
            out.insert(key.clone(), v);
        }
        path.pop();
    }
    // An entry one writer removed stays removed when nothing of it is left.
    if out.is_empty() && (ours.is_none() || theirs.is_none()) {
        return None;
    }
    Some(Value::Object(out))
}

fn fields<'a>(v: Option<&'a Value>, empty: &'a Map<String, Value>) -> &'a Map<String, Value> {
    v.and_then(Value::as_object).unwrap_or(empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(base: Value, ours: Value, theirs: Value) -> (Value, Vec<String>) {
        let m = three_way(Some(&base), Some(&ours), Some(&theirs), 3);
        (m.value.unwrap(), m.conflicts.into_iter().map(|p| p.join("/")).collect())
    }

    #[test]
    fn each_writer_keeps_its_own_fields_and_only_the_same_field_conflicts() {
        let base = json!({"sessions": {"a": {"name": "A", "pinned": false}}});
        let ours = json!({"sessions": {"a": {"name": "A2", "pinned": false}, "b": {"hidden": true}}});
        let theirs = json!({"sessions": {"a": {"name": "A", "pinned": true}, "c": {"name": "C"}}});
        let (merged, conflicts) = run(base.clone(), ours, theirs.clone());
        assert!(conflicts.is_empty());
        assert_eq!(
            merged,
            json!({"sessions": {"a": {"name": "A2", "pinned": true}, "c": {"name": "C"}, "b": {"hidden": true}}})
        );
        let ours = json!({"sessions": {"a": {"name": "Mine", "pinned": false}}});
        let theirs = json!({"sessions": {"a": {"name": "Theirs", "pinned": false}}});
        let (merged, conflicts) = run(base.clone(), ours, theirs);
        assert_eq!(conflicts, ["sessions/a/name"]);
        assert_eq!(merged["sessions"]["a"]["name"], "Theirs", "the other writer's value is kept");
        let same = json!({"sessions": {"a": {"name": "Same", "pinned": false}}});
        assert!(run(base, same.clone(), same).1.is_empty(), "the same change is no conflict");
    }

    #[test]
    fn a_removed_entry_stays_removed_unless_the_other_writer_changed_it() {
        let base = json!({"sessions": {"a": {"name": "A"}, "b": {"seenMs": 1}}});
        let ours = json!({"sessions": {"b": {"seenMs": 1}}});
        let theirs = json!({"sessions": {"a": {"name": "A"}, "b": {"seenMs": 2}}});
        assert_eq!(run(base.clone(), ours.clone(), theirs).0, json!({"sessions": {"b": {"seenMs": 2}}}));
        let theirs = json!({"sessions": {"a": {"name": "Renamed"}, "b": {"seenMs": 1}}});
        assert_eq!(run(base, ours, theirs).1, ["sessions/a/name"]);
    }
}
