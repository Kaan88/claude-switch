//! What a further account shares with the first: links in its CLI folder, and the MCP server list.
//!
//! A shared file is a hard link, which a program ends by saving the file as a new one under the old
//! name. While the two files still hold the same bytes [`apply`] links them again; a diverged file is
//! the owner's to settle.

use crate::util::{io_refusal, publish_new, read_json};
use crate::{Account, Refusal, Roots};
use serde_json::Value;
use std::path::Path;

/// The names shared by link, in order: `projects`, `skills`, `agents`, `plugins`, `mcp-servers`
/// (folders), `settings.json`, `CLAUDE.md`, `keybindings.json` (files).
pub const SHARED: [&str; 8] =
    ["projects", "skills", "agents", "plugins", "mcp-servers", "settings.json", "CLAUDE.md", "keybindings.json"];

/// How one shared name stands in an account's CLI folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// It is a link to (or, for a file, the same file as) the first account's.
    Linked,
    /// The first account has no such folder or file, so there is nothing to share.
    NoSource,
    /// Nothing is there yet; [`apply`] will make the link.
    Missing,
    /// Something of the account's own is there (a real folder with content, a different file). Nothing
    /// of it is lost: only a file with the very bytes of the first account's is linked again by
    /// [`apply`]; the owner decides about everything else.
    Conflict,
}

/// One row of [`status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareState {
    /// One of [`SHARED`].
    pub name: &'static str,
    /// How it stands.
    pub state: LinkState,
}

