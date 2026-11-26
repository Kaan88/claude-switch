use super::*;

impl Compact {
    pub(super) fn agent_name(&self) -> Option<&str> {
        self.agent.checked_sub(1).and_then(|i| self.agents.get(i)).map(|a| a.name.as_str())
    }

    pub(super) fn agent_label(&self) -> &str {
        self.agent_name().unwrap_or(agents::DEFAULT)
    }

    pub(super) fn count(&self) -> usize {
        match self.choice {
            Some(Choice::Model) => self.models.len(),
            Some(Choice::Account) => self.accounts.len(),
            Some(Choice::Agent) => 1 + self.agents.len(),
            None => self.history.len(),
        }
    }

    pub(super) fn pick(&mut self, choice: Choice, st: &AppState) {
        if self.choice == Some(choice) {
            self.choice = None;
            return;
        }
        self.choice = Some(choice);
        self.scroll = match choice {
            Choice::Model => self.model,
            Choice::Account => self.account,
            Choice::Agent => {
                self.agent_focus = self.agent;
                self.load_agents(st);
                self.agent
            }
        }
        .min(self.count().saturating_sub(PAGE));
    }

    fn load_agents(&mut self, st: &AppState) {
        if self.agents_loaded || self.job.is_some() {
            return;
        }
        let Some((slug, _)) = self.accounts.get(self.account) else { return };
        let Source::Real(roots) = &st.source else {
            self.agents = vec![agents::Agent {
                name: format!("{slug}-reviewer"),
                description: "Synthetic demo agent that reviews changes.".into(),
            }];
            self.agents_loaded = true;
            return;
        };
        let Ok(target) = claude_switch_core::account(&st.config, slug).cloned() else { return };
        let roots = roots.clone();
        let cwd = self.session.cwd.clone();
        self.job = Job::start(st.wake_fn(), move |cancel, _| {
            let agents = agents::discover(&roots, &target, &cwd, || cancel.cancelled());
            if cancel.cancelled() {
                return Err("Agent discovery canceled.".into());
            }
            Ok(Update::Agents(target.slug, agents))
        })
        .map_err(|_| self.error("Could not start the agent reader.".into()))
        .ok();
    }

    pub(super) fn choose(&mut self, st: &mut AppState, i: usize) {
        match self.choice.take() {
            Some(Choice::Model) if i < self.models.len() => {
                self.model = i;
                let model = self.models[i].clone();
                if let Source::Real(roots) = &st.source {
                    match claude_switch_core::config::update_compact(roots, |prefs| {
                        prefs.model = model;
                        Ok(())
                    }) {
                        Ok(prefs) => st.config.compact = prefs,
                        Err(e) => self.error(format!("Could not remember the model: {e}")),
                    }
                } else {
                    st.config.compact.model = model;
                }
            }
            Some(Choice::Account) if i < self.accounts.len() && self.account != i => {
                self.account = i;
                self.agents.clear();
                self.agents_loaded = false;
                self.agent = 0;
                self.agent_focus = 0;
            }
            Some(Choice::Agent) if i <= self.agents.len() => self.agent = i,
            _ => {}
        }
    }

    pub(super) fn agent_key(&mut self, key: &KeyName) -> Option<u32> {
        let last = self.count() - 1;
        self.agent_focus = match key {
            KeyName::Home => 0,
            KeyName::End => last,
            KeyName::Up => self.agent_focus.saturating_sub(1),
            KeyName::Down => (self.agent_focus + 1).min(last),
            KeyName::PageUp => self.agent_focus.saturating_sub(PAGE),
            KeyName::PageDown => (self.agent_focus + PAGE).min(last),
            KeyName::Enter | KeyName::Space => return Some(OPTION + self.agent_focus as u32),
            _ => return None,
        };
        self.scroll = self.scroll.min(self.agent_focus).max((self.agent_focus + 1).saturating_sub(PAGE));
        None
    }
}
