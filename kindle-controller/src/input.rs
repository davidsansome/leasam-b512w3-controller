//! Touchscreen input, read straight from evdev on a background thread.
//!
//! We grab the device (EVIOCGRAB) so the Kindle UI underneath doesn't also react to our
//! taps. Only the first finger is tracked: this UI only needs taps and long-presses.

use std::time::Duration;

use anyhow::Result;

use crate::config::TouchConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchEvent {
    Down { x: i32, y: i32 },
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // only produced on the device
    Up { x: i32, y: i32, held: Duration },
}

/// Maps raw panel coordinates to screen pixels.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub x_range: (i32, i32),
    pub y_range: (i32, i32),
    pub screen: (i32, i32),
    pub swap_xy: bool,
    pub invert_x: bool,
    pub invert_y: bool,
}

impl Mapping {
    pub fn map(&self, raw_x: i32, raw_y: i32) -> (i32, i32) {
        let norm = |v: i32, (lo, hi): (i32, i32)| {
            if hi > lo { ((v - lo) as f32 / (hi - lo) as f32).clamp(0.0, 1.0) } else { 0.0 }
        };
        let (mut fx, mut fy) = (norm(raw_x, self.x_range), norm(raw_y, self.y_range));
        if self.swap_xy {
            std::mem::swap(&mut fx, &mut fy);
        }
        if self.invert_x {
            fx = 1.0 - fx;
        }
        if self.invert_y {
            fy = 1.0 - fy;
        }
        let (w, h) = self.screen;
        ((fx * (w - 1) as f32).round() as i32, (fy * (h - 1) as f32).round() as i32)
    }
}

/// Takes exclusive grabs on every input device with a power key, so powerd never sees
/// presses and can't send the Kindle into screensaver mode (which turns touch off). The
/// grabs last as long as the returned files are kept open.
pub fn grab_power_buttons() -> Vec<std::fs::File> {
    imp::grab_power_buttons()
}

