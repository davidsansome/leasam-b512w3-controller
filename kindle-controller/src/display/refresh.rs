//! Telling the e-ink controller to show what's in the framebuffer.
//!
//! This uses libfbink, built and linked in by `build.rs` (see `csrc/fbink_shim.c`), which
//! knows the refresh ioctl for every Kindle generation. A refresh is a function call, the
//! same as `fbink -s top=..,left=..,width=..,height=.. -W MODE [-f]` but without starting a
//! process each time. Builds for other targets (host tests) have no FBInk, and
//! `Fbink::new` fails there.

use anyhow::Result;

use super::canvas::Rect;

/// What kind of update this is. Mapped to a waveform name by config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    /// Black/white only, fast (DU): touch feedback.
    Fast,
    /// Grayscale, non-flashing (GL16): normal content updates.
    Partial,
    /// Grayscale with a black flash (GC16): clears ghosting.
    Full,
}

pub trait Refresher {
    fn refresh(&mut self, rect: Rect, waveform: Waveform) -> Result<()>;
}

pub use imp::Fbink;

#[cfg(fbink)]
mod imp {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::io;

    use anyhow::{Context, Result, anyhow, bail};
    use log::{debug, info};

    use super::{Rect, Refresher, Waveform};
    use crate::config::DisplayConfig;

    unsafe extern "C" {
        fn aircon_fbink_waveform(name: *const c_char) -> c_int;
        fn aircon_fbink_version() -> *const c_char;
        fn aircon_fbink_open() -> c_int;
        fn aircon_fbink_refresh(fd: c_int, top: u32, left: u32, width: u32, height: u32, mode: c_int, flashing: bool) -> c_int;
        fn aircon_fbink_close(fd: c_int);
    }

    pub struct Fbink {
        fd: c_int,
        fast: c_int,
        partial: c_int,
        full: c_int,
    }

    impl Fbink {
        pub fn new(cfg: &DisplayConfig) -> Result<Self> {
            let fast = waveform(&cfg.waveform_fast).context("display.waveform_fast")?;
            let partial = waveform(&cfg.waveform_partial).context("display.waveform_partial")?;
            let full = waveform(&cfg.waveform_full).context("display.waveform_full")?;
            // SAFETY: returns a pointer to a static string.
            let version = unsafe { CStr::from_ptr(aircon_fbink_version()) };
            // SAFETY: no arguments; returns an fd or -errno.
            let fd = unsafe { aircon_fbink_open() };
            if fd < 0 {
                return Err(io::Error::from_raw_os_error(-fd)).context("initialising FBInk");
            }
            info!("using FBInk {}", version.to_string_lossy());
            Ok(Self { fd, fast, partial, full })
        }
    }

    fn waveform(name: &str) -> Result<c_int> {
        let c = CString::new(name).map_err(|_| anyhow!("bad waveform name {name:?}"))?;
        // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
        match unsafe { aircon_fbink_waveform(c.as_ptr()) } {
            -1 => bail!("unknown waveform {name:?}"),
            mode => Ok(mode),
        }
    }

    impl Refresher for Fbink {
        fn refresh(&mut self, rect: Rect, waveform: Waveform) -> Result<()> {
            let mode = match waveform {
                Waveform::Fast => self.fast,
                Waveform::Partial => self.partial,
                Waveform::Full => self.full,
            };
            debug!("refresh {rect:?} {waveform:?} (mode {mode})");
            let dim = |v: i32| u32::try_from(v).map_err(|_| anyhow!("bad refresh region {rect:?}"));
            let (top, left, width, height) = (dim(rect.y)?, dim(rect.x)?, dim(rect.w)?, dim(rect.h)?);
            // SAFETY: plain values; `fd` came from aircon_fbink_open and is still open.
            let rv = unsafe { aircon_fbink_refresh(self.fd, top, left, width, height, mode, waveform == Waveform::Full) };
            if rv < 0 {
                return Err(io::Error::from_raw_os_error(-rv)).with_context(|| format!("refreshing {rect:?}"));
            }
            Ok(())
        }
    }

    impl Drop for Fbink {
        fn drop(&mut self) {
            // SAFETY: `fd` came from aircon_fbink_open and isn't used again.
            unsafe { aircon_fbink_close(self.fd) };
        }
    }
}

#[cfg(not(fbink))]
mod imp {
    use anyhow::{Result, bail};

    use super::{Rect, Refresher, Waveform};
    use crate::config::DisplayConfig;

    pub struct Fbink;

    impl Fbink {
        pub fn new(_cfg: &DisplayConfig) -> Result<Self> {
            bail!("this build has no FBInk (it's only linked into Kindle builds)")
        }
    }

    impl Refresher for Fbink {
        fn refresh(&mut self, _rect: Rect, _waveform: Waveform) -> Result<()> {
            bail!("this build has no FBInk")
        }
    }
}
