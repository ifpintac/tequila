//! Wall-clock time, in the time zone of the person using the application.
//!
//! Everything that shows or writes a time — the console's timestamps, the
//! activity log, `{date}` and `{time}` in a render's filename — used to show
//! UTC, because the epoch is UTC and no date library is linked. In Manila that
//! meant a console stamped 20:57 at five in the morning, and a render made
//! before eight dated yesterday. A calendar dependency for one offset would be
//! a poor trade, so the offset is asked of the operating system directly.

/// Seconds since the Unix epoch, as a local wall clock would count them.
fn local_seconds() -> i64 {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    utc + offset_seconds()
}

/// How far local time is ahead of UTC, right now (daylight saving included).
#[cfg(windows)]
fn offset_seconds() -> i64 {
    /// Windows' `SYSTEMTIME`, field for field.
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
        fn GetSystemTime(time: *mut SystemTime);
    }
    let mut local = SystemTime::default();
    let mut utc = SystemTime::default();
    // SAFETY: each call writes one SYSTEMTIME through the pointer it is given
    // and keeps nothing; both pointers are to live, exclusively borrowed
    // structs with the C layout the functions expect.
    unsafe {
        GetLocalTime(&mut local);
        GetSystemTime(&mut utc);
    }
    let seconds = |t: &SystemTime| {
        days_from_civil(t.year as i64, t.month as u32, t.day as u32) * 86_400
            + t.hour as i64 * 3600
            + t.minute as i64 * 60
            + t.second as i64
    };
    // The two reads can straddle a second. Every real offset is a whole
    // number of quarter hours, so rounding to one removes the tick.
    round_to_quarter_hour(seconds(&local) - seconds(&utc))
}

/// Elsewhere, UTC. The application is built and used on Windows; other
/// platforms keep working, with times in UTC as they always were.
#[cfg(not(windows))]
fn offset_seconds() -> i64 {
    0
}

// Used by the Windows offset and its test; elsewhere the offset is UTC's.
#[cfg_attr(not(windows), allow(dead_code))]
fn round_to_quarter_hour(seconds: i64) -> i64 {
    (seconds as f64 / 900.0).round() as i64 * 900
}

/// Today, as `YYYY-MM-DD`.
pub fn date() -> String {
    let (year, month, day) = civil_from_days(local_seconds().div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// The time of day as `HH-MM-SS`, with dashes because a colon may not appear
/// in a filename on Windows.
pub fn time_for_filename() -> String {
    let (h, m, s) = hms(local_seconds());
    format!("{h:02}-{m:02}-{s:02}")
}

/// Date and time as `YYYY-MM-DD HH:MM`, for saying when something happened.
pub fn stamp() -> String {
    let (h, m, _) = hms(local_seconds());
    format!("{} {h:02}:{m:02}", date())
}

/// The time of day as `HH:MM:SS`, for reading.
/// A moment as the local wall clock shows it, `HH:MM`.
pub fn clock_of(moment: std::time::SystemTime) -> String {
    let utc = moment
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (h, m, _) = hms(utc + offset_seconds());
    format!("{h:02}:{m:02}")
}

pub fn time() -> String {
    let (h, m, s) = hms(local_seconds());
    format!("{h:02}:{m:02}:{s:02}")
}

fn hms(seconds: i64) -> (i64, i64, i64) {
    let day = seconds.rem_euclid(86_400);
    (day / 3600, (day % 3600) / 60, day % 60)
}

/// Howard Hinnant's civil-from-days, which is exact and has no dependencies.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// The inverse: days since the epoch for a calendar date.
#[cfg_attr(not(windows), allow(dead_code))]
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_dates_are_inverses() {
        for days in [-1_000_000, -1, 0, 1, 19_000, 20_720, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2026, 9, 24), 20_720);
    }

    /// A read that straddles a second boundary still gives the true offset.
    #[test]
    fn the_offset_snaps_to_real_time_zones() {
        assert_eq!(round_to_quarter_hour(8 * 3600 - 1), 8 * 3600);
        assert_eq!(round_to_quarter_hour(5 * 3600 + 1800 + 1), 5 * 3600 + 1800);
        assert_eq!(round_to_quarter_hour(-(4 * 3600) + 1), -(4 * 3600));
    }

    /// On Windows the offset is the machine's, which is whatever it is; it
    /// must at least be a real one.
    #[test]
    fn the_offset_is_a_real_time_zone() {
        let offset = offset_seconds();
        assert!(offset.abs() <= 14 * 3600, "{offset}");
        assert_eq!(offset % 900, 0);
    }

    #[test]
    fn times_are_well_formed() {
        assert_eq!(date().len(), 10);
        assert_eq!(time().len(), 8);
        assert!(!time_for_filename().contains(':'), "a colon cannot be in a Windows filename");
    }
}
