//! Finds the aircon's entities by what they are rather than by entity id.
//!
//! Entity ids aren't stable: re-adding a device (e.g. after swapping the ESP32, whose MAC
//! is part of the MQTT unique ids) gives its entities new ids like `switch.aircon_power_2`.
//! So when `aircon.device` is set, we look in HA's entity and device registries for the
//! entities on that device that fill each role (power switch, mode select, ...), and pick
//! whichever candidate actually has a live state. The ids in the config are tried first,
//! so a config that's still right keeps working exactly as before.

use std::collections::HashMap;

use serde_json::Value;

use super::entities::EntityMap;

/// The entity ids the UI should use for the aircon. The optional ones may not exist in HA
/// (e.g. older firmware); the UI hides what they control.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AirconIds {
    pub power: String,
    pub mode: String,
    pub fan: String,
    pub compressor_lockout: Option<String>,
    pub thermostat: Option<String>,
    pub setpoint: Option<String>,
    pub thermostat_room: Option<String>,
    pub operation: Option<String>,
    pub auto_direction: Option<String>,
    pub thermostat_status: Option<String>,
    pub thermostat_temperature: Option<String>,
    pub rooms: Option<String>,
}

impl AirconIds {
    /// The id for each role, in `Role::ALL` order.
    fn get(&self, role: Role) -> Option<&String> {
        match role {
            Role::Power => Some(&self.power),
            Role::Mode => Some(&self.mode),
            Role::Fan => Some(&self.fan),
            Role::Lockout => self.compressor_lockout.as_ref(),
            Role::Thermostat => self.thermostat.as_ref(),
            Role::Setpoint => self.setpoint.as_ref(),
            Role::ThermostatRoom => self.thermostat_room.as_ref(),
            Role::Operation => self.operation.as_ref(),
            Role::AutoDirection => self.auto_direction.as_ref(),
            Role::ThermostatStatus => self.thermostat_status.as_ref(),
            Role::ThermostatTemperature => self.thermostat_temperature.as_ref(),
            Role::Rooms => self.rooms.as_ref(),
        }
    }

    fn set(&mut self, role: Role, id: Option<String>) {
        match role {
            Role::Power => self.power = id.unwrap_or_default(),
            Role::Mode => self.mode = id.unwrap_or_default(),
            Role::Fan => self.fan = id.unwrap_or_default(),
            Role::Lockout => self.compressor_lockout = id,
            Role::Thermostat => self.thermostat = id,
            Role::Setpoint => self.setpoint = id,
            Role::ThermostatRoom => self.thermostat_room = id,
            Role::Operation => self.operation = id,
            Role::AutoDirection => self.auto_direction = id,
            Role::ThermostatStatus => self.thermostat_status = id,
            Role::ThermostatTemperature => self.thermostat_temperature = id,
            Role::Rooms => self.rooms = id,
        }
    }

    pub fn all(&self) -> Vec<String> {
        Role::ALL.iter().filter_map(|r| self.get(*r).cloned()).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Power,
    Mode,
    Fan,
    Lockout,
    Thermostat,
    Setpoint,
    ThermostatRoom,
    Operation,
    AutoDirection,
    ThermostatStatus,
    ThermostatTemperature,
    Rooms,
}

impl Role {
    const ALL: [Role; 12] = [
        Role::Power,
        Role::Mode,
        Role::Fan,
        Role::Lockout,
        Role::Thermostat,
        Role::Setpoint,
        Role::ThermostatRoom,
        Role::Operation,
        Role::AutoDirection,
        Role::ThermostatStatus,
        Role::ThermostatTemperature,
        Role::Rooms,
    ];

    fn domains(self) -> &'static [&'static str] {
        match self {
            Role::Power | Role::Thermostat => &["switch", "input_boolean"],
            Role::Mode | Role::Fan | Role::ThermostatRoom => &["select", "input_select"],
            Role::Setpoint => &["number", "input_number"],
            Role::Lockout
            | Role::Operation
            | Role::AutoDirection
            | Role::ThermostatStatus
            | Role::ThermostatTemperature
            | Role::Rooms => &["sensor"],
        }
    }

