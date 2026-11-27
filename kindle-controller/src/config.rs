//! `config.toml`: everything that differs between installs. See `config.example.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub home_assistant: HaConfig,
    #[serde(default)]
    pub device: DeviceConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub touch: TouchConfig,
    #[serde(default)]
    pub framework: FrameworkConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub aircon: AirconConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing config file {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaConfig {
    /// Base URL, e.g. `http://192.168.1.10:8123`. Plain HTTP only for now.
    pub url: String,
    /// Long-lived access token (HA profile page → Security).
    pub token: String,
    /// How often to push battery / uptime / Wi-Fi to HA.
    #[serde(default = "default_diagnostics_interval")]
    pub diagnostics_interval_secs: u64,
}

fn default_diagnostics_interval() -> u64 {
    300
}

/// How the controller registers itself with HA's mobile_app integration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceConfig {
    pub name: String,
    pub manufacturer: String,
    pub model: String,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            name: "Aircon Wall Controller".into(),
            manufacturer: "Amazon".into(),
            model: "Kindle Paperwhite (10th gen)".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfig {
    pub framebuffer: PathBuf,
    /// TTF/OTF font. If unset, we pick one from the Kindle's own fonts.
    pub font: Option<PathBuf>,
    /// Flash an area of the screen once it has had this many partial refreshes, to clear
    /// ghosting. The whole screen flashes only when over half of it is due.
    pub full_refresh_every: u32,
    /// FBInk waveform names for the three kinds of update we do.
    pub waveform_fast: String,
    pub waveform_partial: String,
    pub waveform_full: String,
    /// Set if black and white come out swapped.
    pub invert: bool,
    /// Clockwise rotation of the UI on the panel: 0 or 180 for portrait, 90 or 270 for
    /// landscape.
    pub rotation: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            framebuffer: "/dev/fb0".into(),
            font: None,
            full_refresh_every: 20,
            waveform_fast: "DU".into(),
            waveform_partial: "GL16".into(),
            waveform_full: "GC16".into(),
            invert: false,
            rotation: 90,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TouchConfig {
    /// e.g. `/dev/input/event1`. If unset, the first multitouch device is used.
    pub device: Option<PathBuf>,
    /// Coordinate fix-ups, applied after scaling to 0..1 (swap first, then invert).
    pub swap_xy: bool,
    pub invert_x: bool,
    pub invert_y: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameworkMode {
    /// Leave the Kindle UI running, but hide its status bar and pause the processes
    /// listed in `pause_processes` so they don't draw over us.
    Coexist,
    /// Stop the Kindle UI entirely (`stop lab126_gui`) and restart it on exit.
    Stop,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrameworkConfig {
    pub mode: FrameworkMode,
    pub pause_processes: Vec<String>,
    /// Swallow power-button presses so the Kindle can't be put to sleep (sleep turns the
    /// touchscreen off). With this and powerd's `preventScreenSaver`, it never sleeps.
    pub block_power_button: bool,
}

impl Default for FrameworkConfig {
    fn default() -> Self {
        Self {
            mode: FrameworkMode::Stop,
            // Resumed in reverse order. Pausing Xorg is what stops existing windows (e.g.
            // blanket's battery indicator) being repainted over us; kindle_browser is started by
            // appmgrd rather than the UI job, and otherwise keeps running (and logging).
            pause_processes: ["awesome", "cvm", "blanket", "Xorg", "kindle_browser"].map(String::from).to_vec(),
            block_power_button: true,
        }
    }
}

/// What the top half of the screen shows.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DashboardConfig {
    /// A `weather.*` entity, for current conditions and the 24-hour forecast.
    pub weather: Option<String>,
    /// No longer used: rooms are set up in HA (the "Aircon rooms" blueprint) and read from
    /// the aircon's Rooms entity. Accepted so older config files still load.
    #[serde(rename = "rooms")]
    pub old_rooms: Option<toml::Value>,
}

/// The aircon's entities in HA. Entities that HA doesn't have are left off the screen, so
/// the thermostat ones can stay at their defaults with firmware that lacks them.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AirconConfig {
    /// Name of the HA device the aircon's power/mode/fan/lockout/no-flow entities are on. When set,
    /// they're found on that device by what they are, so the controller keeps working if
    /// HA gives them new ids (e.g. `switch.aircon_power_2` after the device is re-added).
    /// The ids below are then only tried first.
    pub device: Option<String>,
    /// switch / input_boolean: on/off.
    pub power: String,
    /// select / input_select: heat, cool, fan, auto.
    pub mode: String,
    /// select / input_select: low, medium, high.
    pub fan: String,
    /// sensor: seconds until the compressor may start again.
    pub compressor_lockout: Option<String>,
    /// binary_sensor (device class problem): on = the building's condenser-water pump isn't
    /// proving flow, so the firmware keeps the whole unit off. While it's on, an error panel
    /// takes the place of the aircon controls. Missing or unavailable counts as no error.
    pub no_flow: Option<String>,
    /// switch / input_boolean: on = thermostat (set point), off = continuous.
    pub thermostat: Option<String>,
    /// number / input_number: the set point.
    pub setpoint: Option<String>,
    /// select: "Average" or the room the thermostat goes by.
    pub thermostat_room: Option<String>,
    /// sensor: what the unit is really doing: Off, Heating, Cooling, Fan or No flow.
    /// Subscribed to, but not shown: the no-flow panel goes by `no_flow` instead.
    pub operation: Option<String>,
    /// sensor: which way Auto is working: Heat or Cool.
    pub auto_direction: Option<String>,
    /// No longer used: replaced by `operation`. Accepted so older config files still load.
    #[serde(rename = "actual_power")]
    pub old_actual_power: Option<toml::Value>,
    /// sensor: what the thermostat is doing, e.g. "Idle: at set point".
    pub thermostat_status: Option<String>,
    /// sensor: the temperature the thermostat is working from.
    pub thermostat_temperature: Option<String>,
    /// sensor whose `rooms` attribute lists the rooms and their temperature sensors.
    pub rooms: Option<String>,
    /// How far one tap of − / + moves the set point.
    pub setpoint_step: f64,
    /// The select's option names for each mode.
    pub mode_options: ModeOptions,
    /// The select's option names for each fan speed.
    pub fan_options: FanOptions,
}

impl AirconConfig {
    /// The configured entity ids, before looking for them on the device.
    pub fn ids(&self) -> crate::ha::resolve::AirconIds {
        crate::ha::resolve::AirconIds {
            power: self.power.clone(),
            mode: self.mode.clone(),
            fan: self.fan.clone(),
            compressor_lockout: self.compressor_lockout.clone(),
            no_flow: self.no_flow.clone(),
            thermostat: self.thermostat.clone(),
            setpoint: self.setpoint.clone(),
            thermostat_room: self.thermostat_room.clone(),
            operation: self.operation.clone(),
            auto_direction: self.auto_direction.clone(),
            thermostat_status: self.thermostat_status.clone(),
            thermostat_temperature: self.thermostat_temperature.clone(),
            rooms: self.rooms.clone(),
        }
    }

    /// Uses `ids` from now on.
    pub fn set_ids(&mut self, ids: crate::ha::resolve::AirconIds) {
        self.power = ids.power;
        self.mode = ids.mode;
        self.fan = ids.fan;
        self.compressor_lockout = ids.compressor_lockout;
        self.no_flow = ids.no_flow;
        self.thermostat = ids.thermostat;
        self.setpoint = ids.setpoint;
        self.thermostat_room = ids.thermostat_room;
        self.operation = ids.operation;
        self.auto_direction = ids.auto_direction;
        self.thermostat_status = ids.thermostat_status;
        self.thermostat_temperature = ids.thermostat_temperature;
        self.rooms = ids.rooms;
    }
}

impl Default for AirconConfig {
    fn default() -> Self {
        Self {
            device: None,
            power: "switch.aircon_power".into(),
            mode: "select.aircon_mode".into(),
            fan: "select.aircon_fan_speed".into(),
            compressor_lockout: Some("sensor.aircon_compressor_lockout".into()),
            no_flow: Some("binary_sensor.aircon_no_flow".into()),
            thermostat: Some("switch.aircon_thermostat".into()),
            setpoint: Some("number.aircon_set_point".into()),
            thermostat_room: Some("select.aircon_thermostat_room".into()),
            operation: Some("sensor.aircon_operation".into()),
            auto_direction: Some("sensor.aircon_auto_direction".into()),
            old_actual_power: None,
            thermostat_status: Some("sensor.aircon_thermostat_status".into()),
            thermostat_temperature: Some("sensor.aircon_thermostat_temperature".into()),
            rooms: Some("sensor.aircon_rooms".into()),
            setpoint_step: 0.5,
            mode_options: ModeOptions::default(),
            fan_options: FanOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModeOptions {
    pub heat: String,
    pub cool: String,
    pub fan: String,
    pub auto: String,
}

impl Default for ModeOptions {
    fn default() -> Self {
        Self { heat: "Heat".into(), cool: "Cool".into(), fan: "Fan only".into(), auto: "Auto".into() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FanOptions {
    pub low: String,
    pub medium: String,
    pub high: String,
}

impl Default for FanOptions {
    fn default() -> Self {
        Self { low: "Low".into(), medium: "Medium".into(), high: "High".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_config_uses_defaults() {
        let c = Config::parse(
            r#"
            [home_assistant]
            url = "http://192.168.1.10:8123"
            token = "abc"
            "#,
        )
        .unwrap();
        assert_eq!(c.home_assistant.diagnostics_interval_secs, 300);
        assert_eq!(c.framework.mode, FrameworkMode::Stop);
        assert_eq!(c.aircon.setpoint_step, 0.5);
        assert_eq!(c.aircon.mode_options.fan, "Fan only");
        assert_eq!(c.aircon.rooms.as_deref(), Some("sensor.aircon_rooms"));
        assert_eq!(c.aircon.no_flow.as_deref(), Some("binary_sensor.aircon_no_flow"));
        assert_eq!(c.display.rotation, 90);
        assert_eq!(c.device.name, "Aircon Wall Controller");
    }

    #[test]
    fn example_config_parses() {
        let c = Config::parse(include_str!("../config.example.toml")).unwrap();
        assert_eq!(c.framework.mode, FrameworkMode::Stop);
        assert_eq!(c.aircon.no_flow.as_deref(), Some("binary_sensor.aircon_no_flow"));
        assert!(c.dashboard.old_rooms.is_none());
    }

    #[test]
    fn old_room_lists_still_load() {
        let c = Config::parse(
            r#"
            [home_assistant]
            url = "http://192.168.1.10:8123"
            token = "abc"
            [[dashboard.rooms]]
            name = "Lounge"
            sensor = "sensor.lounge"
            "#,
        )
        .unwrap();
        assert!(c.dashboard.old_rooms.is_some());
    }

    #[test]
    fn old_actual_power_still_loads() {
        let c = Config::parse(
            r#"
            [home_assistant]
            url = "http://192.168.1.10:8123"
            token = "abc"
            [aircon]
            actual_power = "binary_sensor.aircon_actual_power"
            "#,
        )
        .unwrap();
        assert_eq!(c.aircon.operation.as_deref(), Some("sensor.aircon_operation"));
    }
}
