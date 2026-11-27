//! The main screen, laid out like mockup B2 (1448×1072 landscape):
//! weather and 24-hour forecast across the top, room temperatures below, and the aircon
//! panel filling the bottom half.

use tiny_skia::PathBuilder;

use super::draw::{self, Choice, DARK_GRAY, LIGHT_GRAY, MID_GRAY, centred_baseline};
use super::{App, HISTORY_WINDOW, Hit, MARGIN, RAIL_X, Tap, set_call};
use crate::clock;
use crate::display::canvas::{BLACK, Rect, WHITE};
use crate::display::font::Weight;
use crate::display::icons::{self, Icon};
use crate::ha::ForecastHour;

const WEATHER: Rect = Rect::new(MARGIN, MARGIN, 340, 280);
const FORECAST: Rect = Rect::new(MARGIN + 340 + 24, MARGIN, RAIL_X - 24 - (MARGIN + 340 + 24), 280);
const ROOMS: Rect = Rect::new(MARGIN, MARGIN + 280 + 24, super::W - 2 * MARGIN, 140);
const AIRCON: Rect = Rect::new(MARGIN, ROOMS.y + 140 + 24, super::W - 2 * MARGIN, super::H - MARGIN - (ROOMS.y + 140 + 24));

fn weather_icon(condition: &str) -> Icon {
    match condition {
        "sunny" => Icon::Sun,
        "clear-night" => Icon::Moon,
        "partlycloudy" => Icon::PartlySunny,
        "rainy" | "pouring" | "hail" => Icon::Rain,
        "lightning" | "lightning-rainy" | "exceptional" => Icon::Storm,
        "snowy" | "snowy-rainy" => Icon::Snow,
        "fog" => Icon::Fog,
        _ => Icon::Cloud,
    }
}

fn condition_label(condition: &str) -> String {
    match condition {
        "clear-night" => "Clear",
        "cloudy" => "Cloudy",
        "exceptional" => "Extreme weather",
        "fog" => "Fog",
        "hail" => "Hail",
        "lightning" => "Thunderstorms",
        "lightning-rainy" => "Storms",
        "partlycloudy" => "Partly cloudy",
        "pouring" => "Heavy rain",
        "rainy" => "Rain",
        "snowy" => "Snow",
        "snowy-rainy" => "Sleet",
        "sunny" => "Sunny",
        "windy" | "windy-variant" => "Windy",
        "" | "unavailable" | "unknown" => "No weather",
        other => other,
    }
    .to_string()
}

/// "Rain from 22:00", "Raining now" or "No rain in the next 24 h".
fn rain_summary(hours: &[&ForecastHour]) -> String {
    if hours.is_empty() {
        return String::new();
    }
    match hours.iter().position(|h| h.precipitation.unwrap_or(0.0) >= 0.2) {
        Some(0) => "Raining now".into(),
        Some(i) => {
            let (h, _) = clock::local_hm(hours[i].time);
            format!("Rain from {h:02}:00")
        }
        None => "No rain in the next 24 h".into(),
    }
}

/// Grid lines for a temperature axis: every 2, 5 or 10 degrees.
fn ticks(lo: f64, hi: f64) -> Vec<f64> {
    let span = hi - lo;
    let step = if span > 24.0 { 10.0 } else if span > 9.0 { 5.0 } else { 2.0 };
    let mut t = (lo / step).ceil() * step;
    let mut out = Vec::new();
    while t < hi {
        if t > lo {
            out.push(t);
        }
        t += step;
    }
    out
}

impl App {
    pub(super) fn render_dashboard(&mut self) {
        self.draw_weather(WEATHER);
        self.draw_forecast(FORECAST);
        self.draw_rooms(ROOMS);
        self.draw_aircon(AIRCON);
    }

    /// The next 24 hours of forecast (25 hourly points, starting with the current hour).
    fn next_24h(&self) -> Vec<&ForecastHour> {
        let now = self.frame_now;
        self.forecast.iter().filter(|h| h.time >= now - 3600.0).take(25).collect()
    }

