//! Hidden, one-shot developer diagnostic. Does not construct app state or touch the usage cache.

use std::io::{self, Write};
use std::path::Path;

use claude_switch_core::{Config, Roots, usage::Error};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Disabled,
    Account,
    Fetch(Error),
    Output,
}

pub(crate) fn probe(
    config: &Config,
    account: Option<&str>,
    mut out: impl Write,
    fetch: impl FnOnce(&Path) -> Result<Value, Error>,
) -> Result<(), Failure> {
    if !config.show_usage {
        return Err(Failure::Disabled);
    }
    let wanted = account.unwrap_or(&config.current);
    let account = config.accounts.iter().find(|a| a.slug == wanted).ok_or(Failure::Account)?;
    let shape = fetch(&account.cli_dir).map_err(Failure::Fetch)?;
    serde_json::to_writer_pretty(&mut out, &shape).map_err(|_| Failure::Output)?;
    out.write_all(b"\n").and_then(|()| out.flush()).map_err(|_| Failure::Output)
}

pub fn run(demo: bool, account: Option<&str>, labels: bool) -> i32 {
    crate::usage_fetch::attach_diagnostic_console();
    let error = if demo {
        "unavailable in demo mode"
    } else if let Some(roots) = Roots::from_env() {
        let config = claude_switch_core::config::load(&roots);
        match probe(&config, account, io::stdout().lock(), |dir| crate::usage_fetch::fetch_shape(dir, labels)) {
            Ok(()) => return 0,
            Err(Failure::Disabled) => "Show usage is off",
            Err(Failure::Account) => "unknown account slug",
            Err(Failure::Output) => "could not write stdout",
            Err(Failure::Fetch(error)) => {
                let _ = writeln!(io::stderr().lock(), "claude-switch: {error}");
                return 1;
            }
        }
    } else {
        "USERPROFILE or APPDATA is not set"
    };
    let _ = writeln!(io::stderr().lock(), "claude-switch: {error}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_current_or_named_slug_and_refuses_disabled_or_unknown() {
        let mut config = crate::demo::config();
        config.current = "work".into();
        for (requested, index) in [(None, 1), (Some("main"), 0)] {
            let mut out = Vec::new();
            probe(&config, requested, &mut out, |dir| {
                assert_eq!(dir, config.accounts[index].cli_dir);
                Ok(serde_json::json!({"type": "null"}))
            })
            .unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), serde_json::json!({"type": "null"}));
        }
        assert_eq!(probe(&config, Some("missing"), io::sink(), |_| panic!("must not fetch")), Err(Failure::Account));
        config.show_usage = false;
        assert_eq!(probe(&config, None, io::sink(), |_| panic!("must not read")), Err(Failure::Disabled));
    }
}
