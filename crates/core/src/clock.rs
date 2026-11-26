//! The computer's clock as the owner reads it: how far local time is from UTC, so that a time
//! shown in the window is the time on the wall and not UTC.

/// The system's current offset from UTC in minutes, positive east of Greenwich (Berlin in
/// winter is 60, New York in summer is -240), with daylight saving time counted while it is in
/// force. Read again on every call, so a change of the time zone or of daylight time shows at
/// once. 0 where it cannot be read, and on systems other than Windows.
pub fn local_offset_minutes() -> i32 {
    imp::offset()
}

/// The offset from what Windows says: the zone's `Bias` (minutes to add to local time to get
/// UTC) plus the standard or the daylight bias, by which one is in force (`TIME_ZONE_ID_*`: 1
/// standard, 2 daylight, 0 a zone without daylight time). `None` for the error answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn offset_from(zone_id: u32, bias: i32, standard_bias: i32, daylight_bias: i32) -> Option<i32> {
    let extra = match zone_id {
        0 => 0,
        1 => standard_bias,
        2 => daylight_bias,
        _ => return None,
    };
    bias.checked_add(extra)?.checked_neg()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Time::{DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation};

    pub(super) fn offset() -> i32 {
        // SAFETY: an all-zero DYNAMIC_TIME_ZONE_INFORMATION is a valid value of this plain C
        // struct (numbers and fixed arrays only).
        let mut info: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a live, writable struct of the type the call fills in.
        let id = unsafe { GetDynamicTimeZoneInformation(&mut info) };
        super::offset_from(id, info.Bias, info.StandardBias, info.DaylightBias).unwrap_or(0)
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn offset() -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bias_is_turned_into_minutes_east_of_utc_with_daylight_time_when_it_is_in_force() {
        // Berlin: Bias -60, daylight bias -60.
        assert_eq!(offset_from(1, -60, 0, -60), Some(60));
        assert_eq!(offset_from(2, -60, 0, -60), Some(120));
        // New York: Bias 300, daylight bias -60.
        assert_eq!(offset_from(1, 300, 0, -60), Some(-300));
        assert_eq!(offset_from(2, 300, 0, -60), Some(-240));
        // A zone without daylight time, India: Bias -330.
        assert_eq!(offset_from(0, -330, 0, 0), Some(330));
        assert_eq!(offset_from(u32::MAX, -60, 0, -60), None);
    }

    #[test]
    fn the_system_offset_is_a_real_time_zone() {
        let m = local_offset_minutes();
        assert!((-14 * 60..=14 * 60).contains(&m), "{m}");
    }
}
