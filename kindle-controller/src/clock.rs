//! Time helpers: Unix time, ISO 8601 for Home Assistant, and local wall-clock time.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn now() -> f64 {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    #[cfg(test)]
    let t = t + test_offset::get();
    t
}

/// Lets tests move the clock (per thread, so tests running in parallel don't interfere).
#[cfg(test)]
pub mod test_offset {
    use std::cell::Cell;

    thread_local!(static OFFSET: Cell<f64> = const { Cell::new(0.0) });

    pub fn get() -> f64 {
        OFFSET.with(Cell::get)
    }

    pub fn set(secs: f64) {
        OFFSET.with(|o| o.set(secs));
    }
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// (year, month, day) to days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-11-10T03:00:00Z`
pub fn iso_utc(epoch: f64) -> String {
    let secs = epoch.floor() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

/// Parses `YYYY-MM-DDTHH:MM[:SS[.fff]]` with `Z`, `±HH:MM` or no zone (taken as UTC).
pub fn parse_iso(s: &str) -> Option<f64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi) = (num(11, 13)?, num(14, 16)?);
    let mut rest = &s[16..];
    let mut sec = 0.0;
    if let Some(r) = rest.strip_prefix(':') {
        let end = r.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(r.len());
        sec = r[..end].parse().ok()?;
        rest = &r[end..];
    }
    let offset = match rest {
        "" | "Z" => 0,
        r if r.len() >= 6 && (r.starts_with('+') || r.starts_with('-')) => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let oh: i64 = r.get(1..3)?.parse().ok()?;
            let om: i64 = r.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    Some((days * 86_400 + h * 3600 + mi * 60 - offset) as f64 + sec)
}

/// Local (hour, minute) for a Unix time, using the system time zone (`TZ` or
/// `/etc/localtime`).
pub fn local_hm(epoch: f64) -> (u32, u32) {
    #[cfg(target_os = "linux")]
    {
        #[allow(deprecated)] // 32-bit on this musl target; fine until 2038
        let t = epoch.floor() as libc::time_t;
        // SAFETY: localtime_r only writes into the struct we pass.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return (tm.tm_hour as u32, tm.tm_min as u32);
        }
    }
    let secs = (epoch.floor() as i64).rem_euclid(86_400);
    ((secs / 3600) as u32, (secs / 60 % 60) as u32)
}

/// `HH:MM` in local time.
pub fn local_hhmm(epoch: f64) -> String {
    let (h, m) = local_hm(epoch);
    format!("{h:02}:{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for days in [-1000, 0, 20_401, 20_767] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn iso_round_trips_and_offsets() {
        let t = 1_794_284_000.0;
        assert_eq!(parse_iso(&iso_utc(t)), Some(t));
        assert_eq!(parse_iso("2026-11-10T03:00:00+00:00"), parse_iso("2026-11-10T14:00:00+11:00"));
        assert_eq!(parse_iso("2026-11-10T03:00:00.5Z"), Some(parse_iso("2026-11-10T03:00:00Z").unwrap() + 0.5));
        assert_eq!(parse_iso("2026-11-10T03:00"), parse_iso("2026-11-10T03:00:00Z"));
        assert_eq!(parse_iso("nonsense"), None);
    }
}
