//! Keeps the Kindle charging from the tablet board's current-limited USB port.
//!
//! The tablet board's USB output is a TPS2553 holding about 123 mA. The Kindle's charger
//! (ROHM BD71827) normally runs an adaptive input limit: it steps its input current up and
//! backs off if VBUS collapses. Against a hard current limit, VBUS drops sharply instead of
//! sagging, the adaptive limit falls to zero, and the charger suspends until it's replugged.
//! The battery then drains while the Kindle reports that it's charging.
//!
//! The fix is a fixed 100 mA input limit with the adaptive limit off, then a charger reset
//! (the same reset the Kindle's driver does after setting a limit). 100 mA stays under the
//! board's limit, so VBUS never collapses.
//!
//! This only acts when the charger has been seen suspended, with power present, on two
//! checks in a row (a minute apart), so a normal charger is never touched. The driver sets
//! its own limit again on every plug-in and the chip resets on reboot, so the fix is
//! re-applied whenever it's needed.
//!
//! Register map: `include/linux/mfd/bd71827.h` and `drivers/power/bd71827-power.c` in the
//! Kindle (rex) kernel. Registers are read and written through the driver's own
//! `registers` sysfs file.

use log::{info, warn};

const REGISTERS: &str = "/sys/class/power_supply/bd71827_bat/registers";
const CAPACITY: &str = "/sys/class/power_supply/bd71827_bat/capacity";

const REG_SYS_INIT: u8 = 0x38;
const REG_CHG_STATE: u8 = 0x39;
const REG_DCIN_STAT: u8 = 0x3C;
const REG_DCIN_SET: u8 = 0x42;

/// SYS_INIT: holding this resets the charger (the driver holds it for 10 ms).
const CHGRST: u8 = 0x02;
/// DCIN_STAT: input power present.
const DCIN_DET: u8 = 0x01;
/// CHG_STATE: charger suspended.
const CHG_STATE_SUSPEND: u8 = 0x00;
/// DCIN_SET: input enabled (bit 7), adaptive limit off (bit 6 clear), limit field 1
/// (the driver writes `(mA - 1) / 50`, so 1 means 100 mA).
const DCIN_SET_FIXED_100MA: u8 = 0x81;

/// How many checks in a row the charger must be stuck before we step in.
const STUCK_CHECKS: u8 = 2;

#[derive(Default)]
pub struct ChargeGuard {
    stuck: u8,
}

impl ChargeGuard {
    /// Call about once a minute.
    pub fn check(&mut self) {
        let Some(regs) = std::fs::read_to_string(REGISTERS).ok().map(|t| Registers::parse(&t)) else {
            return; // not a BD71827 Kindle (or not running on the Kindle at all)
        };
        let capacity = std::fs::read_to_string(CAPACITY).ok().and_then(|c| c.trim().parse::<u8>().ok());
        if !self.update(&regs, capacity) {
            return;
        }
        info!(
            "charger suspended with power present (DCIN_SET 0x{:02X}): setting a fixed 100 mA input limit",
            regs.get(REG_DCIN_SET).unwrap_or(0)
        );
        if let Err(e) = apply_fix() {
            warn!("couldn't apply the charging fix: {e:#}");
        }
    }

    /// Whether to apply the fix now, given this check's readings.
    fn update(&mut self, regs: &Registers, capacity: Option<u8>) -> bool {
        let stuck = matches!(
            (regs.get(REG_DCIN_STAT), regs.get(REG_CHG_STATE), regs.get(REG_DCIN_SET)),
            (Some(dcin), Some(CHG_STATE_SUSPEND), Some(set))
                if dcin & DCIN_DET != 0
                    // Already applied: if it's still suspended, something else is wrong,
                    // and resetting the charger every minute won't help.
                    && set != DCIN_SET_FIXED_100MA
        ) && capacity.is_none_or(|c| c < 100);
        self.stuck = if stuck { self.stuck.saturating_add(1) } else { 0 };
        if self.stuck >= STUCK_CHECKS {
            self.stuck = 0;
            true
        } else {
            false
        }
    }
}

