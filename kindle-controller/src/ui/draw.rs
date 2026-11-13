//! Drawing helpers shared by the screens: segmented controls, tiles, round buttons, chips
//! and the battery icon. All grayscale, matching the mockups.

use crate::display::canvas::{BLACK, Canvas, Rect, WHITE};
use crate::display::font::{Fonts, Weight};
use crate::display::icons::{self, Icon};

pub const DARK_GRAY: u8 = 0x4d;
pub const MID_GRAY: u8 = 0x8c;
pub const LIGHT_GRAY: u8 = 0xd9;

pub const BORDER: f32 = 3.0;
pub const RADIUS: f32 = 16.0;

/// Baseline that vertically centres capital letters of the given size in `r`.
pub fn centred_baseline(fonts: &Fonts, r: Rect, px: u32, weight: Weight) -> i32 {
    r.y + (r.h + fonts.cap_height(px, weight)) / 2
}

/// One option in a segmented control or tile.
pub struct Choice<'a> {
    pub label: &'a str,
    pub icon: Option<Icon>,
    pub selected: bool,
    pub enabled: bool,
}

/// A row of joined buttons inside one rounded border. Returns each segment's rect.
pub fn segmented(canvas: &mut Canvas, fonts: &mut Fonts, r: Rect, choices: &[Choice], px: u32) -> Vec<Rect> {
    let n = choices.len() as i32;
    let rects: Vec<Rect> = (0..n)
        .map(|i| {
            let x0 = r.x + r.w * i / n;
            let x1 = r.x + r.w * (i + 1) / n;
            Rect::new(x0, r.y, x1 - x0, r.h)
        })
        .collect();
    canvas.fill_round_rect(r, RADIUS, WHITE);
    for (i, (c, s)) in choices.iter().zip(&rects).enumerate() {
        if c.selected {
            // Rounded only on the control's outer corners.
            canvas.fill_round_rect(*s, RADIUS, BLACK);
            if i > 0 {
                canvas.fill_rect(Rect::new(s.x, s.y, RADIUS as i32, s.h), BLACK);
            }
            if i + 1 < rects.len() {
                canvas.fill_rect(Rect::new(s.right() - RADIUS as i32, s.y, RADIUS as i32, s.h), BLACK);
            }
        }
        if i > 0 {
            canvas.fill_rect(Rect::new(s.x - 1, s.y, 3, s.h), BLACK);
        }
        choice_content(canvas, fonts, *s, c, px);
    }
    canvas.stroke_round_rect(r, RADIUS, BORDER, BLACK);
    rects
}

/// A standalone rounded button: black when selected.
pub fn tile(canvas: &mut Canvas, fonts: &mut Fonts, r: Rect, c: &Choice, px: u32) {
    let (fill, border) = match (c.selected, c.enabled) {
        (true, _) => (BLACK, BLACK),
        (false, true) => (WHITE, BLACK),
        (false, false) => (WHITE, LIGHT_GRAY),
    };
    canvas.fill_round_rect(r, RADIUS, fill);
    canvas.stroke_round_rect(r, RADIUS, BORDER, border);
    choice_content(canvas, fonts, r, c, px);
}

/// Icon and label side by side, centred in `r`.
fn choice_content(canvas: &mut Canvas, fonts: &mut Fonts, r: Rect, c: &Choice, px: u32) {
    let color = match (c.selected, c.enabled) {
        (true, _) => WHITE,
        (false, true) => BLACK,
        (false, false) => MID_GRAY,
    };
    let weight = if c.selected { Weight::Bold } else { Weight::Regular };
    let text_w = fonts.text_width(c.label, px, weight);
    let icon_size = (px as i32 * 4) / 3;
    let gap = 14;
    let total = text_w + c.icon.map_or(0, |_| icon_size + gap);
    let mut x = r.x + (r.w - total) / 2;
    if let Some(icon) = c.icon {
        icons::draw(canvas, icon, x, r.y + (r.h - icon_size) / 2, icon_size, 2.0 * icon_size as f32 / 24.0, color);
        x += icon_size + gap;
    }
    let baseline = centred_baseline(fonts, r, px, weight);
    fonts.draw(canvas, c.label, px, weight, x, baseline, color);
}

/// A circular button with an icon in the middle.
pub fn round_button(canvas: &mut Canvas, r: Rect, icon: Icon, enabled: bool) {
    let color = if enabled { BLACK } else { LIGHT_GRAY };
    let (cx, cy, rad) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0, r.w as f32 / 2.0);
    canvas.fill_disc(cx, cy, rad, WHITE);
    canvas.stroke_circle(cx, cy, rad, BORDER, color);
    let size = r.w * 45 / 100;
    icons::draw(canvas, icon, r.x + (r.w - size) / 2, r.y + (r.h - size) / 2, size, 2.5 * size as f32 / 24.0, color);
}