    fn draw_weather(&mut self, r: Rect) {
        let Some(id) = self.dash.weather.clone() else { return };
        let state = self.entities.get(&id);
        let condition = state.map(|s| s.state.clone()).unwrap_or_default();
        let temp = state.and_then(|s| s.attr_f64("temperature"));
        let hours = self.next_24h();
        let rain = rain_summary(&hours);
        let temps: Vec<f64> = hours.iter().filter_map(|h| h.temperature).collect();
        let high_low = if temps.is_empty() {
            String::new()
        } else {
            let hi = temps.iter().cloned().fold(f64::MIN, f64::max);
            let lo = temps.iter().cloned().fold(f64::MAX, f64::min);
            format!("High {:.0}° · Low {:.0}°", hi, lo)
        };

        let top = r.y + (r.h - 230) / 2;
        icons::draw(&mut self.canvas, weather_icon(&condition), r.x, top, 96, 5.0, BLACK);
        let temp_text = temp.map_or("--°".to_string(), |t| format!("{t:.0}°"));
        let cap = self.fonts.cap_height(112, Weight::Bold);
        self.fonts.draw(&mut self.canvas, &temp_text, 112, Weight::Bold, r.x + 96 + 18, top + 48 + cap / 2, BLACK);

        let mut y = top + 96 + 16;
        let label = condition_label(&condition);
        for (text, px, weight, color) in [
            (label.as_str(), 32, Weight::Bold, BLACK),
            (rain.as_str(), 24, Weight::Regular, DARK_GRAY),
            (high_low.as_str(), 24, Weight::Regular, DARK_GRAY),
        ] {
            let (a, d) = self.fonts.line_metrics(px, weight);
            let text = self.fonts.ellipsize(text, px, weight, r.w);
            self.fonts.draw(&mut self.canvas, &text, px, weight, r.x, y + a, color);
            y += a + d + 6;
        }
    }

