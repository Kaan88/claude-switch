//! Local command discovery. No CLI process, plugin code, prompt body or network lookup.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use serde_json::Value;

pub(super) const MODELS: [&str; 6] = ["default", "sonnet", "opus", "haiku", "opusplan", "fable"];
pub(super) const EFFORT: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub(crate) fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.split(':').all(|part| {
            part.starts_with(|c: char| c.is_ascii_alphanumeric())
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

pub(super) fn confirm_name(line: &str) -> Option<&'static str> {
    let name = line.strip_prefix('/')?.split_ascii_whitespace().next()?;
    if name.eq_ignore_ascii_case("compact") {
        Some("Compact")
    } else if ["clear", "reset", "new"].iter().any(|s| name.eq_ignore_ascii_case(s)) {
        Some("Clear")
    } else {
        None
    }
}

#[derive(Debug)]
pub(super) struct Command {
    pub name: String,
    pub description: String,
}

// A local reference catalog, not a claim about the current CLI version, plan or policy.
const BUILTINS: &str = include_str!("builtins.txt");

pub(super) struct Picker {
    pub open: bool,
    pub selected: usize,
    pub moved: bool,
    pub catalog: Vec<Command>,
    loading: Option<Receiver<Vec<Command>>>,
    requested: bool,
}

impl Default for Picker {
    fn default() -> Self {
        let catalog = BUILTINS
            .lines()
            .filter_map(|line| line.split_once('|'))
            .map(|(name, description)| Command { name: name.into(), description: description.into() })
            .collect();
        Self { open: false, selected: 0, moved: false, catalog, loading: None, requested: false }
    }
}

impl Picker {
    pub fn reset(&mut self) {
        self.selected = 0;
        self.moved = false;
    }

    pub fn load(&mut self, account: Option<PathBuf>, primary: Option<PathBuf>, cwd: PathBuf) {
        if !self.requested {
            self.requested = true;
            let (tx, rx) = mpsc::channel();
            if std::thread::Builder::new()
                .name("chat-commands".into())
                .spawn(move || {
                    let _ = tx.send(discover(account.as_deref(), primary.as_deref(), &cwd));
                })
                .is_ok()
            {
                self.loading = Some(rx);
            }
        }
    }

    pub fn collect(&mut self) -> bool {
        let Some(rx) = &self.loading else { return false };
        match rx.try_recv() {
            Ok(commands) => {
                self.catalog.extend(commands.into_iter().filter(|cmd| {
                    !BUILTINS.lines().any(|l| l.split_once('|').is_some_and(|(name, _)| name == cmd.name))
                }));
                self.reset();
                self.loading = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.loading = None;
                true
            }
        }
    }

    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    pub fn matches(&self, draft: &str, pins: &[String]) -> Vec<usize> {
        let query = query(draft).to_ascii_lowercase();
        let mut list: Vec<_> =
            self.catalog.iter().enumerate().filter_map(|(i, cmd)| cmd.name.starts_with(&query).then_some(i)).collect();
        list.sort_unstable_by(|&a, &b| {
            let a = &self.catalog[a].name;
            let b = &self.catalog[b].name;
            (!pins.contains(a), a).cmp(&(!pins.contains(b), b))
        });
        list
    }

    pub fn chosen(&self, draft: &str, pins: &[String]) -> Option<usize> {
        let list = self.matches(draft, pins);
        list.get(self.selected.min(list.len().saturating_sub(1))).copied()
    }

    pub fn step(&mut self, draft: &str, pins: &[String], down: bool) {
        let len = self.matches(draft, pins).len();
        if len > 0 {
            self.selected = if !self.moved && down {
                0
            } else if down {
                (self.selected + 1) % len
            } else {
                (self.selected + len - 1) % len
            };
            self.moved = true;
        }
    }
}

pub(super) fn query(draft: &str) -> &str {
    draft.strip_prefix('/').map_or("", |s| s.split_ascii_whitespace().next().unwrap_or(""))
}

pub(super) fn fill(name: &str, draft: &str) -> String {
    let suffix = draft.strip_prefix('/').and_then(|s| s.find(char::is_whitespace).map(|i| &s[i..])).unwrap_or(" ");
    format!("/{name}{suffix}")
}

pub(super) fn choices(name: &str) -> &'static [&'static str] {
    match name {
        "model" => &MODELS,
        "effort" => &EFFORT,
        _ => &[],
    }
}

const MAX_ENTRIES: usize = 460;
const MAX_PLUGIN_BYTES: u64 = 512 * 1024;
const MAX_HEADER: u64 = 8192;

