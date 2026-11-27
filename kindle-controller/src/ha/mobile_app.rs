//! Registering with HA's `mobile_app` integration, which is how the controller appears as a
//! device ("Aircon Wall Controller") with its own diagnostic sensors.
//!
//! Registration is a one-off REST call (`POST /api/mobile_app/registrations`) that returns a
//! webhook id, which we keep in `state.json`. After that, everything goes through that
//! webhook, which we call over the WebSocket connection with `webhook/handle`.
//!
//! An unknown webhook id still gets HTTP 200 (HA doesn't reveal whether hooks exist), but
//! with an empty body. So if `register_sensor` doesn't answer `{"success": true}`, the
//! registration has been deleted in HA and we register again.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::HttpBase;
use crate::config::DeviceConfig;
use crate::diagnostics::Diagnostics;

pub const APP_ID: &str = "aircon_wall_controller";

/// What we remember between runs, in `state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredState {
    pub device_id: String,
    pub webhook_id: Option<String>,
}

impl StoredState {
    pub fn load(path: &Path) -> Self {
        let mut s: StoredState = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if s.device_id.is_empty() {
            s.device_id = new_device_id();
            s.save(path);
        }
        s
    }

    pub fn save(&self, path: &Path) {
        let text = serde_json::to_string_pretty(self).expect("serializable");
        // Write then rename, so a power cut can't leave a half-written file.
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path)) {
            warn!("can't save {}: {e}", path.display());
        }
    }
}

fn new_device_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{nanos:x}-{:x}", std::process::id())
        })
}

fn os_version() -> String {
    // e.g. "Kindle 5.16.4 (4709130010)"
    std::fs::read_to_string("/etc/prettyversion.txt")
        .ok()
        .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
        .unwrap_or_else(|| "unknown".into())
}

/// Registers this device and returns the new webhook id.
pub fn register(base: &HttpBase, token: &str, device: &DeviceConfig, device_id: &str) -> Result<String> {
    let body = json!({
        "device_id": device_id,
        "app_id": APP_ID,
        "app_name": "Aircon Wall Controller",
        "app_version": env!("CARGO_PKG_VERSION"),
        "device_name": device.name,
        "manufacturer": device.manufacturer,
        "model": device.model,
        "os_name": "Kindle",
        "os_version": os_version(),
        "supports_encryption": false,
        "app_data": {},
    });
    let (status, text) = http_post_json(base, "/api/mobile_app/registrations", token, &body.to_string())
        .context("mobile_app registration request")?;
    if status != 200 && status != 201 {
        bail!(
            "mobile_app registration failed: HTTP {status} {} (is the mobile_app integration loaded?)",
            text.trim()
        );
    }
    let v: Value = serde_json::from_str(&text).context("mobile_app registration response")?;
    let id = v
        .get("webhook_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no webhook_id in registration response"))?;
    info!("registered with HA as \"{}\"", device.name);
    Ok(id.to_string())
}

struct Sensor {
    unique_id: &'static str,
    name: &'static str,
    device_class: Option<&'static str>,
    unit: Option<&'static str>,
    icon: &'static str,
    state_class: Option<&'static str>,
}

const SENSORS: [Sensor; 6] = [
    Sensor {
        unique_id: "battery_level",
        name: "Battery level",
        device_class: Some("battery"),
        unit: Some("%"),
        icon: "mdi:battery",
        state_class: Some("measurement"),
    },
    Sensor {
        unique_id: "charging_status",
        name: "Charging status",
        device_class: None,
        unit: None,
        icon: "mdi:battery-charging",
        state_class: None,
    },
    Sensor {
        unique_id: "battery_current",
        name: "Battery current",
        device_class: Some("current"),
        unit: Some("mA"),
        icon: "mdi:current-dc",
        state_class: Some("measurement"),
    },
    Sensor {
        unique_id: "battery_temperature",
        name: "Battery temperature",
        device_class: Some("temperature"),
        unit: Some("°C"),
        icon: "mdi:thermometer",
        state_class: Some("measurement"),
    },
    Sensor {
        unique_id: "uptime",
        name: "Uptime",
        device_class: Some("duration"),
        unit: Some("s"),
        icon: "mdi:timer-outline",
        state_class: None,
    },
    Sensor {
        unique_id: "wifi_signal",
        name: "Wi-Fi signal",
        device_class: Some("signal_strength"),
        unit: Some("dBm"),
        icon: "mdi:wifi",
        state_class: Some("measurement"),
    },
];

fn sensor_state(d: &Diagnostics, unique_id: &str) -> Value {
    match unique_id {
        "battery_level" => json!(d.battery_percent),
        "charging_status" => json!(d.charging_status),
        "battery_current" => json!(d.battery_current_ma),
        "battery_temperature" => json!(d.battery_temp_c),
        "uptime" => json!(d.uptime_secs),
        "wifi_signal" => json!(d.wifi_dbm),
        _ => Value::Null,
    }
}

