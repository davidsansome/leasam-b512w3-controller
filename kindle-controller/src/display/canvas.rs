//! 8-bit grayscale drawing surface. Everything is drawn here first, then the changed
//! region is copied to the framebuffer (see `Display::present`).

pub const BLACK: u8 = 0x00;
pub const WHITE: u8 = 0xFF;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0), (b - y).max(0))
    }
}

#[derive(Clone)]
pub struct Canvas {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height, pixels: vec![WHITE; (width * height) as usize] }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn row(&self, y: i32) -> &[u8] {
        let start = (y * self.width) as usize;
        &self.pixels[start..start + self.width as usize]
    }

    pub fn row_mut(&mut self, y: i32) -> &mut [u8] {
        let start = (y * self.width) as usize;
        &mut self.pixels[start..start + self.width as usize]
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // used by the framebuffer
    /// The raw 8-bit pixels, row by row.
    #[cfg(test)]
    pub fn raw(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixel(&self, x: i32, y: i32) -> u8 {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn fill(&mut self, color: u8) {
        self.pixels.fill(color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: u8) {
        let r = rect.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            self.row_mut(y)[r.x as usize..r.right() as usize].fill(color);
        }
    }

    /// Mix `color` into the pixel with the given coverage (0 = untouched, 255 = solid).
    pub fn blend(&mut self, x: i32, y: i32, color: u8, coverage: u8) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || coverage == 0 {
            return;
        }
        let p = &mut self.pixels[(y * self.width + x) as usize];
        let a = u32::from(coverage);
        *p = ((u32::from(*p) * (255 - a) + u32::from(color) * a + 127) / 255) as u8;
    }

    /// Fills an anti-aliased vector path.
    pub fn fill_path(&mut self, path: &tiny_skia::Path, color: u8) {
        let b = path.bounds();
        let x0 = (b.left().floor() as i32 - 1).max(0);
        let y0 = (b.top().floor() as i32 - 1).max(0);
        let x1 = (b.right().ceil() as i32 + 1).min(self.width);
        let y1 = (b.bottom().ceil() as i32 + 1).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let Some(mut mask) = tiny_skia::Mask::new(w, h) else { return };
        mask.fill_path(
            path,
            tiny_skia::FillRule::Winding,
            true,
            tiny_skia::Transform::from_translate(-x0 as f32, -y0 as f32),
        );
        for (i, &cov) in mask.data().iter().enumerate() {
            if cov != 0 {
                let (mx, my) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                self.blend(x0 + mx, y0 + my, color, cov);
            }
        }
    }

    /// Strokes a path with round caps and joins.
    pub fn stroke_path(&mut self, path: &tiny_skia::Path, width: f32, color: u8) {
        let stroke = tiny_skia::Stroke {
            width,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        };
        if let Some(outline) = path.stroke(&stroke, 1.0) {
            self.fill_path(&outline, color);
        }
    }

    pub fn fill_round_rect(&mut self, r: Rect, radius: f32, color: u8) {
        if let Some(p) = round_rect_path(r.x as f32, r.y as f32, r.w as f32, r.h as f32, radius) {
            self.fill_path(&p, color);
        }
    }

    /// Border of `width` drawn just inside `r`.
    pub fn stroke_round_rect(&mut self, r: Rect, radius: f32, width: f32, color: u8) {
        let h = width / 2.0;
        let path = round_rect_path(
            r.x as f32 + h,
            r.y as f32 + h,
            r.w as f32 - width,
            r.h as f32 - width,
            (radius - h).max(0.0),
        );
        if let Some(p) = path {
            let stroke = tiny_skia::Stroke { width, ..Default::default() };
            if let Some(outline) = p.stroke(&stroke, 1.0) {
                self.fill_path(&outline, color);
            }
        }
    }

    /// Anti-aliased filled circle.
    pub fn fill_disc(&mut self, cx: f32, cy: f32, r: f32, color: u8) {
        if let Some(p) = tiny_skia::PathBuilder::from_circle(cx, cy, r) {
            self.fill_path(&p, color);
        }
    }

    /// Anti-aliased circle outline, `width` thick, drawn just inside radius `r`.
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, width: f32, color: u8) {
        if let Some(p) = tiny_skia::PathBuilder::from_circle(cx, cy, r - width / 2.0) {
            let stroke = tiny_skia::Stroke { width, ..Default::default() };
            if let Some(outline) = p.stroke(&stroke, 1.0) {
                self.fill_path(&outline, color);
            }
        }
    }

    pub fn copy_from(&mut self, other: &Canvas, rect: Rect) {
        let r = rect.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            let (a, b) = (r.x as usize, r.right() as usize);
            self.row_mut(y)[a..b].copy_from_slice(&other.row(y)[a..b]);
        }
    }

    /// Like `copy_from`, but with the pixels that change rounded to black or white: what
    /// the fast (black/white) waveform shows. Pixels that don't change are left as they
    /// are, so the refresh doesn't touch them, even if they're grey.
    pub fn copy_from_bw(&mut self, other: &Canvas, rect: Rect) {
        let r = rect.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            let (a, b) = (r.x as usize, r.right() as usize);
            for (d, s) in self.row_mut(y)[a..b].iter_mut().zip(&other.row(y)[a..b]) {
                if d != s {
                    *d = if *s >= 0x80 { WHITE } else { BLACK };
                }
            }
        }
    }

    /// Bounding box of the pixels that differ between two same-sized canvases.
    pub fn diff_bounds(&self, other: &Canvas) -> Option<Rect> {
        let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for y in 0..self.height {
            let (a, b) = (self.row(y), other.row(y));
            if a == b {
                continue;
            }
            let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap() as i32;
            let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap() as i32;
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        (y0 <= y1).then(|| Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }
}

/// A rounded rectangle as a path (corners approximated with cubics).
pub fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.552_284_8;
    let (x1, y1) = (x + w, y + h);
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - r + k, y, x1, y + r - k, x1, y + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + r - k, y1, x, y1 - r + k, x, y1 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bw_copy_rounds_changing_grays_inside_the_rect_only() {
        let mut src = Canvas::new(5, 1);
        for (x, v) in [0x20u8, 0x90, 0x7F, 0x55, 0xC0].into_iter().enumerate() {
            src.row_mut(0)[x] = v;
        }
        let mut dst = Canvas::new(5, 1);
        dst.fill(0x55);
        dst.copy_from_bw(&src, Rect::new(0, 0, 4, 1));
        // The unchanged grey (x = 3) and the pixel outside the rect (x = 4) are left alone.
        assert_eq!(dst.row(0), &[BLACK, WHITE, BLACK, 0x55, 0x55]);
    }

    #[test]
    fn vector_shapes_draw_inside_their_bounds() {
        let mut c = Canvas::new(100, 100);
        c.fill_round_rect(Rect::new(10, 10, 40, 30), 8.0, BLACK);
        assert_eq!(c.pixel(30, 25), BLACK);
        assert_eq!(c.pixel(10, 10), WHITE, "corner is rounded off");
        assert_eq!(c.pixel(5, 5), WHITE);
        let mut d = Canvas::new(100, 100);
        d.stroke_circle(50.0, 50.0, 20.0, 3.0, BLACK);
        assert_eq!(d.pixel(50, 50), WHITE);
        assert_eq!(d.pixel(50, 31), BLACK);
    }

    #[test]
    fn diff_bounds_finds_changed_region() {
        let a = Canvas::new(100, 50);
        let mut b = a.clone();
        assert_eq!(a.diff_bounds(&b), None);
        b.fill_rect(Rect::new(10, 5, 20, 3), BLACK);
        b.blend(60, 40, BLACK, 128);
        assert_eq!(a.diff_bounds(&b), Some(Rect::new(10, 5, 51, 36)));
    }

    #[test]
    fn fill_rect_clips() {
        let mut c = Canvas::new(10, 10);
        c.fill_rect(Rect::new(-5, 8, 100, 100), BLACK);
        assert_eq!(c.row(9), &[BLACK; 10]);
        assert_eq!(c.row(7), &[WHITE; 10]);
    }
}
