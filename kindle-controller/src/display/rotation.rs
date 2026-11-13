//! Mapping between the logical canvas we draw on and the physical panel.
//!
//! The PW4's framebuffer is portrait (1072x1448). With a 90° or 270° rotation we draw on a
//! landscape canvas (1448x1072) and rotate as we copy into the framebuffer. Touches arrive
//! in physical coordinates and are mapped back with `to_logical`.

use anyhow::{Result, bail};

use super::canvas::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rotation {
    /// Clockwise rotation of the content on the panel, in degrees.
    degrees: u32,
    /// Physical (framebuffer) size.
    phys: (i32, i32),
}

impl Rotation {
    pub fn new(degrees: u32, phys: (i32, i32)) -> Result<Self> {
        if ![0, 90, 180, 270].contains(&degrees) {
            bail!("display.rotation must be 0, 90, 180 or 270, not {degrees}");
        }
        Ok(Self { degrees, phys })
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // used by the framebuffer
    pub fn is_identity(&self) -> bool {
        self.degrees == 0
    }

    pub fn logical_size(&self) -> (i32, i32) {
        match self.degrees {
            90 | 270 => (self.phys.1, self.phys.0),
            _ => self.phys,
        }
    }

    pub fn to_physical(self, x: i32, y: i32) -> (i32, i32) {
        let (lw, lh) = self.logical_size();
        match self.degrees {
            90 => (lh - 1 - y, x),
            180 => (lw - 1 - x, lh - 1 - y),
            270 => (y, lw - 1 - x),
            _ => (x, y),
        }
    }

    pub fn to_logical(self, px: i32, py: i32) -> (i32, i32) {
        let (lw, lh) = self.logical_size();
        match self.degrees {
            90 => (py, lh - 1 - px),
            180 => (lw - 1 - px, lh - 1 - py),
            270 => (lw - 1 - py, px),
            _ => (px, py),
        }
    }

    pub fn rect_to_physical(&self, r: Rect) -> Rect {
        let (ax, ay) = self.to_physical(r.x, r.y);
        let (bx, by) = self.to_physical(r.right() - 1, r.bottom() - 1);
        Rect::new(ax.min(bx), ay.min(by), (ax - bx).abs() + 1, (ay - by).abs() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_corners() {
        for deg in [0, 90, 180, 270] {
            let r = Rotation::new(deg, (1072, 1448)).unwrap();
            let (lw, lh) = r.logical_size();
            for (x, y) in [(0, 0), (lw - 1, 0), (0, lh - 1), (17, 300)] {
                let (px, py) = r.to_physical(x, y);
                assert!((0..1072).contains(&px) && (0..1448).contains(&py), "{deg}: ({x},{y})");
                assert_eq!(r.to_logical(px, py), (x, y), "{deg}");
            }
        }
        let r = Rotation::new(90, (1072, 1448)).unwrap();
        assert_eq!(r.logical_size(), (1448, 1072));
        // Rotated clockwise: logical top-left ends up physical top-right.
        assert_eq!(r.to_physical(0, 0), (1071, 0));
        assert!(Rotation::new(45, (1, 1)).is_err());
    }

    #[test]
    fn rects_map_to_physical() {
        let r = Rotation::new(90, (1072, 1448)).unwrap();
        assert_eq!(r.rect_to_physical(Rect::new(0, 0, 1448, 1072)), Rect::new(0, 0, 1072, 1448));
        assert_eq!(r.rect_to_physical(Rect::new(10, 20, 100, 50)), Rect::new(1072 - 70, 10, 50, 100));
    }
}