fn apply_fix() -> std::io::Result<()> {
    write_reg(REG_DCIN_SET, DCIN_SET_FIXED_100MA)?;
    write_reg(REG_SYS_INIT, CHGRST)?;
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_reg(REG_SYS_INIT, 0)
}

fn write_reg(reg: u8, value: u8) -> std::io::Result<()> {
    std::fs::write(REGISTERS, format!("{reg:x} {value:x}"))
}

/// The driver's register dump: one `[0x39] = 03` line per register.
struct Registers([Option<u8>; 256]);

impl Registers {
    fn parse(text: &str) -> Self {
        let mut regs = [None; 256];
        for line in text.lines() {
            let Some((reg, value)) = line.split_once('=') else { continue };
            let reg = reg.trim().trim_start_matches('[').trim_end_matches(']').trim_start_matches("0x");
            if let (Ok(reg), Ok(value)) = (u8::from_str_radix(reg, 16), u8::from_str_radix(value.trim(), 16)) {
                regs[usize::from(reg)] = Some(value);
            }
        }
        Self(regs)
    }

    fn get(&self, reg: u8) -> Option<u8> {
        self.0[usize::from(reg)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(chg_state: u8, dcin_stat: u8, dcin_set: u8) -> Registers {
        Registers::parse(&format!(
            "[0x38] = 00\n[0x39] = {chg_state:02X}\n[0x3C] = {dcin_stat:02X}\n[0x42] = {dcin_set:02X}\n"
        ))
    }

    #[test]
    fn parses_the_driver_dump() {
        let r = Registers::parse("[0x00] = 31\n[0x39] = 03\n[0x42] = D1\ngarbage\n[0xFF] = 12\n");
        assert_eq!(r.get(0x00), Some(0x31));
        assert_eq!(r.get(0x39), Some(0x03));
        assert_eq!(r.get(0x42), Some(0xD1));
        assert_eq!(r.get(0xFF), Some(0x12));
        assert_eq!(r.get(0x3C), None);
    }

    #[test]
    fn fixes_a_suspended_charger_on_the_second_check() {
        // What the Kindle showed on the tablet board: power present, suspended, 900 mA adaptive.
        let stuck = regs(0x00, 0x05, 0xD1);
        let mut g = ChargeGuard::default();
        assert!(!g.update(&stuck, Some(42)));
        assert!(g.update(&stuck, Some(42)));
        // Counting starts again afterwards.
        assert!(!g.update(&stuck, Some(42)));
    }

    #[test]
    fn a_charging_reading_in_between_starts_the_count_again() {
        let stuck = regs(0x00, 0x05, 0xD1);
        let mut g = ChargeGuard::default();
        assert!(!g.update(&stuck, Some(42)));
        assert!(!g.update(&regs(0x03, 0x01, 0xC8), Some(42)));
        assert!(!g.update(&stuck, Some(42)));
        assert!(g.update(&stuck, Some(42)));
    }

    #[test]
    fn leaves_normal_states_alone() {
        let mut g = ChargeGuard::default();
        for _ in 0..5 {
            assert!(!g.update(&regs(0x03, 0x01, 0xC8), Some(42))); // fast charging on a PC port
            assert!(!g.update(&regs(0x00, 0x00, 0xD1), Some(42))); // unplugged
            assert!(!g.update(&regs(0x00, 0x05, 0xD1), Some(100))); // full
            assert!(!g.update(&regs(0x00, 0x05, 0x81), Some(42))); // fix already applied
        }
    }

    #[test]
    fn missing_registers_do_nothing() {
        let mut g = ChargeGuard::default();
        let empty = Registers::parse("");
        assert!(!g.update(&empty, None));
        assert!(!g.update(&empty, None));
    }
}
