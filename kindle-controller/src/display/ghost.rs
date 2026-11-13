//! Tracks where on the screen ghosting is building up, so we can clean just those areas.
//!
//! Each non-flashing refresh leaves a little ghosting behind, but only where it refreshed.
//! So the screen is split into tiles, and each tile counts the partial refreshes that have
//! touched it since it was last flashed. A countdown chip that changes every few seconds
//! then only ever wears out its own few tiles, and gets a small local flash now and then,
//! instead of making the whole screen flash.

use super::canvas::Rect;

/// Tile size in logical pixels.
pub const TILE: i32 = 32;

pub struct Ghosting {
    width: i32,
    height: i32,
    cols: i32,
    rows: i32,
    counts: Vec<u32>,
}

impl Ghosting {
    pub fn new(width: i32, height: i32) -> Self {
        let cols = (width + TILE - 1) / TILE;
        let rows = (height + TILE - 1) / TILE;
        Self { width, height, cols, rows, counts: vec![0; (cols * rows) as usize] }
    }

    /// The tiles `r` touches, as (first col, first row, end col, end row), end exclusive.
    fn tiles(&self, r: Rect) -> Option<(i32, i32, i32, i32)> {
        let r = r.intersect(&Rect::new(0, 0, self.width, self.height));
        if r.w <= 0 || r.h <= 0 {
            return None;
        }
        Some((r.x / TILE, r.y / TILE, (r.right() + TILE - 1) / TILE, (r.bottom() + TILE - 1) / TILE))
    }

    /// `r` expanded to whole tiles (clipped to the screen).
    pub fn align(&self, r: Rect) -> Rect {
        match self.tiles(r) {
            Some((c0, r0, c1, r1)) => self.tile_rect(c0, r0, c1, r1),
            None => Rect::new(0, 0, 0, 0),
        }
    }

    fn tile_rect(&self, c0: i32, r0: i32, c1: i32, r1: i32) -> Rect {
        let (x, y) = (c0 * TILE, r0 * TILE);
        Rect::new(x, y, (c1 * TILE).min(self.width) - x, (r1 * TILE).min(self.height) - y)
    }

    /// A partial refresh of `r`.
    pub fn add(&mut self, r: Rect) {
        let Some((c0, r0, c1, r1)) = self.tiles(r) else { return };
        for row in r0..r1 {
            for col in c0..c1 {
                let i = (row * self.cols + col) as usize;
                self.counts[i] = self.counts[i].saturating_add(1);
            }
        }
    }

    /// A flashing refresh of `r`: the tiles wholly inside it are clean again.
    pub fn clear(&mut self, r: Rect) {
        let Some((c0, r0, c1, r1)) = self.tiles(r) else { return };
        for row in r0..r1 {
            for col in c0..c1 {
                let tile = self.tile_rect(col, row, col + 1, row + 1);
                if tile.intersect(&r) == tile {
                    self.counts[(row * self.cols + col) as usize] = 0;
                }
            }
        }
    }

    pub fn clear_all(&mut self) {
        self.counts.fill(0);
    }

    /// Records a refresh of `dirty` and decides how to show it. Worn areas are only
    /// flashed when `may_flash` (normal updates: a flash would spoil the instant feedback
    /// of a fast one), and if they add up to over half the screen, the whole screen is
    /// flashed instead.
    pub fn plan(&mut self, dirty: Rect, may_flash: bool, threshold: u32) -> Plan {
        self.add(dirty);
        let hot = if may_flash { self.hot(threshold) } else { Vec::new() };
        let area: i64 = hot.iter().map(|r| i64::from(r.w) * i64::from(r.h)).sum();
        if area * 2 > i64::from(self.width) * i64::from(self.height) {
            self.clear_all();
            return Plan::Full;
        }
        let mut flashes = Vec::new();
        let mut partial = Some(dirty);
        for h in hot {
            // Flash what's changing too, if it's in or next to a worn area, rather than
            // refreshing it twice.
            let touching = h.intersect(&Rect::new(dirty.x - 1, dirty.y - 1, dirty.w + 2, dirty.h + 2));
            let h = if touching.w > 0 && touching.h > 0 {
                partial = None;
                self.align(h.union(&dirty))
            } else {
                h
            };
            self.clear(h);
            flashes.push(h);
        }
        Plan::Partial { flashes, partial }
    }
}

/// How to show an update.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Flash the whole screen.
    Full,
    /// Flash these areas, then refresh `partial` (if any) without flashing.
    Partial { flashes: Vec<Rect>, partial: Option<Rect> },
}

