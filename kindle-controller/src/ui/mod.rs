//! Screens and touch handling.
//!
//! `App` decides what's on screen: a waking screen while Wi-Fi and HA connect, a
//! full-screen error while HA is unreachable, or the dashboard (weather, rooms and the
//! aircon panel). A ⋮ button in the top-right corner opens a dropdown menu for the
//! backlight, sleep, exiting and rebooting. While asleep, only the sleep screen is shown and
//! touches are ignored.
//!
//! On starting ("Starting up") and after waking ("Waking up"), the waking screen stays up
//! until HA is connected (or `WAKING_TIMEOUT` passes), instead of the connection error that
//! the first attempts would otherwise show while Wi-Fi connects.
//!
//! Taps change what's shown straight away ("optimistic" state) and send a service call; the
//! optimistic state is dropped once HA reports the real one, or after a timeout.

mod dashboard;
pub mod draw;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use log::info;
use serde_json::json;

use crate::clock;
use crate::config::{AirconConfig, DashboardConfig};
use crate::display::canvas::{BLACK, Canvas, Rect, WHITE};
use crate::display::font::{Fonts, Weight};
use crate::display::icons::{self, Icon};
use crate::display::{Display, Update};
use crate::ha::entities::{self, EntityMap, Room};
use crate::ha::{ForecastHour, HaEvent, History, ServiceCall};
use crate::input::TouchEvent;
use draw::{DARK_GRAY, centred_baseline};

/// Something the UI wants done outside itself.
#[derive(Debug, PartialEq)]
pub enum UiAction {
    Call(ServiceCall),
    SetBacklight(bool),
    Sleep,
    Exit,
    Reboot,
}

pub const W: i32 = 1448;
pub const H: i32 = 1072;
const MARGIN: i32 = 32;
/// The right-hand column with the menu button, time and battery.
const RAIL_X: i32 = W - MARGIN - 104;
const RAIL_W: i32 = 104;
const MENU_BUTTON: Rect = Rect::new(RAIL_X + 8, MARGIN, 88, 88);
const MENU_W: i32 = 480;
const MENU_ITEM_H: i32 = 110;
const NOTICE_H: i32 = 90;
/// Give up on optimistic state if HA never confirms it.
const PENDING_TIMEOUT: Duration = Duration::from_secs(8);
const NOTICE_TIMEOUT: Duration = Duration::from_secs(10);
const MENU_TIMEOUT: Duration = Duration::from_secs(30);
/// On starting or after waking, how long to show the waking screen before admitting HA
/// can't be reached. Wi-Fi alone takes about 11 s to come back after Sleep on the PW4, and
/// about 20 s to connect at boot.
const WAKING_TIMEOUT: Duration = Duration::from_secs(60);
/// How much room-temperature history the sparklines show.
pub const HISTORY_WINDOW: Duration = Duration::from_secs(3600);

enum Screen {
    Disconnected { reason: String, attempt: u32, retry_at: Instant },
    /// Just started, or just woken from sleep: waiting for Wi-Fi, then HA. Connection
    /// failures meanwhile are kept in `failure`, to show if it gives up.
    Waking { kind: Waking, since: Instant, wifi_up: bool, failure: Option<(String, u32, Instant)> },
    Dashboard,
}

/// Why we're waiting for Wi-Fi and HA: it changes the wording on the waking screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waking {
    /// The controller has just started, e.g. at boot, when Wi-Fi is still connecting.
    Startup,
    /// Woken from Sleep, which turned Wi-Fi off.
    FromSleep,
}

