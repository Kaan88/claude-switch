use crate::{Refusal, Roots};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub model: String,
    pub consents: BTreeSet<String>,
    pub default_agent: Option<String>,
    /// Handover instead of Claude Code's own compaction, in Claude Switch's tabs only.
    pub auto: AutoHandover,
    /// The context percentage that triggers it; `None` is where Claude Code would compact.
    pub auto_percent: Option<u8>,
    /// Where a handover continues: false (the default) is in place, in the tab that runs the
    /// session (`/clear`, then the handover as the first message); true opens a new tab.
    pub new_tab: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoHandover {
    #[default]
    Off,
    Ask,
    Automatic,
}

impl AutoHandover {
    pub const OPTIONS: [(Self, &'static str, &'static str); 3] =
        [(Self::Off, "off", "Off"), (Self::Ask, "ask", "Ask first"), (Self::Automatic, "automatic", "Automatic")];

    pub fn index(self) -> usize {
        Self::OPTIONS.iter().position(|(mode, _, _)| *mode == self).unwrap_or(0)
    }

    fn key(self) -> &'static str {
        Self::OPTIONS[self.index()].1
    }

    fn parse(key: &str) -> Self {
        Self::OPTIONS.iter().find(|(_, k, _)| *k == key).map_or(Self::Off, |(mode, _, _)| *mode)
    }
}

/// The range a set percentage may take: below it a handover would come too early to be useful,
/// above it Claude Code may run out of room for the summary turn.
pub const AUTO_PERCENT: std::ops::RangeInclusive<u8> = 40..=95;

impl Preferences {
    pub(crate) fn read(value: &Value) -> Self {
        Self {
            model: value["model"].as_str().filter(|m| provider(m).is_some()).unwrap_or_default().into(),
            consents: value["consents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| valid_component(id))
                .map(str::to_owned)
                .collect(),
            default_agent: value["defaultAgent"]
                .as_str()
                .filter(|name| crate::launch::session_agent_args(name).is_ok())
                .map(str::to_owned),
            auto: value["auto"]["mode"].as_str().map_or(AutoHandover::Off, AutoHandover::parse),
            auto_percent: value["auto"]["percent"]
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| AUTO_PERCENT.contains(n)),
            new_tab: value["continueIn"].as_str() == Some("newTab"),
        }
    }

    pub(crate) fn json(&self) -> Value {
        json!({
            "model": self.model,
            "consents": self.consents,
            "defaultAgent": self.default_agent,
            "auto": {"mode": self.auto.key(), "percent": self.auto_percent},
            "continueIn": if self.new_tab { "newTab" } else { "thisTab" },
        })
    }

    pub fn set_default_agent(&mut self, name: &str) -> Result<(), Refusal> {
        let name = name.trim();
        if name.is_empty() {
            self.default_agent = None;
        } else {
            crate::launch::session_agent_args(name)?;
            self.default_agent = Some(name.to_owned());
        }
        Ok(())
    }

    pub fn agent_for<'a>(&'a self, explicit: Option<&'a str>) -> Option<&'a str> {
        explicit.or(self.default_agent.as_deref())
    }
}

fn valid_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub fn provider(model: &str) -> Option<&str> {
    let (id, model) = model.split_once('/')?;
    (valid_component(id) && !model.is_empty() && model.len() <= 250 && model.split(['/', '#']).all(valid_component))
        .then_some(id)
}

fn directory(roots: &Roots, create: bool) -> io::Result<PathBuf> {
    let parent = roots.config_file().parent().expect("config parent").to_owned();
    let dir = parent.join("handover");
    if create {
        fs::create_dir_all(&parent)?;
        let meta = fs::symlink_metadata(&parent)?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(io::Error::other("The app data folder is not an ordinary directory"));
        }
        match fs::create_dir(&dir) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
    }
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(io::Error::other("The handover folder is not an ordinary directory"));
    }
    Ok(dir)
}

fn reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    meta.file_type().is_symlink()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn list(roots: &Roots, id: &str) -> io::Result<Vec<PathBuf>> {
    if !valid_id(id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid session id"));
    }
    let dir = match directory(roots, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let prefix = format!("{id}-");
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(stamp) = name.to_str().and_then(|s| s.strip_prefix(&prefix)).and_then(|s| s.strip_suffix(".md"))
        else {
            continue;
        };
        if stamp.len() == 20 && stamp.bytes().all(|b| b.is_ascii_digit()) {
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.is_file() && !reparse(&meta) {
                files.push(entry.path());
            }
        }
    }
    files.sort_unstable_by(|a, b| b.cmp(a));
    Ok(files)
}

pub fn save(roots: &Roots, id: &str, text: &str) -> io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid session id"));
    }
    let dir = directory(roots, true)?;
    let _lock = crate::util::lock_beside(&dir.join("retention")).map_err(io::Error::other)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    for bump in 0..100 {
        let path = dir.join(format!("{id}-{:020}.md", stamp + bump));
        match fs::symlink_metadata(&path) {
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        export(&path, text)?;
        for old in list(roots, id)?.into_iter().filter(|p| p != &path).skip(19) {
            if let Err(e) = fs::remove_file(old)
                && e.kind() != io::ErrorKind::NotFound
            {
                return Err(e);
            }
        }
        return Ok(path);
    }
    Err(io::Error::other(format!("Could not allocate a handover in {}", Path::display(&dir))))
}

/// Publish a new Markdown file atomically; never replace any existing path.
pub fn export(path: &Path, text: &str) -> io::Result<()> {
    crate::util::write_new_atomic(path, text.as_bytes()).map_err(io::Error::other)
}
