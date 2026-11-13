//! Entity states as kept up to date by HA's `subscribe_entities` command.
//!
//! That command sends compressed messages:
//! * `{"a": {entity_id: {"s": state, "a": attributes, ...}}}`: added (also the initial dump)
//! * `{"c": {entity_id: {"+": {"s": .., "a": {changed attrs}}, "-": {"a": [removed attrs]}}}}`
//! * `{"r": [entity_id, ...]}`: removed

use std::collections::BTreeMap;

use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityState {
    pub state: String,
    pub attributes: Map<String, Value>,
    /// When the state last changed (Unix seconds), if HA told us.
    pub last_changed: Option<f64>,
}

impl EntityState {
    pub fn friendly_name<'a>(&'a self, entity_id: &'a str) -> &'a str {
        self.attributes.get("friendly_name").and_then(Value::as_str).unwrap_or(entity_id)
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(Value::as_f64)
    }

    /// The state as a number, e.g. a temperature sensor's reading.
    pub fn number(&self) -> Option<f64> {
        self.state.parse().ok().filter(|v: &f64| v.is_finite())
    }

    /// The `options` attribute of a select entity.
    pub fn options(&self) -> Vec<&str> {
        self.attributes
            .get("options")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self.state.as_str(), "unavailable" | "unknown")
    }

    /// The aircon controller's room list: its Rooms sensor has
    /// `rooms: [{"name": ..., "sensor": ...}]` as an attribute.
    pub fn rooms(&self) -> Vec<Room> {
        self.attributes
            .get("rooms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let name = r.get("name")?.as_str()?.trim();
                let sensor = r.get("sensor")?.as_str()?.trim();
                (!name.is_empty() && !sensor.is_empty()).then(|| Room { name: name.into(), sensor: sensor.into() })
            })
            .collect()
    }
}

/// A room the aircon's thermostat knows about, and its temperature sensor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Room {
    pub name: String,
    pub sensor: String,
}

/// The rooms listed by the entity `rooms_entity`: `None` while it's missing or unavailable
/// (HA drops its attributes then), so callers can keep the last list they had.
pub fn rooms(map: &EntityMap, rooms_entity: Option<&str>) -> Option<Vec<Room>> {
    rooms_entity.and_then(|id| map.get(id)).filter(|s| !s.is_unavailable()).map(EntityState::rooms)
}

pub type EntityMap = BTreeMap<String, EntityState>;

/// Applies one `subscribe_entities` event to `map`.
pub fn apply_compressed(map: &mut EntityMap, event: &Value) {
    if let Some(added) = event.get("a").and_then(Value::as_object) {
        for (id, st) in added {
            let state = EntityState {
                state: st.get("s").and_then(Value::as_str).unwrap_or_default().to_string(),
                attributes: st.get("a").and_then(Value::as_object).cloned().unwrap_or_default(),
                last_changed: st.get("lc").or_else(|| st.get("lu")).and_then(Value::as_f64),
            };
            map.insert(id.clone(), state);
        }
    }
    if let Some(changed) = event.get("c").and_then(Value::as_object) {
        for (id, diff) in changed {
            let entry = map.entry(id.clone()).or_default();
            if let Some(plus) = diff.get("+") {
                if let Some(s) = plus.get("s").and_then(Value::as_str) {
                    entry.state = s.to_string();
                }
                if let Some(lc) = plus.get("lc").and_then(Value::as_f64) {
                    entry.last_changed = Some(lc);
                }
                if let Some(attrs) = plus.get("a").and_then(Value::as_object) {
                    for (k, v) in attrs {
                        entry.attributes.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(gone) = diff.get("-").and_then(|m| m.get("a")).and_then(Value::as_array) {
                for k in gone.iter().filter_map(Value::as_str) {
                    entry.attributes.remove(k);
                }
            }
        }
    }
    if let Some(removed) = event.get("r").and_then(Value::as_array) {
        for id in removed.iter().filter_map(Value::as_str) {
            map.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn add_change_remove() {
        let mut m = EntityMap::new();
        apply_compressed(
            &mut m,
            &json!({"a": {
                "light.kitchen": {"s": "off", "a": {"friendly_name": "Kitchen", "x": 1}, "c": "id", "lc": 1.0},
                "light.hall": {"s": "on", "a": {"brightness": 128}}
            }}),
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m["light.kitchen"].friendly_name("light.kitchen"), "Kitchen");

        apply_compressed(
            &mut m,
            &json!({"c": {"light.kitchen": {"+": {"s": "on", "a": {"brightness": 255}, "lc": 2.0}, "-": {"a": ["x"]}}}}),
        );
        let k = &m["light.kitchen"];
        assert_eq!(k.state, "on");
        assert_eq!(k.attr_f64("brightness"), Some(255.0));
        assert!(!k.attributes.contains_key("x"));
        assert_eq!(k.friendly_name("light.kitchen"), "Kitchen");
        assert_eq!(k.last_changed, Some(2.0));

        // A change with only a context update leaves state alone.
        apply_compressed(&mut m, &json!({"c": {"light.hall": {"+": {"c": "abc", "lu": 3.0}}}}));
        assert_eq!(m["light.hall"].state, "on");

        apply_compressed(&mut m, &json!({"r": ["light.hall"]}));
        assert!(!m.contains_key("light.hall"));
    }

    #[test]
    fn rooms_from_the_controller() {
        let mut m = EntityMap::new();
        apply_compressed(
            &mut m,
            &json!({"a": {"sensor.aircon_rooms": {"s": "2", "a": {"rooms": [
                {"name": "Lounge", "sensor": "sensor.lounge"},
                {"name": " ", "sensor": "sensor.nameless"},
                {"name": "Study"},
                {"name": "Kitchen", "sensor": "sensor.kitchen"}
            ]}}}}),
        );
        let rooms = rooms(&m, Some("sensor.aircon_rooms")).unwrap();
        assert_eq!(rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Lounge", "Kitchen"]);
        assert_eq!(rooms[1].sensor, "sensor.kitchen");
        assert_eq!(super::rooms(&m, Some("sensor.nope")), None);
        assert_eq!(super::rooms(&m, None), None);
        apply_compressed(&mut m, &json!({"c": {"sensor.aircon_rooms": {"+": {"s": "unavailable"}}}}));
        assert_eq!(super::rooms(&m, Some("sensor.aircon_rooms")), None);
    }
}
