//! Line icons, defined as SVG path data (mostly on a 24×24 grid, like the mockups) and
//! stroked with round caps.

use tiny_skia::{Path, PathBuilder};

use super::canvas::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sun,
    Moon,
    PartlySunny,
    Cloud,
    Rain,
    Storm,
    Snow,
    Fog,
    Flame,
    Snowflake,
    Fan,
    Auto,
    Power,
    Hourglass,
    Pause,
    Minus,
    Plus,
    Retry,
}

struct Def {
    view: f32,
    paths: &'static [&'static str],
    circles: &'static [(f32, f32, f32)],
}

const CLOUD: &str = "M7 16a4 4 0 1 1 .9-7.9A6 6 0 0 1 19 9.5a3.5 3.5 0 0 1-1 6.5H7z";
const SNOWFLAKE: &str = "M12 2v20M3.3 7l17.4 10M20.7 7L3.3 17M9 3.5l3 2.5 3-2.5M9 20.5l3-2.5 3 2.5";

fn def(icon: Icon) -> Def {
    let d = |paths, circles| Def { view: 24.0, paths, circles };
    match icon {
        Icon::Sun => d(
            &["M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8"],
            &[(12.0, 12.0, 4.5)],
        ),
        Icon::Moon => d(&["M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"], &[]),
        Icon::PartlySunny => d(
            &["M8 2v1.5M2 8h1.5M3.8 3.8l1 1M12.2 3.8l-1 1", "M9 18a3.5 3.5 0 1 1 .8-6.9A5 5 0 0 1 19 12a3 3 0 0 1-.5 6H9z"],
            &[(8.0, 8.0, 3.0)],
        ),
        Icon::Cloud => d(&[CLOUD], &[]),
        Icon::Rain => d(&[CLOUD, "M8 19l-1 2M12 19l-1 2M16 19l-1 2"], &[]),
        Icon::Storm => d(
            &["M7 15a4 4 0 1 1 .9-7.9A6 6 0 0 1 19 8.5a3.5 3.5 0 0 1-1 6.5H7z", "M13 15l-2.5 4h3.5l-2 4"],
            &[],
        ),
        Icon::Snow => d(&[CLOUD, "M8 19.5v.1M12 20.5v.1M16 19.5v.1"], &[]),
        Icon::Fog => d(&["M4 9h16M3 13h18M5 17h14"], &[]),
        Icon::Flame => d(&["M12 22c4 0 7-3 7-7 0-4-3-6-4-10-2 2-3 4-3 6-1-1-2-2-2-4-2 2-5 5-5 8 0 4 3 7 7 7z"], &[]),
        Icon::Snowflake => d(&[SNOWFLAKE], &[]),
        Icon::Fan => d(
            &["M12 10c0-4 1-7 4-7 2 0 2 4-4 7zM14 12c4 0 7 1 7 4 0 2-4 2-7-4zM12 14c0 4-1 7-4 7-2 0-2-4 4-7zM10 12c-4 0-7-1-7-4 0-2 4-2 7 4z"],
            &[(12.0, 12.0, 2.0)],
        ),
        Icon::Auto | Icon::Retry => d(&["M4 12a8 8 0 0 1 14-5.3M20 12a8 8 0 0 1-14 5.3M18 3v4h-4M6 21v-4h4"], &[]),
        Icon::Power => d(&["M12 3v9M6.3 6.3a8 8 0 1 0 11.4 0"], &[]),
        Icon::Hourglass => d(&["M6 2h12M6 22h12M7 2v4l5 6-5 6v4M17 2v4l-5 6 5 6v4"], &[]),
        Icon::Pause => d(&["M9 5v14M15 5v14"], &[]),
        Icon::Minus => d(&["M5 12h14"], &[]),
        Icon::Plus => d(&["M5 12h14M12 5v14"], &[]),
    }
}