impl Waking {
    fn title(self) -> &'static str {
        match self {
            Waking::Startup => "Starting up",
            Waking::FromSleep => "Waking up",
        }
    }

    fn waiting_for_wifi(self) -> &'static str {
        match self {
            Waking::Startup => "Waiting for Wi-Fi",
            Waking::FromSleep => "Turning Wi-Fi on",
        }
    }

    fn gave_up(self) -> &'static str {
        match self {
            Waking::Startup => "Still waiting for Wi-Fi",
            Waking::FromSleep => "Still waiting for Wi-Fi after waking up",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItem {
    Backlight,
    Sleep,
    Exit,
    Reboot,
}

const MENU_ITEMS: [MenuItem; 4] = [MenuItem::Backlight, MenuItem::Sleep, MenuItem::Exit, MenuItem::Reboot];

struct Menu {
    opened: Instant,
    /// Reboot needs a second tap to confirm.
    reboot_armed: bool,
}

/// What a tap on a hit region does.
#[derive(Clone, Debug, PartialEq)]
enum Tap {
    /// Set `entity` to `desired` with this service call, showing `desired` meanwhile.
    Set { entity: String, desired: String, call: ServiceCall },
}

struct Hit {
    rect: Rect,
    tap: Tap,
}

/// A state we've asked HA for but not yet seen.
struct Optimistic {
    desired: String,
    since: Instant,
}

pub struct App {
    dash: DashboardConfig,
    aircon: AirconConfig,
    fonts: Fonts,
    canvas: Canvas,
    screen: Screen,
    entities: Arc<EntityMap>,
    forecast: Vec<ForecastHour>,
    /// The aircon's rooms, from its Rooms entity. Kept while that's unavailable.
    rooms: Vec<Room>,
    /// Room sensor readings over the last hour.
    history: History,
    /// The lockout sensor's last value and when we received it, to count down locally.
    lockout: Option<(f64, f64)>,
    optimistic: HashMap<String, Optimistic>,
    hits: Vec<Hit>,
    notice: Option<(String, Instant)>,
    menu: Option<Menu>,
    backlight_on: Option<bool>,
    battery: (Option<u8>, bool),
    /// Things on screen that change with time, to know when to redraw.
    shown_clock: String,
    shown_countdown: String,
    /// The time the screen is drawn for: the start of the minute on the clock. Anything
    /// drawn against time (sparklines, the forecast) uses this, so redraws within a minute
    /// produce identical pixels and the e-ink isn't refreshed for nothing.
    frame_now: f64,
    pending_update: Option<Update>,
    /// Showing the sleep screen; see `set_asleep`.
    asleep: bool,
}

impl App {
    pub fn new(dash: DashboardConfig, aircon: AirconConfig, fonts: Fonts, size: (i32, i32)) -> Self {
        Self {
            dash,
            aircon,
            fonts,
            canvas: Canvas::new(size.0, size.1),
            screen: Screen::Waking { kind: Waking::Startup, since: Instant::now(), wifi_up: false, failure: None },
            entities: Arc::new(EntityMap::new()),
            forecast: Vec::new(),
            rooms: Vec::new(),
            history: History::new(),
            lockout: None,
            optimistic: HashMap::new(),
            hits: Vec::new(),
            notice: None,
            menu: None,
            backlight_on: None,
            battery: (None, false),
            shown_clock: String::new(),
            shown_countdown: String::new(),
            frame_now: 0.0,
            pending_update: Some(Update::Full),
            asleep: false,
        }
    }

    /// Requests a redraw. A full redraw beats a normal one, which beats a fast one (fast
    /// updates are black/white only, so they can't show normal content).
    pub fn invalidate(&mut self, update: Update) {
        let rank = |u: Update| match u {
            Update::Fast => 0,
            Update::Normal => 1,
            Update::Full => 2,
        };
        self.pending_update = Some(match self.pending_update {
            Some(cur) if rank(cur) >= rank(update) => cur,
            _ => update,
        });
    }

    /// Entities to keep up to date. The HA client adds the room sensors once it knows them.
    pub fn entities_wanted(&self) -> Vec<String> {
        let mut ids = self.aircon.ids().all();
        ids.extend(self.dash.weather.iter().cloned());
        ids.sort();
        ids.dedup();
        ids
    }

    fn room_sensors(&self) -> Vec<String> {
        self.rooms.iter().map(|r| r.sensor.clone()).collect()
    }

    /// `id`, if HA has that entity: controls for entities it doesn't have aren't shown.
    fn in_ha(&self, id: &Option<String>) -> Option<String> {
        id.as_ref().filter(|id| self.entities.contains_key(*id)).cloned()
    }

    /// Why the thermostat has switched the unit off, e.g. "at set point".
    fn idle_reason(&self) -> Option<String> {
        let id = self.aircon.thermostat_status.as_ref()?;
        let status = &self.entities.get(id)?.state;
        status.strip_prefix("Idle: ").map(str::to_string)
    }

    pub fn set_backlight_state(&mut self, on: Option<bool>) {
        if self.backlight_on != on {
            self.backlight_on = on;
            if self.menu.is_some() {
                self.invalidate(Update::Normal);
            }
        }
    }

    /// Switches to or from the sleep screen. Both directions flash the whole screen. While
    /// asleep, touches are ignored and nothing else is redrawn; HA updates are still taken
    /// in, so the dashboard is current on waking.
    pub fn set_asleep(&mut self, asleep: bool) {
        if self.asleep != asleep {
            self.asleep = asleep;
            self.menu = None;
            self.notice = None;
            self.optimistic.clear();
            self.pending_update = Some(Update::Full);
        }
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// Leaves sleep for the "Waking up" screen, which gives way to the dashboard once HA
    /// connects.
    pub fn wake(&mut self) {
        self.set_asleep(false);
        self.screen = Screen::Waking { kind: Waking::FromSleep, since: Instant::now(), wifi_up: false, failure: None };
    }

    /// The Kindle reports a working network connection (again), or Wi-Fi was already
    /// connected when we started.
    pub fn set_network_up(&mut self) {
        if let Screen::Waking { wifi_up, .. } = &mut self.screen
            && !*wifi_up
        {
            *wifi_up = true;
            self.invalidate(Update::Normal);
        }
    }

    pub fn set_battery(&mut self, percent: Option<u8>, charging: bool) {
        if self.battery != (percent, charging) {
            self.battery = (percent, charging);
            self.invalidate(Update::Normal);
        }
    }

    /// The state to show: what we've asked for if HA hasn't caught up yet.
    fn effective(&self, entity: &str) -> Option<&str> {
        if let Some(o) = self.optimistic.get(entity) {
            return Some(&o.desired);
        }
        self.entities.get(entity).map(|s| s.state.as_str())
    }

    pub fn on_ha_event(&mut self, event: HaEvent) {
        match event {
            HaEvent::Connected => {
                info!("connected; showing the dashboard");
                self.screen = Screen::Dashboard;
                self.invalidate(Update::Full);
            }
            HaEvent::Disconnected { reason, attempt, retry_in } if matches!(self.screen, Screen::Waking { .. }) => {
                // Expected while Wi-Fi connects: keep showing the waking screen for now.
                if let Screen::Waking { failure, .. } = &mut self.screen {
                    *failure = Some((reason, attempt, Instant::now() + retry_in));
                }
                self.optimistic.clear();
            }
            HaEvent::Disconnected { reason, attempt, retry_in } => {
                let changed_screen = !matches!(self.screen, Screen::Disconnected { .. });
                self.screen = Screen::Disconnected { reason, attempt, retry_at: Instant::now() + retry_in };
                self.optimistic.clear();
                self.invalidate(if changed_screen { Update::Full } else { Update::Normal });
            }
            HaEvent::Aircon(ids) => {
                let old = self.aircon.ids();
                if old != ids {
                    info!("using aircon entities {ids:?}");
                    for id in old.all() {
                        self.optimistic.remove(&id);
                    }
                    if old.compressor_lockout != ids.compressor_lockout {
                        self.lockout = None;
                    }
                    self.aircon.set_ids(ids);
                    self.invalidate(Update::Normal);
                }
            }
            HaEvent::Entities(entities) => {
                let now = clock::now();
                if let Some(rooms) = entities::rooms(&entities, self.aircon.rooms.as_deref()) {
                    self.rooms = rooms;
                }
                for sensor in self.room_sensors() {
                    let new = entities.get(&sensor).and_then(|s| s.number());
                    let old = self.entities.get(&sensor).and_then(|s| s.number());
                    if let Some(v) = new.filter(|v| Some(*v) != old) {
                        self.history.entry(sensor).or_default().push((now, v));
                    }
                }
                if let Some(id) = &self.aircon.compressor_lockout {
                    let new = entities.get(id).map(|s| (s.state.clone(), s.last_changed));
                    let old = self.entities.get(id).map(|s| (s.state.clone(), s.last_changed));
                    if new != old {
                        self.lockout = entities.get(id).and_then(|s| s.number()).map(|v| (v, now));
                    }
                }
                self.entities = entities;
                let ents = &self.entities;
                self.optimistic.retain(|id, o| match ents.get(id) {
                    Some(s) => !same_state(&s.state, &o.desired),
                    None => true,
                });
                self.trim_history();
                if matches!(self.screen, Screen::Dashboard) {
                    self.invalidate(Update::Normal);
                }
            }
            HaEvent::ServiceCallFailed { call, error } => {
                self.optimistic.remove(&call.entity_id);
                let name = self
                    .entities
                    .get(&call.entity_id)
                    .map(|s| s.friendly_name(&call.entity_id).to_string())
                    .unwrap_or(call.entity_id);
                self.notice = Some((format!("{name}: {error}"), Instant::now()));
                self.invalidate(Update::Normal);
            }
            HaEvent::Forecast(hours) => {
                self.forecast = hours;
                self.invalidate(Update::Normal);
            }
            HaEvent::History(history) => {
                for (id, mut points) in history {
                    // Keep anything newer that arrived live while the request was running.
                    let latest = points.last().map_or(f64::MIN, |p| p.0);
                    if let Some(live) = self.history.get(&id) {
                        points.extend(live.iter().filter(|p| p.0 > latest));
                    }
                    self.history.insert(id, points);
                }
                self.trim_history();
                self.invalidate(Update::Normal);
            }
        }
    }

    /// Drops readings older than the window, keeping one just before it so the line starts
    /// at the left edge.
    fn trim_history(&mut self) {
        let start = clock::now() - HISTORY_WINDOW.as_secs_f64();
        for points in self.history.values_mut() {
            let first_inside = points.iter().position(|p| p.0 >= start).unwrap_or(points.len());
            if first_inside > 1 {
                points.drain(..first_inside - 1);
            }
        }
    }

    fn menu_item_rect(index: usize) -> Rect {
        let b = MENU_BUTTON;
        Rect::new(b.right() - MENU_W, b.bottom() + 8 + index as i32 * MENU_ITEM_H, MENU_W, MENU_ITEM_H)
    }

    /// The button's touch target reaches the screen corner.
    fn menu_button_hit(x: i32, y: i32) -> bool {
        let b = MENU_BUTTON;
        Rect::new(b.x - 12, 0, W - b.x + 12, b.bottom() + 12).contains(x, y)
    }

    fn menu_label(&self, item: MenuItem) -> &'static str {
        let reboot_armed = self.menu.as_ref().is_some_and(|m| m.reboot_armed);
        match item {
            MenuItem::Backlight => match self.backlight_on {
                Some(true) => "Turn backlight off",
                _ => "Turn backlight on",
            },
            MenuItem::Sleep => "Sleep",
            MenuItem::Exit => "Exit controller",
            MenuItem::Reboot if reboot_armed => "Tap again to reboot",
            MenuItem::Reboot => "Reboot",
        }
    }

    pub fn on_touch(&mut self, event: TouchEvent) -> Option<UiAction> {
        let TouchEvent::Down { x, y } = event else { return None };
        if self.asleep {
            return None;
        }

        if let Some(menu) = &self.menu {
            // Any tap while the menu is open goes to the menu: a tap outside just closes it.
            let item = (0..MENU_ITEMS.len()).find(|&i| Self::menu_item_rect(i).contains(x, y)).map(|i| MENU_ITEMS[i]);
            let reboot_armed = menu.reboot_armed;
            self.invalidate(Update::Normal);
            if item == Some(MenuItem::Reboot) && !reboot_armed {
                self.menu = Some(Menu { opened: Instant::now(), reboot_armed: true });
                return None;
            }
            self.menu = None;
            return match item? {
                MenuItem::Backlight => Some(UiAction::SetBacklight(self.backlight_on != Some(true))),
                MenuItem::Sleep => Some(UiAction::Sleep),
                MenuItem::Exit => Some(UiAction::Exit),
                MenuItem::Reboot => Some(UiAction::Reboot),
            };
        }

        if Self::menu_button_hit(x, y) {
            self.menu = Some(Menu { opened: Instant::now(), reboot_armed: false });
            self.invalidate(Update::Normal);
            return None;
        }

        if !matches!(self.screen, Screen::Dashboard) {
            return None;
        }
        let tap = self.hits.iter().find(|h| h.rect.contains(x, y))?.tap.clone();
        match tap {
            Tap::Set { entity, desired, call } => {
                if self.effective(&entity).is_some_and(|s| same_state(s, &desired)) {
                    return None;
                }
                self.optimistic.insert(entity, Optimistic { desired, since: Instant::now() });
                self.invalidate(Update::Fast);
                Some(UiAction::Call(call))
            }
        }
    }

    /// Checks timeouts and time-dependent text. Call it whenever the main loop wakes.
    pub fn tick(&mut self) {
        if self.asleep {
            return;
        }
        if let Screen::Waking { kind, since, failure, .. } = &mut self.screen
            && since.elapsed() >= WAKING_TIMEOUT
        {
            // Give up on the friendly screen and show what's wrong.
            let (reason, attempt, retry_at) =
                failure.take().unwrap_or_else(|| (kind.gave_up().into(), 0, Instant::now()));
            self.screen = Screen::Disconnected { reason, attempt, retry_at };
            self.invalidate(Update::Full);
        }
        let before = self.optimistic.len();
        self.optimistic.retain(|_, o| o.since.elapsed() < PENDING_TIMEOUT);
        if self.optimistic.len() != before {
            self.invalidate(Update::Normal);
        }
        if self.notice.as_ref().is_some_and(|(_, t)| t.elapsed() >= NOTICE_TIMEOUT) {
            self.notice = None;
            self.invalidate(Update::Normal);
        }
        if self.menu.as_ref().is_some_and(|m| m.opened.elapsed() >= MENU_TIMEOUT) {
            self.menu = None;
            self.invalidate(Update::Normal);
        }
        if self.clock_text() != self.shown_clock || self.countdown_text() != self.shown_countdown {
            self.trim_history();
            self.invalidate(Update::Normal);
        }
    }

    /// How long until `tick` could have something to do, so the main loop can sleep until
    /// then: the next minute for the clock, the next 10 s step of the lockout countdown (only
    /// while locked out), the next 5 s step of the retry countdown (only while disconnected),
    /// and any pending timeouts.
    pub fn next_wakeup(&self) -> Duration {
        if self.asleep {
            // Nothing on the sleep screen changes; wake-up comes from the power button.
            return Duration::from_secs(3600);
        }
        let mut secs = 60.0 - clock::now().rem_euclid(60.0);
        let since = |at: Instant, after: Duration| after.saturating_sub(at.elapsed()).as_secs_f64();
        if let Some(left) = self.lockout_left().filter(|_| self.lockout_shown().is_some()) {
            secs = secs.min(until_step(left, 10.0));
        }
        if let Screen::Disconnected { retry_at, .. } = &self.screen {
            secs = secs.min(until_step(retry_at.saturating_duration_since(Instant::now()).as_secs_f64(), 5.0));
        }
        if let Screen::Waking { since: started, .. } = &self.screen {
            secs = secs.min(since(*started, WAKING_TIMEOUT));
        }
        for o in self.optimistic.values() {
            secs = secs.min(since(o.since, PENDING_TIMEOUT));
        }
        if let Some((_, t)) = &self.notice {
            secs = secs.min(since(*t, NOTICE_TIMEOUT));
        }
        if let Some(m) = &self.menu {
            secs = secs.min(since(m.opened, MENU_TIMEOUT));
        }
        // A little slack, to wake just after a boundary rather than just before it.
        Duration::from_secs_f64(secs.max(0.0) + 0.05)
    }

    fn clock_text(&self) -> String {
        clock::local_hhmm(clock::now())
    }

    /// What the time-dependent bits of the current screen would show: the compressor
    /// lockout (in 10 s steps) or the disconnected screen's retry countdown.
    fn countdown_text(&self) -> String {
        match &self.screen {
            Screen::Disconnected { retry_at, .. } => format!("{}", retry_secs(*retry_at)),
            _ => self.lockout_shown().map(|s| s.to_string()).unwrap_or_default(),
        }
    }

    /// Seconds of compressor lockout left, if any.
    fn lockout_left(&self) -> Option<f64> {
        let (value, at) = self.lockout?;
        let left = value - (clock::now() - at);
        (left > 0.05).then_some(left)
    }

    /// The lockout time to display: rounded up to 10 s, so it changes every 10 s. Not shown
    /// while the thermostat has the unit idle: the idle chip takes its place.
    fn lockout_shown(&self) -> Option<u64> {
        if self.idle_reason().is_some() {
            return None;
        }
        self.lockout_left().map(|left| ((left / 10.0).ceil() * 10.0) as u64)
    }

    /// Draws and shows the screen if anything changed.
    pub fn present(&mut self, display: &mut Display) -> Result<()> {
        let Some(update) = self.pending_update.take() else { return Ok(()) };
        if self.asleep {
            // Redrawn only on entering sleep: anything else would just repeat it.
            if update != Update::Full {
                return Ok(());
            }
            self.render_sleep();
            return display.present(&self.canvas, update);
        }
        self.render();
        display.present(&self.canvas, update)
    }

    fn render(&mut self) {
        self.canvas.fill(WHITE);
        self.hits.clear();
        let now = clock::now();
        self.frame_now = (now / 60.0).floor() * 60.0;
        self.shown_clock = clock::local_hhmm(now);
        self.shown_countdown = self.countdown_text();
        match &self.screen {
            Screen::Disconnected { reason, attempt, retry_at } => {
                let reason = reason.clone();
                let secs = retry_secs(*retry_at);
                let retry = if secs > 0 {
                    format!("Trying again in {secs} s · attempt {attempt}")
                } else {
                    format!("Trying again now · attempt {}", attempt + 1)
                };
                self.draw_message("Can't reach Home Assistant", Some(&reason), Some(&retry));
            }
            Screen::Waking { kind, wifi_up, .. } => {
                let kind = *kind;
                let detail = if *wifi_up { "Connecting to Home Assistant" } else { kind.waiting_for_wifi() };
                self.draw_house_screen(icons::draw_waking_house, kind.title(), detail);
            }
            Screen::Dashboard => self.render_dashboard(),
        }
        if let Some((text, _)) = &self.notice {
            let bar = Rect::new(0, H - NOTICE_H, W, NOTICE_H);
            self.canvas.fill_rect(bar, BLACK);
            let text = self.fonts.ellipsize(text, 32, Weight::Regular, W - 2 * MARGIN);
            let baseline = centred_baseline(&self.fonts, bar, 32, Weight::Regular);
            self.fonts.draw(&mut self.canvas, &text, 32, Weight::Regular, MARGIN, baseline, WHITE);
        }
        self.draw_rail();
        if self.menu.is_some() {
            self.draw_menu();
        }
    }

    /// Menu button, time and battery, top right on every screen.
    fn draw_rail(&mut self) {
        draw::menu_button(&mut self.canvas, MENU_BUTTON, self.menu.is_some(), BLACK, WHITE);
        let cx = RAIL_X + RAIL_W / 2;
        let (asc, _) = self.fonts.line_metrics(36, Weight::Bold);
        let time_top = MENU_BUTTON.bottom() + 14;
        let clock = self.shown_clock.clone();
        self.fonts.draw_centered(&mut self.canvas, &clock, 36, Weight::Bold, cx, time_top + asc, BLACK);
        let batt_y = time_top + asc + 22;
        let (percent, charging) = self.battery;
        draw::battery(&mut self.canvas, cx - 46, batt_y, percent, charging, BLACK, WHITE);
        if let Some(p) = percent {
            let (a, _) = self.fonts.line_metrics(20, Weight::Regular);
            let text = format!("{p}%");
            self.fonts.draw_centered(&mut self.canvas, &text, 20, Weight::Regular, cx, batt_y + 34 + 6 + a, DARK_GRAY);
        }
    }

    fn draw_menu(&mut self) {
        let first = Self::menu_item_rect(0);
        let all = Rect::new(first.x, first.y, first.w, first.h * MENU_ITEMS.len() as i32);
        self.canvas.fill_rect(Rect::new(all.x - 12, all.y - 12, all.w + 24, all.h + 24), WHITE);
        self.canvas.fill_round_rect(all, 12.0, WHITE);
        self.canvas.stroke_round_rect(all, 12.0, 4.0, BLACK);
        for (i, item) in MENU_ITEMS.iter().enumerate() {
            let r = Self::menu_item_rect(i);
            if i > 0 {
                self.canvas.fill_rect(Rect::new(r.x, r.y - 1, r.w, 2), BLACK);
            }
            let label = self.menu_label(*item);
            let baseline = centred_baseline(&self.fonts, r, 40, Weight::Regular);
            self.fonts.draw(&mut self.canvas, label, 40, Weight::Regular, r.x + 36, baseline, BLACK);
        }
    }

    /// The sleep screen: the sleeping house and how to wake up. No clock or battery, so it
    /// never needs redrawing.
    fn render_sleep(&mut self) {
        self.canvas.fill(WHITE);
        self.hits.clear();
        self.draw_house_screen(icons::draw_sleeping_house, "Sleeping", "Press the power button to wake");
    }

    /// A big house icon with a title and one line under it: the sleep and waking screens.
    fn draw_house_screen(&mut self, icon: fn(&mut Canvas, i32, i32, i32, u8), title: &str, detail: &str) {
        let cx = W / 2;
        let size = 300;
        let mut y = 250;
        icon(&mut self.canvas, cx - size / 2, y, size, BLACK);
        y += size + 36;
        let (a, d) = self.fonts.line_metrics(68, Weight::Bold);
        self.fonts.draw_centered(&mut self.canvas, title, 68, Weight::Bold, cx, y + a, BLACK);
        y += a + d + 24;
        let (a, _) = self.fonts.line_metrics(32, Weight::Regular);
        self.fonts.draw_centered(&mut self.canvas, detail, 32, Weight::Regular, cx, y + a, DARK_GRAY);
    }

    /// Shows a plain full-screen message straight away, e.g. while the Kindle UI restarts
    /// after we exit (that takes about a minute in "stop" mode).
    pub fn show_farewell(&mut self, display: &mut Display, title: &str, detail: &str) -> Result<()> {
        self.canvas.fill(WHITE);
        let cx = W / 2;
        let (a, d) = self.fonts.line_metrics(60, Weight::Bold);
        let top = H / 2 - (a + d);
        self.fonts.draw_centered(&mut self.canvas, title, 60, Weight::Bold, cx, top + a, BLACK);
        let (a2, _) = self.fonts.line_metrics(32, Weight::Regular);
        self.fonts.draw_centered(&mut self.canvas, detail, 32, Weight::Regular, cx, top + a + d + 24 + a2, DARK_GRAY);
        display.present(&self.canvas, Update::Full)
    }

    /// Full-screen message, used for "connecting" and the disconnected error.
    fn draw_message(&mut self, title: &str, detail: Option<&str>, pill: Option<&str>) {
        let cx = W / 2;
        let icon = 240;
        let mut y = if detail.is_some() { 230 } else { 300 };
        icons::draw_house_slash(&mut self.canvas, cx - icon / 2, y, icon, BLACK, WHITE);
        y += icon + 36;
        let inner = W - 2 * (RAIL_W + 2 * MARGIN);
        for line in self.fonts.wrap(title, 68, Weight::Bold, inner) {
            let (a, d) = self.fonts.line_metrics(68, Weight::Bold);
            self.fonts.draw_centered(&mut self.canvas, &line, 68, Weight::Bold, cx, y + a, BLACK);
            y += a + d + 8;
        }
        if let Some(detail) = detail {
            y += 16;
            for line in self.fonts.wrap(detail, 32, Weight::Regular, inner).into_iter().take(3) {
                let (a, d) = self.fonts.line_metrics(32, Weight::Regular);
                self.fonts.draw_centered(&mut self.canvas, &line, 32, Weight::Regular, cx, y + a, DARK_GRAY);
                y += a + d + 6;
            }
        }
        if let Some(pill) = pill {
            y += 40;
            let h = 72;
            let parts = [(pill, Weight::Regular)];
            // Measure first, then draw centred.
            let width = {
                let mut scratch = Canvas::new(1, 1);
                draw::chip(&mut scratch, &mut self.fonts, 0, 0, h, Icon::Retry, &parts, 30).w
            };
            draw::chip(&mut self.canvas, &mut self.fonts, cx - width / 2, y, h, Icon::Retry, &parts, 30);
        }
    }
}

/// Seconds until the next reconnect attempt, rounded up to 5 so the screen only redraws
/// every 5 seconds.
fn retry_secs(at: Instant) -> u64 {
    ((at.saturating_duration_since(Instant::now()).as_secs_f64() / 5.0).ceil() * 5.0) as u64
}

/// Seconds until a value counting down from `left`, shown rounded up to multiples of
/// `step`, next changes.
fn until_step(left: f64, step: f64) -> f64 {
    left - ((left / step).ceil() - 1.0) * step
}

/// Compares HA states, treating numbers numerically ("21" == "21.0").
fn same_state(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-6,
        _ => a == b,
    }
}

/// The service call that sets `entity` to `value`, chosen by its domain.
fn set_call(entity: &str, value: &str) -> ServiceCall {
    let domain = entity.split('.').next().unwrap_or_default();
    match domain {
        "select" | "input_select" => {
            let mut c = ServiceCall::new(domain, "select_option", entity);
            c.data = Some(json!({"option": value}));
            c
        }
        "number" | "input_number" => {
            let mut c = ServiceCall::new(domain, "set_value", entity);
            c.data = Some(json!({"value": value.parse::<f64>().unwrap_or_default()}));
            c
        }
        "switch" | "input_boolean" | "light" | "fan" => {
            ServiceCall::new(domain, if value == "on" { "turn_on" } else { "turn_off" }, entity)
        }
        _ => ServiceCall::new("homeassistant", if value == "on" { "turn_on" } else { "turn_off" }, entity),
    }
}

#[cfg(test)]
mod tests;
