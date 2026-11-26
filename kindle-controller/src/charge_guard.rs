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
//! With the adaptive limit off, the chip doesn't stop current flowing backwards: once the
//! cable is unplugged, the battery holds the Kindle's own USB port at about 3.8 V, so the
//! unplug is never detected and the Kindle keeps showing "charging" while it drains. So
//! while the fix is in place, a VBUS reading below 4.3 V means the cable has gone, and the
//! adaptive limit is turned back on. VBUS then falls away, and the driver sees the unplug
//! and sets its own limit again.
//!
//! Register map: `include/linux/mfd/bd71827.h` and `drivers/power/bd71827-power.c` in the
//! Kindle (rex) kernel. Registers are read and written through the driver's own
//! `registers` sysfs file.

use log::{info, warn};

const REGISTERS: &str = "/sys/class/power_supply/bd71827_bat/registers";
const CAPACITY: &str = "/sys/class/power_supply/bd71827_bat/capacity";
const VBUS: &str = "/sys/class/power_supply/bd71827_ac/voltage_now";

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
/// DCIN_SET: adaptive input limit on.
const DCIN_ILIM_EN: u8 = 0x40;

/// Below this (µV) with the fix in place, the cable has been unplugged: the board holds
/// VBUS at about 5.0 V at 100 mA, and the battery back-feeds it at about 3.8 V.
const UNPLUGGED_BELOW_UV: u32 = 4_300_000;

/// How many checks in a row the charger must be stuck before we step in.
const STUCK_CHECKS: u8 = 2;

#[derive(Default)]
pub struct ChargeGuard {
    stuck: u8,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    None,
    /// Stuck: set the fixed 100 mA limit and reset the charger.
    Fix,
    /// Unplugged with the fix still in place: turn the adaptive limit back on.
    Undo,
}

impl ChargeGuard {
    /// Call about once a minute.
    pub fn check(&mut self) {
        let Some(regs) = std::fs::read_to_string(REGISTERS).ok().map(|t| Registers::parse(&t)) else {
            return; // not a BD71827 Kindle (or not running on the Kindle at all)
        };
        let capacity = std::fs::read_to_string(CAPACITY).ok().and_then(|c| c.trim().parse::<u8>().ok());
        let vbus_uv = std::fs::read_to_string(VBUS).ok().and_then(|v| v.trim().parse::<u32>().ok());
        let set = regs.get(REG_DCIN_SET).unwrap_or(0);
        let result = match self.update(&regs, capacity, vbus_uv) {
            Action::None => return,
            Action::Fix => {
                info!("charger suspended with power present (DCIN_SET 0x{set:02X}): setting a fixed 100 mA input limit");
                apply_fix()
            }
            Action::Undo => {
                info!(
                    "VBUS {} mV with the fixed input limit in place: cable unplugged, turning the adaptive limit back on",
                    vbus_uv.unwrap_or(0) / 1000
                );
                write_reg(REG_DCIN_SET, set | DCIN_ILIM_EN)
            }
        };
        if let Err(e) = result {
            warn!("couldn't change the charger's input limit: {e:#}");
        }
    }

    /// What to do now, given this check's readings.
    fn update(&mut self, regs: &Registers, capacity: Option<u8>, vbus_uv: Option<u32>) -> Action {
        let set = regs.get(REG_DCIN_SET);
        if set == Some(DCIN_SET_FIXED_100MA) {
            // The fix is in place. If it's still suspended with power present, something
            // else is wrong, and resetting the charger every minute won't help; just watch
            // for the cable going.
            self.stuck = 0;
            return match vbus_uv {
                Some(v) if v < UNPLUGGED_BELOW_UV => Action::Undo,
                _ => Action::None,
            };
        }
        let stuck = matches!(
            (regs.get(REG_DCIN_STAT), regs.get(REG_CHG_STATE)),
            (Some(dcin), Some(CHG_STATE_SUSPEND)) if dcin & DCIN_DET != 0
        ) && set.is_some()
            && capacity.is_none_or(|c| c < 100);
        self.stuck = if stuck { self.stuck.saturating_add(1) } else { 0 };
        if self.stuck >= STUCK_CHECKS {
            self.stuck = 0;
            Action::Fix
        } else {
            Action::None
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

    const BOARD_UV: Option<u32> = Some(5_100_000);

    #[test]
    fn fixes_a_suspended_charger_on_the_second_check() {
        // What the Kindle showed on the tablet board: power present, suspended, 900 mA adaptive.
        let stuck = regs(0x00, 0x05, 0xD1);
        let mut g = ChargeGuard::default();
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::None);
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::Fix);
        // Counting starts again afterwards.
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::None);
    }

    #[test]
    fn a_charging_reading_in_between_starts_the_count_again() {
        let stuck = regs(0x00, 0x05, 0xD1);
        let mut g = ChargeGuard::default();
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::None);
        assert_eq!(g.update(&regs(0x03, 0x01, 0xC8), Some(42), BOARD_UV), Action::None);
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::None);
        assert_eq!(g.update(&stuck, Some(42), BOARD_UV), Action::Fix);
    }

    #[test]
    fn leaves_normal_states_alone() {
        let mut g = ChargeGuard::default();
        for _ in 0..5 {
            // Fast charging on a PC port.
            assert_eq!(g.update(&regs(0x03, 0x01, 0xC8), Some(42), Some(4_590_000)), Action::None);
            // Unplugged.
            assert_eq!(g.update(&regs(0x00, 0x00, 0xC8), Some(42), Some(25_000)), Action::None);
            // Full.
            assert_eq!(g.update(&regs(0x00, 0x05, 0xD1), Some(100), BOARD_UV), Action::None);
            // Fix applied and charging from the board.
            assert_eq!(g.update(&regs(0x03, 0x01, 0x81), Some(42), Some(5_000_000)), Action::None);
            // Fix applied but still suspended: don't keep resetting the charger.
            assert_eq!(g.update(&regs(0x00, 0x05, 0x81), Some(42), BOARD_UV), Action::None);
        }
    }

    #[test]
    fn turns_the_adaptive_limit_back_on_when_unplugged() {
        // What the Kindle showed after unplugging with the fix in place: the battery holding
        // its own USB port at 3.8 V, so it still looked plugged in and charging.
        let back_fed = regs(0x03, 0x03, 0x81);
        let mut g = ChargeGuard::default();
        assert_eq!(g.update(&back_fed, Some(45), Some(3_805_000)), Action::Undo);
        // No VBUS reading: leave it.
        assert_eq!(g.update(&back_fed, Some(45), None), Action::None);
    }

    #[test]
    fn missing_registers_do_nothing() {
        let mut g = ChargeGuard::default();
        let empty = Registers::parse("");
        assert_eq!(g.update(&empty, None, None), Action::None);
        assert_eq!(g.update(&empty, None, None), Action::None);
    }
}
