//! Settings › Claude Code › Advanced: the rest of Claude Code's `/config` (the table and the
//! careful write are `claude_switch_core::cc_config`). `settings.json` entries are saved in the
//! master account and shared, as on the Claude Code page; `.claude.json` entries belong to one
//! account, chosen on the page, and are read-only while it has a running Claude Code.
//!
//! The page reads both files when it opens, after each change and on "Check again"; a change is
//! compared with what the page showed, so a value changed elsewhere meanwhile is never
//! overwritten.
use super::{AppState, Dialog, Row};
use claude_switch_core::Account;
use claude_switch_core::cc_config::{self, ENTRIES, Entry, Kind, Store};
use mdshared_ui::settings::Value;
use serde_json::Value as Json;

pub(crate) const ADVANCED: &str = "Claude Code › Advanced";
pub(super) const SEARCH: u32 = 500;
pub(super) const ACCOUNT: u32 = 501;
pub(super) const RECHECK: u32 = 502;
/// Entry `i`'s control is `FIRST + 2 * i`, its Reset button the next id.
pub(super) const FIRST: u32 = 510;
const LAST: u32 = FIRST + 2 * ENTRIES.len() as u32;
const DEFAULT: &str = "Default";

/// What the page read, and its search and account.
#[derive(Default)]
pub(crate) struct Page {
    pub(super) search: String,
    /// The account whose `.claude.json` the page shows; the master when unset or gone.
    account: Option<String>,
    settings: Option<Json>,
    global: Option<Json>,
    settings_error: Option<String>,
    global_error: Option<String>,
    /// The chosen account has a running Claude Code.
    busy: bool,
}

pub(super) fn owns(id: u32) -> bool {
    (SEARCH..LAST).contains(&id)
}

fn master(st: &AppState) -> Option<&Account> {
    claude_switch_core::settings_sync::master_account(&st.config, &st.config.settings_share)
}

/// The account the per-account entries are read from and written to.
pub(super) fn chosen(st: &AppState) -> Option<&Account> {
    let slug = st.cc_page.account.as_deref();
    st.config.accounts.iter().find(|a| Some(a.slug.as_str()) == slug).or_else(|| master(st))
}

/// Chooses the account whose `.claude.json` the page shows, and reads again.
pub(super) fn set_account(st: &mut AppState, index: usize) {
    st.cc_page.account = st.config.accounts.get(index).map(|a| a.slug.clone());
    load(st);
}

/// Reads both files again (and whether the chosen account runs Claude Code).
pub(super) fn load(st: &mut AppState) {
    let crate::source::Source::Real(roots) = &st.source else { return };
    let read = |account: Option<&Account>, store| match account {
        Some(a) => match cc_config::read(&cc_config::file(roots, a, store)) {
            Ok(v) => (v, None),
            Err(e) => (None, Some(e.to_string())),
        },
        None => (None, None),
    };
    let (settings, settings_error) = read(master(st), Store::Settings);
    let (global, global_error) = read(chosen(st), Store::Global);
    let slug = chosen(st).map(|a| a.slug.clone());
    let busy = st.source.running(&st.config).iter().any(|r| Some(&r.account) == slug.as_ref());
    let page = &mut st.cc_page;
    (page.settings, page.settings_error, page.global, page.global_error, page.busy) =
        (settings, settings_error, global, global_error, busy);
}

fn file(st: &AppState, store: Store) -> Option<&Json> {
    match store {
        Store::Settings => st.cc_page.settings.as_ref(),
        Store::Global => st.cc_page.global.as_ref(),
    }
}

fn saved<'a>(st: &'a AppState, entry: &Entry) -> Option<&'a Json> {
    file(st, entry.store).and_then(|f| cc_config::get(f, entry.key))
}

