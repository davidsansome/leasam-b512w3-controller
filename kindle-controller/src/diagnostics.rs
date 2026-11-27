//! Device health reported to HA: battery level, charging status, current and temperature,
//! uptime and Wi-Fi signal strength.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub battery_percent: Option<u8>,
    /// The battery's status as sysfs reports it: "Charging", "Discharging", "Full",
    /// "Not charging" or "Unknown".
    pub charging_status: Option<String>,
    /// The fuel gauge's average battery current in mA: positive while charging, negative
    /// while draining.
    pub battery_current_ma: Option<i32>,
    /// Battery temperature in °C.
    pub battery_temp_c: Option<i32>,
    pub uptime_secs: Option<u64>,
    pub wifi_dbm: Option<i32>,
}

pub fn collect() -> Diagnostics {
    Diagnostics {
        battery_percent: battery_percent(),
        charging_status: charging_status(),
        battery_current_ma: battery_sysfs("current_avg").and_then(|v| parse_micro_to_milli(&v)),
        battery_temp_c: battery_sysfs("temp").and_then(|v| v.parse().ok()),
        uptime_secs: std::fs::read_to_string("/proc/uptime").ok().and_then(|t| parse_uptime(&t)),
        wifi_dbm: std::fs::read_to_string("/proc/net/wireless").ok().and_then(|t| parse_wireless(&t)),
    }
}

/// Battery percentage and whether it's charging, for the on-screen battery icon.
pub fn battery_status() -> (Option<u8>, bool) {
    // The battery's own status, not powerd's isCharging: powerd says "charging" whenever
    // USB power is present, even while the charger is suspended and the battery drains.
    let charging = match charging_status() {
        Some(s) => s == "Charging",
        None => crate::lipc::get_int("com.lab126.powerd", "isCharging").is_some_and(|v| v != 0),
    };
    (battery_percent(), charging)
}

/// The battery's own status from sysfs (see [`Diagnostics::charging_status`]).
pub fn charging_status() -> Option<String> {
    battery_sysfs("status")
}

/// A file from the first battery under `/sys/class/power_supply`, trimmed.
fn battery_sysfs(file: &str) -> Option<String> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        let is_battery = std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery");
        if is_battery && let Ok(v) = std::fs::read_to_string(p.join(file)) {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn battery_percent() -> Option<u8> {
    // powerd is the authoritative source on the Kindle.
    if let Some(v) = crate::lipc::get_int("com.lab126.powerd", "battLevel").and_then(|v| u8::try_from(v).ok()) {
        return Some(v);
    }
    // Fallback: any battery in sysfs.
    battery_sysfs("capacity")?.parse().ok()
}

/// A sysfs reading in micro-units (µA), rounded to milli-units (mA).
fn parse_micro_to_milli(text: &str) -> Option<i32> {
    let micro: i64 = text.parse().ok()?;
    i32::try_from((micro as f64 / 1000.0).round() as i64).ok()
}

fn parse_uptime(text: &str) -> Option<u64> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(secs as u64)
}

/// `/proc/net/wireless` looks like:
/// ```text
/// Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
///  face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
///  wlan0: 0000   54.  -56.  -256        0      0      0      0      0        0
/// ```
fn parse_wireless(text: &str) -> Option<i32> {
    for line in text.lines().skip(2) {
        let Some((_iface, rest)) = line.split_once(':') else { continue };
        let level: f32 = rest.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
        let level = level as i32;
        // Some drivers report dBm as an unsigned 8-bit value.
        let dbm = if level > 0 { level - 256 } else { level };
        if dbm < 0 {
            return Some(dbm);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_microamps() {
        assert_eq!(parse_micro_to_milli("56666"), Some(57));
        assert_eq!(parse_micro_to_milli("-120400"), Some(-120));
        assert_eq!(parse_micro_to_milli(""), None);
    }

    #[test]
    fn parses_uptime() {
        assert_eq!(parse_uptime("12345.67 23456.78\n"), Some(12345));
    }

    #[test]
    fn parses_wireless() {
        let text = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
                    face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n \
                    wlan0: 0000   54.  -56.  -256        0      0      0      0      0        0\n";
        assert_eq!(parse_wireless(text), Some(-56));
        let unsigned = text.replace("-56.", "200.");
        assert_eq!(parse_wireless(&unsigned), Some(-56));
        assert_eq!(parse_wireless("a\nb\n"), None);
    }
}
