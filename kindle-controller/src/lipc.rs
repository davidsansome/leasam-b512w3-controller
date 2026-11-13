//! Helpers for running Kindle commands, mainly `lipc-get-prop` / `lipc-set-prop`, which
//! talk to Kindle services such as powerd.

use std::process::{Command, Stdio};

use log::{debug, warn};

/// Reads an integer property, e.g. `get_int("com.lab126.powerd", "battLevel")`.
pub fn get_int(service: &str, prop: &str) -> Option<i64> {
    let out = Command::new("lipc-get-prop").args(["-i", service, prop]).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Reads a string property, e.g. `get_string("com.lab126.powerd", "state")`.
pub fn get_string(service: &str, prop: &str) -> Option<String> {
    let out = Command::new("lipc-get-prop").args(["-s", service, prop]).stdin(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Sets a property; numeric values are sent as integers, everything else as strings.
pub fn set(service: &str, prop: &str, value: &str) -> bool {
    let kind = if value.parse::<i64>().is_ok() { "-i" } else { "-s" };
    run("lipc-set-prop", &[kind, service, prop, value])
}

/// Runs a command, logging (not failing) if it doesn't work. Returns whether it succeeded.
pub fn run(cmd: &str, args: &[&str]) -> bool {
    debug!("running {cmd} {}", args.join(" "));
    match Command::new(cmd).args(args).stdin(Stdio::null()).output() {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            debug!(
                "{cmd} {} failed ({}): {}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
            false
        }
        Err(e) => {
            warn!("can't run {cmd}: {e}");
            false
        }
    }
}
