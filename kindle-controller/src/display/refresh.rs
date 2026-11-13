//! Telling the e-ink controller to show what's in the framebuffer.
//!
//! For now this runs the `fbink` CLI (`fbink -s top=..,left=..,width=..,height=.. -W MODE`),
//! which knows the refresh ioctl for every Kindle generation. Spawning it costs a few tens
//! of milliseconds per refresh, which is fine for this UI. The `Refresher` trait lets us
//! swap in a linked FBInk or a direct ioctl later without touching anything else.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use log::{debug, info};

use super::canvas::Rect;
use crate::config::DisplayConfig;

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

pub struct FbinkCli {
    path: PathBuf,
    fast: String,
    partial: String,
    full: String,
}

impl FbinkCli {
    pub fn new(cfg: &DisplayConfig) -> Result<Self> {
        let path = find_fbink(cfg.fbink.as_deref())?;
        info!("using fbink at {}", path.display());
        Ok(Self {
            path,
            fast: cfg.waveform_fast.clone(),
            partial: cfg.waveform_partial.clone(),
            full: cfg.waveform_full.clone(),
        })
    }
}

impl Refresher for FbinkCli {
    fn refresh(&mut self, rect: Rect, waveform: Waveform) -> Result<()> {
        let mode = match waveform {
            Waveform::Fast => &self.fast,
            Waveform::Partial => &self.partial,
            Waveform::Full => &self.full,
        };
        let mut cmd = Command::new(&self.path);
        cmd.arg("-q").arg("-W").arg(mode);
        if waveform == Waveform::Full {
            cmd.arg("-f");
        }
        // The region goes in its own argument straight after -s.
        cmd.arg("-s")
            .arg(format!("top={},left={},width={},height={}", rect.y, rect.x, rect.w, rect.h));
        debug!("refresh {rect:?} {waveform:?} ({mode})");
        let out = cmd
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running {}", self.path.display()))?;
        if !out.status.success() {
            bail!(
                "fbink failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

fn find_fbink(configured: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = configured {
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        bail!("configured fbink {} not found", p.display());
    }
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("fbink"));
    }
    candidates.extend(
        [
            "/mnt/us/libkh/bin/fbink",
            "/mnt/us/usbnet/bin/fbink",
            "/mnt/us/koreader/fbink",
            "/usr/local/bin/fbink",
            "/usr/bin/fbink",
        ]
        .map(PathBuf::from),
    );
    if let Some(p) = candidates.iter().find(|p| p.is_file()) {
        return Ok(p.clone());
    }
    bail!(
        "fbink not found (looked in {}); put it next to the controller binary or set display.fbink",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}