    fn draw_forecast(&mut self, r: Rect) {
        let (a22, _) = self.fonts.line_metrics(22, Weight::Regular);
        let header_y = r.y + 12;
        self.fonts.draw(&mut self.canvas, "NEXT 24 HOURS", 22, Weight::Regular, r.x, header_y + a22, DARK_GRAY);
        // Legend, right-aligned.
        let (a20, _) = self.fonts.line_metrics(20, Weight::Regular);
        let mut x = r.right();
        for (label, swatch) in [("Rain mm/h", Rect::new(0, 0, 14, 20)), ("Temp °C", Rect::new(0, 0, 28, 4))] {
            let tw = self.fonts.text_width(label, 20, Weight::Regular);
            x -= tw;
            self.fonts.draw(&mut self.canvas, label, 20, Weight::Regular, x, header_y + a20, DARK_GRAY);
            x -= 8 + swatch.w;
            let color = if swatch.h == 4 { BLACK } else { MID_GRAY };
            self.canvas.fill_rect(Rect::new(x, header_y + a20 - 8 - swatch.h / 2, swatch.w, swatch.h), color);
            x -= 24;
        }

        let plot = Rect::new(r.x, header_y + 36, r.w, 190);
        let labels_y = plot.bottom() + 8;
        let hours: Vec<ForecastHour> = self.next_24h().into_iter().cloned().collect();
        let temps: Vec<f64> = hours.iter().filter_map(|h| h.temperature).collect();
        if hours.len() < 2 || temps.len() < 2 {
            let baseline = centred_baseline(&self.fonts, plot, 26, Weight::Regular);
            self.fonts.draw_centered(&mut self.canvas, "No forecast yet", 26, Weight::Regular, plot.x + plot.w / 2, baseline, MID_GRAY);
            return;
        }

        let (left, right, top, bottom) = (48.0f32, 8.0f32, 12.0f32, 8.0f32);
        let pw = plot.w as f32 - left - right;
        let ph = plot.h as f32 - top - bottom;
        let tmin = temps.iter().cloned().fold(f64::MAX, f64::min);
        let tmax = temps.iter().cloned().fold(f64::MIN, f64::max);
        let (lo, hi) = (tmin.floor() - 3.0, tmax.ceil() + 3.0);
        let n = hours.len();
        let xf = |i: usize| plot.x as f32 + left + i as f32 * pw / (n - 1) as f32;
        let yf = |t: f64| plot.y as f32 + top + ((hi - t) / (hi - lo)) as f32 * ph;
        let base = plot.y as f32 + top + ph;

        // Grid and labels.
        let mut grid = PathBuilder::new();
        for t in ticks(lo, hi) {
            grid.move_to(plot.x as f32 + left, yf(t));
            grid.line_to(plot.right() as f32 - right, yf(t));
            let label = format!("{t:.0}°");
            let cap = self.fonts.cap_height(20, Weight::Regular);
            self.fonts.draw_right(&mut self.canvas, &label, 20, Weight::Regular, plot.x + left as i32 - 10, yf(t) as i32 + cap / 2, DARK_GRAY);
        }
        grid.move_to(plot.x as f32 + left, base);
        grid.line_to(plot.right() as f32 - right, base);
        if let Some(p) = grid.finish() {
            self.canvas.stroke_path(&p, 2.0, LIGHT_GRAY);
        }

        // A light dotted line at midnight (local time), to show where tomorrow starts.
        let midnight = (1..n).find(|&i| {
            clock::local_hm(hours[i].time).0 == 0 && clock::local_hm(hours[i - 1].time).0 != 0
        });
        if let Some(i) = midnight {
            let x = xf(i);
            let mut y = plot.y as f32 + top;
            while y <= base {
                self.canvas.fill_disc(x, y, 1.6, MID_GRAY);
                y += 8.0;
            }
        }

        // Rain bars (scale: 4 mm/h full height, or the wettest hour if heavier).
        let rmax = hours.iter().filter_map(|h| h.precipitation).fold(4.0f64, f64::max);
        let bw = pw / (n - 1) as f32 * 0.6;
        for (i, h) in hours.iter().enumerate() {
            let mm = h.precipitation.unwrap_or(0.0);
            if mm > 0.0 {
                let bh = ((mm / rmax) as f32 * ph).max(2.0);
                let bar = Rect::new((xf(i) - bw / 2.0) as i32, (base - bh) as i32, bw as i32, bh as i32);
                self.canvas.fill_rect(bar, MID_GRAY);
            }
        }

        // Temperature line.
        let mut line = PathBuilder::new();
        let mut started = false;
        for (i, h) in hours.iter().enumerate() {
            if let Some(t) = h.temperature {
                if started {
                    line.line_to(xf(i), yf(t));
                } else {
                    line.move_to(xf(i), yf(t));
                    started = true;
                }
            }
        }
        if let Some(p) = line.finish() {
            self.canvas.stroke_path(&p, 4.0, BLACK);
        }

        // Hour labels every 6 hours.
        let (a, _) = self.fonts.line_metrics(20, Weight::Regular);
        for i in (0..n).step_by(6) {
            let label = if i == 0 { "Now".to_string() } else { clock::local_hhmm(hours[i].time) };
            let x = xf(i) as i32;
            if i == 0 {
                self.fonts.draw(&mut self.canvas, &label, 20, Weight::Regular, x, labels_y + a, DARK_GRAY);
            } else if i + 6 >= n {
                self.fonts.draw_right(&mut self.canvas, &label, 20, Weight::Regular, x, labels_y + a, DARK_GRAY);
            } else {
                self.fonts.draw_centered(&mut self.canvas, &label, 20, Weight::Regular, x, labels_y + a, DARK_GRAY);
            }
        }
    }

    /// Whether the thermostat is on (and the aircon reachable), so its room can be picked.
    fn thermostat_on(&self) -> bool {
        let reachable = self.entities.get(&self.aircon.power).is_some_and(|p| !p.is_unavailable());
        reachable && self.in_ha(&self.aircon.thermostat).is_some_and(|t| self.effective(&t) == Some("on"))
    }

    /// Which way Auto is working ("Heat" or "Cool"), from **Auto direction**.
    pub(super) fn auto_direction(&self) -> Option<&str> {
        let id = self.aircon.auto_direction.as_ref()?;
        let state = self.entities.get(id)?.state.as_str();
        matches!(state, "Heat" | "Cool").then_some(state)
    }

