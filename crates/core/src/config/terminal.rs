//! Terminal appearance values shared by config parsing and the settings UI.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalCursor {
    #[default]
    Block,
    Bar,
    Underline,
}

impl TerminalCursor {
    pub const OPTIONS: [(Self, &str, &str); 3] =
        [(Self::Block, "block", "Block"), (Self::Bar, "bar", "Bar"), (Self::Underline, "underline", "Underline")];

    pub fn parse(key: &str) -> Option<Self> {
        Self::OPTIONS.iter().find(|(_, k, _)| *k == key).map(|&(shape, _, _)| shape)
    }

    pub fn index(self) -> usize {
        match self {
            Self::Block => 0,
            Self::Bar => 1,
            Self::Underline => 2,
        }
    }

    pub fn key(self) -> &'static str {
        Self::OPTIONS[self.index()].1
    }
}

pub const TERMINAL_OPACITY_MIN: i32 = 60;
pub const TERMINAL_OPACITY_MAX: i32 = 100;
pub const TERMINAL_OPACITY_STEP: i32 = 5;

pub fn terminal_opacity(percent: i64) -> u8 {
    let percent = percent.clamp(i64::from(TERMINAL_OPACITY_MIN), i64::from(TERMINAL_OPACITY_MAX)) as i32;
    (((percent + TERMINAL_OPACITY_STEP / 2) / TERMINAL_OPACITY_STEP) * TERMINAL_OPACITY_STEP) as u8
}
