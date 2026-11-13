//! Text rendering. Uses TTF/OTF fonts from the Kindle's own font directories (so we don't
//! have to ship any): a regular face and, when one sits next to it, a bold face. Falls back
//! to a scaled-up 8x8 bitmap font so error screens always work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use font8x8::UnicodeFonts;
use log::{info, warn};

use super::canvas::Canvas;

/// Searched in order when no font is configured. `/mnt/us/fonts` is where Kindle users
/// put their own fonts.
const FONT_DIRS: &[&str] = &["/mnt/us/fonts", "/usr/java/lib/fonts", "/usr/share/fonts"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Bold,
}

struct Glyph {
    xmin: i32,
    ymin: i32,
    width: i32,
    height: i32,
    advance: f32,
    coverage: Vec<u8>,
}

enum Face {
    Ttf(fontdue::Font),
    Bitmap,
}

pub struct Fonts {
    regular: Face,
    bold: Option<Face>,
    cache: HashMap<(Weight, char, u32), Glyph>,
}

fn load_face(path: &Path) -> Option<Face> {
    match std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|data| fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).map_err(str::to_string))
    {
        Ok(font) => Some(Face::Ttf(font)),
        Err(e) => {
            warn!("can't load font {}: {e}", path.display());
            None
        }
    }
}

/// The bold sibling of a regular font file, e.g. `X-Regular.ttf` → `X-Bold.ttf`.
fn bold_sibling(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let bold = if name.contains("Regular") { name.replacen("Regular", "Bold", 1) } else { return None };
    let p = path.with_file_name(bold);
    p.is_file().then_some(p)
}

impl Fonts {
    pub fn load(configured: Option<&Path>) -> Self {
        let candidates: Vec<PathBuf> = match configured {
            Some(p) => vec![p.to_path_buf()],
            None => find_system_fonts(),
        };
        for path in candidates {
            if let Some(regular) = load_face(&path) {
                info!("using font {}", path.display());
                let bold = bold_sibling(&path).and_then(|b| {
                    info!("using bold font {}", b.display());
                    load_face(&b)
                });
                return Self { regular, bold, cache: HashMap::new() };
            }
        }
        warn!("no usable TTF/OTF font found; using the built-in bitmap font");
        Self::bitmap()
    }

    /// A specific regular and bold font, for previews in tests.
    #[cfg(test)]
    pub fn from_files(regular: &Path, bold: &Path) -> Self {
        Self { regular: load_face(regular).expect("regular font"), bold: load_face(bold), cache: HashMap::new() }
    }

    pub fn bitmap() -> Self {
        Self { regular: Face::Bitmap, bold: None, cache: HashMap::new() }
    }

    fn face(&self, weight: Weight) -> &Face {
        match weight {
            Weight::Bold => self.bold.as_ref().unwrap_or(&self.regular),
            Weight::Regular => &self.regular,
        }
    }

    /// (ascent, descent) in pixels, both positive.
    pub fn line_metrics(&self, px: u32, weight: Weight) -> (i32, i32) {
        match self.face(weight) {
            Face::Ttf(font) => match font.horizontal_line_metrics(px as f32) {
                Some(m) => (m.ascent.round() as i32, (-m.descent).round() as i32),
                None => ((px as f32 * 0.8) as i32, (px as f32 * 0.2) as i32),
            },
            Face::Bitmap => {
                let s = bitmap_scale(px);
                (7 * s, s)
            }
        }
    }

    /// Height of a capital letter, for centring text visually.
    pub fn cap_height(&self, px: u32, weight: Weight) -> i32 {
        match self.face(weight) {
            Face::Ttf(font) => {
                let (m, _) = font.rasterize('H', px as f32);
                m.height as i32
            }
            Face::Bitmap => 7 * bitmap_scale(px),
        }
    }

    pub fn text_width(&mut self, text: &str, px: u32, weight: Weight) -> i32 {
        let mut pen = 0.0f32;
        let mut prev = None;
        for c in text.chars() {
            pen += self.kern(prev, c, px, weight) + self.glyph(c, px, weight).advance;
            prev = Some(c);
        }
        pen.ceil() as i32
    }

    /// Draws `text` with its baseline at `baseline`, starting at `x`.
    pub fn draw(&mut self, canvas: &mut Canvas, text: &str, px: u32, weight: Weight, x: i32, baseline: i32, color: u8) {
        let mut pen = x as f32;
        let mut prev = None;
        for c in text.chars() {
            pen += self.kern(prev, c, px, weight);
            let g = self.glyph(c, px, weight);
            let gx = pen.round() as i32 + g.xmin;
            let gy = baseline - (g.height + g.ymin);
            for row in 0..g.height {
                for col in 0..g.width {
                    let cov = g.coverage[(row * g.width + col) as usize];
                    canvas.blend(gx + col, gy + row, color, cov);
                }
            }
            pen += g.advance;
            prev = Some(c);
        }
    }

    /// Draws `text` horizontally centred on `cx`.
    pub fn draw_centered(&mut self, canvas: &mut Canvas, text: &str, px: u32, weight: Weight, cx: i32, baseline: i32, color: u8) {
        let w = self.text_width(text, px, weight);
        self.draw(canvas, text, px, weight, cx - w / 2, baseline, color);
    }