    /// The room the thermostat goes by, if one is picked (rather than the average).
    pub(super) fn selected_room(&self) -> Option<String> {
        let select = self.in_ha(&self.aircon.thermostat_room)?;
        let option = self.effective(&select)?;
        self.rooms.iter().any(|r| r.name == option).then(|| option.to_string())
    }

    fn draw_rooms(&mut self, r: Rect) {
        let rooms = self.rooms.clone();
        let n = rooms.len().min(4) as i32;
        if n == 0 {
            self.canvas.stroke_round_rect(r, 16.0, 2.0, LIGHT_GRAY);
            let (a, d) = self.fonts.line_metrics(30, Weight::Bold);
            let (a2, _) = self.fonts.line_metrics(24, Weight::Regular);
            let top = r.y + (r.h - (a + d + 10 + a2)) / 2;
            let cx = r.x + r.w / 2;
            self.fonts.draw_centered(&mut self.canvas, "No rooms set up", 30, Weight::Bold, cx, top + a, DARK_GRAY);
            let detail = "Add them in Home Assistant with the Aircon rooms blueprint";
            self.fonts.draw_centered(&mut self.canvas, detail, 24, Weight::Regular, cx, top + a + d + 10 + a2, DARK_GRAY);
            return;
        }
        // In thermostat mode, tapping a room makes the thermostat go by it; tapping it
        // again goes back to the average of all of them.
        let select = self.in_ha(&self.aircon.thermostat_room).filter(|_| self.thermostat_on());
        let selected = select.as_ref().and_then(|_| self.selected_room());
        let gap = 20;
        let w = (r.w - gap * (n - 1)) / n;
        for (i, room) in rooms.iter().take(4).enumerate() {
            let chip = Rect::new(r.x + i as i32 * (w + gap), r.y, w, r.h);
            if selected.as_deref() == Some(room.name.as_str()) {
                self.canvas.stroke_round_rect(chip, 16.0, 6.0, BLACK);
            } else {
                self.canvas.stroke_round_rect(chip, 16.0, 2.0, MID_GRAY);
            }
            let (a, _) = self.fonts.line_metrics(24, Weight::Regular);
            let name = self.fonts.ellipsize(&room.name, 24, Weight::Regular, chip.w - 36);
            self.fonts.draw(&mut self.canvas, &name, 24, Weight::Regular, chip.x + 18, chip.y + 12 + a, DARK_GRAY);
            let value = self.entities.get(&room.sensor).and_then(|s| s.number());
            let points = self.history.get(&room.sensor).cloned().unwrap_or_default();
            let text = value.map_or("--.-°".to_string(), |v| format!("{v:.1}°"));
            self.fonts.draw(&mut self.canvas, &text, 56, Weight::Bold, chip.x + 18, chip.bottom() - 18, BLACK);
            let spark = Rect::new(chip.right() - 18 - 136, chip.bottom() - 14 - 60, 136, 60);
            self.draw_sparkline(spark, &points);
            if let Some(select) = &select {
                let desired = if selected.as_deref() == Some(room.name.as_str()) { "Average" } else { room.name.as_str() };
                self.hits.push(Hit {
                    rect: chip,
                    tap: Tap::Set { entity: select.clone(), desired: desired.into(), call: set_call(select, desired) },
                });
            }
        }
    }

