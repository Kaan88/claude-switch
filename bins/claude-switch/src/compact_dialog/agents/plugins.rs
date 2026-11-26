//! Only explicitly enabled cache installs with an unambiguous scope and namespace.
use super::*;
use serde_json::Value;
use std::path::Component;

const JSON_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn discover(
    roots: &Roots,
    target: &Account,
    project: &Path,
    cwd: &Path,
    cancelled: &impl Fn() -> bool,
) -> BTreeMap<String, Agent> {
    let Some(dir) = shared_folder(roots, target, "plugins") else { return BTreeMap::new() };
    let Some(registry) = json(&dir.join("installed_plugins.json")) else { return BTreeMap::new() };
    let Some(installed) = registry.get("plugins").and_then(Value::as_object).filter(|_| registry["version"] == 2)
    else {
        return BTreeMap::new();
    };
    let mut enabled = BTreeMap::new();
    let mut settings = vec![target.cli_dir.join("settings.json")];
    for base in if project == cwd { vec![cwd] } else { vec![project, cwd] } {
        settings.push(base.join(".claude/settings.json"));
        settings.push(base.join(".claude/settings.local.json"));
    }
    for path in settings {
        if let Some(settings) = json(&path)
            && let Some(plugins) = settings.get("enabledPlugins").and_then(Value::as_object)
        {
            enabled.extend(plugins.iter().map(|(name, value)| (name.clone(), value.as_bool() == Some(true))));
        }
    }
    let mut found = BTreeMap::new();
    for (id, entries) in installed.iter().filter(|(id, _)| enabled.get(*id) == Some(&true)).take(32) {
        if cancelled() {
            return BTreeMap::new();
        }
        let Some((plugin, marketplace)) = id.split_once('@').filter(|(p, m)| valid_segment(p) && valid_segment(m))
        else {
            continue;
        };
        let Some(entries) = entries.as_array() else { continue };
        let mut paths: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                let rank = match entry.get("scope")?.as_str()? {
                    "user" => 0,
                    "project" if in_project(entry, project, cwd) => 1,
                    "local" if in_project(entry, project, cwd) => 2,
                    _ => return None,
                };
                let path = Path::new(entry.get("installPath")?.as_str()?);
                let relative =
                    path.strip_prefix(&dir).or_else(|_| path.strip_prefix(target.cli_dir.join("plugins"))).ok()?;
                let mut components = relative.components();
                if components.next()? != Component::Normal("cache".as_ref())
                    || components.next()? != Component::Normal(marketplace.as_ref())
                    || components.next()? != Component::Normal(plugin.as_ref())
                    || !matches!(components.next()?, Component::Normal(_))
                    || components.next().is_some()
                {
                    return None;
                }
                Some((rank, dir.join(relative)))
            })
            .collect();
        paths.sort();
        paths.dedup();
        let Some((rank, path)) = paths.pop() else { continue };
        if paths.last().is_some_and(|(other, _)| *other == rank) {
            continue;
        }
        for (key, agent) in scan(&path.join("agents"), Some(plugin), cancelled) {
            found.entry(key).and_modify(|old| *old = None).or_insert(Some(agent));
        }
    }
    found.into_iter().filter_map(|(name, item)| Some((name, item?))).collect()
}

fn in_project(entry: &Value, project: &Path, cwd: &Path) -> bool {
    let Some(path) = entry.get("projectPath").and_then(Value::as_str) else { return false };
    [project, cwd].into_iter().any(|dir| link_matches(Path::new(path), dir))
}

fn json(path: &Path) -> Option<Value> {
    let file = Volume::at(path)?.open(path, true).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(JSON_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > JSON_BYTES {
        return None;
    }
    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes)).ok()
}
