//! Immutable, worker-built prompt index for one complete scan.

use claude_switch_core::grouping::{self, Project};
use claude_switch_core::{Session, sessions::folder_key};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
struct Candidate {
    id: String,
    project: usize,
    at: u64,
    words: HashSet<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RelatedIndex {
    candidates: Vec<Candidate>,
    by_id: HashMap<String, usize>,
    members: Vec<Vec<usize>>,
}

impl RelatedIndex {
    pub(super) fn ranks(&self, id: &str, cancelled: impl Fn() -> bool) -> Vec<usize> {
        let Some(&i) = self.by_id.get(id) else { return Vec::new() };
        let selected = &self.candidates[i];
        if selected.words.is_empty() {
            return Vec::new();
        }
        let mut scored = Vec::new();
        for &j in &self.members[selected.project] {
            if cancelled() {
                return Vec::new();
            }
            if j != i {
                let other = &self.candidates[j];
                let score = grouping::similarity(&selected.words, &other.words);
                if score != 0 {
                    scored.push((Reverse((score, other.at)), &other.id, j));
                }
            }
        }
        scored.sort_unstable();
        scored.into_iter().map(|(_, _, j)| j).collect()
    }

    pub(super) fn id(&self, index: usize) -> &str {
        &self.candidates[index].id
    }

    pub(crate) fn new(sessions: &[Session], projects: &HashMap<String, Project>) -> Self {
        let mut index = Self::default();
        let mut project_ids = HashMap::new();
        for s in sessions {
            let Some(project) = projects.get(&folder_key(&s.cwd)) else { continue };
            let project = *project_ids.entry(&project.key).or_insert_with(|| {
                index.members.push(Vec::new());
                index.members.len() - 1
            });
            let i = index.candidates.len();
            index.by_id.insert(s.id.clone(), i);
            index.members[project].push(i);
            index.candidates.push(Candidate {
                id: s.id.clone(),
                project,
                at: s.last_active_ms,
                words: grouping::words(s.grouping.first_prompt.as_deref().unwrap_or_default(), &s.last_prompt),
            });
        }
        index
    }
}
