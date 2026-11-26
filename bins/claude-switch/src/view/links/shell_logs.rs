use super::{NamedFile, absolute, local};
use std::path::{Path, PathBuf};

/// Trusted temp/project scope; only the exact CLI-named output in this scope is
/// eligible. Claude Code has used both project- and session-scoped `tasks` directories.
#[derive(Clone)]
pub(in crate::view) struct ShellLogs(PathBuf);

impl ShellLogs {
    pub fn new(temp: &Path, cwd: &Path) -> Self {
        Self(temp.join("claude").join(super::super::model::project_slug(cwd)))
    }

    pub fn named(&self, task: &str, raw: &str, session: Option<&str>) -> Option<NamedFile> {
        if !local(raw)
            || raw.len() > 4096
            || !(1..=96).contains(&task.len())
            || !task.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return None;
        }
        let path = absolute(Path::new(raw))?;
        let basename = format!("{task}.output");
        if path.file_name()?.to_str()? != basename {
            return None;
        }
        let dir = path.parent()?;
        let allowed = same(dir, &self.0.join("tasks"))
            || session
                .filter(|id| claude_switch_core::launch::validate_session_id(id).is_ok())
                .is_some_and(|id| same(dir, &self.0.join(id).join("tasks")));
        allowed.then_some(NamedFile { path, view_only: true, other: None })
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a == b
        || cfg!(windows)
            && a.components()
                .map(|c| c.as_os_str())
                .zip(b.components().map(|c| c.as_os_str()))
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
            && a.components().count() == b.components().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_cli_task_output_in_the_current_project_or_session_is_view_only() {
        let demo = crate::demo::Transcript::new().unwrap();
        let temp = demo.cwd().join("temp");
        let logs = ShellLogs::new(&temp, demo.cwd());
        let session = "a1b2c3d4-def1-4071-b7ee-deadbeefcafe";
        let named = |path: &Path| logs.named("b23", path.to_str().unwrap(), Some(session));
        let good = [logs.0.join("tasks/b23.output"), logs.0.join(session).join("tasks/b23.output")];
        for path in good {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "fixture").unwrap();
            let file = named(&path).expect("CLI task path under the approved root");
            assert!(file.available());
            assert!(!file.supports_default_app());
            assert!(file.external_path().is_none());
        }
        for bad in [
            PathBuf::from("tasks/b23.output"),
            temp.join("unrelated/tasks/b23.output"),
            temp.join("claude/unrelated/tasks/b23.output"),
            logs.0.join("a1b2c3d4-def1-4071-b7ee-deadbeefcaff/tasks/b23.output"),
            logs.0.join("tasks/b24.output"),
            logs.0.join("tasks/b23.txt"),
            logs.0.join("tasks/../tasks/b23.output"),
            logs.0.join("tasks/b23.output:other"),
        ] {
            assert!(named(&bad).is_none(), "{bad:?}");
        }
    }
}
