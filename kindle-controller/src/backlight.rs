//! The Kindle's frontlight. It's set through powerd (`flIntensity`, 0 to `flMaxIntensity`,
//! 24 on the PW4) so powerd stays in charge of it. The on/off state is read from sysfs,
//! because powerd's property can say 10 while the light is actually off.

use std::path::PathBuf;

use log::info;

use crate::lipc;

const POWERD: &str = "com.lab126.powerd";
/// Used when turning on if we've never seen a non-zero level.
const DEFAULT_LEVEL: i64 = 10;

pub struct Backlight {
    /// Level to restore when turning back on.
    level: i64,
}

impl Backlight {
    pub fn new() -> Self {
        let level = lipc::get_int(POWERD, "flIntensity").filter(|&l| l > 0).unwrap_or(DEFAULT_LEVEL);
        Self { level }
    }

    /// Whether the light is on, if we can tell.
    pub fn is_on(&self) -> Option<bool> {
        let dir = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
        let read = |name: &str| -> Option<i64> {
            std::fs::read_to_string(PathBuf::from(&dir).join(name)).ok()?.trim().parse().ok()
        };
        read("actual_brightness").or_else(|| read("brightness")).map(|b| b > 0)
    }

    pub fn set(&mut self, on: bool) {
        if on {
            info!("backlight on (level {})", self.level);
            lipc::set(POWERD, "flIntensity", &self.level.to_string());
        } else {
            if let Some(l) = lipc::get_int(POWERD, "flIntensity").filter(|&l| l > 0) {
                self.level = l;
            }
            info!("backlight off");
            lipc::set(POWERD, "flIntensity", "0");
        }
    }
}