/// Starts the input thread. `on_event` is called from that thread.
pub fn spawn(
    cfg: &TouchConfig,
    screen: (i32, i32),
    on_event: impl Fn(TouchEvent) + Send + 'static,
) -> Result<()> {
    imp::spawn(cfg, screen, on_event)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use anyhow::{Context, Result, bail};
    use log::{debug, error, info, warn};

    use super::{Mapping, TouchEvent};
    use crate::config::TouchConfig;

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_ABS: u16 = 0x03;
    const SYN_REPORT: u16 = 0;
    const BTN_TOUCH: u16 = 0x14a;
    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;
    const ABS_MT_SLOT: u16 = 0x2f;
    const ABS_MT_POSITION_X: u16 = 0x35;
    const ABS_MT_POSITION_Y: u16 = 0x36;
    const ABS_MT_TRACKING_ID: u16 = 0x39;
    const KEY_POWER: u16 = 116;

    /// Kernel `struct input_event`: a timeval (two longs) then type, code, value.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct InputEvent {
        time: [libc::c_long; 2],
        type_: u16,
        code: u16,
        value: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }

    const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
        (dir << 30) | ((size as u32) << 16) | ((b'E' as u32) << 8) | nr
    }
    const IOC_WRITE: u32 = 1;
    const IOC_READ: u32 = 2;

    fn ioctl<T>(file: &File, request: u32, arg: *mut T) -> std::io::Result<()> {
        // SAFETY: callers pass a buffer of the size encoded in `request`.
        let r = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg) };
        if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }

    fn device_name(file: &File) -> String {
        let mut buf = [0u8; 256];
        match ioctl(file, ioc(IOC_READ, 0x06, buf.len()), buf.as_mut_ptr()) {
            Ok(()) => String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string(),
            Err(_) => "?".into(),
        }
    }

    fn has_abs(file: &File, code: u16) -> bool {
        let mut bits = [0u8; 8];
        ioctl(file, ioc(IOC_READ, 0x20 + u32::from(EV_ABS), bits.len()), bits.as_mut_ptr()).is_ok()
            && bits[(code / 8) as usize] & (1 << (code % 8)) != 0
    }

    fn has_key(file: &File, code: u16) -> bool {
        let mut bits = [0u8; 96];
        ioctl(file, ioc(IOC_READ, 0x20 + u32::from(EV_KEY), bits.len()), bits.as_mut_ptr()).is_ok()
            && bits[(code / 8) as usize] & (1 << (code % 8)) != 0
    }

    fn grab(file: &File) -> std::io::Result<()> {
        let mut on: libc::c_int = 1;
        ioctl(file, ioc(IOC_WRITE, 0x90, size_of::<libc::c_int>()), &mut on)
    }

    pub fn grab_power_buttons() -> Vec<File> {
        let Ok(dir) = std::fs::read_dir("/dev/input") else { return Vec::new() };
        let mut grabbed = Vec::new();
        for path in dir.flatten().map(|e| e.path()) {
            if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")) {
                continue;
            }
            let Ok(file) = File::open(&path) else { continue };
            if !has_key(&file, KEY_POWER) || has_abs(&file, ABS_MT_POSITION_X) {
                continue;
            }
            match grab(&file) {
                Ok(()) => {
                    info!("blocking the power button on {} ({})", path.display(), device_name(&file));
                    grabbed.push(file);
                }
                Err(e) => warn!("couldn't grab power button {}: {e}", path.display()),
            }
        }
        grabbed
    }

    fn abs_range(file: &File, code: u16) -> Result<(i32, i32)> {
        let mut info = AbsInfo::default();
        ioctl(file, ioc(IOC_READ, 0x40 + u32::from(code), size_of::<AbsInfo>()), &mut info)
            .with_context(|| format!("EVIOCGABS({code:#x})"))?;
        Ok((info.minimum, info.maximum))
    }

    fn find_device(configured: Option<&Path>) -> Result<PathBuf> {
        if let Some(p) = configured {
            return Ok(p.to_path_buf());
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir("/dev/input")?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
            .collect();
        paths.sort();
        for p in &paths {
            if let Ok(f) = File::open(p) {
                debug!("{}: {}", p.display(), device_name(&f));
                if has_abs(&f, ABS_MT_POSITION_X) {
                    return Ok(p.clone());
                }
            }
        }
        bail!("no multitouch device found in /dev/input; set touch.device")
    }

    pub fn spawn(
        cfg: &TouchConfig,
        screen: (i32, i32),
        on_event: impl Fn(TouchEvent) + Send + 'static,
    ) -> Result<()> {
        let path = find_device(cfg.device.as_deref())?;
        let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let multitouch = has_abs(&file, ABS_MT_POSITION_X);
        let (cx, cy) = if multitouch { (ABS_MT_POSITION_X, ABS_MT_POSITION_Y) } else { (ABS_X, ABS_Y) };
        let mapping = Mapping {
            x_range: abs_range(&file, cx)?,
            y_range: abs_range(&file, cy)?,
            screen,
            swap_xy: cfg.swap_xy,
            invert_x: cfg.invert_x,
            invert_y: cfg.invert_y,
        };
        info!(
            "touch: {} ({}), x {:?} y {:?}, multitouch={multitouch}",
            path.display(),
            device_name(&file),
            mapping.x_range,
            mapping.y_range
        );
        if let Err(e) = grab(&file) {
            warn!("couldn't grab the touch device, the Kindle UI will see touches too: {e}");
        }
        std::thread::Builder::new().name("input".into()).spawn(move || {
            if let Err(e) = read_loop(file, cx, cy, mapping, on_event) {
                error!("touch input stopped: {e:#}");
            }
        })?;
        Ok(())
    }

    fn read_loop(
        mut file: File,
        cx: u16,
        cy: u16,
        mapping: Mapping,
        on_event: impl Fn(TouchEvent),
    ) -> Result<()> {
        let mut slot = 0;
        let (mut raw_x, mut raw_y) = (0, 0);
        let mut tracking = false; // from ABS_MT_TRACKING_ID on slot 0
        let mut btn = false; // from BTN_TOUCH
        let mut was_down = false;
        let mut down_at = Instant::now();
        let mut buf = [0u8; size_of::<InputEvent>() * 64];
        loop {
            let n = file.read(&mut buf)?;
            for chunk in buf[..n].chunks_exact(size_of::<InputEvent>()) {
                // SAFETY: InputEvent is plain old data; read_unaligned copes with alignment.
                let ev: InputEvent = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
                match (ev.type_, ev.code) {
                    (EV_ABS, ABS_MT_SLOT) => slot = ev.value,
                    (EV_ABS, ABS_MT_TRACKING_ID) if slot == 0 => tracking = ev.value >= 0,
                    (EV_ABS, c) if c == cx && slot == 0 => raw_x = ev.value,
                    (EV_ABS, c) if c == cy && slot == 0 => raw_y = ev.value,
                    (EV_KEY, BTN_TOUCH) => btn = ev.value != 0,
                    (EV_SYN, SYN_REPORT) => {
                        let down = tracking || btn;
                        let (x, y) = mapping.map(raw_x, raw_y);
                        if down && !was_down {
                            down_at = Instant::now();
                            debug!("touch down raw ({raw_x},{raw_y}) -> ({x},{y})");
                            on_event(TouchEvent::Down { x, y });
                        } else if !down && was_down {
                            let held = down_at.elapsed();
                            debug!("touch up ({x},{y}) after {held:?}");
                            on_event(TouchEvent::Up { x, y, held });
                        }
                        was_down = down;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use anyhow::{Result, bail};

    use super::TouchEvent;
    use crate::config::TouchConfig;

    pub fn spawn(_: &TouchConfig, _: (i32, i32), _: impl Fn(TouchEvent) + Send + 'static) -> Result<()> {
        bail!("touch input is only available on Linux")
    }

    pub fn grab_power_buttons() -> Vec<std::fs::File> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> Mapping {
        Mapping {
            x_range: (0, 1000),
            y_range: (0, 2000),
            screen: (101, 201),
            swap_xy: false,
            invert_x: false,
            invert_y: false,
        }
    }

    #[test]
    fn maps_and_clamps() {
        let m = mapping();
        assert_eq!(m.map(500, 1000), (50, 100));
        assert_eq!(m.map(-5, 5000), (0, 200));
    }

    #[test]
    fn swap_then_invert() {
        let m = Mapping { swap_xy: true, invert_x: true, ..mapping() };
        // raw x is 10% across, raw y 25% down: swapped -> (0.25, 0.1), inverted x -> 0.75.
        assert_eq!(m.map(100, 500), (75, 20));
    }
}