/// A rounded "pill" outline around an icon and text; returns its rect.
pub fn chip(canvas: &mut Canvas, fonts: &mut Fonts, x: i32, y: i32, h: i32, icon: Icon, parts: &[(&str, Weight)], px: u32) -> Rect {
    let icon_size = (px as i32 * 11) / 10;
    let pad_l = h / 3;
    let pad_r = h * 2 / 5;
    let gap = 10;
    let text_w: i32 = parts.iter().map(|(t, w)| fonts.text_width(t, px, *w)).sum();
    let r = Rect::new(x, y, pad_l + icon_size + gap + text_w + pad_r, h);
    canvas.fill_round_rect(r, h as f32 / 2.0, WHITE);
    canvas.stroke_round_rect(r, h as f32 / 2.0, 2.0, BLACK);
    icons::draw(canvas, icon, x + pad_l, y + (h - icon_size) / 2, icon_size, 2.0 * icon_size as f32 / 24.0, BLACK);
    let mut tx = x + pad_l + icon_size + gap;
    let baseline = centred_baseline(fonts, r, px, Weight::Regular);
    for (t, w) in parts {
        fonts.draw(canvas, t, px, *w, tx, baseline, BLACK);
        tx += fonts.text_width(t, px, *w);
    }
    r
}

/// Battery outline 92×34 at (x, y): filled to `percent`, with a bolt when charging.
pub fn battery(canvas: &mut Canvas, x: i32, y: i32, percent: Option<u8>, charging: bool, color: u8, background: u8) {
    let (fx, fy) = (x as f32, y as f32);
    canvas.stroke_round_rect(Rect::new(x, y, 83, 34), 7.0, 3.0, color);
    canvas.fill_round_rect(Rect::new(x + 83, y + 11, 6, 12), 1.5, color);
    if let Some(p) = percent {
        let w = (71 * i32::from(p.min(100)) + 50) / 100;
        if w > 0 {
            canvas.fill_round_rect(Rect::new(x + 6, y + 6, w, 22), 2.0, color);
        }
    }
    if charging {
        if let Some(path) = icons::parse_path("M47 3 L33 19 H41 L37 31 L51 14 H43 Z", fx, fy, 1.0) {
            canvas.fill_path(&path, background);
            canvas.stroke_path(&path, 1.5, color);
        }
    }
}

/// The three-dots menu glyph, centred in `r`; inverted when the menu is open.
pub fn menu_button(canvas: &mut Canvas, r: Rect, open: bool, color: u8, background: u8) {
    let fg = if open {
        canvas.fill_round_rect(r, 12.0, color);
        background
    } else {
        color
    };
    let cx = r.x as f32 + r.w as f32 / 2.0;
    let cy = r.y as f32 + r.h as f32 / 2.0;
    for dy in [-22.0, 0.0, 22.0] {
        canvas.fill_disc(cx, cy + dy, 7.0, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmented_splits_evenly_and_fills_selection() {
        let mut c = Canvas::new(400, 100);
        let mut f = Fonts::bitmap();
        let choices = [
            Choice { label: "A", icon: None, selected: false, enabled: true },
            Choice { label: "B", icon: None, selected: true, enabled: true },
        ];
        let rects = segmented(&mut c, &mut f, Rect::new(0, 0, 400, 100), &choices, 20);
        assert_eq!(rects, vec![Rect::new(0, 0, 200, 100), Rect::new(200, 0, 200, 100)]);
        assert_eq!(c.pixel(260, 20), BLACK);
        assert_eq!(c.pixel(60, 20), WHITE);
    }

    #[test]
    fn battery_fill_scales_with_charge() {
        let mut empty = Canvas::new(100, 40);
        battery(&mut empty, 0, 0, Some(0), false, BLACK, WHITE);
        let mut full = Canvas::new(100, 40);
        battery(&mut full, 0, 0, Some(100), false, BLACK, WHITE);
        assert_eq!(empty.pixel(40, 17), WHITE);
        assert_eq!(full.pixel(40, 17), BLACK);
        let mut charging = Canvas::new(100, 40);
        battery(&mut charging, 0, 0, Some(100), true, BLACK, WHITE);
        assert_eq!(charging.pixel(42, 17), WHITE, "the bolt shows through the fill");
    }
}
