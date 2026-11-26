//! Workers-page state and background jobs. No CLI or Studio launch in demo.
use crate::dialogs::{Dialog, Pending};
use crate::state::AppState;
use claude_switch_core::{Place, launch};
use mdshared_app::job::Job;
use opencode_cli::Status;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Default)]
pub struct Workers {
    pub status: Option<Status>,
    pub studio_installed: bool,
    job: Option<Job<Update>>,
}

enum Update {
    Refreshed(Status, bool),
    Confirm { path: PathBuf, running: Option<usize> },
    Restarted { path: PathBuf, running: usize },
    StudioOpened,
    Failed(&'static str),
}

#[derive(Clone, Debug)]
pub struct Restart {
    path: PathBuf,
    running: Option<usize>,
}

fn studio_path(local: &Path) -> Option<PathBuf> {
    let path = local.join("Programs/OpenCode Config Studio/OpenCodeConfigStudio.exe");
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn studio() -> Option<PathBuf> {
    studio_path(Path::new(&std::env::var_os("LOCALAPPDATA")?))
}

fn command(program: &Path) -> Command {
    let mut spec = launch::shell(&std::env::temp_dir());
    spec.place = Place::Detached;
    spec.env_remove.push("PWD".into());
    // Probes and service commands should not discover or import a Claude configuration.
    spec.env.insert("OPENCODE_DISABLE_CLAUDE_CODE".into(), "1".into());
    crate::run::command_for(&spec, program)
}

fn restart_if_unchanged(
    request: Restart,
    current: impl FnOnce(&Path) -> Option<usize>,
    restart: impl FnOnce(&Path) -> Result<usize, String>,
) -> Update {
    let Restart { path, running } = request;
    let current = current(&path);
    if current != running {
        return Update::Confirm { path, running: current };
    }
    match restart(&path) {
        Ok(running) => Update::Restarted { path, running },
        Err(_) => Update::Failed("OpenCode restart could not be verified. Refresh to check its status."),
    }
}

impl Workers {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

impl AppState {
    pub fn opencode_settings(&mut self, workers_page: bool) {
        self.dialog = Some(crate::dialogs::settings(self));
        if workers_page
            && let Some(Dialog::Settings(form)) = &mut self.dialog
            && let Some(category) = form.categories().iter().position(|name| *name == "Workers")
        {
            form.jump_to_category(category);
        }
        if !workers_page {
            self.refresh_opencode();
        }
    }

    pub fn refresh_opencode(&mut self) {
        self.opencode_job(|| {
            let path = opencode_cli::find_program();
            Update::Refreshed(opencode_cli::status(|| path.as_deref().map(command)), studio().is_some())
        });
    }

    pub fn ask_restart_opencode(&mut self) {
        self.opencode_job(|| {
            let Some(path) = opencode_cli::find_program() else { return Update::Failed("OpenCode is not installed.") };
            let running = opencode_cli::service_sessions(|| Some(command(&path)));
            Update::Confirm { path, running }
        });
    }

    pub fn confirm_restart_opencode(&mut self, request: Restart, proceed: bool) {
        self.opencode_settings(true);
        if proceed {
            self.opencode_job(move || {
                restart_if_unchanged(
                    request,
                    |path| opencode_cli::service_sessions(|| Some(command(path))),
                    |path| opencode_cli::restart_service(|| command(path)),
                )
            });
        }
    }

    pub fn open_config_studio(&mut self) {
        self.opencode_job(|| {
            let Some(path) = studio() else { return Update::Failed("OpenCode Config Studio is no longer installed.") };
            let mut spec = launch::shell(path.parent().expect("fixed Studio executable path has a parent"));
            spec.program = path;
            spec.place = Place::Detached;
            match crate::run::run(&spec) {
                Ok(_) => Update::StudioOpened,
                Err(_) => Update::Failed("Could not open OpenCode Config Studio."),
            }
        });
    }

    fn opencode_job(&mut self, work: impl FnOnce() -> Update + Send + 'static) {
        if self.source.is_demo() || self.opencode.busy() {
            return;
        }
        match Job::start(self.wake_fn(), move |_, _| work()) {
            Ok(job) => self.opencode.job = Some(job),
            Err(_) => self.toast("Could not start the OpenCode worker.".into(), Instant::now()),
        }
        crate::dialogs::update_opencode_rows(self);
    }

    pub fn take_opencode_job(&mut self, now: Instant) -> bool {
        let Some(job) = self.opencode.job.take_if(|job| !job.running()) else { return false };
        match job.take().unwrap_or(Update::Failed("The OpenCode worker stopped unexpectedly.")) {
            Update::Refreshed(status, studio) => {
                self.opencode.status = Some(status);
                self.opencode.studio_installed = studio;
            }
            Update::Confirm { path, running } if matches!(self.dialog, Some(Dialog::Settings(_))) => {
                self.confirm_scroll = 0.0;
                self.dialog = Some(Dialog::Confirm {
                    title: "Restart OpenCode?".into(),
                    text: restart_warning(running),
                    ok: "Restart".into(),
                    then: Pending::RestartOpenCode(Restart { path, running }),
                });
            }
            Update::Confirm { .. } | Update::StudioOpened => {}
            Update::Restarted { path, running } => {
                let old = self.opencode.status.take().filter(|old| old.path.as_ref() == Some(&path));
                self.opencode.status =
                    Some(Status { path: Some(path), version: old.and_then(|old| old.version), running: Some(running) });
                self.toast("OpenCode restarted and answered.".into(), now);
            }
            Update::Failed(error) => self.toast(error.into(), now),
        }
        crate::dialogs::update_opencode_rows(self);
        true
    }
}

fn restart_warning(running: Option<usize>) -> String {
    let count = match running {
        Some(1) => "1 OpenCode session is running.".into(),
        Some(n) => format!("{n} OpenCode sessions are running."),
        None => "OpenCode did not answer; the number of running sessions is unknown.".into(),
    };
    format!("{count} Restarting will cut off any running jobs.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dialogs, source::Source, winfile::WindowFile};

    #[test]
    fn restart_warning_counts_and_discloses_unknown() {
        assert_eq!(
            restart_warning(Some(1)),
            "1 OpenCode session is running. Restarting will cut off any running jobs."
        );
        assert!(restart_warning(Some(0)).starts_with("0 "));
        assert!(restart_warning(Some(5)).starts_with("5 "));
        assert!(restart_warning(None).contains("unknown"));
    }

    #[test]
    fn demo_buttons_start_nothing_and_leave_settings_open() {
        let mut st = AppState::new(Source::Demo, None, WindowFile::default());
        st.opencode_settings(true);
        st.refresh_opencode();
        st.ask_restart_opencode();
        st.open_config_studio();
        assert!(!st.opencode.busy());
        assert!(st.opencode.status.is_none());
        assert!(matches!(st.dialog, Some(dialogs::Dialog::Settings(_))));
    }

    #[test]
    fn changed_or_lost_count_requires_a_fresh_confirmation() {
        let request = Restart { path: "fixed.exe".into(), running: Some(2) };
        for current in [Some(3), None] {
            let result = restart_if_unchanged(request.clone(), |_| current, |_| panic!("restart not confirmed"));
            assert!(matches!(result, Update::Confirm { running, .. } if running == current));
        }
        let result = restart_if_unchanged(
            request,
            |_| Some(2),
            |p| {
                assert_eq!(p, Path::new("fixed.exe"));
                Ok(0)
            },
        );
        assert!(matches!(result, Update::Restarted { running: 0, .. }));
    }

    #[test]
    fn studio_detection_accepts_only_the_fixed_install_location() {
        let dir = std::env::temp_dir().join(format!("cs-studio-test-{}", std::process::id()));
        let fixed = dir.join("Programs/OpenCode Config Studio/OpenCodeConfigStudio.exe");
        std::fs::create_dir_all(fixed.parent().unwrap()).unwrap();
        let nearby = dir.join("OpenCodeConfigStudio.exe");
        std::fs::write(&nearby, "").unwrap();
        assert_eq!(studio_path(&dir), None);
        std::fs::write(&fixed, "").unwrap();
        assert_eq!(studio_path(&dir), Some(fixed.clone()));
        for p in [fixed, nearby] {
            std::fs::remove_file(p).unwrap();
        }
    }
}
