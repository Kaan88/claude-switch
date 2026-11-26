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
    read_warning_windows(&path).unwrap_or_default()
}

fn read_warning_windows(path: &std::path::Path) -> Result<WarningWindows, Refusal> {
    let bytes = match crate::read_bounded(path, 256 * 1024) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(crate::util::io_refusal("read usage notices", path, &e)),
    };
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|e| crate::util::io_refusal("decode usage notices", path, &e))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let slug = row["account"].as_str()?.to_owned();
            let slot = row["slot"].as_str()?.to_owned();
            let label = row["label"].as_str()?.to_owned();
            let threshold = u8::try_from(row["threshold"].as_u64()?).ok()?;
            matches!(threshold, 80 | 95).then_some(((slug, slot, label, row["reset"].as_i64()), threshold))
        })
        .collect())
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

pub fn claim_warning(
    roots: &Roots,
    key: (String, String, String, Option<i64>),
    threshold: u8,
) -> Result<bool, Refusal> {
    claim_warning_at(roots, key, threshold, (crate::usage::now_ms() / 1000) as i64)
}

fn claim_warning_at(
    roots: &Roots,
    key: (String, String, String, Option<i64>),
    threshold: u8,
    now: i64,
) -> Result<bool, Refusal> {
    let path = roots.config_file().with_file_name(FILE);
    let _lock = crate::util::lock_beside(&path)?;
    crate::util::regular_target(&path)?;
    let mut windows = read_warning_windows(&path)?;
    let len = windows.len();
    windows.retain(|(_, _, _, reset), _| reset.is_none_or(|reset| reset > now));
    let claimed = key.3.is_none_or(|reset| reset > now) && windows.get(&key).is_none_or(|prior| *prior < threshold);
    let pruned = len != windows.len();
    if claimed {
        windows.insert(key, threshold);
    }
    if claimed || pruned {
        save_warning_windows(roots, &windows)?;
    }
    Ok(claimed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_claims_exclude_a_second_window_without_losing_other_keys() {
        let dir = crate::util::testing::TempDir::new("usage-claims");
        let roots = dir.roots();
        let a = ("main".into(), "week".into(), "Week".into(), Some(100));
        let b = ("work".into(), "week".into(), "Week".into(), Some(200));
        assert!(claim_warning_at(&roots, a.clone(), 80, 0).unwrap());
        assert!(claim_warning_at(&roots, b.clone(), 95, 0).unwrap());
        assert!(!claim_warning_at(&roots, a.clone(), 80, 0).unwrap());
        assert!(claim_warning_at(&roots, a.clone(), 95, 0).unwrap());
        assert_eq!(load_warning_windows(&roots), BTreeMap::from([(a, 95), (b.clone(), 95)]));
        assert!(!claim_warning_at(&roots, b.clone(), 95, 100).unwrap());
        assert_eq!(load_warning_windows(&roots), BTreeMap::from([(b, 95)]));
    }

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