/// A choice's options with the value each saves (`None`: the key goes) and the shown index. A
/// saved value the list lacks is added as it is, and kept until another is chosen.
fn options(entry: &Entry, saved: Option<&Json>) -> (Vec<(String, Option<Json>)>, usize) {
    let mut list: Vec<(String, Option<Json>)> = Vec::new();
    let mut default_at = None;
    match entry.kind {
        Kind::Switch(_) => {}
        Kind::Choice { options, default } => {
            if default.is_none() {
                list.push((DEFAULT.into(), None));
                default_at = Some(0);
            }
            let skip = list.len();
            list.extend(options.iter().map(|(label, v)| ((*label).to_owned(), Some(v.json()))));
            default_at = default_at.or(default.map(|d| d + skip));
        }
    }
    let index = match saved {
        None => default_at.unwrap_or(0),
        Some(v) => list.iter().position(|(_, o)| o.as_ref() == Some(v)).unwrap_or_else(|| {
            list.push((v.as_str().map_or_else(|| v.to_string(), str::to_owned), Some(v.clone())));
            list.len() - 1
        }),
    };
    (list, index)
}

/// The label of the value an entry shows.
fn shown(entry: &Entry, saved: Option<&Json>) -> String {
    match entry.kind {
        Kind::Switch(default) => if saved.and_then(Json::as_bool).unwrap_or(default) { "On" } else { "Off" }.to_owned(),
        Kind::Choice { .. } => {
            let (list, index) = options(entry, saved);
            list[index].0.clone()
        }
    }
}

/// What Claude Code does with the key absent, for the Reset row.
fn default_label(entry: &Entry) -> Option<String> {
    match entry.kind {
        Kind::Switch(on) => Some(if on { "On" } else { "Off" }.to_owned()),
        Kind::Choice { options, default } => default.map(|d| options[d].0.to_owned()),
    }
}

fn matches(entry: &Entry, search: &str) -> bool {
    let search = search.trim().to_lowercase();
    search.is_empty()
        || [entry.label, entry.cli_label, entry.key, entry.help].iter().any(|t| t.to_lowercase().contains(&search))
}

fn entry_rows(st: &AppState, i: usize, entry: &Entry, read_only: Option<&str>) -> Vec<Row> {
    let id = FIRST + 2 * i as u32;
    let saved = saved(st, entry);
    if let Some(why) = read_only {
        return vec![Row::choice(id, entry.label, &[shown(entry, saved).as_str()], 0).help(why)];
    }
    let help = if saved.is_none() { format!("{} · default", entry.help) } else { entry.help.to_owned() };
    let row = match entry.kind {
        Kind::Switch(default) => Row::toggle(id, entry.label, saved.and_then(Json::as_bool).unwrap_or(default)),
        Kind::Choice { .. } => {
            let (list, index) = options(entry, saved);
            let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
            Row::choice(id, entry.label, &names, index)
        }
    };
    let mut rows = vec![row.help(help)];
    if saved.is_some()
        && let Some(default) = default_label(entry)
    {
        rows.push(Row::action(id + 1, format!("Claude Code's default: {default}"), "Reset"));
    }
    rows
}

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let page = &st.cc_page;
    let search = page.search.as_str();
    let mut rows =
        vec![Row::text(SEARCH, "Search", search).placeholder("Name, key or what it does").help("Filters this page.")];
    let master_name = master(st).map_or("the first account", |a| a.name.as_str());
    let shared: Vec<_> =
        ENTRIES.iter().enumerate().filter(|(_, e)| e.store == Store::Settings && matches(e, search)).collect();
    if !shared.is_empty() {
        rows.push(Row::header(format!("Shared · settings.json of {master_name}")));
        if page.settings_error.is_some() {
            rows.push(Row::action(RECHECK, "settings.json is not valid JSON", "Check again"));
        } else {
            for (i, entry) in shared {
                rows.extend(entry_rows(st, i, entry, None));
            }
        }
    }
    let own: Vec<_> =
        ENTRIES.iter().enumerate().filter(|(_, e)| e.store == Store::Global && matches(e, search)).collect();
    if !own.is_empty() {
        let account = chosen(st);
        let name = account.map_or("", |a| a.name.as_str());
        rows.push(Row::header("Per account · .claude.json"));
        let names: Vec<&str> = st.config.accounts.iter().map(|a| a.name.as_str()).collect();
        let index = account.and_then(|a| st.config.accounts.iter().position(|b| b.slug == a.slug)).unwrap_or(0);
        let why_busy = format!("Close {name}'s sessions to change this.");
        let account_help = if page.busy {
            format!("{name} runs Claude Code: read-only.")
        } else {
            "Only this account's file changes.".to_owned()
        };
        rows.push(Row::choice(ACCOUNT, "Account", &names, index).help(account_help));
        if page.global_error.is_some() {
            rows.push(Row::action(RECHECK, ".claude.json is not valid JSON", "Check again"));
        } else {
            if page.busy {
                rows.push(Row::action(RECHECK, "Closed its sessions?", "Check again"));
            }
            let read_only = page.busy.then_some(why_busy.as_str());
            for (i, entry) in own {
                rows.extend(entry_rows(st, i, entry, read_only));
            }
        }
    }
    if rows.len() == 1 {
        rows.push(Row::header(format!("Nothing matches \u{201c}{}\u{201d}", search.trim())));
    }
    rows
}