/// Whether two paths name the same file on disk (a hard link counts).
#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    fn identity(path: &Path) -> Option<(u32, u32, u32)> {
        let file = std::fs::File::open(path).ok()?;
        // SAFETY: the struct is plain data, so all zeroes is a valid value to be overwritten.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle belongs to `file`, which is open for the whole call, and `info` is a
        // live struct of the type the call writes.
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0;
        ok.then_some((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    }
    matches!((identity(a), identity(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(not(any(windows, unix)))]
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// Whether `dir` is a real folder (not a link) with nothing in it.
fn is_empty_real_dir(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir())
        && std::fs::read_dir(dir).is_ok_and(|mut rd| rd.next().is_none())
}

fn state_of(source: &Path, target: &Path) -> LinkState {
    let Ok(src) = std::fs::metadata(source) else { return LinkState::NoSource };
    let Ok(dst) = std::fs::symlink_metadata(target) else { return LinkState::Missing };
    if src.is_dir() {
        let same = matches!(
            (std::fs::canonicalize(source), std::fs::canonicalize(target)),
            (Ok(a), Ok(b)) if a == b
        );
        if same {
            LinkState::Linked
        } else if is_empty_real_dir(target) {
            LinkState::Missing
        } else {
            LinkState::Conflict
        }
    } else if !dst.file_type().is_dir() && same_file(source, target) {
        // The same file: a hard link, or a file link that leads to it.
        LinkState::Linked
    } else if is_empty_real_dir(target) {
        LinkState::Missing
    } else {
        // A different file, a folder with content, or a link elsewhere: the owner's, left alone.
        LinkState::Conflict
    }
}

/// How each shared name stands for the account. For the first account every row is `Linked`.
pub fn status(roots: &Roots, account: &Account) -> Vec<ShareState> {
    let primary = roots.primary_cli_dir();
    SHARED
        .iter()
        .map(|&name| ShareState {
            name,
            state: if account.primary {
                LinkState::Linked
            } else {
                state_of(&primary.join(name), &account.cli_dir.join(name))
            },
        })
        .collect()
}

/// Makes a directory junction `link` pointing at `target`.
#[cfg(windows)]
#[path = "share/junction.rs"]
pub(crate) mod junction;

#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) -> Result<(), Refusal> {
    junction::create(link, target).map_err(|e| io_refusal("link", link, &e))
}

#[cfg(unix)]
fn make_junction(link: &Path, target: &Path) -> Result<(), Refusal> {
    std::os::unix::fs::symlink(target, link).map_err(|e| io_refusal("link", link, &e))
}

#[cfg(not(any(windows, unix)))]
fn make_junction(link: &Path, _target: &Path) -> Result<(), Refusal> {
    Err(io_refusal("link", link, &"links are not supported here"))
}

/// Makes every `Missing` link (directory junctions for folders; hard links for files, and an empty
/// real folder in the way is removed first) and returns the states afterwards. An existing copy
/// is left alone: Claude does not participate in our locks. An error making one
/// link does not stop the others; the first such error is returned after all were tried.
pub fn apply(roots: &Roots, account: &Account) -> Result<Vec<ShareState>, Refusal> {
    if account.primary {
        return Ok(status(roots, account));
    }
    let primary = roots.primary_cli_dir();
    let mut first_error = None;
    let mut note = |r: Result<(), Refusal>| {
        if let Err(e) = r
            && first_error.is_none()
        {
            first_error = Some(e);
        }
    };
    for row in status(roots, account) {
        let source = primary.join(row.name);
        let target = account.cli_dir.join(row.name);
        if row.state != LinkState::Missing {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&account.cli_dir) {
            note(Err(io_refusal("create the folder", &account.cli_dir, &e)));
            continue;
        }
        if is_empty_real_dir(&target) {
            // `remove_dir` removes only an empty folder, so nothing of the owner's can go here.
            if let Err(e) = std::fs::remove_dir(&target) {
                note(Err(io_refusal("remove the empty folder", &target, &e)));
                continue;
            }
        }
        if source.is_dir() {
            note(make_junction(&target, &source));
        } else {
            note(std::fs::hard_link(&source, &target).map_err(|e| io_refusal("link", &target, &e)));
        }
    }
    let states = status(roots, account);
    match first_error {
        Some(e) => Err(e),
        None => Ok(states),
    }
}

/// Seeds `mcpServers` before the secondary profile has its own `.claude.json`. Never replaces
/// an existing file: Claude might change it while we work and does not honor our locks.
pub fn sync_mcp(roots: &Roots, account: &Account) -> Result<bool, Refusal> {
    if account.primary {
        return Ok(false);
    }
    let Some(servers) =
        read_json(&roots.home.join(".claude.json")).and_then(|mut v| v.get_mut("mcpServers").map(Value::take))
    else {
        return Ok(false);
    };
    let path = crate::accounts::claude_json(roots, account);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_refusal("inspect", &path, &e)),
    }
    let text = format!("{}\n", serde_json::json!({"mcpServers": servers}));
    publish_new(&path, text.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, further, put};

    fn states(rows: &[ShareState]) -> Vec<(&'static str, LinkState)> {
        rows.iter().map(|r| (r.name, r.state)).collect()
    }

    #[test]
    fn the_first_account_is_always_linked() {
        let t = TempDir::new("sh-primary");
        let roots = t.roots();
        let main = crate::config::load(&roots).accounts.remove(0);
        assert!(status(&roots, &main).iter().all(|r| r.state == LinkState::Linked));
        assert!(apply(&roots, &main).unwrap().iter().all(|r| r.state == LinkState::Linked));
        assert!(!roots.primary_cli_dir().exists(), "nothing is made for the first account");
    }

    #[test]
    fn apply_makes_junctions_and_hard_links_and_leaves_a_conflict() {
        let t = TempDir::new("sh-apply");
        let roots = t.roots();
        let p = roots.primary_cli_dir();
        put(&p.join("projects").join("C--x").join("s.jsonl"), "{}");
        std::fs::create_dir_all(p.join("skills")).unwrap();
        put(&p.join("agents").join("a.md"), "primary agent");
        put(&p.join("settings.json"), r#"{"theme":"dark"}"#);
        put(&p.join("CLAUDE.md"), "primary notes");
        let work = further(&roots, "work", "Work");
        // The account's own things: a folder with content and a different file; an empty folder.
        put(&work.cli_dir.join("agents").join("mine.md"), "my agent");
        put(&work.cli_dir.join("CLAUDE.md"), "my notes");
        std::fs::create_dir_all(work.cli_dir.join("skills")).unwrap();
        let before = status(&roots, &work);
        assert_eq!(
            states(&before),
            [
                ("projects", LinkState::Missing),
                ("skills", LinkState::Missing),
                ("agents", LinkState::Conflict),
                ("plugins", LinkState::NoSource),
                ("mcp-servers", LinkState::NoSource),
                ("settings.json", LinkState::Missing),
                ("CLAUDE.md", LinkState::Conflict),
                ("keybindings.json", LinkState::NoSource),
            ]
        );
        let after = apply(&roots, &work).unwrap();
        let expected: Vec<_> = states(&before)
            .into_iter()
            .map(|(n, s)| (n, if s == LinkState::Missing { LinkState::Linked } else { s }))
            .collect();
        assert_eq!(states(&after), expected);
        assert!(work.cli_dir.join("projects").join("C--x").join("s.jsonl").is_file());
        assert!(std::fs::symlink_metadata(work.cli_dir.join("projects")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(work.cli_dir.join("agents").join("mine.md")).unwrap(), "my agent");
        assert!(!work.cli_dir.join("agents").join("a.md").exists());
        assert_eq!(std::fs::read_to_string(work.cli_dir.join("CLAUDE.md")).unwrap(), "my notes");
        // The hard link is the same file: a change through it shows in the first account's.
        std::fs::write(work.cli_dir.join("settings.json"), r#"{"theme":"light"}"#).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("settings.json")).unwrap(), r#"{"theme":"light"}"#);
        // Doing it again changes nothing.
        assert_eq!(states(&apply(&roots, &work).unwrap()), expected);
    }

    #[test]
    fn existing_settings_copies_are_never_replaced_even_when_identical() {
        let t = TempDir::new("sh-relink");
        let roots = t.roots();
        let p = roots.primary_cli_dir();
        put(&p.join("settings.json"), "{\"a\":1}");
        put(&p.join("CLAUDE.md"), "notes one");
        let big = "k".repeat(1024);
        put(&p.join("keybindings.json"), &big);
        put(&p.join("skills").join("s.md"), "skill");
        let work = further(&roots, "work", "Work");
        // Copies of the account's own: identical and diverged files, and a nonempty directory.
        put(&work.cli_dir.join("settings.json"), "{\"a\":1}");
        put(&work.cli_dir.join("CLAUDE.md"), "notes two");
        put(&work.cli_dir.join("keybindings.json"), &big);
        put(&work.cli_dir.join("skills").join("s.md"), "skill");
        let state = |rows: &[ShareState], name: &str| rows.iter().find(|r| r.name == name).unwrap().state;
        assert_eq!(state(&status(&roots, &work), "settings.json"), LinkState::Conflict);
        let after = apply(&roots, &work).unwrap();
        assert_eq!(state(&after, "settings.json"), LinkState::Conflict);
        assert_eq!(state(&after, "CLAUDE.md"), LinkState::Conflict);
        assert_eq!(state(&after, "keybindings.json"), LinkState::Conflict);
        assert_eq!(state(&after, "skills"), LinkState::Conflict);
        assert_eq!(std::fs::read_to_string(work.cli_dir.join("CLAUDE.md")).unwrap(), "notes two");
        assert_eq!(std::fs::read_to_string(p.join("CLAUDE.md")).unwrap(), "notes one");
        std::fs::write(work.cli_dir.join("settings.json"), "{\"a\":2}").unwrap();
        assert_eq!(std::fs::read_to_string(p.join("settings.json")).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn provisioning_does_not_overwrite_a_file_created_since_the_absence_check() {
        let t = TempDir::new("sh-putback");
        let target = t.0.join("settings.json");
        put(&target, "mine");
        let refused = publish_new(&target, b"new");
        assert!(matches!(refused, Err(Refusal::Io(_))));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
    }

    #[test]
    fn a_junction_to_somewhere_else_is_a_conflict() {
        let t = TempDir::new("sh-elsewhere");
        let roots = t.roots();
        std::fs::create_dir_all(roots.primary_cli_dir().join("skills")).unwrap();
        let elsewhere = t.0.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let work = further(&roots, "work", "Work");
        std::fs::create_dir_all(&work.cli_dir).unwrap();
        make_junction(&work.cli_dir.join("skills"), &elsewhere).unwrap();
        let row = status(&roots, &work).into_iter().find(|r| r.name == "skills").unwrap();
        assert_eq!(row.state, LinkState::Conflict);
        apply(&roots, &work).unwrap();
        assert_eq!(
            std::fs::canonicalize(work.cli_dir.join("skills")).unwrap(),
            std::fs::canonicalize(&elsewhere).unwrap()
        );
    }

    #[test]
    fn sync_mcp_changes_only_mcp_servers() {
        let t = TempDir::new("sh-mcp");
        let roots = t.roots();
        let work = further(&roots, "work", "Work");
        let main = crate::config::load(&roots).accounts.remove(0);
        assert_eq!(sync_mcp(&roots, &work), Ok(false), "the first account has no file yet");
        put(&roots.home.join(".claude.json"), r#"{"numStartups":3,"mcpServers":{"a":{"command":"run-a"}}}"#);
        assert_eq!(sync_mcp(&roots, &main), Ok(false));
        assert_eq!(sync_mcp(&roots, &work), Ok(true));
        let v = read_json(&work.cli_dir.join(".claude.json")).unwrap();
        assert_eq!(v, serde_json::json!({"mcpServers":{"a":{"command":"run-a"}}}));
        put(
            &work.cli_dir.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"u"},"mcpServers":{"old":{}},"projects":{"C:/x":{"n":1}}}"#,
        );
        assert_eq!(sync_mcp(&roots, &work), Ok(false));
        let v = read_json(&work.cli_dir.join(".claude.json")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"oauthAccount":{"accountUuid":"u"},"mcpServers":{"old":{}},
                               "projects":{"C:/x":{"n":1}}})
        );
        assert_eq!(sync_mcp(&roots, &work), Ok(false), "already the same");
        put(&work.cli_dir.join(".claude.json"), "[]");
        assert_eq!(sync_mcp(&roots, &work), Ok(false));
        assert_eq!(std::fs::read_to_string(work.cli_dir.join(".claude.json")).unwrap(), "[]");
        // A first account's file without the key changes nothing.
        put(&roots.home.join(".claude.json"), r#"{"numStartups":3}"#);
        put(&work.cli_dir.join(".claude.json"), r#"{"mcpServers":{"own":{}}}"#);
        assert_eq!(sync_mcp(&roots, &work), Ok(false));
    }
}