/// Parses SVG path data, scaling by `scale` and offsetting by (`x`, `y`).
pub fn parse_path(data: &str, x: f32, y: f32, scale: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let p = |px: f64, py: f64| (x + px as f32 * scale, y + py as f32 * scale);
    for seg in svgtypes::SimplifyingPathParser::from(data) {
        match seg.ok()? {
            svgtypes::SimplePathSegment::MoveTo { x: px, y: py } => {
                let (a, b) = p(px, py);
                pb.move_to(a, b);
            }
            svgtypes::SimplePathSegment::LineTo { x: px, y: py } => {
                let (a, b) = p(px, py);
                pb.line_to(a, b);
            }
            svgtypes::SimplePathSegment::CurveTo { x1, y1, x2, y2, x: px, y: py } => {
                let (a1, b1) = p(x1, y1);
                let (a2, b2) = p(x2, y2);
                let (a, b) = p(px, py);
                pb.cubic_to(a1, b1, a2, b2, a, b);
            }
            svgtypes::SimplePathSegment::Quadratic { x1, y1, x: px, y: py } => {
                let (a1, b1) = p(x1, y1);
                let (a, b) = p(px, py);
                pb.quad_to(a1, b1, a, b);
            }
            svgtypes::SimplePathSegment::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

/// Draws `icon` in a `size`×`size` box at (`x`, `y`), with lines `stroke` pixels wide.
pub fn draw(canvas: &mut Canvas, icon: Icon, x: i32, y: i32, size: i32, stroke: f32, color: u8) {
    let d = def(icon);
    let scale = size as f32 / d.view;
    let (x, y) = (x as f32, y as f32);
    for data in d.paths {
        if let Some(path) = parse_path(data, x, y, scale) {
            canvas.stroke_path(&path, stroke, color);
        }
    }
    for &(cx, cy, r) in d.circles {
        if let Some(path) = PathBuilder::from_circle(x + cx * scale, y + cy * scale, r * scale) {
            canvas.stroke_path(&path, stroke, color);
        }
    }
}

/// A house with a line through it, for "can't reach Home Assistant". The line gets a halo
/// in `background` so it reads as crossing the house.
pub fn draw_house_slash(canvas: &mut Canvas, x: i32, y: i32, size: i32, color: u8, background: u8) {
    let scale = size as f32 / 48.0;
    let (fx, fy) = (x as f32, y as f32);
    if let Some(p) = parse_path("M6 22 L24 7 L42 22M10 19 V41 H38 V19M20 41 V30 H28 V41", fx, fy, scale) {
        canvas.stroke_path(&p, 2.4 * scale, color);
    }
    if let Some(p) = parse_path("M6 6 L42 42", fx, fy, scale) {
        canvas.stroke_path(&p, 7.0 * scale, background);
        canvas.stroke_path(&p, 2.6 * scale, color);
    }
}

/// A sleepy house for the sleep screen: closed eyes for windows, a little smile, and two Zs
/// off the roof. The drawing is on a 68×72 grid starting at (-2, -6).
pub fn draw_sleeping_house(canvas: &mut Canvas, x: i32, y: i32, size: i32, color: u8) {
    let scale = size as f32 / 72.0;
    let fx = x as f32 + (size as f32 - 68.0 * scale) / 2.0 + 2.0 * scale;
    let fy = y as f32 + 6.0 * scale;
    for data in [
        "M14 36 L32 21 L50 36M18 33 V55 H46 V33",
        "M22.5 40 q3 3.2 6 0M35.5 40 q3 3.2 6 0",
        "M29 48.5 q3 2.2 6 0",
        "M45 8h4.4l-4.4 4.4h4.4",
        "M52.5 1.5h5.6l-5.6 5.6h5.6",
    ] {
        if let Some(p) = parse_path(data, fx, fy, scale) {
            canvas.stroke_path(&p, 2.6 * scale, color);
        }
    }
}

/// The same house waking up, for the waking screen: eyes open, a smile, little arms
/// stretching out from the walls, and Wi-Fi arcs over the roof. Same grid as
/// `draw_sleeping_house`, so the house doesn't move.
pub fn draw_waking_house(canvas: &mut Canvas, x: i32, y: i32, size: i32, color: u8) {
    let scale = size as f32 / 72.0;
    let fx = x as f32 + (size as f32 - 68.0 * scale) / 2.0 + 2.0 * scale;
    let fy = y as f32 + 6.0 * scale;
    let w = 2.6 * scale;
    for data in [
        "M14 36 L32 21 L50 36M18 33 V55 H46 V33",
        "M28.5 47.5 q3.5 3 7 0",
        "M18 44 L10 37 l-2 -4M10 37 l-4 -1",
        "M46 44 L54 37 l2 -4M54 37 l4 -1",
        "M26 11 a8.5 8.5 0 0 1 12 0",
        "M21.5 6.5 a15 15 0 0 1 21 0",
    ] {
        if let Some(p) = parse_path(data, fx, fy, scale) {
            canvas.stroke_path(&p, w, color);
        }
    }
    // Eyes, and the dot under the Wi-Fi arcs.
    for (cx, cy, r) in [(25.5, 40.5, 1.4), (38.5, 40.5, 1.4), (32.0, 15.5, 0.6)] {
        canvas.fill_disc(fx + cx * scale, fy + cy * scale, r * scale + w / 2.0, color);
    }
}

/// An aircon unit with a "!" badge, for "aircon unavailable".
pub fn draw_aircon_alert(canvas: &mut Canvas, x: i32, y: i32, size: i32, color: u8, background: u8) {
    let scale = size as f32 / 48.0;
    let (fx, fy) = (x as f32, y as f32);
    let w = 2.2 * scale;
    for data in [
        "M6 8h32a3 3 0 0 1 3 3v14a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z",
        "M7 22h30M12 32l-2 6M22 32v7",
    ] {
        if let Some(p) = parse_path(data, fx, fy, scale) {
            canvas.stroke_path(&p, w, color);
        }
    }
    canvas.fill_disc(fx + 36.0 * scale, fy + 36.0 * scale, 9.0 * scale + w / 2.0, background);
    canvas.stroke_circle(fx + 36.0 * scale, fy + 36.0 * scale, 9.0 * scale + w / 2.0, w, color);
    if let Some(p) = parse_path("M36 31v6M36 40.6v0.2", fx, fy, scale) {
        canvas.stroke_path(&p, w, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::canvas::{BLACK, WHITE};

    #[test]
    fn every_icon_parses_and_draws() {
        for icon in [
            Icon::Sun,
            Icon::Moon,
            Icon::PartlySunny,
            Icon::Cloud,
            Icon::Rain,
            Icon::Storm,
            Icon::Snow,
            Icon::Fog,
            Icon::Flame,
            Icon::Snowflake,
            Icon::Fan,
            Icon::Auto,
            Icon::Power,
            Icon::Hourglass,
            Icon::Pause,
            Icon::Minus,
            Icon::Plus,
            Icon::Retry,
        ] {
            for data in def(icon).paths {
                assert!(parse_path(data, 0.0, 0.0, 1.0).is_some(), "{icon:?}: {data}");
            }
            let mut c = Canvas::new(64, 64);
            draw(&mut c, icon, 0, 0, 64, 3.0, BLACK);
            assert!(c.diff_bounds(&Canvas::new(64, 64)).is_some(), "{icon:?} drew nothing");
        }
        let mut c = Canvas::new(100, 100);
        draw_house_slash(&mut c, 0, 0, 100, BLACK, WHITE);
        draw_aircon_alert(&mut c, 0, 0, 100, BLACK, WHITE);
        let mut c = Canvas::new(100, 100);
        draw_sleeping_house(&mut c, 0, 0, 100, BLACK);
        assert!(c.diff_bounds(&Canvas::new(100, 100)).is_some(), "sleeping house drew nothing");
        let mut c = Canvas::new(100, 100);
        draw_waking_house(&mut c, 0, 0, 100, BLACK);
        assert!(c.diff_bounds(&Canvas::new(100, 100)).is_some(), "waking house drew nothing");
    }
}
