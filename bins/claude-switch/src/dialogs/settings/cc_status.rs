//! Settings › Claude Code › Status: what `/status` and `/stats` show that can be read from the
//! account's files without running Claude Code (`claude_switch_core::cc_status`). Usage has its
//! own page; this one links to it. Read when the page opens and when another account is chosen.
use super::{AppState, Dialog, Row};
use claude_switch_core::cc_status::{self, MODEL_SESSIONS, SignIn, Stats, Status};
use mdshared_ui::settings::Value;

pub(crate) const STATUS: &str = "Claude Code › Status";
pub(super) const ACCOUNT: u32 = 600;
pub(super) const OPEN_USAGE: u32 = 601;
/// The read-only rows, numbered from here in the order shown.
pub(super) const INFO: u32 = 602;
const LAST: u32 = 640;
/// MCP servers listed by name; the rest are counted.
const MCP_ROWS: usize = 8;

#[derive(Default)]
pub(crate) struct Read {
    status: Option<Status>,
    stats: Option<Stats>,
    /// The read, off the window thread; a newer request waits for it (`again`).
    job: Option<mdshared_app::job::Job<(Status, Stats)>>,
    again: bool,
}

pub(super) fn owns(id: u32) -> bool {
    (ACCOUNT..LAST).contains(&id)
}

/// Reads the chosen account's status in the background ([`take_job`] shows it).
pub(super) fn load(st: &mut AppState) {
    match (&st.source, super::cc_advanced::chosen(st)) {
        (crate::source::Source::Real(roots), Some(account)) => {
            if st.cc_status.job.is_some() {
                st.cc_status.again = true;
                return;
            }
            let files: std::collections::HashMap<String, std::path::PathBuf> =
                st.sessions.iter().map(|s| (s.id.clone(), s.file.clone())).collect();
            let (roots, account) = (roots.clone(), account.clone());
            let job = mdshared_app::job::Job::start(st.wake_fn(), move |_, _| {
                let offset = claude_switch_core::clock::local_offset_minutes();
                let status = cc_status::status(&roots, &account);
                (status, cc_status::stats(&account, offset, |id| files.get(id).cloned()))
            });
            match job {
                Ok(job) => st.cc_status.job = Some(job),
                Err(e) => st.error("Claude Code's status was not read", e.to_string()),
            }
        }
        (crate::source::Source::Demo, _) => demo(st),
        _ => st.cc_status = Read::default(),
    }
}

/// Shows the status once its read ended. True when there was one.
pub(crate) fn take_job(st: &mut AppState) -> bool {
    if !st.cc_status.job.as_ref().is_some_and(|job| !job.running()) {
        return false;
    }
    if let Some((status, stats)) = st.cc_status.job.take().and_then(|job| job.take()) {
        (st.cc_status.status, st.cc_status.stats) = (Some(status), Some(stats));
    }
    if std::mem::take(&mut st.cc_status.again) {
        load(st);
    }
    refresh(st);
    true
}

fn demo(st: &mut AppState) {
    st.cc_status = Read {
        status: Some(Status {
            installed: Some(claude_switch_core::cc_config::VERSION.into()),
            sign_in: SignIn::ClaudeAccount,
            email: Some("owner@example.com".into()),
            organization: Some("Example".into()),
            plan: Some("Max".into()),
            user_settings: Some(Ok(6)),
            managed: None,
            mcp: vec![("files".into(), "stdio".into()), ("docs".into(), "http".into())],
            memory: Some(42),
        }),
        stats: Some(Stats {
            prompts: 1280,
            sessions: 96,
            days: 31,
            folders: 12,
            partial: false,
            models: vec![("claude-opus-5-5".into(), 30), ("claude-sonnet-5-5".into(), 9)],
            model_sessions: MODEL_SESSIONS,
        }),
        ..Read::default()
    };
}

