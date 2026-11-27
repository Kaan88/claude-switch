use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use claude_switch_core::transcript::{Item, ItemId, ItemKind, ToolState};

use super::super::links::{self, Files, NamedFile};
use super::named_paths;

#[derive(Default)]
pub(super) struct FileRefs {
    refs: BTreeMap<ItemId, (Vec<String>, Files)>,
    refresh: VecDeque<ItemId>,
    grants: BTreeMap<ItemId, PathBuf>,
    touched: BTreeSet<PathBuf>,
    /// The granted paths, newest write first: what a bare file name prefers.
    recent: Vec<PathBuf>,
}

/// What a name is checked against: the session folder, and the transcript's outside writes.
struct Scope<'a> {
    cwd: &'a Path,
    touched: &'a BTreeSet<PathBuf>,
    recent: &'a [PathBuf],
}

impl FileRefs {
    pub fn get(&self, id: ItemId) -> Option<&Files> {
        self.refs.get(&id).map(|(_, files)| files)
    }

    pub fn update(
        &mut self,
        reset: bool,
        items: &[(ItemId, Item)],
        cwd: &Path,
        extra: impl Fn(ItemId) -> Vec<String>,
    ) -> Vec<(ItemId, Files)> {
        if reset {
            *self = Self::default();
        }
        let mut checked = BTreeMap::new();
        let mut changed = BTreeMap::new();
        // Learn successful writes before resolving any references in this batch. Keep the grants
        // scoped to the displayed reader; a reset (including switching agents) discards them.
        let mut grants_changed = false;
        for (id, item) in items {
            let path = match &item.kind {
                ItemKind::Tool { name, file_path: Some(path), state: ToolState::Ok, .. }
                    if matches!(name.as_str(), "Write" | "Edit") =>
                {
                    links::transcript_path(path, cwd).filter(|p| !links::inside(p, cwd))
                }
                _ => None,
            };
            match path {
                Some(path) if self.grants.get(id) != Some(&path) => {
                    self.grants.insert(*id, path);
                    grants_changed = true;
                }
                None => grants_changed |= self.grants.remove(id).is_some(),
                _ => {}
            }
        }
        let affected = if grants_changed {
            let next: BTreeSet<_> = self.grants.values().cloned().collect();
            let affected = self.touched.symmetric_difference(&next).cloned().collect::<BTreeSet<_>>();
            self.touched = next;
            self.recent.clear();
            for path in self.grants.values().rev() {
                if !self.recent.contains(path) {
                    self.recent.push(path.clone());
                }
            }
            affected
        } else {
            BTreeSet::new()
        };
        let scope = Scope { cwd, touched: &self.touched, recent: &self.recent };
        for (id, item) in items {
            let mut names = named_paths(item);
            names.extend(extra(*id));
            names.sort_unstable();
            names.dedup();
            let Some((existing, previous)) = self.refs.get_mut(id) else {
                if !names.is_empty() {
                    let files = check(&mut checked, &scope, &names);
                    if !files.is_empty() {
                        changed.insert(*id, files.clone());
                    }
                    self.refs.insert(*id, (names, files));
                    self.refresh.push_back(*id);
                }
                continue;
            };
            *existing = names;
            let next = check(&mut checked, &scope, existing);
            if *previous != next {
                changed.insert(*id, next.clone());
                *previous = next;
            }
        }
        if !affected.is_empty() {
            // A bare name is affected by a write of that name in any folder.
            let names: BTreeSet<String> =
                affected.iter().filter_map(|p| Some(p.file_name()?.to_str()?.to_lowercase())).collect();
            let hit = |name: &String| {
                links::transcript_path(name, cwd).is_some_and(|p| affected.contains(&p))
                    || (!name.contains(['/', '\\']) && names.contains(&name.to_lowercase()))
            };
            for (&id, (list, previous)) in &mut self.refs {
                if !changed.contains_key(&id) && list.iter().any(hit) {
                    update(&mut checked, &scope, &mut changed, id, list, previous);
                }
            }
        }
        // A file can appear/disappear without a new transcript entry. Recheck a
        // slice on each visible poll, then rotate it to the end of the queue.
        for _ in 0..self.refresh.len().min(32) {
            let id = self.refresh.pop_front().expect("bounded by queue length");
            let (names, previous) = self.refs.get_mut(&id).expect("queued file references");
            if names.is_empty() {
                self.refs.remove(&id);
                continue;
            }
            update(&mut checked, &scope, &mut changed, id, names, previous);
            self.refresh.push_back(id);
        }
        changed.into_iter().collect()
    }
}

