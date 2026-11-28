//! Made-up data for `--demo`: two accounts, hundreds of sessions in several folders, two of them
//! running. It stands in for every call of [`crate::source::Source`] so the window can be seen
//! without accessing real accounts or starting a process. The live view uses temporary fixtures.

#[path = "view/demo.rs"]
mod transcript;
pub use transcript::Transcript;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use claude_switch_core::sessions::PreviewLine;
use claude_switch_core::share::{LinkState, SHARED, ShareState};
use claude_switch_core::{
    Account, AccountStatus, App, Config, Launch, Place, Refusal, Running, Session, WorkersUse, running,
};

use crate::source::Scan;

const FOLDERS: [(&str, &str); 7] = [
    (r"C:\Users\demo\Desktop\trail-planner", "main"),
    (r"C:\Users\demo\Desktop\weather-station", "main"),
    (r"C:\Users\demo\Desktop\pixel-forge", "feature/export"),
    (r"C:\Users\demo\Desktop\recipe-box", "main"),
    (r"C:\Users\demo\Desktop\tide-tables", "dev"),
    (r"C:\Users\demo\Documents\notes", "master"),
    (r"C:\Users\demo\Desktop\garden-sensors", "main"),
];

const TITLES: [&str; 24] = [
    "Fix the notification that comes back after a swipe",
    "Add the account dropdown to the top bar",
    "Why does the release build take four minutes",
    "Write tests for the splitter drag",
    "Move the map tiles to a worker thread",
    "Sort out the CRLF noise in git status",
    "Review the upload path for double sends",
    "Make the search field fuzzy",
    "Draft the README for the API client",
    "Track down the crash on resume",
    "Clean up the old worktrees",
    "Plan the offline mode",
    "Show the rain radar as a map layer",
    "Speed up the sensor dashboard",
    "Export a sprite sheet with its JSON atlas",
    "Scale a recipe without rounding to odd amounts",
    "Read the tide station feed every ten minutes",
    "Merge my meeting notes into one weekly page",
    "Calibrate the soil moisture sensors",
    "Find out why the backup job skipped Sunday",
    "Add keyboard shortcuts to the editor",
    "Rename the config keys without breaking old files",
    "Profile the start-up time",
    "Write the release notes for 2.4",
];

/// Each title's last prompt, in the same order.
const PROMPTS: [&str; 24] = [
    "Can you show me why it comes back after the second swipe?",
    "Keep the account picker compact at smaller window sizes.",
    "Compare a clean build with an incremental build and report the difference.",
    "Add a regression test for dragging past the minimum width.",
    "Keep the main thread free while the tiles decode.",
    "Make git stop reporting every file as changed after a checkout.",
    "Check that a retried upload can never be sent twice.",
    "Let \"trl plnr\" find \"trail planner\", like the command palette does.",
    "Write the quick start first, then one example for each endpoint.",
    "It crashes when I resume a session from yesterday. Find out why.",
    "List the worktrees with no unmerged work and remove them.",
    "Sketch how the planner should work with no network for a weekend.",
    "Draw the last two hours of radar under the route, with a time slider.",
    "The dashboard takes three seconds to draw a week of readings. Make it fast.",
    "Pack the frames tightly and write the atlas next to the image.",
    "Halving a recipe should give 1 egg, not 0.5 eggs.",
    "Poll the station every ten minutes and keep a day of readings.",
    "Put this week's notes on one page, newest first, with the action items on top.",
    "The readings drift after a week. Add a calibration step to the setup.",
    "The log says the job started on Sunday but nothing was written. Why?",
    "Ctrl+B and Ctrl+I for bold and italic, and a list of shortcuts in Help.",
    "Old config files must still load after the rename.",
    "Measure what happens between the click and the first frame.",
    "Summarise what changed since 2.3 for people who use the app.",
];

