use crate::dialogs;
use crate::state::AppState;
use claude_switch_core::config::{TerminalCursor, terminal_opacity};
use mdshared_app::backdrop::Backdrop;

pub struct Preview(TerminalCursor, u8, Backdrop);

pub fn show_settings(st: &mut AppState) {
    if st.source.is_demo() {
        st.dialog = Some(dialogs::settings(st, Some(dialogs::TERMINAL)));
    }
}

impl Preview {
    pub fn parse(spec: &str) -> Option<Self> {
        let mut parts = spec.split(',');
        let shape = TerminalCursor::parse(parts.next()?)?;
        let opacity = terminal_opacity(parts.next()?.parse().ok()?);
        let backdrop = Backdrop::parse(parts.next()?)?;
        parts.next().is_none().then_some(Self(shape, opacity, backdrop))
    }

    pub fn apply(self, st: &mut AppState) {
        if st.source.is_demo() {
            let prefs = &mut st.config.appearance;
            prefs.cursor_shape = self.0;
            prefs.terminal_opacity = self.1;
            prefs.backdrop = self.2.key().into();
            st.retheme();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_requires_a_shape_percent_and_backdrop() {
        let Preview(cursor, opacity, backdrop) = Preview::parse("bar,80,acrylic").unwrap();
        assert_eq!((cursor, opacity, backdrop), (TerminalCursor::Bar, 80, Backdrop::Acrylic));
        for invalid in ["bar,80", "beam,80,mica", "bar,bright,mica", "bar,80,garbage", "bar,80,mica,extra"] {
            assert!(Preview::parse(invalid).is_none());
        }
    }
}