fn update(
    checked: &mut BTreeMap<String, Option<NamedFile>>,
    scope: &Scope<'_>,
    changed: &mut BTreeMap<ItemId, Files>,
    id: ItemId,
    names: &[String],
    previous: &mut Files,
) {
    let next = check(checked, scope, names);
    if *previous != next {
        changed.insert(id, next.clone());
        *previous = next;
    }
}

fn check(checked: &mut BTreeMap<String, Option<NamedFile>>, scope: &Scope<'_>, names: &[String]) -> Files {
    names
        .iter()
        .filter_map(|name| {
            let file = checked
                .entry(name.clone())
                .or_insert_with(|| links::resolve_recent(name, scope.cwd, scope.touched, scope.recent));
            Some((name.clone(), file.clone()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::transcript::ItemKind;

    #[test]
    fn saved_output_labels_do_not_grant_outside_access() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let inside = demo.cwd().join("report.txt");
        let narrower = demo.cwd().join("session");
        std::fs::write(&inside, "saved").unwrap();
        std::fs::create_dir(&narrower).unwrap();
        std::fs::write(narrower.join("inside.txt"), "saved").unwrap();
        let outside = inside.to_str().unwrap();
        let id = ItemId { offset: 1, block: 0 };
        let report = Item {
            timestamp: None,
            kind: ItemKind::Tool {
                id: "one".into(),
                name: "Bash".into(),
                summary: String::new(),
                file_path: None,
                state: ToolState::Ok,
                duration_ms: None,
                pictures: vec![],
                subagent: None,
                denial: None,
                card: None,
            },
        };
        let mut refs = FileRefs::default();
        let result = claude_switch_core::transcript::ResultDetails {
            text: format!("Saved to: `{outside}`\nWrote file: inside.txt\nLook at another.exe"),
            timestamp: None,
        };
        refs.update(true, &[(id, report)], &narrower, |_| result.saved_paths().map(str::to_owned).collect());
        let files = refs.get(id).unwrap();
        assert_eq!(files.keys().map(String::as_str).collect::<Vec<_>>(), ["inside.txt"]);
    }

    #[test]
    fn refresh_adds_and_removes_links_without_a_new_transcript_record() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let mut refs = FileRefs::default();
        let id = ItemId { offset: 10, block: 0 };
        let path = "appears-later.md";
        let item = Item { kind: ItemKind::Assistant { markdown: format!("Open `{path}`.") }, timestamp: None };
        assert!(refs.update(true, &[(id, item)], cwd, |_| Vec::new()).is_empty());
        std::fs::write(cwd.join(path), "# Present now").unwrap();
        let change = refs.update(false, &[], cwd, |_| Vec::new());
        assert_eq!(change[0].0, id);
        assert!(change[0].1.contains_key(path));
        assert!(refs.update(false, &[], cwd, |_| Vec::new()).is_empty());
        std::fs::remove_file(cwd.join(path)).unwrap();
        assert_eq!(refs.update(false, &[], cwd, |_| Vec::new()), [(id, Files::new())]);
    }

    #[test]
    fn only_the_current_transcripts_successful_write_or_edit_grants_an_exact_outside_path() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let cwd = demo.cwd().join("session");
        std::fs::create_dir(&cwd).unwrap();
        let outside = demo.cwd().join("field-notes.md");
        let other = demo.cwd().join("other.md");
        std::fs::write(&other, "# Unrelated").unwrap();
        let name = outside.to_str().unwrap();
        let other = other.to_str().unwrap();
        let id = |offset| ItemId { offset, block: 0 };
        let prose =
            Item { kind: ItemKind::Assistant { markdown: format!("Open `{name}` or `{other}`.") }, timestamp: None };
        let tool = |name_of_tool: &str, state| Item {
            kind: ItemKind::Tool {
                id: "tool".into(),
                name: name_of_tool.into(),
                summary: name.into(),
                file_path: Some(name.into()),
                state,
                duration_ms: None,
                pictures: vec![],
                subagent: None,
                denial: None,
                card: None,
            },
            timestamp: None,
        };
        let mut refs = FileRefs::default();
        let no_extra = |_| Vec::new();
        assert!(
            refs.update(true, &[(id(1), prose.clone()), (id(2), tool("Read", ToolState::Ok))], &cwd, no_extra)
                .is_empty()
        );
        assert!(refs.update(false, &[(id(3), tool("Write", ToolState::Failed))], &cwd, no_extra).is_empty());
        let granted = refs.update(false, &[(id(3), tool("Write", ToolState::Ok))], &cwd, no_extra);
        assert_eq!(granted.len(), 3, "the tool and already loaded rows all see the grant");
        assert!(granted.iter().all(|(_, r)| r.keys().map(String::as_str).eq([name])));
        assert_eq!(refs.update(false, &[(id(3), tool("Write", ToolState::Failed))], &cwd, no_extra).len(), 3);
        assert!(refs.update(true, &[(id(1), prose)], &cwd, no_extra).is_empty());
    }

    #[test]
    fn a_bare_name_prefers_the_file_of_that_name_the_transcript_wrote_elsewhere_and_offers_both() {
        let demo = crate::demo::Transcript::new_chat().unwrap();
        let cwd = demo.cwd().join("session");
        let repo = demo.cwd().join("other-repo");
        let older = demo.cwd().join("older-repo");
        for dir in [&cwd, &repo, &older, &cwd.join("docs")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for dir in [&cwd, &repo, &older, &cwd.join("docs")] {
            std::fs::write(dir.join("CHANGELOG.md"), "# Changes").unwrap();
        }
        let id = |offset| ItemId { offset, block: 0 };
        let edit = |path: &Path| Item {
            kind: ItemKind::Tool {
                id: "tool".into(),
                name: "Edit".into(),
                summary: String::new(),
                file_path: Some(path.to_str().unwrap().into()),
                state: ToolState::Ok,
                duration_ms: None,
                pictures: vec![],
                subagent: None,
                denial: None,
                card: None,
            },
            timestamp: None,
        };
        let answer = Item {
            kind: ItemKind::Assistant { markdown: "See `CHANGELOG.md` and `docs/CHANGELOG.md`.".into() },
            timestamp: None,
        };
        let no_extra = |_| Vec::new();
        let mut refs = FileRefs::default();
        refs.update(true, &[(id(5), answer.clone())], &cwd, no_extra);
        let files = refs.get(id(5)).unwrap();
        assert_eq!(files["CHANGELOG.md"].path(), cwd.join("CHANGELOG.md"), "no write elsewhere: the session's");
        assert!(files["CHANGELOG.md"].other().is_none());

        // Writes elsewhere arrive later: the already loaded answer is resolved again.
        let changed = refs.update(
            false,
            &[(id(1), edit(&older.join("CHANGELOG.md"))), (id(3), edit(&repo.join("changelog.md")))],
            &cwd,
            no_extra,
        );
        assert!(changed.iter().any(|(i, _)| *i == id(5)));
        let files = refs.get(id(5)).unwrap();
        let bare = &files["CHANGELOG.md"];
        assert_eq!(bare.path(), repo.join("changelog.md"), "the newest write of that name");
        let other = bare.other().expect("the session's file is offered too");
        assert_eq!(other.path(), cwd.join("CHANGELOG.md"));
        assert_eq!(other.other().unwrap().path(), bare.path(), "and back");
        assert_eq!(files["docs/CHANGELOG.md"].path(), cwd.join("docs").join("CHANGELOG.md"), "a folder keeps it here");

        // With no file of that name in the session folder, the outside one stands alone.
        std::fs::remove_file(cwd.join("CHANGELOG.md")).unwrap();
        let mut refs = FileRefs::default();
        refs.update(true, &[(id(3), edit(&repo.join("changelog.md"))), (id(5), answer)], &cwd, no_extra);
        let bare = &refs.get(id(5)).unwrap()["CHANGELOG.md"];
        assert_eq!(bare.path(), repo.join("changelog.md"));
        assert!(bare.other().is_none());
    }
}