fn fixed(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, QueryDosDeviceW};
        let Some(Component::Prefix(prefix)) = path.components().next() else { return false };
        let Prefix::Disk(letter) = prefix.kind() else { return false };
        let name = [u16::from(letter), u16::from(b':'), 0];
        let root = [name[0], name[1], u16::from(b'\\'), 0];
        let mut device = [0; 512];
        // SAFETY: both name and root are NUL-terminated and the output buffer is writable.
        if unsafe { QueryDosDeviceW(name.as_ptr(), device.as_mut_ptr(), device.len() as u32) } == 0 {
            return false;
        }
        let end = device.iter().position(|&c| c == 0).unwrap_or(device.len());
        let direct = String::from_utf16_lossy(&device[..end])
            .strip_prefix(r"\Device\HarddiskVolume")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        direct && unsafe { GetDriveTypeW(root.as_ptr()) } == 3
    }
    #[cfg(not(windows))]
    true
}

fn plain_meta(path: &Path) -> Option<fs::Metadata> {
    let meta = fs::symlink_metadata(path).ok()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    (!meta.file_type().is_symlink()).then_some(meta)
}

fn plain_root(path: &Path) -> bool {
    if !path.is_absolute() || !fixed(path) {
        return false;
    }
    let mut partial = PathBuf::new();
    for part in path.components() {
        if matches!(part, Component::ParentDir) {
            return false;
        }
        if let Component::Normal(name) = part
            && name.to_str().is_none_or(|s| s.ends_with(['.', ' ']) || s.contains(':'))
        {
            return false;
        }
        partial.push(part.as_os_str());
        if matches!(part, Component::Normal(_)) && !plain_meta(&partial).is_some_and(|m| m.is_dir()) {
            return false;
        }
    }
    true
}