    /// Last hour of readings, no axes: a filled area under a line.
    fn draw_sparkline(&mut self, r: Rect, points: &[(f64, f64)]) {
        let now = self.frame_now;
        let window = HISTORY_WINDOW.as_secs_f64();
        let start = now - window;
        // Extend the last reading to "now", since the sensor holds its value until it changes.
        let mut pts: Vec<(f64, f64)> = points.iter().map(|&(t, v)| (t.max(start), v)).collect();
        if let Some(&(t, v)) = pts.last() {
            if t < now {
                pts.push((now, v));
            }
        }
        if pts.len() < 2 {
            return;
        }
        let lo = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let hi = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(0.8);
        let mid = (hi + lo) / 2.0;
        let base = mid - span / 2.0;
        let (x0, y0, w, h) = (r.x as f32 + 2.0, r.y as f32 + 4.0, r.w as f32 - 4.0, r.h as f32 - 8.0);
        let xf = |t: f64| x0 + ((t - start) / window).clamp(0.0, 1.0) as f32 * w;
        let yf = |v: f64| y0 + (1.0 - ((v - base) / span) as f32) * h;

        let mut line = PathBuilder::new();
        let mut area = PathBuilder::new();
        area.move_to(xf(pts[0].0), r.bottom() as f32);
        for (i, &(t, v)) in pts.iter().enumerate() {
            if i == 0 {
                line.move_to(xf(t), yf(v));
            } else {
                line.line_to(xf(t), yf(v));
            }
            area.line_to(xf(t), yf(v));
        }
        area.line_to(xf(pts[pts.len() - 1].0), r.bottom() as f32);
        area.close();
        if let Some(area) = area.finish() {
            self.canvas.fill_path(&area, LIGHT_GRAY);
        }
        if let Some(line) = line.finish() {
            self.canvas.stroke_path(&line, 3.0, BLACK);
        }
    }

