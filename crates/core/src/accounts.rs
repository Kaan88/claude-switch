//! The accounts: what the files say about each login, and adding, renaming and removing one.

use crate::util::{io_refusal, read_json, str_field};
use crate::{Account, AccountStatus, Config, Refusal, Roots, WorkersUse};
use std::path::PathBuf;

/// The colours given to accounts in order of adding; the first is the first account's.
pub(crate) const COLORS: [[u8; 3]; 8] = [
    [217, 119, 87],
    [74, 144, 226],
    [80, 167, 110],
    [155, 107, 196],
    [224, 169, 59],
    [64, 170, 170],
    [204, 87, 135],
    [130, 130, 140],
];

/// The `.claude.json` of an account: `<home>/.claude.json` for the first account, `<cli_dir>/.claude.json`
/// for any other.
pub fn claude_json(roots: &Roots, account: &Account) -> PathBuf {
    if account.primary { roots.home.join(".claude.json") } else { account.cli_dir.join(".claude.json") }
}

/// What the files say about the account's login. Never fails: whatever cannot be read is `false`/`None`.
/// Never opens `.credentials.json`; only asks whether it exists.
pub fn status(roots: &Roots, account: &Account) -> AccountStatus {
    let desktop = &account.desktop_dir;
    let desktop_used = desktop.join("claude-code-sessions").is_dir() || desktop.join("Local State").is_file();
    let mut status = AccountStatus { desktop_used, ..AccountStatus::default() };
    if !account.cli_dir.join(".credentials.json").exists() {
        return status;
    }
    let Some(json) = read_json(&claude_json(roots, account)) else { return status };
    let Some(oauth) = json.get("oauthAccount").filter(|v| v.is_object()) else { return status };
    let field = |k: &str| str_field(oauth, k).map(str::to_string);
    status.cli_logged_in = true;
    status.account_uuid = field("accountUuid");
    status.email = field("emailAddress");
    status.display_name = field("displayName");
    status.organization = field("organizationName");
    status.plan = field("planDisplayName");
    status
}

/// The longest a slug is before a number is appended, well inside what the settings file accepts.
const SLUG_BASE_MAX: usize = 48;

/// A slug for a new account's name: lower-case ASCII letters and digits, other runs become one `-`,
/// cut to 48 characters, trimmed, `"account"` when nothing is left, and `-2`, `-3`… appended until it
/// is not in `taken`.
pub fn slug_for(name: &str, taken: &[&str]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    // Only ASCII is in `base`, so any length is a character boundary.
    base.truncate(SLUG_BASE_MAX);
    while base.ends_with('-') {
        base.pop();
    }
    if base.is_empty() {
        base = "account".into();
    }
    if !taken.contains(&base.as_str()) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|s| !taken.contains(&s.as_str())).unwrap_or(base)
}

/// The names of the folders directly under the accounts folder, in lower case (the file system does
/// not tell cases apart). A removed account's folder is one of them and still holds its login.
fn account_folders(roots: &Roots) -> Result<Vec<String>, Refusal> {
    let dir = roots.accounts_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_refusal("read the folder", &dir, &e)),
    };
    Ok(entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect())
}

/// Adds an account: makes its slug (one that no account and no folder under `<accounts_dir>` has, so
/// a removed account's login is never picked up again), picks the next colour of a fixed list, creates
/// `<accounts_dir>/<slug>/cli` and `…/desktop`, sets up the shared links ([`crate::share::apply`]) and
/// pushes it onto `config.accounts`. Does not save the config. An empty name is [`Refusal::Invalid`].
pub fn add(roots: &Roots, config: &mut Config, name: &str) -> Result<Account, Refusal> {
    add_with_claim(roots, config, name, |path| std::fs::create_dir(path))
}

