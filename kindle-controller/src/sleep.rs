//! Sleep mode: Wi-Fi and the touchscreen off, until the power button is pressed.
//!
//! The Kindle itself stays awake: powerd's screensaver would draw over us and needs the
//! framework, and it never suspends on USB power anyway. Only the radio and touch panel are
//! switched off. Both go through lipc so the Kindle's own services stay in charge of them.

use log::{info, warn};

use crate::lipc;

/// Turns Wi-Fi on or off through the Kindle's connection manager.
pub fn set_wifi(on: bool) {
    info!("Wi-Fi {}", if on { "on" } else { "off" });
    if !lipc::set("com.lab126.cmd", "wirelessEnable", if on { "1" } else { "0" }) {
        warn!("couldn't turn Wi-Fi {}", if on { "on" } else { "off" });
    }
}

/// Turns the touch panel on or off through deviced. If that doesn't work (the property
/// isn't there on every firmware), touches are still ignored while asleep, but the panel
/// stays powered.
pub fn set_touch(on: bool) {
    info!("touchscreen {}", if on { "on" } else { "off" });
    if !lipc::set("com.lab126.deviced", "enable_touch", if on { "1" } else { "0" }) {
        warn!("couldn't turn the touchscreen {} through deviced; ignoring touches instead", if on { "on" } else { "off" });
    }
}

/// Whether Wi-Fi is connected right now, according to wifid. Used at startup, since
/// `NetworkWatcher` only hears about connections made after it starts.
pub fn wifi_connected() -> bool {
    lipc::get_string("com.lab126.wifid", "cmState").as_deref() == Some("CONNECTED")
}

/// Listens for the Kindle saying it has a working network connection again, through
/// `lipc-wait-event` on the connection manager. That's lipc's event bus, which is how the
/// Kindle's services announce changes (it's built on D-Bus underneath). Turning Wi-Fi on
/// gives, about 11 s later on the PW4:
///
/// ```text
/// com.lab126.wifid cmConnected
/// com.lab126.cmd   InternetConnected 1
/// com.lab126.cmd   connectionAvailable "wifi" "internet"
/// ```
///
/// We wait for the last one, which comes once the interface has an address. The child
/// process is killed when this is dropped.
pub struct NetworkWatcher {
    child: std::process::Child,
}

impl NetworkWatcher {
    /// Calls `on_up` (from a background thread) each time the network comes up.
    pub fn spawn(on_up: impl Fn() + Send + 'static) -> Option<Self> {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let mut child = match Command::new("lipc-wait-event")
            .args(["-m", "com.lab126.cmd", "connectionAvailable"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("can't listen for network changes: {e}");
                return None;
            }
        };
        let stdout = child.stdout.take()?;
        let spawned = std::thread::Builder::new().name("network".into()).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                info!("network up: {}", line.trim());
                on_up();
            }
        });
        if let Err(e) = spawned {
            warn!("couldn't start the network thread: {e}");
        }
        Some(Self { child })
    }
}

impl Drop for NetworkWatcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