    /// Draws `text` ending at `right`.
    pub fn draw_right(&mut self, canvas: &mut Canvas, text: &str, px: u32, weight: Weight, right: i32, baseline: i32, color: u8) {
        let w = self.text_width(text, px, weight);
        self.draw(canvas, text, px, weight, right - w, baseline, color);
    }

    /// Shortens `text` with "..." so it fits in `max_w`.
    pub fn ellipsize(&mut self, text: &str, px: u32, weight: Weight, max_w: i32) -> String {
        if self.text_width(text, px, weight) <= max_w {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        for n in (0..chars.len()).rev() {
            let s: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "...";
            if self.text_width(&s, px, weight) <= max_w {
                return s;
            }
        }
        String::new()
    }

    /// Greedy word wrap. Words longer than a line are ellipsized.
    pub fn wrap(&mut self, text: &str, px: u32, weight: Weight, max_w: i32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.text_width(&candidate, px, weight) <= max_w {
                line = candidate;
            } else {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                line = self.ellipsize(word, px, weight, max_w);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn kern(&self, prev: Option<char>, c: char, px: u32, weight: Weight) -> f32 {
        match (self.face(weight), prev) {
            (Face::Ttf(font), Some(p)) => font.horizontal_kern(p, c, px as f32).unwrap_or(0.0),
            _ => 0.0,
        }
    }

    fn glyph(&mut self, c: char, px: u32, weight: Weight) -> &Glyph {
        let face = match weight {
            Weight::Bold => self.bold.as_ref().unwrap_or(&self.regular),
            Weight::Regular => &self.regular,
        };
        self.cache.entry((weight, c, px)).or_insert_with(|| match face {
            Face::Ttf(font) => {
                let c = if font.lookup_glyph_index(c) == 0 && !c.is_whitespace() { '?' } else { c };
                let (m, coverage) = font.rasterize(c, px as f32);
                Glyph {
                    xmin: m.xmin,
                    ymin: m.ymin,
                    width: m.width as i32,
                    height: m.height as i32,
                    advance: m.advance_width,
                    coverage,
                }
            }
            Face::Bitmap => bitmap_glyph(c, bitmap_scale(px)),
        })
    }
}

fn bitmap_scale(px: u32) -> i32 {
    ((px as f32 / 8.0).round() as i32).max(1)
}

fn bitmap_glyph(c: char, s: i32) -> Glyph {
    // The bitmap font has no degree sign; a small 'o' reads fine.
    let c = if c == '°' { 'o' } else { c };
    let rows = font8x8::BASIC_FONTS
        .get(c)
        .or_else(|| font8x8::BASIC_FONTS.get('?'))
        .unwrap_or([0; 8]);
    let size = 8 * s;
    let mut coverage = vec![0u8; (size * size) as usize];
    for (ry, bits) in rows.iter().enumerate() {
        for rx in 0..8 {
            if bits & (1 << rx) != 0 {
                for dy in 0..s {
                    let y = ry as i32 * s + dy;
                    let x0 = (y * size + rx * s) as usize;
                    coverage[x0..x0 + s as usize].fill(255);
                }
            }
        }
    }
    Glyph { xmin: 0, ymin: -s, width: size, height: size, advance: size as f32, coverage }
}

fn find_system_fonts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in FONT_DIRS {
        collect_fonts(Path::new(dir), 3, &mut found);
    }
    found.sort_by_key(|p| std::cmp::Reverse(font_score(p)));
    found
}

fn collect_fonts(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect_fonts(&path, depth - 1, out);
            }
        } else if matches!(
            path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
            Some("ttf" | "otf")
        ) {
            out.push(path);
        }
    }
}

/// Prefer a plain, regular-weight sans for a UI.
fn font_score(path: &Path) -> i32 {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut score = 0;
    for (needle, points) in [
        ("ember", 20), // Amazon Ember, the Kindle UI font
        ("sans", 10),
        ("roboto", 8),
        ("noto", 5),
        ("regular", 5),
        ("italic", -30),
        ("oblique", -30),
        ("bold", -15),
        ("light", -10),
        ("thin", -10),
        ("condensed", -10),
        ("mono", -10),
        ("cjk", -20),
    ] {
        if name.contains(needle) {
            score += points;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_font_measures_and_wraps() {
        let mut f = Fonts::bitmap();
        assert_eq!(f.text_width("abc", 16, Weight::Regular), 48);
        assert_eq!(f.text_width("abc", 16, Weight::Bold), 48);
        assert_eq!(f.wrap("one two three", 8, Weight::Regular, 64), vec!["one two", "three"]);
        assert_eq!(f.ellipsize("abcdefghij", 8, Weight::Regular, 48), "abc...");
    }

    #[test]
    fn font_score_prefers_regular_sans() {
        let a = font_score(Path::new("/x/Amazon-Ember-Regular.ttf"));
        let b = font_score(Path::new("/x/Amazon-Ember-BoldItalic.ttf"));
        let c = font_score(Path::new("/x/Bookerly.ttf"));
        assert!(a > c && c > b);
    }
}