pub(super) fn rows(st: &AppState) -> Vec<Row> {
    let mut next = INFO;
    let mut info = |label: &str, value: &str, help: &str| {
        let row = Row::choice(next, label, &[value], 0);
        next += 1;
        if help.is_empty() { row } else { row.help(help) }
    };
    let names: Vec<&str> = st.config.accounts.iter().map(|a| a.name.as_str()).collect();
    let account = super::cc_advanced::chosen(st);
    let index = account.and_then(|a| st.config.accounts.iter().position(|b| b.slug == a.slug)).unwrap_or(0);
    let mut rows = vec![Row::choice(ACCOUNT, "Account", &names, index).help("Read from its files; nothing runs.")];
    let Some(status) = &st.cc_status.status else { return rows };
    let made_for = claude_switch_core::cc_config::VERSION;
    rows.push(Row::header("Claude Code"));
    let version = status.installed.as_deref().unwrap_or("Not in ~/.local");
    rows.push(info("Installed version", version, &format!("Advanced was made for {made_for}.")));
    rows.push(Row::header("Account"));
    rows.push(info("Sign-in method", status.sign_in.label(), ""));
    if let Some(email) = &status.email {
        rows.push(info("Signed in as", email, ""));
    }
    if let Some(org) = &status.organization {
        rows.push(info("Organization", org, ""));
    }
    if let Some(plan) = &status.plan {
        rows.push(info("Plan", plan, ""));
    }
    rows.push(Row::header("Settings in effect"));
    let user = match status.user_settings {
        None => "None".to_owned(),
        Some(Ok(n)) => format!("{n} keys"),
        Some(Err(())) => "Not valid JSON".to_owned(),
    };
    rows.push(info("User settings.json", &user, "Each folder can add its own .claude settings."));
    let managed = if status.managed.is_some() { "managed-settings.json" } else { "None" };
    rows.push(info("Managed settings", managed, "Set for the whole computer; they win."));
    let memory = status.memory.map_or_else(|| "None".to_owned(), |n| format!("CLAUDE.md, {n} lines"));
    rows.push(info("User memory", &memory, "Each folder can add its own CLAUDE.md."));
    rows.push(Row::header("MCP servers (user scope)"));
    if status.mcp.is_empty() {
        rows.push(info("Servers", "None", "Plugins and folders can add more."));
    }
    for (name, kind) in status.mcp.iter().take(MCP_ROWS) {
        rows.push(info(name, kind, ""));
    }
    if status.mcp.len() > MCP_ROWS {
        rows.push(info("More", &format!("{}", status.mcp.len() - MCP_ROWS), ""));
    }
    if let Some(stats) = &st.cc_status.stats {
        rows.push(Row::header(if stats.partial { "Stats (newest 8 MiB of prompts)" } else { "Stats" }));
        rows.push(info("Prompts", &stats.prompts.to_string(), "Typed in this account."));
        rows.push(info("Sessions", &stats.sessions.to_string(), ""));
        rows.push(info("Days active", &stats.days.to_string(), ""));
        rows.push(info("Folders", &stats.folders.to_string(), ""));
        let models = &stats.models;
        let help = format!("In the newest {} sessions.", stats.model_sessions);
        if models.is_empty() {
            rows.push(info("Models used", "None found", &help));
        }
        for (i, (model, sessions)) in models.iter().take(4).enumerate() {
            let label = if i == 0 { "Models used" } else { "" };
            let row = info(label, &format!("{model} ({sessions})"), if i == 0 { &help } else { "" });
            rows.push(row);
        }
    }
    rows.push(Row::header("Usage"));
    rows.push(Row::action(OPEN_USAGE, "Plan usage is on the Usage page", "Open"));
    rows
}

fn refresh(st: &mut AppState) {
    let rows = rows(st);
    if let Some(Dialog::Settings(settings)) = &mut st.dialog
        && settings.category() == STATUS
    {
        settings.form.set_rows(rows);
    }
}

/// A change on the page: another account, or the link to Usage (handled by the caller).
pub(super) fn apply(st: &mut AppState, id: u32, value: Option<Value>) {
    if let (ACCOUNT, Some(Value::Choice(i))) = (id, value) {
        super::cc_advanced::set_account(st, i);
        load(st);
        super::cc_advanced::load(st);
    }
    refresh(st);
}
