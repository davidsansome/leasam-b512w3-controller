//! The e-ink display: draw into a `Canvas`, then `present` it. `present` works out which
//! region changed, copies just that into the framebuffer (rotating it to the panel's
//! orientation) and refreshes it with a suitable waveform. Ghosting is tracked per area
//! (see `ghost`), and an area is flashed once it has had enough partial refreshes; the
//! whole screen flashes only when most of it is worn.

pub mod canvas;
pub mod fb;
pub mod font;
pub mod ghost;
pub mod icons;
pub mod refresh;
pub mod rotation;

use std::time::{Duration, Instant};

use anyhow::Result;
use log::{debug, info, warn};

use canvas::{Canvas, Rect};
use fb::Framebuffer;
use ghost::{Ghosting, Plan};
use refresh::{Fbink, Refresher, Waveform};
use rotation::Rotation;

use crate::config::DisplayConfig;

/// How the caller wants this update shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    /// Instant feedback in black and white (e.g. a button press).
    Fast,
    /// A normal content change.
    Normal,
    /// A whole new screen: redraw and flash everything.
    Full,
}

pub struct Display {
    fb: Framebuffer,
    refresher: Box<dyn Refresher>,
    rotation: Rotation,
    /// What's currently on the panel, in logical coordinates.
    front: Canvas,
    /// Where ghosting has built up since each area was last flashed.
    ghosting: Ghosting,
    /// The panel's contents are unknown (at startup, or after a failed refresh).
    needs_full: bool,
    full_refresh_every: u32,
    /// Area last shown with the fast black/white waveform, which needs a proper redraw.
    rough: Option<(Rect, Instant)>,
}

impl Display {
    pub fn open(cfg: &DisplayConfig) -> Result<Self> {
        let fb = Framebuffer::open(&cfg.framebuffer, cfg.invert)?;
        let refresher = Box::new(Fbink::new(cfg)?);
        let rotation = Rotation::new(cfg.rotation, fb.size())?;
        let (w, h) = rotation.logical_size();
        info!("drawing at {w}x{h} (rotated {}°)", cfg.rotation);
        Ok(Self {
            fb,
            refresher,
            rotation,
            front: Canvas::new(w, h),
            ghosting: Ghosting::new(w, h),
            // Unknown contents: the first frame has to be a full one.
            needs_full: true,
            full_refresh_every: cfg.full_refresh_every.max(1),
            rough: None,
        })
    }

    /// Size of the canvas to draw on.
    pub fn size(&self) -> (i32, i32) {
        self.rotation.logical_size()
    }

    /// Size of the physical panel (what touch coordinates are in).
    pub fn physical_size(&self) -> (i32, i32) {
        self.fb.size()
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// Whether a fast refresh's rough area has been waiting at least `after` for a redraw.
    pub fn needs_cleanup(&self, after: Duration) -> bool {
        self.rough.is_some_and(|(_, at)| at.elapsed() >= after)
    }

    /// How long until the rough area needs its redraw, if there is one.
    pub fn cleanup_in(&self, after: Duration) -> Option<Duration> {
        self.rough.map(|(_, at)| after.saturating_sub(at.elapsed()))
    }

    pub fn present(&mut self, back: &Canvas, update: Update) -> Result<()> {
        if update == Update::Full || self.needs_full {
            return self.flash_all(back);
        }
        let diff = self.front.diff_bounds(back);
        let dirty = match (diff, update, self.rough) {
            (Some(d), Update::Fast, _) => {
                let area = self.rough.map_or(d, |(r, _)| r.union(&d));
                self.rough = Some((area, Instant::now()));
                d
            }
            // A normal update also redraws whatever the fast waveform left rough.
            (d, Update::Normal, Some((r, _))) => {
                self.rough = None;
                d.map_or(r, |d| d.union(&r))
            }
            (Some(d), _, _) => d,
            (None, _, _) => return Ok(()),
        };
        let phys = self.rotation.rect_to_physical(dirty);
        if update == Update::Fast {
            // The fast waveform can only show black or white, so put exactly that in the
            // framebuffer. Then the grays really change when the normal update redraws the
            // area, and its partial-mode refresh, which skips unchanged pixels, shows them.
            self.front.copy_from_bw(back, dirty);
            self.fb.write_rect(&self.front, phys, self.rotation);
        } else {
            self.fb.write_rect(back, phys, self.rotation);
            self.front.copy_from(back, dirty);
        }
        let plan = self.ghosting.plan(dirty, update == Update::Normal, self.full_refresh_every);
        debug!("{update:?} update of {dirty:?}: {plan:?}");
        let (flashes, partial) = match plan {
            Plan::Full => return self.flash_all(back),
            Plan::Partial { flashes, partial } => (flashes, partial),
        };
        let waveform = if update == Update::Fast { Waveform::Fast } else { Waveform::Partial };
        let refreshes = flashes.into_iter().map(|r| (r, Waveform::Full)).chain(partial.map(|r| (r, waveform)));
        for (r, waveform) in refreshes {
            if let Err(e) = self.refresher.refresh(self.rotation.rect_to_physical(r), waveform) {
                // Make sure the next update repaints everything.
                self.needs_full = true;
                warn!("refresh failed: {e:#}");
                return Err(e);
            }
        }
        Ok(())
    }

    /// Shows all of `back` with a flash, which also clears all ghosting.
    fn flash_all(&mut self, back: &Canvas) -> Result<()> {
        let all = back.bounds();
        let phys = self.rotation.rect_to_physical(all);
        self.fb.write_rect(back, phys, self.rotation);
        self.front.copy_from(back, all);
        self.ghosting.clear_all();
        self.rough = None;
        self.needs_full = false;
        let result = self.refresher.refresh(phys, Waveform::Full);
        if result.is_err() {
            self.needs_full = true;
        }
        result
    }
}