    /// Endings of the MQTT unique id (`<node id>_<object id>`) our firmware uses.
    fn unique_id_suffixes(self) -> &'static [&'static str] {
        match self {
            Role::Power => &["_power"],
            Role::Mode => &["_mode"],
            Role::Fan => &["_fan", "_fan_speed"],
            Role::Lockout => &["_lockout", "_compressor_lockout"],
            Role::Thermostat => &["_thermostat"],
            Role::Setpoint => &["_setpoint"],
            Role::ThermostatRoom => &["_thermostat_room"],
            Role::Operation => &["_operation"],
            Role::AutoDirection => &["_auto_direction"],
            Role::ThermostatStatus => &["_thermostat_status"],
            Role::ThermostatTemperature => &["_thermostat_temperature"],
            Role::Rooms => &["_rooms"],
        }
    }

    /// The entity's own name on the device (without the device name).
    fn names(self) -> &'static [&'static str] {
        match self {
            Role::Power => &["power"],
            Role::Mode => &["mode"],
            Role::Fan => &["fan speed", "fan"],
            Role::Lockout => &["compressor lockout"],
            Role::Thermostat => &["thermostat"],
            Role::Setpoint => &["set point"],
            Role::ThermostatRoom => &["thermostat room"],
            Role::Operation => &["operation"],
            Role::AutoDirection => &["auto direction"],
            Role::ThermostatStatus => &["thermostat status"],
            Role::ThermostatTemperature => &["thermostat temperature"],
            Role::Rooms => &["rooms"],
        }
    }
}

#[derive(Clone, Debug)]
struct RegEntity {
    entity_id: String,
    unique_id: String,
    device_id: Option<String>,
    original_name: Option<String>,
    name: Option<String>,
    disabled: bool,
    created_at: f64,
}

/// The parts of HA's entity and device registries we need.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    entities: Vec<RegEntity>,
    /// device id → names it goes by (user-given and integration-given).
    devices: HashMap<String, Vec<String>>,
}

impl Registry {
    /// Parses the results of `config/entity_registry/list` and `config/device_registry/list`.
    pub fn parse(entities: &Value, devices: &Value) -> Self {
        let s = |v: &Value| v.as_str().map(str::to_string);
        let entities = entities
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(RegEntity {
                    entity_id: s(&e["entity_id"])?,
                    unique_id: s(&e["unique_id"]).unwrap_or_default(),
                    device_id: s(&e["device_id"]),
                    original_name: s(&e["original_name"]),
                    name: s(&e["name"]),
                    disabled: !e["disabled_by"].is_null(),
                    created_at: e["created_at"].as_f64().unwrap_or(0.0),
                })
            })
            .collect();
        let devices = devices
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let names = [&d["name_by_user"], &d["name"]].into_iter().filter_map(s).collect();
                Some((s(&d["id"])?, names))
            })
            .collect();
        Self { entities, devices }
    }

    /// Entities on a device called `device` that could fill `role`, newest first.
    pub fn candidates(&self, device: &str, role: Role) -> Vec<String> {
        let device = device.trim();
        let mut found: Vec<&RegEntity> = self
            .entities
            .iter()
            .filter(|e| !e.disabled)
            .filter(|e| {
                e.device_id
                    .as_ref()
                    .and_then(|d| self.devices.get(d))
                    .is_some_and(|names| names.iter().any(|n| n.trim().eq_ignore_ascii_case(device)))
            })
            .filter(|e| role.domains().contains(&e.entity_id.split('.').next().unwrap_or("")))
            .filter(|e| {
                let by_uid = role.unique_id_suffixes().iter().any(|s| e.unique_id.ends_with(s));
                let by_name = [&e.original_name, &e.name]
                    .into_iter()
                    .flatten()
                    .any(|n| role.names().iter().any(|r| n.trim().eq_ignore_ascii_case(r)));
                by_uid || by_name
            })
            .collect();
        found.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));
        found.into_iter().map(|e| e.entity_id.clone()).collect()
    }
}

/// Candidate ids for each role (in `Role::ALL` order): the configured one first, then what
/// the registry found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Candidates {
    by_role: Vec<Vec<String>>,
}

impl Candidates {
    pub fn new(configured: &AirconIds, registry: Option<(&Registry, &str)>) -> Self {
        let list = |role: Role| {
            let mut v: Vec<String> = configured.get(role).cloned().into_iter().collect();
            if let Some((reg, device)) = registry {
                for id in reg.candidates(device, role) {
                    if !v.contains(&id) {
                        v.push(id);
                    }
                }
            }
            v
        };
        Self { by_role: Role::ALL.iter().map(|r| list(*r)).collect() }
    }

    pub fn all(&self) -> Vec<String> {
        self.by_role.iter().flatten().cloned().collect()
    }