fn now_ms() -> u64 {
    static BASE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *BASE.get_or_init(|| SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
}

fn account(slug: &str, name: &str, color: [u8; 3], primary: bool) -> Account {
    let base = PathBuf::from(r"C:\Users\demo");
    Account {
        slug: slug.into(),
        name: name.into(),
        color,
        primary,
        cli_dir: if primary { base.join(".claude") } else { base.join(".claude-accounts").join(slug).join("cli") },
        desktop_dir: base.join(".claude-accounts").join(slug).join("desktop"),
    }
}

pub fn config() -> Config {
    Config {
        accounts: vec![account("main", "Main", [217, 119, 87], true), account("work", "Work", [72, 133, 237], false)],
        current: "main".into(),
        workers: WorkersUse::Same,
        embedded_terminal: true,
        classic_terminal_view: true,
        show_archived: false,
        show_workers: false,
        show_usage: true,
        notifications: Default::default(),
        show_hidden: false,
        sessions_mode: Default::default(),
        group_by_folder: false,
        show_sittings: false,
        suggest_topics: false,
        settings_share: Default::default(),
        terminal_scheme: "auto".into(),
        terminal_font_size: claude_switch_core::config::TERMINAL_FONT_DEFAULT,
        window_theme: "system".into(),
        appearance: Default::default(),
        chat: Default::default(),
        compact: Default::default(),
        meaning: Default::default(),
    }
}

/// The `i`th newest session's title and last prompt: the newest from [`TITLES`], older ones a
/// task on some part of the projects, so few share a title.
fn title_and_prompt(i: usize) -> (String, String) {
    const VERBS: [&str; 8] =
        ["Fix", "Test", "Document", "Speed up", "Review", "Clean up", "Simplify", "Add logging to"];
    const PARTS: [&str; 15] = [
        "the route export",
        "the map tile cache",
        "the sensor dashboard",
        "the sprite packer",
        "the recipe scaler",
        "the tide feed",
        "the notes index",
        "the backup job",
        "the settings page",
        "the login flow",
        "the upload queue",
        "the search box",
        "the offline sync",
        "the chart legend",
        "the photo importer",
    ];
    const ASKS: [&str; 4] = [
        "Start with the part that fails.",
        "Keep the change small.",
        "Tell me the plan before you edit anything.",
        "Run the tests when you are done.",
    ];
    if i < TITLES.len() {
        return (TITLES[i].into(), PROMPTS[i].into());
    }
    let title = format!("{} {}", VERBS[i % VERBS.len()], PARTS[(i / VERBS.len()) % PARTS.len()]);
    let prompt = format!("{title}. {}", ASKS[i % ASKS.len()]);
    (title, prompt)
}

fn sessions() -> Vec<Session> {
    let now = now_ms();
    let mut out = Vec::new();
    for i in 0..360u64 {
        let (cwd, branch) = FOLDERS[(i as usize * 5 + i as usize / 3) % FOLDERS.len()];
        let age_min = i * i * 60_480 / (359 * 359) + i.min(20) * 3 + i.saturating_sub(280) * 1_000;
        let at = now.saturating_sub(age_min * 60_000);
        let (title, last_prompt) = title_and_prompt(i as usize);
        let desktop = i % 4 == 1;
        out.push(Session {
            id: format!("{:08x}-1c2d-4e5f-8a9b-{:012x}", 0x5eed_0000 + i * 7919, i * 104_729),
            title: title.clone(),
            last_prompt,
            grouping: claude_switch_core::grouping::Transcript {
                first_active: Some(at.saturating_sub(14 * 60_000)),
                first_prompt: Some(title),
                parent: None,
            },
            turn_ended: i % 3 == 1,
            transcript_ms: at,
            cwd: PathBuf::from(cwd),
            branch: (i % 6 != 5).then(|| branch.to_string()),
            last_active_ms: at,
            bytes: (i * 7_919 % 97 + 1) * 180_000,
            last_app: if desktop { App::Desktop } else { App::Cli },
            desktop_id: desktop.then(|| format!("local_{i:04}")),
            desktop_account: desktop.then(|| if i % 8 == 1 { "work" } else { "main" }.to_string()),
            archived: i % 13 == 12,
            worker: i % 9 == 4,
            file: PathBuf::from(format!(r"C:\Users\demo\.claude\projects\demo\{i}.jsonl")),
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.last_active_ms));
    out
}

pub fn running() -> Vec<Running> {
    let list = sessions();
    let cli = &list[0];
    let desk = list.iter().find(|s| s.last_app == App::Desktop).unwrap_or(&list[1]);
    vec![
        Running {
            session_id: cli.id.clone(),
            pid: 18_244,
            app: App::Cli,
            account: "main".into(),
            cwd: cli.cwd.clone(),
        },
        Running {
            session_id: desk.id.clone(),
            pid: 22_910,
            app: App::Desktop,
            account: "work".into(),
            cwd: desk.cwd.clone(),
        },
    ]
}

pub fn scan(c: &Config) -> Scan {
    let statuses = c
        .accounts
        .iter()
        .map(|a| {
            let signed = a.slug == "main" || a.slug == "work";
            let status = AccountStatus {
                cli_logged_in: signed,
                desktop_used: signed,
                account_uuid: None,
                email: signed.then(|| format!("{}@example.com", a.slug)),
                display_name: signed.then(|| "Demo".to_string()),
                organization: None,
                plan: signed.then(|| if a.primary { "Max" } else { "Pro" }.to_string()),
            };
            (a.slug.clone(), status)
        })
        .collect();
    let mut sessions = sessions();
    let main = PathBuf::from(FOLDERS[0].0);
    sessions[3].cwd = main.join("src");
    sessions[6].cwd = main.with_file_name("trail-planner-offline");
    sessions[6].branch = Some("offline".into());
    let projects = sessions
        .iter()
        .map(|s| {
            use claude_switch_core::{grouping::Project, sessions::folder_key};
            let mut p = Project::folder(&s.cwd);
            p.branch = FOLDERS
                .iter()
                .find(|(folder, _)| s.cwd == std::path::Path::new(folder))
                .map(|(_, branch)| (*branch).to_owned());
            if s.cwd.starts_with(&main) || s.cwd == main.with_file_name("trail-planner-offline") {
                p = Project::folder(&main);
                p.branch = Some("main".into());
                p.location = match s.cwd.file_name().and_then(|s| s.to_str()) {
                    Some("src") => Some("src".into()),
                    Some("trail-planner-offline") => Some("Worktree: trail-planner-offline".into()),
                    _ => None,
                };
            }
            (folder_key(&s.cwd), p)
        })
        .collect();
    Scan { sessions, running: running(), statuses, projects, ..Scan::default() }.with_related()
}

pub fn seed_marks(marks: &mut claude_switch_core::marks::Marks, sessions: &[Session]) {
    for (i, s) in sessions.iter().enumerate() {
        if i < 32 {
            let topics = ["Interface", "Build checks", "Session navigation"];
            marks.set_topic(&s.id, &s.file, topics[i % topics.len()]);
        }
        if let Some(parent) = match i {
            0 => sessions.get(3),
            3 => sessions.get(6),
            6 => sessions.get(9),
            _ => None,
        } {
            marks.set_parent(&s.id, &parent.id);
            if i == 0 {
                marks.set_handover_from(&s.id, &parent.title);
            }
        }
        if i == 2 {
            marks.set_folder_default(&claude_switch_core::sessions::folder_key(&s.cwd), Some(("work", App::Cli)));
        }
        if matches!(i, 80 | 156 | 292) {
            marks.archive(&s.id, &s.file, Some(now_ms()));
        }
        if matches!(i, 2 | 5 | 18) {
            marks.pin(&s.id, &s.file, true);
        }
        if i == 5 {
            marks.rename(&s.id, &s.file, "Quiet the CRLF noise");
        }
        if i % 3 == 0 {
            marks.see(&s.id, &s.file, s.transcript_ms);
        }
    }
}

pub fn preview(s: &Session, max: usize) -> Vec<PreviewLine> {
    let lines = [
        (true, format!("{}. Start with the part that fails.", s.title)),
        (false, "I read the module and the failing test. The state is reset before the scan ends, so the second call sees an empty list. I moved the reset after the scan and the test passes.".to_string()),
        (true, "Good. Does anything else call it that way?".to_string()),
        (false, "Two other places do. Both read the list after the scan has finished, so they are not affected. I left them as they are.".to_string()),
        (true, "Run the whole suite once more, then stop.".to_string()),
        (false, "All tests pass. Nothing is committed.".to_string()),
    ];
    let skip = lines.len().saturating_sub(max);
    lines.into_iter().skip(skip).map(|(from_user, text)| PreviewLine { from_user, text }).collect()
}

pub fn last_account(s: &Session) -> String {
    s.desktop_account.clone().unwrap_or_else(|| "main".into())
}

pub fn carry_warning(s: &Session, account: &str) -> Option<String> {
    (last_account(s) != account).then(|| {
        format!(
            "This conversation continues, but the account \"{account}\" has nothing of it cached. Its first message re-reads all {} of it at full price.",
            mdshared_app::fmt::size(s.bytes)
        )
    })
}

pub fn launch(account: &str, what: &str) -> Launch {
    Launch {
        program: "claude".into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
        env_remove: Vec::new(),
        place: Place::Terminal,
        label: format!("{account} · {what}"),
    }
}

pub fn resume(s: &Session, account: &str, app: App, run: &[Running]) -> Result<Launch, Refusal> {
    if let Some(r) = running::find(run, &s.id) {
        return Err(Refusal::AlreadyRunning(r.clone()));
    }
    let mut l = launch(account, "Resume");
    l.args = match app {
        App::Cli => vec!["--resume".into(), s.id.clone()],
        App::Desktop => vec!["--desktop".into(), "--resume".into(), s.id.clone()],
    };
    l.place = if app == App::Cli { Place::Terminal } else { Place::Detached };
    Ok(l)
}

pub fn branch(new_id: &str, account: &str) -> Launch {
    let mut l = launch(account, "Branch");
    l.args = vec!["--resume".into(), new_id.into()];
    l
}

pub fn fork(s: &Session, account: &str) -> Launch {
    let mut l = launch(account, "Copy");
    l.args = vec!["--resume".into(), s.id.clone(), "--fork-session".into()];
    l
}

pub fn share_status(a: &Account) -> Vec<ShareState> {
    SHARED
        .iter()
        .map(|&name| ShareState {
            name,
            state: match (a.primary, name) {
                (true, _) => LinkState::Linked,
                (false, "keybindings.json") => LinkState::NoSource,
                (false, "CLAUDE.md") => LinkState::Conflict,
                (false, "agents") => LinkState::Missing,
                _ => LinkState::Linked,
            },
        })
        .collect()
}

pub fn add_account(c: &mut Config, name: &str) -> Result<Account, Refusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Refusal::Invalid("An account needs a name.".into()));
    }
    let slug: String =
        name.to_lowercase().chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect();
    let a = account(&format!("{slug}-{}", c.accounts.len()), name, [46, 160, 110], false);
    c.accounts.push(a.clone());
    Ok(a)
}

pub fn rename(c: &mut Config, slug: &str, name: &str) -> Result<(), Refusal> {
    if name.trim().is_empty() {
        return Err(Refusal::Invalid("An account needs a name.".into()));
    }
    let a = c.accounts.iter_mut().find(|a| a.slug == slug).ok_or_else(|| Refusal::NoSuchAccount(slug.into()))?;
    a.name = name.trim().into();
    Ok(())
}

pub fn remove(c: &mut Config, slug: &str) -> Result<(), Refusal> {
    match c.accounts.iter().position(|a| a.slug == slug) {
        Some(0) => Err(Refusal::Invalid("The first account cannot be removed.".into())),
        Some(i) => {
            c.accounts.remove(i);
            if c.current == slug {
                c.current = c.accounts[0].slug.clone();
            }
            Ok(())
        }
        None => Err(Refusal::NoSuchAccount(slug.into())),
    }
}
