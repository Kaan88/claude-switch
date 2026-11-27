use super::*;

impl AppState {
    pub fn apply_committed_usage(&mut self, enabled: bool) {
        self.config.show_usage = enabled;
        self.config_baseline.show_usage = enabled;
        if !enabled {
            self.disable_usage();
        }
    }

    pub fn save_config(&mut self) {
        if self.config_damaged {
            return;
        }
        match self.source.save_config(&self.config_baseline, &self.config) {
            Ok(config) => {
                let workers_changed =
                    config.workers != self.config_baseline.workers || config.accounts != self.config_baseline.accounts;
                if !config.show_usage {
                    self.disable_usage();
                }
                self.config_baseline = config.clone();
                self.config = config;
                if workers_changed {
                    self.sync_worker_accounts();
                }
            }
            Err(error) => {
                let (config, damaged) = self.source.load_config();
                self.config_damaged = damaged;
                self.config_baseline = config.clone();
                self.config = config;
                if !self.config.show_usage {
                    self.disable_usage();
                }
                self.error("The settings were not saved", error.to_string());
            }
        }
    }

    pub fn reset_settings(&mut self, now: Instant) {
        if !self.config_damaged {
            return;
        }
        let Source::Real(roots) = &self.source else { return };
        match claude_switch_core::config::reset(roots) {
            Ok(config) => {
                self.config_baseline = config.clone();
                self.config = config;
                self.config_damaged = false;
                self.disable_usage();
                self.info = None;
                self.retheme();
                self.rebuild_accounts();
                self.refresh(now);
                self.toast("Settings reset; the old file was backed up. Usage is off.".into(), now);
                self.sync_worker_accounts();
            }
            Err(error) => self.toast(error.to_string(), now),
        }
    }

    pub fn sync_worker_accounts(&mut self) {
        if let Source::Real(roots) = &self.source
            && let Err(error) = claude_switch_core::config::sync_workers(roots)
        {
            self.error(
                "Worker accounts were not synced",
                format!("{error} Use Settings → OpenCode → Sync bridge account settings to retry."),
            );
        }
    }
}

#[test]
fn ui_settings_publish_bridge_only_after_commit_and_after_reset() {
    use claude_switch_core::{Roots, WorkersUse, config};
    let scratch = crate::testing::Scratch::new("cs-w3-ui-bridge");
    let folder = scratch.to_path_buf();
    let roots = Roots { home: folder.clone(), appdata: folder };
    let initial = config::load(&roots);
    config::save(&roots, &initial).unwrap();
    config::sync_workers(&roots).unwrap();
    let bridge = || std::fs::read_to_string(roots.workers_file()).unwrap();
    let before = bridge();
    let mut app = AppState::new(Source::Real(roots.clone()), None, Default::default());
    let lock = config::usage_permit(&roots, &initial.accounts[0].cli_dir).unwrap();
    app.config.workers = WorkersUse::Other;
    app.save_config();
    assert_eq!(app.config.workers, WorkersUse::Same);
    assert_eq!(bridge(), before);
    drop(lock);
    app.config.workers = WorkersUse::Other;
    app.save_config();
    assert!(bridge().contains("\"claudeWorkersUse\": \"other\""));

    std::fs::write(roots.config_file(), b"invalid").unwrap();
    app.save_config();
    assert!(app.config_damaged);
    app.reset_settings(Instant::now());
    assert!(!app.config_damaged);
    assert_eq!(bridge(), before);
}
