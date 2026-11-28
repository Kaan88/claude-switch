//! A split tab's panes in the saved tabs (`terminal-tabs.json`): the tree with its split ratios,
//! the folder each shell was started in, and which pane had the keyboard. No commands, names or
//! screen contents, as for the rest of that file.
//!
//! `{"tree": <node>, "folders": ["C:\\…", …], "focus": <leaf>}`, where a node is a leaf number
//! (0 the tab's own pane, `k` the shell started in `folders[k - 1]`) or
//! `{"axis": "right" | "down", "ratio": <0..1>, "a": <node>, "b": <node>}`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{AppState, Axis, MAIN, MAX_PANES, Node, PaneId, Panes};

/// A layout read back: leaves numbered as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub tree: Node,
    pub folders: Vec<PathBuf>,
    pub focus: PaneId,
}

/// The panes of a split tab as saved; `None` when it is not split.
pub fn save(panes: &Panes) -> Option<Value> {
    if !panes.is_split() {
        return None;
    }
    // Shells are numbered in reading order, so the file does not depend on the ids of this run.
    let order: Vec<PaneId> = panes.ids().into_iter().filter(|id| *id != MAIN).collect();
    let number = |id: PaneId| if id == MAIN { 0 } else { order.iter().position(|p| *p == id).map_or(0, |i| i + 1) };
    fn node(n: &Node, number: &dyn Fn(PaneId) -> usize) -> Value {
        match n {
            Node::Leaf(id) => json!(number(*id)),
            Node::Split { axis, ratio, a, b } => json!({
                "axis": if *axis == Axis::Right { "right" } else { "down" },
                "ratio": ratio,
                "a": node(a, number),
                "b": node(b, number),
            }),
        }
    }
    let folders: Option<Vec<&str>> =
        order.iter().map(|id| panes.folders.iter().find(|(p, _)| p == id).and_then(|(_, f)| f.to_str())).collect();
    Some(json!({"tree": node(&panes.tree, &number), "folders": folders?, "focus": number(panes.focus)}))
}

/// Reads a saved layout; `None` for anything that is not one this app writes.
pub fn load(v: &Value) -> Option<Saved> {
    let folders: Vec<PathBuf> =
        v["folders"].as_array()?.iter().map(|f| f.as_str().map(PathBuf::from)).collect::<Option<_>>()?;
    if folders.is_empty() || folders.len() >= MAX_PANES || !folders.iter().all(|f| f.is_absolute()) {
        return None;
    }
    fn node(v: &Value, depth: usize, seen: &mut Vec<PaneId>) -> Option<Node> {
        if let Some(n) = v.as_u64() {
            let id = PaneId::try_from(n).ok()?;
            seen.push(id);
            return Some(Node::Leaf(id));
        }
        if depth >= MAX_PANES {
            return None;
        }
        let axis = match v["axis"].as_str()? {
            "right" => Axis::Right,
            "down" => Axis::Down,
            _ => return None,
        };
        let ratio = v["ratio"].as_f64()? as f32;
        if !(0.02..=0.98).contains(&ratio) {
            return None;
        }
        let a = Box::new(node(&v["a"], depth + 1, seen)?);
        let b = Box::new(node(&v["b"], depth + 1, seen)?);
        Some(Node::Split { axis, ratio, a, b })
    }
    let mut seen = Vec::new();
    let tree = node(&v["tree"], 0, &mut seen)?;
    seen.sort_unstable();
    // Every pane once: the tab's own and one per folder.
    if seen != (0..=folders.len() as PaneId).collect::<Vec<_>>() {
        return None;
    }
    let focus = PaneId::try_from(v["focus"].as_u64()?).ok().filter(|f| *f as usize <= folders.len())?;
    Some(Saved { tree, folders, focus })
}

impl Panes {
    /// Puts back a saved layout with its shells, `terms[k - 1]` for leaf `k` started in `folders[k - 1]`.
    fn restore(&mut self, saved: Saved, terms: Vec<crate::term::Term>, folders: Vec<PathBuf>) {
        let n = terms.len() as PaneId;
        self.tree = saved.tree;
        self.extra = (1..=n).zip(terms).collect();
        self.folders = (1..=n).zip(folders).collect();
        self.focus = saved.focus;
        self.next = n + 1;
        self.drag = None;
    }
}

impl AppState {
    /// Gives a reopened tab the panes it had: each shell starts again in its folder (the tab's
    /// own when that folder is gone). A layout that cannot be read, or a shell that cannot start,
    /// leaves the tab unsplit.
    pub(crate) fn restore_panes(&mut self, id: u64, saved: &Value) {
        let Some(saved) = load(saved) else { return };
        let fallback = self.pane_folder(id);
        let folders: Vec<PathBuf> = saved
            .folders
            .iter()
            .map(|f| if self.source.is_demo() || Path::new(f).is_dir() { f.clone() } else { fallback.clone() })
            .collect();
        let mut terms = Vec::new();
        for folder in &folders {
            match self.shell_term(folder, self.grid) {
                Ok(t) => terms.push(t),
                Err(_) => return,
            }
        }
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.panes.restore(saved, terms, folders);
        }
    }
}