/// One `register_sensor` webhook payload per sensor.
pub fn register_sensor_payloads(d: &Diagnostics) -> Vec<Value> {
    SENSORS
        .iter()
        .map(|s| {
            let mut data = json!({
                "type": "sensor",
                "unique_id": s.unique_id,
                "name": s.name,
                "icon": s.icon,
                "entity_category": "diagnostic",
                "state": sensor_state(d, s.unique_id),
            });
            if let Some(dc) = s.device_class {
                data["device_class"] = json!(dc);
            }
            if let Some(unit) = s.unit {
                data["unit_of_measurement"] = json!(unit);
            }
            if let Some(sc) = s.state_class {
                data["state_class"] = json!(sc);
            }
            json!({"type": "register_sensor", "data": data})
        })
        .collect()
}

/// A single `update_sensor_states` webhook payload covering all sensors.
pub fn update_sensors_payload(d: &Diagnostics) -> Value {
    let data: Vec<Value> = SENSORS
        .iter()
        .map(|s| {
            // The icon has to be resent, or HA resets it to the default.
            json!({"type": "sensor", "unique_id": s.unique_id, "icon": s.icon, "state": sensor_state(d, s.unique_id)})
        })
        .collect();
    json!({"type": "update_sensor_states", "data": data})
}

/// Whether a `register_sensor` webhook result shows the registration is alive.
pub fn register_result_ok(result: &Value) -> bool {
    let body = result.get("body").and_then(Value::as_str).unwrap_or("");
    // register_sensor answers 201 Created; an unknown webhook answers 200 with no body.
    webhook_status_ok(result)
        && serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|b| b.get("success").and_then(Value::as_bool))
            .unwrap_or(false)
}

/// Whether a `webhook/handle` result carries a 2xx HTTP status.
pub fn webhook_status_ok(result: &Value) -> bool {
    result.get("status").and_then(Value::as_u64).is_some_and(|s| (200..300).contains(&s))
}

/// Bare-bones HTTP/1.1 POST over plain TCP. Enough for the one registration call, without
/// pulling in an HTTP client crate. Returns (status, body).
fn http_post_json(base: &HttpBase, path: &str, token: &str, body: &str) -> Result<(u16, String)> {
    let addr = base
        .authority
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("can't resolve {}", base.authority))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(Duration::from_secs(20)))?;
    write!(
        stream,
        "POST {}{path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        base.path_prefix,
        base.authority,
        body.len()
    )?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    parse_http_response(&raw)
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, String)> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response"))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut body = raw[split + 4..].to_vec();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("malformed HTTP status line"))?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    if chunked {
        body = dechunk(&body)?;
    }
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = data.windows(2).position(|w| w == b"\r\n").ok_or_else(|| anyhow!("bad chunk"))?;
        let size_str = String::from_utf8_lossy(&data[..eol]);
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16)?;
        data = &data[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            bail!("truncated chunk");
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_chunked_responses() {
        let plain = b"HTTP/1.1 201 Created\r\nContent-Length: 5\r\n\r\nhello";
        assert_eq!(parse_http_response(plain).unwrap(), (201, "hello".into()));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(chunked).unwrap(), (200, "hello world".into()));
    }

    #[test]
    fn register_result_detects_dead_webhook() {
        assert!(register_result_ok(&json!({"status": 200, "body": "{\"success\": true}"})));
        // What HA 2026.10 actually sends.
        assert!(register_result_ok(&json!({"status": 201, "body": "{\"success\":true}"})));
        assert!(!register_result_ok(&json!({"status": 200, "body": ""})));
        assert!(!register_result_ok(&json!({"status": 200, "body": null})));
    }

    #[test]
    fn sensor_payloads() {
        let d = Diagnostics {
            battery_percent: Some(80),
            charging_status: Some("Not charging".into()),
            battery_current_ma: Some(-12),
            battery_temp_c: Some(27),
            uptime_secs: Some(60),
            wifi_dbm: None,
        };
        let regs = register_sensor_payloads(&d);
        assert_eq!(regs.len(), 6);
        assert_eq!(regs[0]["data"]["state"], json!(80));
        assert_eq!(regs[0]["data"]["unit_of_measurement"], json!("%"));
        assert_eq!(regs[0]["data"]["entity_category"], json!("diagnostic"));
        let status = &regs[1]["data"];
        assert_eq!(status["state"], json!("Not charging"));
        assert!(status.get("device_class").is_none());
        assert!(status.get("unit_of_measurement").is_none());
        assert_eq!(regs[2]["data"]["state"], json!(-12));
        assert_eq!(regs[2]["data"]["unit_of_measurement"], json!("mA"));
        assert_eq!(regs[3]["data"]["device_class"], json!("temperature"));
        assert!(regs[4]["data"].get("state_class").is_none());
        let upd = update_sensors_payload(&d);
        assert_eq!(upd["data"][1]["state"], json!("Not charging"));
        assert_eq!(upd["data"][3]["state"], json!(27));
        assert_eq!(upd["data"][5]["state"], Value::Null);
    }
}
