use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use claude_switch_core::transcript::{Item, ItemId};

use super::super::links::{self, Files, NamedFile};
use super::named_paths;

#[derive(Default)]
pub(super) struct FileRefs {
    refs: BTreeMap<ItemId, (Vec<String>, Files)>,
    refresh: VecDeque<ItemId>,
}

impl FileRefs {
    pub fn update(&mut self, reset: bool, items: &[(ItemId, Item)], cwd: &Path) -> Vec<(ItemId, Files)> {
        if reset {
            *self = Self::default();
        }
        let mut checked = BTreeMap::new();
        let mut changed = Vec::new();
        for (id, item) in items {
            let names = named_paths(item);
            let Some((existing, previous)) = self.refs.get_mut(id) else {
                if !names.is_empty() {
                    let files = check(&mut checked, cwd, &names);
                    if !files.is_empty() {
                        changed.push((*id, files.clone()));
                    }
                    self.refs.insert(*id, (names, files));
                    self.refresh.push_back(*id);
                }
                continue;
            };
            *existing = names;
            let next = check(&mut checked, cwd, existing);
            if *previous != next {
                changed.push((*id, next.clone()));
                *previous = next;
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
            let next = check(&mut checked, cwd, names);
            if *previous != next {
                changed.push((id, next.clone()));
                *previous = next;
            }
            self.refresh.push_back(id);
        }
        changed
    }
}

fn check(checked: &mut BTreeMap<String, Option<NamedFile>>, cwd: &Path, names: &[String]) -> Files {
    names
        .iter()
        .filter_map(|name| {
            let file = checked.entry(name.clone()).or_insert_with(|| links::resolve(name, cwd));
            Some((name.clone(), file.clone()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::transcript::ItemKind;

    #[test]
    fn refresh_adds_and_removes_links_without_a_new_transcript_record() {
        let demo = crate::demo::Transcript::new().unwrap();
        let cwd = demo.cwd();
        let mut refs = FileRefs::default();
        let id = ItemId { offset: 10, block: 0 };
        let path = "appears-later.md";
        let item = Item { kind: ItemKind::Assistant { markdown: format!("Open `{path}`.") }, timestamp: None };
        assert!(refs.update(true, &[(id, item)], cwd).is_empty());
        std::fs::write(cwd.join(path), "# Present now").unwrap();
        let change = refs.update(false, &[], cwd);
        assert_eq!(change[0].0, id);
        assert!(change[0].1.contains_key(path));
        assert!(refs.update(false, &[], cwd).is_empty());
        std::fs::remove_file(cwd.join(path)).unwrap();
        assert_eq!(refs.update(false, &[], cwd), [(id, Files::new())]);
    }
}
