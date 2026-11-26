use std::collections::BTreeMap;

use crate::{Refusal, Roots};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attention {
    #[default]
    ToastAndMark,
    MarkOnly,
    Off,
}

impl Attention {
    pub const OPTIONS: [(Self, &'static str, &'static str); 3] = [
        (Self::ToastAndMark, "toast", "Toast and mark"),
        (Self::MarkOnly, "mark", "Mark only"),
        (Self::Off, "off", "Off"),
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notifications {
    pub attention: Attention,
    pub usage_warnings: bool,
    pub tray_icon: bool,
    pub close_to_tray: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self { attention: Attention::default(), usage_warnings: true, tray_icon: false, close_to_tray: false }
    }
}

impl Notifications {
    pub(super) fn read(value: &serde_json::Value) -> Self {
        let flags = Self::default();
        Self {
            attention: Attention::OPTIONS
                .iter()
                .find(|(_, id, _)| Some(*id) == value["attention"].as_str())
                .map_or(flags.attention, |(mode, _, _)| *mode),
            usage_warnings: value["usageWarnings"].as_bool().unwrap_or(flags.usage_warnings),
            tray_icon: value["trayIcon"].as_bool().unwrap_or(flags.tray_icon),
            close_to_tray: value["closeToTray"].as_bool().unwrap_or(flags.close_to_tray),
        }
    }

    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "attention": Attention::OPTIONS[self.attention.index()].1,
            "usageWarnings": self.usage_warnings,
            "trayIcon": self.tray_icon,
            "closeToTray": self.close_to_tray,
        })
    }
}

pub type WarningWindows = BTreeMap<(String, String, String, Option<i64>), u8>;
const FILE: &str = "usage-notices.json";

pub fn load_warning_windows(roots: &Roots) -> WarningWindows {
    let path = roots.config_file().with_file_name(FILE);
    if !path.metadata().is_ok_and(|meta| meta.len() <= 256 * 1024) {
        return BTreeMap::new();
    }
    let Some(json) = crate::util::read_json(&path) else { return BTreeMap::new() };
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let slug = row["account"].as_str()?.to_owned();
            let slot = row["slot"].as_str()?.to_owned();
            let label = row["label"].as_str()?.to_owned();
            let threshold = u8::try_from(row["threshold"].as_u64()?).ok()?;
            matches!(threshold, 80 | 95).then_some(((slug, slot, label, row["reset"].as_i64()), threshold))
        })
        .collect()
}

pub fn save_warning_windows(roots: &Roots, windows: &WarningWindows) -> Result<(), Refusal> {
    let rows = windows
        .iter()
        .filter(|(_, n)| matches!(**n, 80 | 95))
        .map(|((slug, slot, label, reset), threshold)| {
            serde_json::json!({"account": slug, "slot": slot, "label": label, "reset": reset, "threshold": threshold})
        })
        .collect();
    crate::util::write_json(&roots.config_file().with_file_name(FILE), &serde_json::Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_every_attention_choice_round_trip() {
        assert_eq!(Notifications::read(&serde_json::Value::Null), Notifications::default());
        for (attention, _, _) in Attention::OPTIONS {
            let settings = Notifications { attention, usage_warnings: false, tray_icon: true, close_to_tray: true };
            assert_eq!(Notifications::read(&settings.json()), settings);
        }
    }

    #[test]
    fn only_notified_thresholds_persist() {
        let dir = crate::util::testing::TempDir::new("usage-notices");
        let roots = Roots { home: dir.0.join("home"), appdata: dir.0.join("appdata") };
        let window = ("main".into(), "week".into(), "Week".into(), Some(42));
        let other = ("main".into(), "model".into(), "Week, Opus".into(), None);
        save_warning_windows(&roots, &BTreeMap::from([(window.clone(), 95), (other, 51)])).unwrap();
        assert_eq!(load_warning_windows(&roots), BTreeMap::from([(window, 95)]));
    }
}
