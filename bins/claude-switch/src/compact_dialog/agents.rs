//! Read-only agent catalog. Keep only names and descriptions; never load a prompt or run Claude.
use claude_switch_core::grouping::local::Volume;
use claude_switch_core::launch::session_agent_args;
use claude_switch_core::{Account, Roots};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

mod plugins;

const FRONTMATTER_BYTES: u64 = 16 * 1024;
const SOURCE_ENTRIES: usize = 400;
const MAX_DEPTH: usize = 6;
pub(super) const DEFAULT: &str = "Default (Claude)";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Agent {
    pub name: String,
    pub description: String,
}

/// Select from the actual target account and launch folder, not the transcript's origin account.
pub(crate) fn discover(roots: &Roots, target: &Account, cwd: &Path, cancelled: impl Fn() -> bool) -> Vec<Agent> {
    if cancelled() {
        return Vec::new();
    }
    let project = project_folder(cwd);
    let mut agents = plugins::discover(roots, target, &project, cwd, &cancelled);
    if let Some(dir) = shared_folder(roots, target, "agents") {
        agents.extend(scan(&dir, None, &cancelled));
    }
    agents.extend(scan(&project.join(".claude/agents"), None, &cancelled));
    if project != cwd {
        agents.extend(scan(&cwd.join(".claude/agents"), None, &cancelled));
    }
    agents.into_values().collect()
}

fn project_folder(cwd: &Path) -> PathBuf {
    if let Some(volume) = Volume::at(cwd)
        && let Some(cwd) = volume.folder(cwd)
    {
        for candidate in cwd.ancestors().take(16) {
            match volume.metadata(&candidate.join(".git")) {
                Ok(meta) if meta.is_file() || meta.is_dir() => return candidate.into(),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                _ => break,
            }
        }
    }
    cwd.into()
}

/// Secondary accounts can have a Switch-provisioned junction to Main. Compare its destination
/// as text and open only the expected path, not the link. Don't guess if the target is different.
fn shared_folder(roots: &Roots, target: &Account, name: &str) -> Option<PathBuf> {
    let cli = &target.cli_dir;
    let own = cli.join(name);
    let volume = Volume::at(cli)?;
    if volume.folder(&own).is_some() {
        if target.primary || volume.entries(&own, 1).map_or(true, |names| !names.is_empty()) {
            return Some(own);
        }
    } else if target.primary {
        return None;
    }
    let primary = roots.primary_cli_dir().join(name);
    if volume.folder(cli).is_some() {
        match std::fs::read_link(&own) {
            Ok(path) if link_matches(&path, &primary) => {}
            Ok(_) => return None,
            Err(_) if volume.folder(&own).is_some() => {} // An empty directory will be provisioned.
            Err(_) if std::fs::symlink_metadata(&own).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(_) => return None,
        }
    } else if volume.metadata(cli).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        // A missing account directory can be provisioned at launch, without mutating it here.
    } else {
        return None;
    }
    Volume::at(&primary)?.folder(&primary)
}

