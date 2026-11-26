use super::*;

impl AppState {
    pub fn save_config(&mut self) {
        if self.config_damaged {
            return;
        }
        if let Err(error) = self.source.save_config(&self.config) {
            if self.source.load_config().1 {
                self.config_damaged = true;
                self.config.show_usage = false;
                self.usage.disable();
            }
            self.error("The settings were not saved", error.to_string());
        }
    }

    pub fn reset_settings(&mut self, now: Instant) {
        if !self.config_damaged {
            return;
        }
        let Source::Real(roots) = &self.source else { return };
        match claude_switch_core::config::reset(roots) {
            Ok(config) => {
                self.config = config;
                self.config_damaged = false;
                self.info = None;
                self.retheme();
                self.rebuild_accounts();
                self.refresh(now);
                self.toast("Settings reset; the old file was backed up. Usage is off.".into(), now);
            }
            Err(error) => self.toast(error.to_string(), now),
        }
    }
}