    /// Picks, for each role, the first candidate with a live state; failing that, the first
    /// HA knows about at all; failing that, the first (the configured id).
    pub fn choose(&self, states: &EntityMap) -> AirconIds {
        let pick = |c: &[String]| -> Option<String> {
            c.iter()
                .find(|id| states.get(*id).is_some_and(|s| !s.is_unavailable()))
                .or_else(|| c.iter().find(|id| states.contains_key(*id)))
                .or_else(|| c.first())
                .cloned()
        };
        let mut ids = AirconIds::default();
        for (role, candidates) in Role::ALL.iter().zip(&self.by_role) {
            ids.set(*role, pick(candidates));
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ha::entities::EntityState;
    use serde_json::json;

    fn registry() -> Registry {
        Registry::parse(
            &json!([
                // The old device's entities, still in the registry but gone from HA's states.
                {"entity_id": "switch.aircon_power", "unique_id": "aircon_aaaa_power", "device_id": "old", "original_name": "Power", "disabled_by": null, "created_at": 100.0},
                {"entity_id": "select.aircon_mode", "unique_id": "aircon_aaaa_mode", "device_id": "old", "original_name": "Mode", "disabled_by": null, "created_at": 100.0},
                // The new device.
                {"entity_id": "switch.aircon_power_2", "unique_id": "aircon_bbbb_power", "device_id": "new", "original_name": "Power", "disabled_by": null, "created_at": 200.0},
                {"entity_id": "select.aircon_mode_2", "unique_id": "aircon_bbbb_mode", "device_id": "new", "original_name": "Mode", "disabled_by": null, "created_at": 200.0},
                {"entity_id": "select.aircon_fan_speed_2", "unique_id": "aircon_bbbb_fan", "device_id": "new", "original_name": "Fan speed", "disabled_by": null, "created_at": 200.0},
                {"entity_id": "sensor.aircon_compressor_lockout_2", "unique_id": "aircon_bbbb_lockout", "device_id": "new", "original_name": "Compressor lockout", "disabled_by": null, "created_at": 200.0},
                {"entity_id": "binary_sensor.aircon_compressor_2", "unique_id": "aircon_bbbb_compressor", "device_id": "new", "original_name": "Compressor", "disabled_by": null, "created_at": 200.0},
                // Same name, but on another device, or disabled.
                {"entity_id": "switch.kettle_power", "unique_id": "kettle_power", "device_id": "kettle", "original_name": "Power", "disabled_by": null},
                {"entity_id": "switch.aircon_power_3", "unique_id": "aircon_cccc_power", "device_id": "new", "disabled_by": "user", "created_at": 300.0},
            ]),
            &json!([
                {"id": "old", "name": "Aircon", "name_by_user": null},
                {"id": "new", "name": "Aircon", "name_by_user": null},
                {"id": "kettle", "name": "Kettle", "name_by_user": "Aircon kettle"},
            ]),
        )
    }

    fn live(ids: &[&str]) -> EntityMap {
        ids.iter().map(|id| (id.to_string(), EntityState { state: "on".into(), ..Default::default() })).collect()
    }

    fn configured() -> AirconIds {
        AirconIds {
            power: "switch.aircon_power".into(),
            mode: "select.aircon_mode".into(),
            fan: "select.aircon_fan_speed".into(),
            compressor_lockout: Some("sensor.aircon_compressor_lockout".into()),
            ..Default::default()
        }
    }

    #[test]
    fn finds_the_thermostat_entities() {
        let r = Registry::parse(
            &json!([
                {"entity_id": "switch.aircon_power", "unique_id": "aircon_bbbb_power", "device_id": "d", "original_name": "Power", "disabled_by": null},
                {"entity_id": "switch.aircon_thermostat_2", "unique_id": "aircon_bbbb_thermostat", "device_id": "d", "original_name": "Thermostat", "disabled_by": null},
                {"entity_id": "number.aircon_set_point_2", "unique_id": "aircon_bbbb_setpoint", "device_id": "d", "original_name": "Set point", "disabled_by": null},
                {"entity_id": "select.aircon_thermostat_room_2", "unique_id": "aircon_bbbb_thermostat_room", "device_id": "d", "original_name": "Thermostat room", "disabled_by": null},
                {"entity_id": "sensor.aircon_operation_2", "unique_id": "aircon_bbbb_operation", "device_id": "d", "original_name": "Operation", "disabled_by": null},
                {"entity_id": "sensor.aircon_auto_direction_2", "unique_id": "aircon_bbbb_auto_direction", "device_id": "d", "original_name": "Auto direction", "disabled_by": null},
                // The binary sensor older firmware had ends in "_power", but isn't a Power candidate.
                {"entity_id": "binary_sensor.aircon_actual_power", "unique_id": "aircon_bbbb_actual_power", "device_id": "d", "original_name": "Actual power", "disabled_by": null},
                {"entity_id": "sensor.aircon_thermostat_status_2", "unique_id": "aircon_bbbb_thermostat_status", "device_id": "d", "original_name": "Thermostat status", "disabled_by": null},
                {"entity_id": "sensor.aircon_thermostat_temperature_2", "unique_id": "aircon_bbbb_thermostat_temperature", "device_id": "d", "original_name": "Thermostat temperature", "disabled_by": null},
                {"entity_id": "sensor.aircon_rooms_2", "unique_id": "aircon_bbbb_rooms", "device_id": "d", "original_name": "Rooms", "disabled_by": null},
            ]),
            &json!([{"id": "d", "name": "Aircon", "name_by_user": null}]),
        );
        // "_actual_power" ends in "_power", but it's a binary sensor, so it isn't a Power candidate.
        assert_eq!(r.candidates("Aircon", Role::Power), ["switch.aircon_power"]);
        let c = Candidates::new(&configured(), Some((&r, "Aircon")));
        let ids = c.choose(&live(&[
            "switch.aircon_thermostat_2",
            "number.aircon_set_point_2",
            "select.aircon_thermostat_room_2",
            "sensor.aircon_operation_2",
            "sensor.aircon_auto_direction_2",
            "sensor.aircon_thermostat_status_2",
            "sensor.aircon_thermostat_temperature_2",
            "sensor.aircon_rooms_2",
        ]));
        assert_eq!(ids.thermostat.as_deref(), Some("switch.aircon_thermostat_2"));
        assert_eq!(ids.setpoint.as_deref(), Some("number.aircon_set_point_2"));
        assert_eq!(ids.thermostat_room.as_deref(), Some("select.aircon_thermostat_room_2"));
        assert_eq!(ids.operation.as_deref(), Some("sensor.aircon_operation_2"));
        assert_eq!(ids.auto_direction.as_deref(), Some("sensor.aircon_auto_direction_2"));
        assert_eq!(ids.thermostat_status.as_deref(), Some("sensor.aircon_thermostat_status_2"));
        assert_eq!(ids.thermostat_temperature.as_deref(), Some("sensor.aircon_thermostat_temperature_2"));
        assert_eq!(ids.rooms.as_deref(), Some("sensor.aircon_rooms_2"));
    }

    #[test]
    fn finds_role_candidates_on_the_device_newest_first() {
        let r = registry();
        assert_eq!(r.candidates("aircon", Role::Power), ["switch.aircon_power_2", "switch.aircon_power"]);
        assert_eq!(r.candidates("Aircon", Role::Fan), ["select.aircon_fan_speed_2"]);
        assert_eq!(r.candidates("Aircon", Role::Lockout), ["sensor.aircon_compressor_lockout_2"]);
        assert!(r.candidates("Nope", Role::Power).is_empty());
    }

    #[test]
    fn follows_renamed_entities() {
        let r = registry();
        let c = Candidates::new(&configured(), Some((&r, "Aircon")));
        let states = live(&[
            "switch.aircon_power_2",
            "select.aircon_mode_2",
            "select.aircon_fan_speed_2",
            "sensor.aircon_compressor_lockout_2",
        ]);
        let ids = c.choose(&states);
        assert_eq!(ids.power, "switch.aircon_power_2");
        assert_eq!(ids.mode, "select.aircon_mode_2");
        assert_eq!(ids.fan, "select.aircon_fan_speed_2");
        assert_eq!(ids.compressor_lockout.as_deref(), Some("sensor.aircon_compressor_lockout_2"));
    }

    #[test]
    fn prefers_the_configured_id_while_it_is_live() {
        let r = registry();
        let c = Candidates::new(&configured(), Some((&r, "Aircon")));
        let states = live(&["switch.aircon_power", "switch.aircon_power_2"]);
        assert_eq!(c.choose(&states).power, "switch.aircon_power");
        // Unavailable loses to a live one.
        let mut states = states;
        states.get_mut("switch.aircon_power").unwrap().state = "unavailable".into();
        assert_eq!(c.choose(&states).power, "switch.aircon_power_2");
    }

    #[test]
    fn without_a_registry_uses_the_config() {
        let c = Candidates::new(&configured(), None);
        assert_eq!(c.choose(&EntityMap::new()), configured());
        let mut cfg = configured();
        cfg.compressor_lockout = None;
        assert_eq!(Candidates::new(&cfg, None).choose(&EntityMap::new()).compressor_lockout, None);
    }
}