fn add_with_claim(
    roots: &Roots,
    config: &mut Config,
    name: &str,
    claim: impl Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<Account, Refusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Refusal::Invalid("An account needs a name.".into()));
    }
    let folders = account_folders(roots)?;
    let taken: Vec<&str> = config
        .accounts
        .iter()
        .map(|a| a.slug.as_str())
        .chain(folders.iter().map(String::as_str))
        .chain(["main"])
        .collect();
    let parent = roots.accounts_dir();
    std::fs::create_dir_all(&parent).map_err(|e| io_refusal("create the folder", &parent, &e))?;
    let mut collisions: Vec<String> = Vec::new();
    let (slug, base) = loop {
        let unavailable: Vec<_> = taken.iter().copied().chain(collisions.iter().map(String::as_str)).collect();
        let slug = slug_for(name, &unavailable);
        let base = parent.join(&slug);
        match claim(&base) {
            Ok(()) => break (slug, base),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => collisions.push(slug),
            Err(e) => return Err(io_refusal("claim the folder", &base, &e)),
        }
    };
    let account = Account {
        slug,
        name: name.to_string(),
        color: COLORS[config.accounts.len() % COLORS.len()],
        primary: false,
        cli_dir: base.join("cli"),
        desktop_dir: base.join("desktop"),
    };
    for dir in [&account.cli_dir, &account.desktop_dir] {
        std::fs::create_dir(dir).map_err(|e| io_refusal("create the folder", dir, &e))?;
    }
    // A link that cannot be made now is tried again before every launch; it does not stop the add.
    let _ = crate::share::apply(roots, &account);
    config.accounts.push(account.clone());
    Ok(account)
}

/// Changes what an account is called (never its slug). An empty name is [`Refusal::Invalid`].
pub fn rename(config: &mut Config, slug: &str, name: &str) -> Result<(), Refusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Refusal::Invalid("An account needs a name.".into()));
    }
    let account =
        config.accounts.iter_mut().find(|a| a.slug == slug).ok_or_else(|| Refusal::NoSuchAccount(slug.to_string()))?;
    account.name = name.to_string();
    Ok(())
}