fn link_matches(path: &Path, expected: &Path) -> bool {
    #[cfg(windows)]
    {
        let Some(path) = path.to_str() else { return false };
        let path = path.strip_prefix(r"\\?\").or_else(|| path.strip_prefix(r"\??\")).unwrap_or(path);
        expected.to_str().is_some_and(|expected| path.eq_ignore_ascii_case(expected))
    }
    #[cfg(not(windows))]
    {
        path == expected
    }
}

fn scan(root: &Path, namespace: Option<&str>, cancelled: &impl Fn() -> bool) -> BTreeMap<String, Agent> {
    let Some(volume) = Volume::at(root) else { return BTreeMap::new() };
    let mut found = BTreeMap::new();
    let mut pending = vec![(root.to_owned(), namespace.map(str::to_owned), 0)];
    let mut remaining = SOURCE_ENTRIES;
    while let Some((dir, scope, depth)) = pending.pop() {
        if remaining == 0 || cancelled() {
            break;
        }
        let Ok(mut names) = volume.entries(&dir, remaining) else { continue };
        remaining -= names.len();
        names.sort();
        for name in names {
            if cancelled() {
                return BTreeMap::new();
            }
            let path = dir.join(name);
            let Some(meta) = volume.metadata(&path).ok() else { continue };
            if depth < MAX_DEPTH && meta.is_dir() {
                let sub = match (&scope, path.file_name().and_then(|n| n.to_str())) {
                    (Some(scope), Some(segment)) if valid_segment(segment) => Some(format!("{scope}:{segment}")),
                    (Some(_), _) => continue,
                    _ => None,
                };
                pending.push((path, sub, depth + 1));
            } else if meta.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md"))
                && let Ok(file) = volume.open(&path, true)
                && let Some(mut agent) = frontmatter(BufReader::new(file.take(FRONTMATTER_BYTES)))
            {
                if let Some(scope) = &scope {
                    // For plugin agents only offer the namespace we can derive unambiguously.
                    if !path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s == agent.name) {
                        continue;
                    }
                    agent.name = format!("{scope}:{}", agent.name);
                    if session_agent_args(&agent.name).is_err() {
                        continue;
                    }
                }
                let key = agent.name.clone();
                found.entry(key).and_modify(|old| *old = None).or_insert(Some(agent));
            }
        }
    }
    found.into_iter().filter_map(|(name, item)| Some((name, item?))).collect()
}

fn valid_segment(name: &str) -> bool {
    !name.contains(':') && session_agent_args(name).is_ok()
}

/// Claude's common YAML frontmatter forms. Interpret only two top-level scalar fields; unknown,
/// ambiguous or unsupported name syntax is not turned into a launch option.
fn frontmatter(reader: impl BufRead) -> Option<Agent> {
    let mut lines = reader.lines();
    if lines.next()?.ok()?.trim_start_matches('\u{feff}').trim_end() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = None;
    let mut block = false;
    let mut block_text = String::new();
    for line in lines {
        let line = line.ok()?;
        if matches!(line.trim_end(), "---" | "...") {
            let name = name?;
            let description = if block { block_text } else { description.unwrap_or_default() };
            let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
            let description = description.chars().filter(|c| !display_control(*c)).take(512).collect();
            return Some(Agent { name, description });
        }
        if line.starts_with([' ', '\t']) {
            if block {
                block_text.push(' ');
                block_text.push_str(line.trim());
            }
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if block {
            description = Some(std::mem::take(&mut block_text));
            block = false;
        }
        if let Some(value) = line.strip_prefix("name:") {
            if name.is_some() {
                return None;
            }
            name = Some(scalar(value).filter(|s| valid_segment(s))?);
        } else if let Some(value) = line.strip_prefix("description:") {
            description = scalar(value);
            block = matches!(value.trim(), ">" | ">-" | ">+" | "|" | "|-" | "|+");
        }
    }
    None
}

fn display_control(c: char) -> bool {
    c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn scalar(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('"') {
        let mut escaped = false;
        let end = rest.char_indices().find_map(|(i, c)| {
            let end = c == '"' && !escaped;
            escaped = c == '\\' && !escaped;
            end.then_some(i + 2)
        })?;
        comment_only(&value[end..]).then(|| serde_json::from_str(&value[..end]).ok()).flatten()
    } else if let Some(mut rest) = value.strip_prefix('\'') {
        let mut text = String::new();
        loop {
            let (head, tail) = rest.split_once('\'')?;
            text.push_str(head);
            if let Some(next) = tail.strip_prefix('\'') {
                text.push('\'');
                rest = next;
            } else {
                return comment_only(tail).then_some(text);
            }
        }
    } else if !value.starts_with(['&', '*', '!', '[', '{', '>', '|', '#', '%', '`', '@']) {
        Some(value.split_once(" #").map_or(value, |(text, _)| text).trim_end().into())
    } else {
        None
    }
}

fn comment_only(s: &str) -> bool {
    s.trim().is_empty() || s.starts_with(char::is_whitespace) && s.trim_start().starts_with('#')
}

#[cfg(test)]
mod tests;