fn shared_dir(account: &Path, primary: Option<&Path>, name: &str) -> PathBuf {
    let path = account.join(name);
    if let Some(primary) = primary.filter(|p| *p != account)
        && let Ok(target) = fs::read_link(&path)
        && let Some(t) = target.to_str()
        && primary.join(name).to_str().is_some_and(|p| p.eq_ignore_ascii_case(t.trim_start_matches(r"\\?\")))
    {
        return primary.join(name);
    }
    path
}

fn valid_dir(path: &Path) -> bool {
    plain_meta(path).is_some_and(|m| m.is_dir())
}

fn read_header(path: &Path) -> Option<(Option<String>, String, bool)> {
    let file = BufReader::new(File::open(path).ok()?.take(MAX_HEADER));
    let mut lines = file.lines().map_while(Result::ok);
    if lines.next()?.trim_start_matches('\u{feff}').trim() != "---" {
        return Some((None, String::new(), true));
    }
    let mut name = None;
    let mut description = String::new();
    let mut visible = true;
    let mut description_block = false;
    for line in lines.take(64) {
        if line.trim() == "---" {
            return Some((name, description, visible));
        }
        if description_block && line.starts_with(' ') && !line.trim().is_empty() {
            description = one_line(line.trim());
            description_block = false;
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        description_block = false;
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(['\'', '"']);
        match key {
            "name" if valid_name(value) => name = Some(value.to_ascii_lowercase()),
            "description" => {
                description_block = matches!(value, "" | "|" | ">" | ">-" | "|-" | ">+" | "|+");
                description = if description_block { String::new() } else { one_line(value) };
            }
            "user-invocable" if value == "false" => visible = false,
            _ => {}
        }
    }
    None
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(140).collect()
}

fn read_json(path: &Path) -> Option<Value> {
    if !plain_meta(path).is_some_and(|m| m.is_file()) {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(MAX_PLUGIN_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_PLUGIN_BYTES).then(|| serde_json::from_slice(&bytes).ok()).flatten()
}

fn plugin_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut out = root.to_owned();
    for part in Path::new(relative).components() {
        match part {
            Component::CurDir => (),
            Component::Normal(s) if s.to_str().is_some_and(|s| !s.contains(':') && !s.ends_with([' ', '.'])) => {
                out.push(s)
            }
            _ => return None,
        }
    }
    if out == root || !plain_root(out.parent()?) {
        return None;
    }
    Some(out)
}

struct Discovery {
    commands: BTreeMap<String, String>,
    entries: usize,
}

impl Discovery {
    fn scan(&mut self, dir: &Path, namespace: &str, skills: bool, depth: usize) {
        if !valid_dir(dir) || self.entries == 0 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.take(self.entries).flatten() {
            if self.entries == 0 {
                break;
            }
            self.entries -= 1;
            let path = entry.path();
            let Some(kind) = plain_meta(&path) else { continue };
            let name = if kind.is_dir() { path.file_name() } else { path.file_stem() };
            let Some(name) = name.and_then(|s| s.to_str()).filter(|s| valid_name(s)) else { continue };
            if skills && kind.is_dir() {
                let file = path.join("SKILL.md");
                self.add(&file, namespace, name, "Skill", true);
            } else if !skills && kind.is_dir() && depth != 0 {
                self.scan(&path, &format!("{namespace}{name}:"), false, depth - 1);
            } else if !skills && kind.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
                self.add(&path, namespace, name, "Custom command", false);
            }
        }
    }

    fn add(&mut self, path: &Path, namespace: &str, stem: &str, label: &str, named: bool) {
        if !plain_meta(path).is_some_and(|m| m.is_file()) {
            return;
        }
        let Some((name, description, visible)) = read_header(path) else { return };
        // A file without frontmatter is still callable; only an explicit user-invocable:false hides it.
        if !visible {
            return;
        }
        let name = if named { name.as_deref().unwrap_or(stem) } else { stem };
        let name = if !namespace.is_empty() && name.starts_with(namespace) {
            name.to_owned()
        } else {
            format!("{namespace}{name}")
        };
        let name = name.to_ascii_lowercase();
        if valid_name(&name) {
            self.commands
                .entry(name)
                .or_insert_with(|| if description.is_empty() { label.into() } else { description });
        }
    }

    fn plugin(&mut self, root: &Path, name: &str) {
        let ns = format!("{name}:");
        let metadata = root.join(".claude-plugin");
        let manifest =
            valid_dir(&metadata).then(|| read_json(&metadata.join("plugin.json"))).flatten().unwrap_or_default();
        self.scan(&root.join("skills"), &ns, true, 0);
        if manifest["commands"].is_null() {
            self.scan(&root.join("commands"), &ns, false, 3);
        }
        for (field, skills) in [("commands", false), ("skills", true)] {
            let value = &manifest[field];
            let specs = value.as_array().map_or(std::slice::from_ref(value), Vec::as_slice);
            for relative in specs.iter().take(32).filter_map(Value::as_str) {
                let Some(path) = plugin_path(root, relative) else { continue };
                if valid_dir(&path) {
                    if skills && plain_meta(&path.join("SKILL.md")).is_some_and(|m| m.is_file()) {
                        let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        self.add(&path.join("SKILL.md"), &ns, stem, "Plugin skill", true);
                    } else {
                        self.scan(&path, &ns, skills, 3);
                    }
                } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    self.add(&path, &ns, stem, "Plugin command", skills);
                }
            }
        }
        if let Some(commands) = manifest["commands"].as_object() {
            for (name, spec) in commands.iter().take(32).filter(|(name, _)| valid_name(name)) {
                let id = format!("{ns}{name}").to_ascii_lowercase();
                if let Some(relative) = spec["source"].as_str() {
                    if let Some(path) = plugin_path(root, relative)
                        && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                    {
                        self.add(&path, &ns, name, "Plugin command", false);
                    }
                } else if spec["content"].is_string() && spec["user-invocable"].as_bool() != Some(false) {
                    let desc = spec["description"].as_str().map_or_else(|| "Plugin command".into(), one_line);
                    self.commands.entry(id).or_insert(desc);
                }
            }
        }
        if manifest["skills"].is_null() && !valid_dir(&root.join("skills")) {
            self.add(&root.join("SKILL.md"), &ns, name, "Plugin skill", true);
        }
    }

    fn plugins(&mut self, root: &Path, alias: &Path, cwd: &Path) {
        let cache = root.join("cache");
        let alias = alias.join("cache");
        if !valid_dir(&cache) {
            return;
        }
        let Some(data) = read_json(&root.join("installed_plugins.json")) else { return };
        let Some(plugins) = data["plugins"].as_object() else { return };
        for (key, versions) in plugins.iter().take(80) {
            let Some((name, _)) = key.split_once('@').filter(|(name, _)| valid_name(name)) else { continue };
            let Some(versions) = versions.as_array() else { continue };
            for install in versions.iter().take(8) {
                if self.entries == 0 {
                    return;
                }
                self.entries -= 1;
                if matches!(install["scope"].as_str(), Some("project" | "local"))
                    && !install["projectPath"].as_str().is_some_and(|p| cwd.starts_with(p))
                {
                    continue;
                }
                let Some(path) = install["installPath"].as_str().map(Path::new) else { continue };
                let Ok(relative) = path.strip_prefix(&cache).or_else(|_| path.strip_prefix(&alias)) else { continue };
                let path = cache.join(relative);
                if path != cache && plain_root(&path) {
                    self.plugin(&path, name);
                }
            }
        }
    }
}

fn discover(account: Option<&Path>, primary: Option<&Path>, cwd: &Path) -> Vec<Command> {
    let mut scan = Discovery { commands: BTreeMap::new(), entries: MAX_ENTRIES };
    if let Some(account) = account.filter(|p| plain_root(p)) {
        let skills = shared_dir(account, primary, "skills");
        if plain_root(&skills) {
            scan.scan(&skills, "", true, 0);
        }
        scan.scan(&account.join("commands"), "", false, 3);
    }
    if plain_root(cwd) {
        // Without a repository boundary use just the launch directory, never unrelated parents.
        let root = cwd.ancestors().take(12).find(|p| plain_meta(&p.join(".git")).is_some()).unwrap_or(cwd);
        for dir in cwd.ancestors().take_while(|p| p.starts_with(root)) {
            let project = dir.join(".claude");
            if valid_dir(&project) {
                scan.scan(&project.join("skills"), "", true, 0);
                scan.scan(&project.join("commands"), "", false, 3);
            }
        }
    }
    if let Some(account) = account.filter(|p| plain_root(p)) {
        let root = shared_dir(account, primary, "plugins");
        if plain_root(&root) {
            scan.plugins(&root, &account.join("plugins"), cwd);
        }
    }
    scan.commands.into_iter().map(|(name, description)| Command { name, description }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_puts_pins_first_and_keeps_arguments_when_completing() {
        let mut p = Picker::default();
        let pins = ["model".into()];
        let matches = p.matches("/mo", &pins);
        assert_eq!(p.catalog[matches[0]].name, "model");
        assert_eq!(fill("effort", "/eff high"), "/effort high");
        p.step("/mo", &pins, true);
        assert_eq!(p.chosen("/mo", &pins), Some(matches[0]));
        p.step("/mo", &pins, false);
        assert_eq!(p.chosen("/mo", &pins), matches.last().copied());
        assert_eq!(confirm_name("/reset name"), Some("Clear"));
        assert_eq!(confirm_name("/clear-ish"), None);
        assert!(!valid_name("good:bad/evil"));
    }

    #[test]
    fn reads_fixture_headers_and_names_not_bodies_or_outside_registrations() {
        let root = std::env::temp_dir().join(format!("chat-commands-{}", std::process::id()));
        let account = root.join("account");
        let cwd = root.join("project");
        let plug = account.join("plugins");
        let cache = plug.join("cache/example/review/1.0");
        for dir in [account.join("skills/review"), cwd.join(".claude/commands/team"), cache.join("commands")] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(
            account.join("skills/review/SKILL.md"),
            "---\nname: review-me\ndescription: One line\n---\nSecret body",
        )
        .unwrap();
        fs::write(cwd.join(".claude/commands/team/check.md"), "No frontmatter; don't show this").unwrap();
        fs::write(cache.join("commands/run.md"), "---\nuser-invocable: true\n---\nCustom body").unwrap();
        fs::write(cache.join("commands/private.md"), "---\nuser-invocable: false\n---").unwrap();
        let extra = plug.join("cache/example/extra/1.0");
        fs::create_dir_all(extra.join(".claude-plugin")).unwrap();
        fs::write(extra.join("info.md"), "---\ndescription: >-\n  First line\n  Not shown\n---").unwrap();
        let manifest = serde_json::json!({"commands": {
            "inline": {"description": "Short info\nNot shown", "content": "Not returned or executed"},
            "from-file": {"source": "./info.md"},
            "outside": {"source": "../../outside.md"}
        }});
        fs::write(extra.join(".claude-plugin/plugin.json"), manifest.to_string()).unwrap();
        let data = serde_json::json!({"plugins": {"review@example": [{"installPath": cache}],
            "extra@example": [{"installPath": extra}], "other-project@example": [{
                "installPath": extra, "scope": "local", "projectPath": root.join("somewhere-else")
            }],
            "outside@example": [{"installPath": cwd}]}});
        fs::write(plug.join("installed_plugins.json"), data.to_string()).unwrap();
        let found =
            discover(Some(&account), None, &cwd).into_iter().map(|c| (c.name, c.description)).collect::<Vec<_>>();
        assert_eq!(
            found,
            [
                ("extra:from-file".into(), "First line".into()),
                ("extra:inline".into(), "Short info".into()),
                ("review-me".into(), "One line".into()),
                ("review:run".into(), "Custom command".into()),
                ("team:check".into(), "Custom command".into())
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