/// Takes an account off the list. Its folders, and so its logins, stay on disk untouched. The first
/// account cannot be removed ([`Refusal::Invalid`]). `config.current` and `config.workers` fall back
/// to the first account / `Same` when they named it.
pub fn remove(config: &mut Config, slug: &str) -> Result<(), Refusal> {
    let index =
        config.accounts.iter().position(|a| a.slug == slug).ok_or_else(|| Refusal::NoSuchAccount(slug.to_string()))?;
    if config.accounts[index].primary {
        return Err(Refusal::Invalid("The first account cannot be removed.".into()));
    }
    config.accounts.remove(index);
    if config.current == slug {
        config.current = config.accounts.first().map_or_else(|| "main".to_string(), |a| a.slug.clone());
    }
    if config.workers == WorkersUse::Account(slug.to_string()) {
        config.workers = WorkersUse::Same;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, log_in, put};

    #[test]
    fn slugs_are_lower_case_with_single_dashes() {
        assert_eq!(slug_for("  Work Account!! 2 ", &[]), "work-account-2");
        assert_eq!(slug_for("Été", &[]), "t");
        assert_eq!(slug_for("***", &[]), "account");
        assert_eq!(slug_for("--a--b--", &[]), "a-b");
    }

    #[test]
    fn a_taken_slug_gets_a_number() {
        assert_eq!(slug_for("Work", &["work"]), "work-2");
        assert_eq!(slug_for("Work", &["work", "work-2"]), "work-3");
        assert_eq!(slug_for("Main", &["main"]), "main-2");
    }

    #[test]
    fn a_long_name_gives_a_slug_the_settings_file_keeps() {
        let long = "a".repeat(100);
        assert_eq!(slug_for(&long, &[]), "a".repeat(48));
        assert_eq!(slug_for(&long, &[&"a".repeat(48)]), format!("{}-2", "a".repeat(48)));
        let cut_at_a_dash = format!("{} tail", "b".repeat(47));
        assert_eq!(slug_for(&cut_at_a_dash, &[]), "b".repeat(47));
        let t = TempDir::new("acc-long");
        let roots = t.roots();
        let mut config = crate::config::load(&roots);
        add(&roots, &mut config, &long).unwrap();
        add(&roots, &mut config, &long).unwrap();
        crate::config::save(&roots, &config).unwrap();
        assert_eq!(crate::config::load(&roots).accounts, config.accounts);
    }

    #[test]
    fn a_removed_accounts_name_gets_a_fresh_folder() {
        let t = TempDir::new("acc-again");
        let roots = t.roots();
        let mut config = crate::config::load(&roots);
        let old = add(&roots, &mut config, "Work").unwrap();
        log_in(&roots, &old);
        remove(&mut config, "work").unwrap();
        let new = add(&roots, &mut config, "Work").unwrap();
        assert_eq!(new.slug, "work-2");
        assert!(!status(&roots, &new).cli_logged_in);
        assert!(status(&roots, &old).cli_logged_in, "the old login stays where it was");
        std::fs::create_dir_all(roots.accounts_dir().join("Home")).unwrap();
        assert_eq!(add(&roots, &mut config, "home").unwrap().slug, "home-2");
    }

    #[test]
    fn the_first_account_has_its_claude_json_beside_the_folder() {
        let t = TempDir::new("acc-json");
        let roots = t.roots();
        let config = crate::config::load(&roots);
        assert_eq!(claude_json(&roots, &config.accounts[0]), roots.home.join(".claude.json"));
        let other = crate::util::testing::further(&roots, "work", "Work");
        assert_eq!(claude_json(&roots, &other), other.cli_dir.join(".claude.json"));
    }

    #[test]
    fn a_login_needs_both_the_credentials_file_and_the_oauth_account() {
        let t = TempDir::new("acc-status");
        let roots = t.roots();
        let main = crate::config::load(&roots).accounts.remove(0);
        assert_eq!(status(&roots, &main), AccountStatus::default());
        put(&roots.home.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u-1"}}"#);
        assert!(!status(&roots, &main).cli_logged_in, "no credentials file yet");
        log_in(&roots, &main);
        let s = status(&roots, &main);
        assert!(s.cli_logged_in);
        assert_eq!(s.account_uuid.as_deref(), Some("u-1"));
        assert_eq!(s.display_name.as_deref(), Some("A"));
        assert!(s.email.is_some());
        assert!(!s.desktop_used);
        put(&roots.primary_desktop_dir().join("Local State"), "{}");
        assert!(status(&roots, &main).desktop_used);
    }

    #[test]
    fn further_account_status_reads_only_its_own_login_record() {
        let t = TempDir::new("acc-separate-status");
        let roots = t.roots();
        let config = crate::util::testing::config_with(&roots, &[("work", "Work")]);
        let [main, work] = config.accounts.as_slice() else { panic!("expected two accounts") };
        log_in(&roots, main);
        put(&work.cli_dir.join(".credentials.json"), "{}");
        assert!(!status(&roots, work).cli_logged_in, "it must not fall back to the primary login");
        put(&work.cli_dir.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"different","displayName":"Work"}}"#);
        assert_eq!(status(&roots, work).account_uuid.as_deref(), Some("different"));
        assert_eq!(status(&roots, work).display_name.as_deref(), Some("Work"));
        assert_eq!(status(&roots, main).account_uuid.as_deref(), Some("u-1"));
    }

    #[test]
    fn a_damaged_claude_json_is_not_a_login() {
        let t = TempDir::new("acc-damaged");
        let roots = t.roots();
        let main = crate::config::load(&roots).accounts.remove(0);
        put(&main.cli_dir.join(".credentials.json"), "{}");
        put(&roots.home.join(".claude.json"), "{not json");
        assert!(!status(&roots, &main).cli_logged_in);
    }

    #[test]
    fn adding_an_account_makes_its_folders_and_links() {
        let t = TempDir::new("acc-add");
        let roots = t.roots();
        std::fs::create_dir_all(roots.primary_cli_dir().join("projects")).unwrap();
        let mut config = crate::config::load(&roots);
        assert!(add(&roots, &mut config, "  ").is_err());
        let a = add(&roots, &mut config, "Work").unwrap();
        let b = add(&roots, &mut config, "Work").unwrap();
        assert_eq!((a.slug.as_str(), b.slug.as_str()), ("work", "work-2"));
        assert_ne!(a.color, b.color);
        assert!(a.cli_dir.is_dir() && a.desktop_dir.is_dir());
        assert_eq!(a.cli_dir, roots.accounts_dir().join("work").join("cli"));
        assert_eq!(config.accounts.len(), 3);
        let projects = crate::share::status(&roots, &a).into_iter().find(|s| s.name == "projects").unwrap();
        assert_eq!(projects.state, crate::share::LinkState::Linked);
    }

    #[test]
    fn account_slug_claim_skips_occupied_entries_not_in_the_folder_listing() {
        let t = TempDir::new("acc-claim");
        let roots = t.roots();
        put(&roots.accounts_dir().join("work"), "existing");
        put(&roots.accounts_dir().join("work-2"), "also existing");
        let mut config = crate::config::load(&roots);
        assert_eq!(add(&roots, &mut config, "Work").unwrap().slug, "work-3");
        assert_eq!(std::fs::read_to_string(roots.accounts_dir().join("work")).unwrap(), "existing");
    }

    #[test]
    fn simultaneous_same_name_adds_cannot_share_a_login_directory() {
        let t = TempDir::new("acc-concurrent-claim");
        let roots = t.roots();
        let barrier = std::sync::Barrier::new(2);
        let add = || {
            let mut config = crate::config::load(&roots);
            add_with_claim(&roots, &mut config, "Work", |path| {
                if path.ends_with("work") {
                    barrier.wait();
                }
                std::fs::create_dir(path)
            })
            .unwrap()
        };
        let mut accounts = std::thread::scope(|scope| {
            let a = scope.spawn(add);
            let b = scope.spawn(add);
            [a.join().unwrap(), b.join().unwrap()]
        });
        accounts.sort_by(|a, b| a.slug.cmp(&b.slug));
        assert_eq!(accounts.each_ref().map(|a| a.slug.as_str()), ["work", "work-2"]);
        assert_ne!(accounts[0].cli_dir, accounts[1].cli_dir);
        assert!(accounts.iter().all(|a| a.cli_dir.is_dir()));
    }

    #[test]
    fn renaming_keeps_the_slug() {
        let t = TempDir::new("acc-rename");
        let roots = t.roots();
        let mut config = crate::util::testing::config_with(&roots, &[("work", "Work")]);
        rename(&mut config, "work", " Job ").unwrap();
        assert_eq!((config.accounts[1].slug.as_str(), config.accounts[1].name.as_str()), ("work", "Job"));
        assert!(matches!(rename(&mut config, "work", ""), Err(Refusal::Invalid(_))));
        assert!(matches!(rename(&mut config, "nope", "X"), Err(Refusal::NoSuchAccount(_))));
    }

    #[test]
    fn removing_an_account_leaves_its_folders_and_resets_what_named_it() {
        let t = TempDir::new("acc-remove");
        let roots = t.roots();
        let mut config = crate::config::load(&roots);
        let a = add(&roots, &mut config, "Work").unwrap();
        config.current = "work".into();
        config.workers = WorkersUse::Account("work".into());
        assert!(matches!(remove(&mut config, "main"), Err(Refusal::Invalid(_))));
        remove(&mut config, "work").unwrap();
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.current, "main");
        assert_eq!(config.workers, WorkersUse::Same);
        assert!(a.cli_dir.is_dir());
        assert!(matches!(remove(&mut config, "work"), Err(Refusal::NoSuchAccount(_))));
    }
}