    fn draw_aircon(&mut self, r: Rect) {
        let a = self.aircon.clone();
        self.canvas.stroke_round_rect(r, 20.0, 3.0, BLACK);
        let power = self.entities.get(&a.power).cloned();
        match &power {
            None => {
                let detail = format!("{} isn't in Home Assistant. Check config.toml.", a.power);
                return self.draw_aircon_message(r, "Aircon not found", &detail, None);
            }
            Some(p) if p.is_unavailable() => {
                let seen = p.last_changed.map(|t| {
                    let mins = ((self.frame_now - t) / 60.0).max(0.0) as u64;
                    let ago = if mins < 120 { format!("{mins} min ago") } else { format!("{} h ago", mins / 60) };
                    format!("Last seen {} · {ago}", clock::local_hhmm(t))
                });
                return self.draw_aircon_message(
                    r,
                    "Aircon unavailable",
                    "Home Assistant can't reach the aircon controller.",
                    seen.as_deref(),
                );
            }
            Some(_) => {}
        }
        if self.no_flow() {
            return self.draw_no_flow(r);
        }

        let inner = Rect::new(r.x + 31, r.y + 25, r.w - 62, r.h - 50);
        let header = Rect::new(inner.x, inner.y, inner.w, 84);
        let title_base = centred_baseline(&self.fonts, header, 40, Weight::Bold);
        self.fonts.draw(&mut self.canvas, "Aircon", 40, Weight::Bold, header.x, title_base, BLACK);

        let chip_x = header.x + self.fonts.text_width("Aircon", 40, Weight::Bold) + 24;
        if let Some(reason) = self.idle_reason().filter(|_| self.effective(&a.power) == Some("on")) {
            let detail = format!(" · {reason}");
            let parts = [("Idle", Weight::Bold), (detail.as_str(), Weight::Regular)];
            draw::chip(&mut self.canvas, &mut self.fonts, chip_x, header.y + 18, 48, Icon::Pause, &parts, 24);
        } else if let Some(left) = self.lockout_shown() {
            let countdown = format!("{:02}:{:02}", left / 60, left % 60);
            let parts = [("Compressor lockout ", Weight::Regular), (countdown.as_str(), Weight::Bold)];
            draw::chip(&mut self.canvas, &mut self.fonts, chip_x, header.y + 18, 48, Icon::Hourglass, &parts, 24);
        }

        let on = self.effective(&a.power) == Some("on");
        let power_rect = Rect::new(header.right() - 160, header.y, 160, 84);
        let label = if on { "On" } else { "Off" };
        draw::tile(&mut self.canvas, &mut self.fonts, power_rect, &Choice { label, icon: Some(Icon::Power), selected: on, enabled: true }, 32);
        let desired = if on { "off" } else { "on" };
        self.hits.push(Hit {
            rect: power_rect,
            tap: Tap::Set { entity: a.power.clone(), desired: desired.into(), call: set_call(&a.power, desired) },
        });

        let body = Rect::new(inner.x, header.bottom() + 22, inner.w, inner.bottom() - header.bottom() - 22);
        let thermostat = self.in_ha(&a.thermostat);
        let setpoint = self.in_ha(&a.setpoint);
        let has_left = thermostat.is_some() || setpoint.is_some();
        let left = Rect::new(body.x, body.y, 600, body.h);
        let right = if has_left { Rect::new(body.x + 632, body.y, body.w - 632, body.h) } else { body };

        // Auto needs the thermostat, so Continuous is greyed out in Auto, and Auto in Continuous.
        let current_mode = self.effective(&a.mode).map(str::to_string);
        let in_auto = current_mode.as_deref() == Some(a.mode_options.auto.as_str());
        let continuous = thermostat.as_ref().is_some_and(|t| self.effective(t) != Some("on"));

        if has_left {
            let mut setpoint_rect = left;
            let thermostat_on = match &thermostat {
                Some(t) => {
                    let is_on = self.effective(t) == Some("on");
                    let toggle = Rect::new(left.x, left.y, left.w, 96);
                    let choices = [
                        Choice { label: "Continuous", icon: None, selected: !is_on, enabled: !in_auto },
                        Choice { label: "Thermostat", icon: None, selected: is_on, enabled: true },
                    ];
                    let segs = draw::segmented(&mut self.canvas, &mut self.fonts, toggle, &choices, 30);
                    for ((seg, value), choice) in segs.into_iter().zip(["off", "on"]).zip(&choices) {
                        if choice.enabled {
                            self.hits.push(Hit {
                                rect: seg,
                                tap: Tap::Set { entity: t.clone(), desired: value.into(), call: set_call(t, value) },
                            });
                        }
                    }
                    setpoint_rect = Rect::new(left.x, left.y + 116, left.w, left.h - 116);
                    is_on
                }
                None => true,
            };
            if let (true, Some(sp)) = (thermostat_on, &setpoint) {
                if let Some(now) = self.in_ha(&a.thermostat_temperature) {
                    let line_h = 52;
                    let box_rect = Rect::new(setpoint_rect.x, setpoint_rect.y, setpoint_rect.w, setpoint_rect.h - line_h);
                    self.draw_setpoint(box_rect, sp, a.setpoint_step);
                    let value = self.entities.get(&now).and_then(|s| s.number());
                    let from = self.selected_room().unwrap_or_else(|| "average".into());
                    let text = format!("Now {} · {from}", value.map_or("--.-°".to_string(), |v| format!("{v:.1}°")));
                    let text = self.fonts.ellipsize(&text, 28, Weight::Regular, setpoint_rect.w);
                    let baseline = setpoint_rect.bottom() - 8;
                    self.fonts.draw_centered(&mut self.canvas, &text, 28, Weight::Regular, setpoint_rect.x + setpoint_rect.w / 2, baseline, DARK_GRAY);
                } else {
                    self.draw_setpoint(setpoint_rect, sp, a.setpoint_step);
                }
            }
        }

        // Mode tiles, 2×2.
        let fan_h = 96;
        let modes = Rect::new(right.x, right.y, right.w, right.h - fan_h - 20);
        let mode_options = self.entities.get(&a.mode).map(|s| s.options().iter().map(|o| o.to_string()).collect::<Vec<_>>()).unwrap_or_default();
        let (tw, th) = ((modes.w - 16) / 2, (modes.h - 16) / 2);
        let mo = &a.mode_options;
        // In Auto, the tile says which way it's working.
        let auto_label = match self.auto_direction().filter(|_| in_auto) {
            Some(dir) => format!("Auto · {dir}"),
            None => "Auto".to_string(),
        };
        for (i, (label, icon, option)) in [
            ("Heat", Icon::Flame, &mo.heat),
            ("Cool", Icon::Snowflake, &mo.cool),
            ("Fan", Icon::Fan, &mo.fan),
            (auto_label.as_str(), Icon::Auto, &mo.auto),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::new(modes.x + (i as i32 % 2) * (tw + 16), modes.y + (i as i32 / 2) * (th + 16), tw, th);
            let is_auto = option == &mo.auto;
            let enabled = (mode_options.is_empty() || mode_options.contains(option)) && !(is_auto && continuous);
            let selected = current_mode.as_deref() == Some(option.as_str());
            draw::tile(&mut self.canvas, &mut self.fonts, rect, &Choice { label, icon: Some(icon), selected, enabled }, 30);
            if enabled {
                self.hits.push(Hit {
                    rect,
                    tap: Tap::Set { entity: a.mode.clone(), desired: option.clone(), call: set_call(&a.mode, option) },
                });
            }
        }

        // Fan speed.
        let fan_rect = Rect::new(right.x, right.bottom() - fan_h, right.w, fan_h);
        let current_fan = self.effective(&a.fan).map(str::to_string);
        let fo = &a.fan_options;
        let speeds = [("Low", &fo.low), ("Medium", &fo.medium), ("High", &fo.high)];
        let choices: Vec<Choice> = speeds
            .iter()
            .map(|(label, option)| Choice { label, icon: None, selected: current_fan.as_deref() == Some(option.as_str()), enabled: true })
            .collect();
        let segs = draw::segmented(&mut self.canvas, &mut self.fonts, fan_rect, &choices, 30);
        for (seg, (_, option)) in segs.into_iter().zip(speeds) {
            self.hits.push(Hit {
                rect: seg,
                tap: Tap::Set { entity: a.fan.clone(), desired: option.clone(), call: set_call(&a.fan, option) },
            });
        }
    }

