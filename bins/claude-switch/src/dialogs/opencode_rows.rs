use super::*;
use mdshared_ui::settings::Row;

pub(super) const REFRESH: u32 = 111;
pub(super) const RESTART: u32 = 112;
pub(super) const STUDIO: u32 = 113;

pub(super) fn rows(st: &AppState) -> impl Iterator<Item = Row> {
    let workers = &st.opencode;
    let demo = st.source.is_demo();
    let status = workers.status.as_ref();
    let installed = if demo {
        "OpenCode · installed (demo v2.0.0)".into()
    } else {
        match status {
            Some(s) if s.path.is_none() => "OpenCode · not installed".into(),
            Some(s) => format!("OpenCode · installed ({})", s.version.as_deref().unwrap_or("version unavailable")),
            None if workers.busy() => "OpenCode · checking…".into(),
            None => "OpenCode · not checked".into(),
        }
    };
    let running = if demo { Some(3) } else { status.and_then(|s| s.running) };
    let service = match running {
        Some(1) => "Service answering · 1 session running".into(),
        Some(n) => format!("Service answering · {n} sessions running"),
        None if status.is_none() => "Service not checked · count unknown".into(),
        None => "Service not answering · count unknown".into(),
    };
    let refresh = Row::action(REFRESH, installed, if workers.busy() { "Working…" } else { "Refresh" });
    let restart = Row::action(RESTART, service, "Restart OpenCode…");
    let studio = (demo || workers.studio_installed).then(|| Row::action(STUDIO, "Configuration", "Open Config Studio"));
    [refresh, restart].into_iter().chain(studio)
}

pub(crate) fn update_opencode_rows(st: &mut AppState) {
    let block = rows(st).collect::<Vec<_>>();
    if let Some(Dialog::Settings(form)) = &mut st.dialog {
        let mut rows = form.rows().to_vec();
        if let Some(start) = rows.iter().position(|r| r.id() == Some(REFRESH)) {
            let end =
                start + rows[start..].iter().take_while(|r| matches!(r.id(), Some(REFRESH | RESTART | STUDIO))).count();
            rows.splice(start..end, block);
            form.set_rows(rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, winfile::WindowFile};
    use claude_switch_core::Roots;

    #[test]
    fn settings_rows_show_count_and_only_offer_studio_when_detected() {
        let demo = AppState::new(Source::Demo, None, WindowFile::default());
        assert!(
            rows(&demo)
                .any(|r| matches!(r, Row::Action { id: RESTART, label, .. } if label.contains("3 sessions running")))
        );
        let mut real = AppState::new(
            Source::Real(Roots { home: std::env::temp_dir(), appdata: std::env::temp_dir() }),
            None,
            WindowFile::default(),
        );
        assert!(!rows(&real).any(|r| r.id() == Some(STUDIO)));
        real.opencode.studio_installed = true;
        assert!(rows(&real).any(|r| r.id() == Some(STUDIO)));
    }
}