impl Ghosting {
    /// Areas with at least `threshold` partial refreshes since their last flash: the
    /// bounding boxes of connected groups of worn tiles.
    pub fn hot(&self, threshold: u32) -> Vec<Rect> {
        let worn = |c: i32, r: i32| self.counts[(r * self.cols + c) as usize] >= threshold;
        let mut seen = vec![false; self.counts.len()];
        let mut out = Vec::new();
        for start in 0..self.counts.len() as i32 {
            let (c, r) = (start % self.cols, start / self.cols);
            if seen[start as usize] || !worn(c, r) {
                continue;
            }
            let (mut c0, mut r0, mut c1, mut r1) = (c, r, c + 1, r + 1);
            let mut stack = vec![(c, r)];
            seen[start as usize] = true;
            while let Some((c, r)) = stack.pop() {
                (c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c + 1), r1.max(r + 1));
                for (nc, nr) in [(c - 1, r), (c + 1, r), (c, r - 1), (c, r + 1)] {
                    if nc < 0 || nr < 0 || nc >= self.cols || nr >= self.rows {
                        continue;
                    }
                    let i = (nr * self.cols + nc) as usize;
                    if !seen[i] && worn(nc, nr) {
                        seen[i] = true;
                        stack.push((nc, nr));
                    }
                }
            }
            out.push(self.tile_rect(c0, r0, c1, r1));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_the_tiles_a_refresh_touches() {
        let mut g = Ghosting::new(1448, 1072);
        for _ in 0..3 {
            g.add(Rect::new(210, 540, 300, 48)); // a chip
        }
        g.add(Rect::new(0, 0, 1448, 1072)); // one whole-screen partial
        assert_eq!(g.hot(4), vec![Rect::new(192, 512, 320, 96)]);
        assert!(g.hot(5).is_empty());
    }

    #[test]
    fn separate_areas_stay_separate_and_clear_independently() {
        let mut g = Ghosting::new(1448, 1072);
        for _ in 0..2 {
            g.add(Rect::new(40, 40, 50, 50));
            g.add(Rect::new(1000, 900, 60, 20));
        }
        let hot = g.hot(2);
        assert_eq!(hot, vec![Rect::new(32, 32, 64, 64), Rect::new(992, 896, 96, 32)]);
        g.clear(hot[0]);
        assert_eq!(g.hot(2), vec![hot[1]]);
        g.clear_all();
        assert!(g.hot(1).is_empty());
    }

    #[test]
    fn clearing_spares_tiles_only_partly_flashed() {
        let mut g = Ghosting::new(100, 100);
        g.add(Rect::new(0, 0, 100, 100));
        g.clear(Rect::new(0, 0, 40, 40)); // covers tile (0,0) only
        assert_eq!(g.hot(1).len(), 1);
        assert_eq!(g.hot(1)[0], Rect::new(0, 0, 100, 100));
        g.clear(g.align(Rect::new(5, 5, 95, 95)));
        assert!(g.hot(1).is_empty());
    }

    #[test]
    fn a_ticking_chip_gets_local_flashes_only() {
        let mut g = Ghosting::new(1448, 1072);
        let chip = Rect::new(210, 540, 300, 48);
        let mut flashes = 0;
        for i in 1..=60 {
            match g.plan(chip, true, 20) {
                Plan::Full => panic!("whole screen flashed at update {i}"),
                Plan::Partial { flashes: f, partial } if !f.is_empty() => {
                    assert_eq!(i % 20, 0);
                    assert_eq!(f, vec![Rect::new(192, 512, 320, 96)]);
                    assert_eq!(partial, None, "the chip is shown by the flash itself");
                    flashes += 1;
                }
                Plan::Partial { partial, .. } => assert_eq!(partial, Some(chip)),
            }
        }
        assert_eq!(flashes, 3);
    }

    #[test]
    fn worn_areas_wait_for_a_normal_update() {
        let mut g = Ghosting::new(1448, 1072);
        let r = Rect::new(0, 0, 100, 100);
        for _ in 0..5 {
            assert_eq!(g.plan(r, false, 2), Plan::Partial { flashes: vec![], partial: Some(r) });
        }
        // A normal update elsewhere also flashes the worn area, and refreshes its own.
        let other = Rect::new(800, 800, 50, 50);
        assert_eq!(
            g.plan(other, true, 2),
            Plan::Partial { flashes: vec![Rect::new(0, 0, 128, 128)], partial: Some(other) }
        );
    }

    #[test]
    fn mostly_worn_screen_flashes_everything() {
        let mut g = Ghosting::new(1448, 1072);
        let big = Rect::new(0, 0, 1448, 700);
        assert!(matches!(g.plan(big, true, 2), Plan::Partial { .. }));
        assert_eq!(g.plan(big, true, 2), Plan::Full);
        assert!(g.hot(1).is_empty());
    }

    #[test]
    fn edge_tiles_are_clipped_to_the_screen() {
        let mut g = Ghosting::new(1448, 1072); // 1072 isn't a multiple of 32
        g.add(Rect::new(1440, 1060, 8, 12));
        assert_eq!(g.hot(1), vec![Rect::new(1440, 1056, 8, 16)]);
        assert_eq!(g.align(Rect::new(-10, -10, 20, 20)), Rect::new(0, 0, 32, 32));
    }
}
