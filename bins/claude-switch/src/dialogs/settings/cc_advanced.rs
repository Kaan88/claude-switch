//! Settings › Claude Code › Advanced: the rest of Claude Code's `/config` (the table and the
//! careful write are `claude_switch_core::cc_config`). `settings.json` entries are saved in the
//! master account and shared, as on the Claude Code page; `.claude.json` entries belong to one
//! account, chosen on the page, and are read-only while it has a running Claude Code.
//!
//! The page reads both files when it opens, after each change and on "Check again"; a change is
//! compared with what the page showed, so a value changed elsewhere meanwhile is never
//! overwritten. Reads and saves run off the window thread; a change while one runs is not taken.
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

/// What the page read from the files.
#[derive(Default)]
pub(crate) struct Snapshot {
    settings: Option<Json>,
    global: Option<Json>,
    settings_error: Option<String>,
    global_error: Option<String>,
    /// The chosen account may run Claude Code: its rows are read-only.
    busy: bool,
}

/// What a background job of the page gives: what it read, after a save if there was one.
pub(crate) struct Done {
    saved: Option<Result<cc_config::Saved, claude_switch_core::Refusal>>,
    read: Snapshot,
}

/// What the page read, its search and account, and its file work in the background.
#[derive(Default)]
pub(crate) struct Page {
    pub(super) search: String,
    /// The account whose `.claude.json` the page shows; the master when unset or gone.
    account: Option<String>,
    read: Snapshot,
    /// A read, or a save then a read, off the window thread.
    job: Option<mdshared_app::job::Job<Done>>,
    /// Read again when the job ends: the account changed, or Check again, meanwhile.
    read_again: bool,
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

/// A program the app started for `account` runs in one of its tabs (it may not have written its
/// session record yet).
fn tab_running(st: &AppState, account: &Account) -> bool {
    st.tabs.list.iter().any(|t| t.opencode.is_none() && t.account.as_deref() == Some(&account.slug) && t.term.running())
}

/// Both files, and whether the chosen account may run Claude Code. Off the window thread.
fn read_files(
    roots: &claude_switch_core::Roots,
    config: &claude_switch_core::Config,
    chosen: Option<&Account>,
    tab_running: bool,
) -> Snapshot {
    let read = |account: Option<&Account>, store| match account {
        Some(a) => match cc_config::read(&cc_config::file(roots, a, store)) {
            Ok(v) => (v, None),
            Err(e) => (None, Some(e.to_string())),
        },
        None => (None, None),
    };
    let master = claude_switch_core::settings_sync::master_account(config, &config.settings_share);
    let (settings, settings_error) = read(master, Store::Settings);
    let (global, global_error) = read(chosen, Store::Global);
    let busy = chosen.is_some_and(|a| cc_config::refuse_if_running(roots, config, a, tab_running).is_err());
    Snapshot { settings, global, settings_error, global_error, busy }
}

/// A save (the entry, the value the page showed, the new one), if any, then a read, in the
/// background; [`take_job`] shows the result. With a job running, a read waits for it; a save
/// never gets here then ([`apply`]).
fn start(st: &mut AppState, save: Option<(&'static Entry, Option<Json>, Option<Json>)>) {
    let crate::source::Source::Real(roots) = &st.source else { return };
    if st.cc_page.job.is_some() {
        st.cc_page.read_again = true;
        return;
    }
    let (roots, config) = (roots.clone(), st.config.clone());
    let chosen = chosen(st).cloned();
    let tab = chosen.as_ref().is_some_and(|a| tab_running(st, a));
    let job = mdshared_app::job::Job::start(st.wake_fn(), move |_, _| {
        let saved = save.map(|(entry, expected, value)| match &chosen {
            Some(account) => cc_config::save(&roots, &config, account, entry, expected.as_ref(), value, tab),
            None => Err(claude_switch_core::Refusal::Invalid("There is no account.".into())),
        });
        Done { saved, read: read_files(&roots, &config, chosen.as_ref(), tab) }
    });
    match job {
        Ok(job) => st.cc_page.job = Some(job),
        Err(e) => st.error("Claude Code's settings were not read", e.to_string()),
    }
}

/// Reads both files again, in the background.
pub(super) fn load(st: &mut AppState) {
    start(st, None);
}

/// Shows what the page's job did, once it ended. True when there was something.
pub(crate) fn take_job(st: &mut AppState) -> bool {
    if !st.cc_page.job.as_ref().is_some_and(|job| !job.running()) {
        return false;
    }
    let Some(done) = st.cc_page.job.take().and_then(|job| job.take()) else {
        st.error("Claude Code's settings", "The worker stopped.".into());
        return true;
    };
    st.cc_page.read = done.read;
    match done.saved {
        Some(Err(e)) => st.error("Claude Code's setting was not saved", e.to_string()),
        Some(Ok(saved)) => {
            if let Some(e) = saved.sync_error {
                let text = format!("Saved in the master account; bringing the other accounts in line failed: {e}");
                st.warn("Not shared yet", text);
            } else if saved.stale_lock {
                let text = "Claude Code's lock .claude.json.lock was over a minute old, left by a Claude Code                             that ended; it was taken over.";
                st.note("Saved", text.into());
            }
        }
        None => {}
    }
    if std::mem::take(&mut st.cc_page.read_again) {
        load(st);
    }
    refresh(st);
    true
}

/// Waits for the page's jobs and shows their results.
#[cfg(test)]
pub(super) fn wait(st: &mut AppState) {
    while st.cc_page.job.is_some() {
        while st.cc_page.job.as_ref().is_some_and(|job| job.running()) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        take_job(st);
    }
}

fn file(st: &AppState, store: Store) -> Option<&Json> {
    match store {
        Store::Settings => st.cc_page.read.settings.as_ref(),
        Store::Global => st.cc_page.read.global.as_ref(),
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
        || [entry.label, entry.cli_label, entry.key, entry.help, entry.when().unwrap_or_default()]
            .iter()
            .any(|t| t.to_lowercase().contains(&search))
}

fn entry_rows(st: &AppState, i: usize, entry: &Entry, read_only: Option<&str>) -> Vec<Row> {
    let id = FIRST + 2 * i as u32;
    let saved = saved(st, entry);
    if let Some(why) = read_only {
        return vec![Row::choice(id, entry.label, &[shown(entry, saved).as_str()], 0).help(why)];
    }
    let help = match entry.when() {
        Some(when) => format!("{} {when}", entry.help),
        None => entry.help.to_owned(),
    };
    let help = if saved.is_none() { format!("{help} · default") } else { help };
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
        if page.read.settings_error.is_some() {
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
        let account_help = if page.read.busy {
            format!("{name} runs Claude Code: read-only.")
        } else {
            "Only this account's file changes.".to_owned()
        };
        rows.push(Row::choice(ACCOUNT, "Account", &names, index).help(account_help));
        if page.read.global_error.is_some() {
            rows.push(Row::action(RECHECK, ".claude.json is not valid JSON", "Check again"));
        } else {
            if page.read.busy {
                rows.push(Row::action(RECHECK, "Closed its sessions?", "Check again"));
            }
            let read_only = page.read.busy.then_some(why_busy.as_str());
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
    if entry.store == Store::Global && st.cc_page.read.busy || st.cc_page.job.is_some() {
        // Read-only rows, or the files are being read or saved: the row shows its value again.
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
    if st.source.is_demo() {
        let page = &mut st.cc_page.read;
        let file = match entry.store {
            Store::Settings => &mut page.settings,
            Store::Global => &mut page.global,
        };
        cc_config::put(file.get_or_insert_with(|| Json::Object(Default::default())), entry.key, new);
        return refresh(st);
    }
    // Saved in the background against the value the page showed; the page shows the result.
    start(st, Some((entry, saved, new)));
}

/// Every entry set to a value other than its default, as a page with Reset rows looks; `busy`
/// makes the per-account rows read-only.
#[cfg(test)]
pub(super) fn fill_for_pictures(st: &mut AppState, busy: bool) {
    let page = &mut st.cc_page.read;
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