    fn draw_setpoint(&mut self, r: Rect, entity: &str, step: f64) {
        self.canvas.stroke_round_rect(r, 20.0, 3.0, BLACK);
        let value = self.effective(entity).and_then(|s| s.parse::<f64>().ok());
        let attrs = self.entities.get(entity);
        let min = attrs.and_then(|s| s.attr_f64("min")).unwrap_or(f64::MIN);
        let max = attrs.and_then(|s| s.attr_f64("max")).unwrap_or(f64::MAX);
        let size = 112;
        let cy = r.y + r.h / 2;
        let minus = Rect::new(r.x + 24, cy - size / 2, size, size);
        let plus = Rect::new(r.right() - 24 - size, cy - size / 2, size, size);
        let text = value.map_or("--.-°".to_string(), |v| format!("{v:.1}°"));
        let cap = self.fonts.cap_height(110, Weight::Bold);
        self.fonts.draw_centered(&mut self.canvas, &text, 110, Weight::Bold, r.x + r.w / 2, cy + cap / 2, BLACK);
        for (rect, icon, delta) in [(minus, Icon::Minus, -step), (plus, Icon::Plus, step)] {
            let target = value.map(|v| ((v + delta) / step).round() * step).filter(|t| *t >= min && *t <= max);
            draw::round_button(&mut self.canvas, rect, icon, target.is_some());
            if let Some(t) = target {
                let desired = format!("{t}");
                self.hits.push(Hit {
                    rect,
                    tap: Tap::Set { entity: entity.to_string(), desired: desired.clone(), call: set_call(entity, &desired) },
                });
            }
        }
    }