/// Shows the page again, if it is open.
fn refresh(st: &mut AppState) {
    let rows = rows(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == ADVANCED
    {
        settings.form.set_rows(rows);
    }
}

/// A change on the page.
pub(super) fn apply(st: &mut AppState, id: u32, value: Option<Value>) {
    match (id, &value) {
        (SEARCH, Some(Value::Text(text))) => {
            st.cc_page.search.clone_from(text);
            return refresh(st);
        }
        (ACCOUNT, Some(Value::Choice(i))) => {
            set_account(st, *i);
            return refresh(st);
        }
        (RECHECK, None) => {
            load(st);
            return refresh(st);
        }
        _ => {}
    }
    let Some(offset) = id.checked_sub(FIRST) else { return };
    let Some(entry) = ENTRIES.get(offset as usize / 2) else { return };
    let reset = offset % 2 == 1;
    if entry.store == Store::Global && st.cc_page.busy {
        // Read-only rows: nothing to save.
        return refresh(st);
    }
    let saved = saved(st, entry).cloned();
    let new = match (reset, value, entry.kind) {
        (true, None, _) => None,
        (false, Some(Value::Bool(on)), Kind::Switch(_)) => Some(Json::Bool(on)),
        (false, Some(Value::Choice(i)), Kind::Choice { .. }) => {
            let (list, index) = options(entry, saved.as_ref());
            match list.into_iter().nth(i) {
                Some(_) if i == index => return,
                Some((_, v)) => v,
                None => return,
            }
        }
        _ => return,
    };
    // A read-only row (a single option) never gets here with a change; a stale one is refused.
    let result = match &st.source {
        crate::source::Source::Real(roots) => match chosen(st) {
            Some(account) => cc_config::save(roots, &st.config, account, entry, saved.as_ref(), new),
            None => Err(claude_switch_core::Refusal::Invalid("There is no account.".into())),
        },
        crate::source::Source::Demo => {
            let page = &mut st.cc_page;
            let file = match entry.store {
                Store::Settings => &mut page.settings,
                Store::Global => &mut page.global,
            };
            cc_config::put(file.get_or_insert_with(|| Json::Object(Default::default())), entry.key, new);
            Ok(())
        }
    };
    if let Err(e) = result {
        st.error("Claude Code's setting was not saved", e.to_string());
    }
    load(st);
    refresh(st);
}

/// Every entry set to a value other than its default, as a page with Reset rows looks; `busy`
/// makes the per-account rows read-only.
#[cfg(test)]
pub(super) fn fill_for_pictures(st: &mut AppState, busy: bool) {
    let page = &mut st.cc_page;
    for entry in ENTRIES {
        let value = match entry.kind {
            Kind::Switch(on) => Json::Bool(!on),
            Kind::Choice { options, .. } => options[options.len() - 1].1.json(),
        };
        let file = match entry.store {
            Store::Settings => &mut page.settings,
            Store::Global => &mut page.global,
        };
        cc_config::put(file.get_or_insert_with(|| Json::Object(Default::default())), entry.key, Some(value));
    }
    page.busy = busy;
}
