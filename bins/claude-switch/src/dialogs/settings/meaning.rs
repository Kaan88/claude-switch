//! Settings › Sessions › Search by meaning: on or off, the model, and the kept summaries.
use super::*;
use claude_switch_core::meaning::Engine;

pub(super) const SET_MEANING: u32 = 160;
pub(super) const SET_MEANING_MODEL: u32 = 161;
pub(super) const LIST_MEANING_MODELS: u32 = 162;
pub(super) const FORGET_MEANING: u32 = 163;

/// Claude first, then the OpenCode models this window listed, with a saved one kept.
fn engines(st: &AppState) -> Vec<Engine> {
    let mut models = st.listed_models().map(<[String]>::to_vec).unwrap_or_default();
    if let Engine::OpenCode(saved) = &st.config.meaning.engine
        && !models.contains(saved)
    {
        models.insert(0, saved.clone());
    }
    std::iter::once(Engine::Claude).chain(models.into_iter().map(Engine::OpenCode)).collect()
}

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let prefs = &st.config.meaning;
    let engines = engines(st);
    let worker = &claude_switch_core::naming::worker_account(&st.config).name;
    let labels: Vec<String> = engines.iter().map(|e| e.label(worker)).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let value = engines.iter().position(|e| *e == prefs.engine).unwrap_or(0);
    let kept = st.meaning.cache.digests.values().filter(|d| !d.is_marker()).count();
    let listing = if st.meaning.listing_models() { "Listing…" } else { "List models" };
    vec![
        Row::header("Search by meaning"),
        Row::toggle(SET_MEANING, "Search and group by meaning", prefs.enabled)
            .help("Off by default. Asks before anything is sent."),
        Row::choice(SET_MEANING_MODEL, "Model", &refs, value).help("Sees only what Suggest names sends."),
        Row::action(LIST_MEANING_MODELS, "OpenCode models", listing),
        Row::action(
            FORGET_MEANING,
            if kept == 0 { "No summaries kept".to_owned() } else { format!("{kept} summaries kept here") },
            "Forget",
        ),
    ]
}

/// A Sessions page event of this section; false when it is not one.
pub(super) fn outcome(st: &mut AppState, o: &Outcome) -> bool {
    match o {
        Outcome::Action(LIST_MEANING_MODELS) => st.list_meaning_models(),
        Outcome::Action(FORGET_MEANING) => {
            st.forget_meaning();
            refresh(st);
        }
        Outcome::Changed { id: SET_MEANING, value: Value::Bool(on) } if !st.config_damaged => {
            st.config.meaning.enabled = *on;
            if !on && st.list_mode() == claude_switch_core::grouping::Mode::Topics {
                st.config.meaning.group_topics = false;
            }
            st.load_meaning();
            st.save_config();
            st.rebuild_view();
            refresh(st);
        }
        Outcome::Changed { id: SET_MEANING_MODEL, value: Value::Choice(i) } if !st.config_damaged => {
            let Some(engine) = engines(st).into_iter().nth(*i) else { return true };
            st.config.meaning.engine = engine;
            st.save_config();
            refresh(st);
        }
        _ => return false,
    }
    true
}

/// Shows the section's current values when the Sessions page is open.
pub(crate) fn refresh(st: &mut AppState) {
    let rows = sessions(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == SESSIONS
    {
        settings.form.set_rows(rows);
    }
}
