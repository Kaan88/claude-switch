use super::*;

impl Compact {
    pub(super) fn reset_agents(&mut self) {
        self.agents.clear();
        self.agents_loaded = false;
        self.agent = usize::from(self.default_agent.is_some());
        self.agent_focus = self.agent;
        if let Some(name) = &self.default_agent {
            // Preselected until the account's list is read; kept only if that list has it.
            self.agents.push(agents::Agent { name: name.clone(), description: "From Settings; checking…".into() });
        }
    }

    pub(super) fn apply_agents(&mut self, agents: Vec<agents::Agent>) {
        let name_at = |i: usize| self.agents.get(i.checked_sub(1)?).map(|a| a.name.clone());
        let (chosen, focus) = (name_at(self.agent), name_at(self.agent_focus));
        self.agents = agents;
        let account = self.accounts.get(self.account).map_or("this account", |a| a.1.as_str());
        if let Some(default) = &self.default_agent
            && chosen.as_deref() == Some(default.as_str())
            && let Err(notice) = agents::checked_default(default, &self.agents, account)
        {
            self.message = Some((notice, false));
        }
        let index = |name: Option<String>| {
            name.and_then(|name| self.agents.iter().position(|agent| agent.name == name)).map_or(0, |i| i + 1)
        };
        self.agent = index(chosen);
        self.agent_focus = index(focus);
        self.agents_loaded = true;
        if self.choice == Some(Choice::Agent) {
            self.scroll = self.agent_focus.min(self.count().saturating_sub(PAGE));
        }
    }

    /// Reads the account's list now if it is not read yet, so a launch never passes an
    /// unchecked Settings default. The notice, if the default was dropped.
    pub(super) fn check_agents_now(&mut self, st: &AppState) -> Option<String> {
        if self.agents_loaded || self.agent_name().is_none() {
            return None;
        }
        let (slug, _) = self.accounts.get(self.account)?.clone();
        let found = match &st.source {
            Source::Real(roots) => {
                let target = claude_switch_core::account(&st.config, &slug).ok()?;
                agents::discover(roots, target, &self.session.cwd, || false)
            }
            Source::Demo => demo_agents(&slug),
        };
        let before = self.message.clone();
        self.apply_agents(found);
        self.message.clone().filter(|m| Some(m) != before.as_ref()).map(|(text, _)| text)
    }

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
        if choice == Choice::Agent {
            self.agent_focus = self.agent;
            self.load_agents(st);
        }
        self.scroll = match choice {
            Choice::Model => self.model,
            Choice::Account => self.account,
            Choice::Agent => self.agent,
        }
        .min(self.count().saturating_sub(PAGE));
    }

    fn load_agents(&mut self, st: &AppState) {
        if self.agents_loaded || self.job.is_some() {
            return;
        }
        let Some((slug, _)) = self.accounts.get(self.account) else { return };
        let Source::Real(roots) = &st.source else {
            let found = demo_agents(slug);
            self.apply_agents(found);
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
                self.reset_agents();
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

pub(crate) fn demo_agents(slug: &str) -> Vec<agents::Agent> {
    vec![agents::Agent {
        name: format!("{slug}-reviewer"),
        description: "Synthetic demo agent that reviews changes.".into(),
    }]
}
