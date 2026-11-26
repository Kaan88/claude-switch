//! The transcript, rather than a separate list of Chat sends, owns prompt history.
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use claude_switch_core::transcript::{Item, ItemId, ItemKind};
use mdshared_ui::keys::KeyName;

use super::{Chat, editor::ComposerEditor};
use crate::view::model::Model;

#[derive(Default)]
pub(super) struct Recall {
    browsing: Option<Browsing>,
    loading: Option<(u64, u64)>,
}

struct Browsing {
    generation: u64,
    shown: ItemId,
    draft: ComposerEditor,
}

fn text(item: &Item) -> Option<&str> {
    match &item.kind {
        ItemKind::Prompt { text, synthetic: false, .. } if !text.trim().is_empty() => Some(text),
        _ => None,
    }
}

fn step(items: &BTreeMap<ItemId, Item>, current: Option<ItemId>, up: bool) -> Option<(ItemId, &str)> {
    let previous = current.and_then(|id| items.get(&id)).and_then(text).map(str::trim);
    let different = |(&id, item)| {
        let next = text(item)?;
        // Adjacent copies are one stop; repeated prompts separated by another prompt still count.
        (previous != Some(next.trim())).then_some((id, next))
    };
    match (current, up) {
        (None, true) => items.iter().rev().find_map(different),
        (Some(id), true) => items.range(..id).rev().find_map(different),
        (Some(id), false) => items.range((Excluded(id), Unbounded)).find_map(different),
        (None, false) => None,
    }
}

impl Recall {
    /// Commit the text currently in the editor as the draft, without restoring the old draft.
    pub fn end(&mut self) {
        *self = Self::default();
    }

    fn active(&self) -> bool {
        self.browsing.is_some()
    }

    pub fn waiting(&self) -> bool {
        self.loading.is_some()
    }

    pub fn cancel_load(&mut self) {
        self.loading = None;
    }

    fn restore(&mut self, editor: &mut ComposerEditor) -> bool {
        if let Some(saved) = self.browsing.take() {
            *editor = saved.draft;
            editor.follow = true;
            true
        } else {
            false
        }
    }

    fn show(&mut self, editor: &mut ComposerEditor, model: &Model, up: bool) -> bool {
        let current = self.browsing.as_ref().map(|b| b.shown);
        let Some((id, prompt)) = step(&model.items, current, up) else {
            return !up && self.restore(editor);
        };
        let browsing = self.browsing.get_or_insert_with(|| Browsing {
            generation: model.generation,
            shown: id,
            draft: std::mem::take(editor),
        });
        browsing.shown = id;
        editor.set(prompt.to_owned());
        editor.move_to(if up { 0 } else { prompt.len() }, false);
        true
    }

    fn load_earlier(&mut self, model: &Model) {
        if model.has_earlier && model.error.is_none() {
            self.loading = Some((model.generation, model.earlier_version));
            model.earlier();
        }
    }

    pub fn key(&mut self, editor: &mut ComposerEditor, model: &Model, key: &KeyName) -> bool {
        self.reset_if_changed(editor, model);
        match key {
            KeyName::Escape => {
                let was_loading = self.loading.take().is_some();
                self.restore(editor) || was_loading
            }
            KeyName::Up if editor.on_first_line() && model.agent.is_none() => {
                if self.loading.is_none() && !self.show(editor, model, true) {
                    self.load_earlier(model);
                }
                true
            }
            KeyName::Down => {
                self.cancel_load();
                self.active() && editor.on_last_line() && self.show(editor, model, false)
            }
            _ => false,
        }
    }

    fn reset_if_changed(&mut self, editor: &mut ComposerEditor, model: &Model) -> bool {
        if self.loading.is_some_and(|(generation, _)| generation != model.generation)
            || model.agent.is_some()
            || model.error.is_some()
        {
            self.cancel_load();
        }
        self.browsing.as_ref().is_some_and(|b| b.generation != model.generation || model.agent.is_some())
            && self.restore(editor)
    }

    pub fn advance(&mut self, editor: &mut ComposerEditor, model: &Model, focused: bool) -> bool {
        let restored = self.reset_if_changed(editor, model);
        if focused && self.loading.is_some_and(|(_, version)| version != model.earlier_version) {
            self.cancel_load();
            if !self.show(editor, model, true) {
                // A page of tool or assistant output may contain no prompts at all.
                self.load_earlier(model);
            }
            return true;
        }
        restored
    }
}

impl Chat {
    pub(super) fn recall_key(&mut self, model: &Model, key: &KeyName) -> bool {
        !self.commands.open && !self.locked() && self.recall.key(&mut self.editor, model, key)
    }
}

#[cfg(test)]
mod tests;