    /// In place of the aircon controls while the aircon reports no condenser-water flow (the
    /// firmware then keeps the whole unit off, fan included): a black frame with an inverted
    /// "ERROR" heading, big enough to read across the room, and the sad house with what's
    /// wrong beside it. No controls are drawn, so none can be tapped.
    fn draw_no_flow(&mut self, r: Rect) {
        const FRAME: i32 = 8;
        const BAND_H: i32 = 160;
        const TITLE: &str = "ERROR";
        self.canvas.fill_round_rect(r, 20.0, BLACK);
        let body = Rect::new(r.x + FRAME, r.y + BAND_H, r.w - 2 * FRAME, r.h - BAND_H - FRAME);
        self.canvas.fill_round_rect(body, 12.0, WHITE);
        // Square off the body's top corners against the heading.
        self.canvas.fill_rect(Rect::new(body.x, body.y, body.w, 16), WHITE);

        let band = Rect::new(r.x + FRAME, r.y + FRAME, r.w - 2 * FRAME, BAND_H - FRAME);
        let baseline = centred_baseline(&self.fonts, band, 120, Weight::Bold);
        self.fonts.draw_centered(&mut self.canvas, TITLE, 120, Weight::Bold, band.x + band.w / 2, baseline, WHITE);

        // The house on white, where its face reads better than inverted, and the text beside it.
        let pad = 48;
        let house = (body.h - 24).min(340);
        icons::draw_sad_house(&mut self.canvas, body.x + pad, body.y + (body.h - house) / 2, house, BLACK);
        let text_x = body.x + pad + house + 48;
        let text_w = body.right() - pad - text_x;
        let mut lines: Vec<(String, u32, Weight)> = Vec::new();
        for (text, px, weight) in [
            ("No water flow", 64, Weight::Bold),
            ("The aircon is off until the building's water pump is running.", 44, Weight::Regular),
        ] {
            lines.extend(self.fonts.wrap(text, px, weight, text_w).into_iter().take(3).map(|l| (l, px, weight)));
        }
        // More space between the two sentences than between wrapped lines.
        let (line_gap, sentence_gap) = (10, 26);
        let gap = |i: usize, lines: &[(String, u32, Weight)]| {
            if i == 0 { 0 } else if lines[i - 1].1 != lines[i].1 { sentence_gap } else { line_gap }
        };
        let total: i32 = (0..lines.len())
            .map(|i| {
                let (a, d) = self.fonts.line_metrics(lines[i].1, lines[i].2);
                a + d + gap(i, &lines)
            })
            .sum();
        let mut y = body.y + (body.h - total) / 2;
        for i in 0..lines.len() {
            y += gap(i, &lines);
            let (line, px, weight) = &lines[i];
            let (a, d) = self.fonts.line_metrics(*px, *weight);
            self.fonts.draw(&mut self.canvas, line, *px, *weight, text_x, y + a, BLACK);
            y += a + d;
        }
    }

    /// The aircon panel's error card: icon, title, detail and an optional footnote.
    fn draw_aircon_message(&mut self, r: Rect, title: &str, detail: &str, foot: Option<&str>) {
        let cx = r.x + r.w / 2;
        let mut y = r.y + (r.h - 320) / 2;
        icons::draw_aircon_alert(&mut self.canvas, cx - 66, y, 132, BLACK, WHITE);
        y += 132 + 18;
        let (a, d) = self.fonts.line_metrics(52, Weight::Bold);
        self.fonts.draw_centered(&mut self.canvas, title, 52, Weight::Bold, cx, y + a, BLACK);
        y += a + d + 18;
        let (a, d) = self.fonts.line_metrics(30, Weight::Regular);
        let detail = self.fonts.ellipsize(detail, 30, Weight::Regular, r.w - 80);
        self.fonts.draw_centered(&mut self.canvas, &detail, 30, Weight::Regular, cx, y + a, DARK_GRAY);
        y += a + d + 14;
        if let Some(foot) = foot {
            let (a, _) = self.fonts.line_metrics(26, Weight::Regular);
            self.fonts.draw_centered(&mut self.canvas, foot, 26, Weight::Regular, cx, y + a, DARK_GRAY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_the_screen() {
        assert_eq!(FORECAST.right() + 24, RAIL_X);
        assert_eq!(AIRCON.bottom(), super::super::H - MARGIN);
        assert_eq!(ROOMS.right(), super::super::W - MARGIN);
    }

    #[test]
    fn tick_spacing() {
        assert_eq!(ticks(14.0, 32.0), vec![15.0, 20.0, 25.0, 30.0]);
        assert_eq!(ticks(1.0, 9.0), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn rain_summaries() {
        let h = |p: f64| ForecastHour { time: 0.0, temperature: Some(10.0), precipitation: Some(p), condition: String::new() };
        let (dry, wet) = (h(0.0), h(1.0));
        assert_eq!(rain_summary(&[&wet, &dry]), "Raining now");
        assert_eq!(rain_summary(&[&dry, &dry]), "No rain in the next 24 h");
        assert!(rain_summary(&[&dry, &wet]).starts_with("Rain from "));
    }
}
