use serde_json::{Value, json};

use super::*;
use crate::ha::entities::apply_compressed;

fn aircon() -> AirconConfig {
    AirconConfig {
        compressor_lockout: Some("sensor.lockout".into()),
        thermostat: Some("input_boolean.thermostat".into()),
        setpoint: Some("input_number.setpoint".into()),
        ..AirconConfig::default()
    }
}

fn dash() -> DashboardConfig {
    DashboardConfig { weather: Some("weather.home".into()), old_rooms: None }
}

const ROOM_SELECT: &str = "select.aircon_thermostat_room";

fn room_select(option: &str) -> Value {
    json!({"s": option, "a": {"options": ["Average", "Lounge", "Study"]}})
}

fn states(extra: Value) -> Arc<EntityMap> {
    let mut m = EntityMap::new();
    let mut all = json!({
        "switch.aircon_power": {"s": "on", "lc": 1.0},
        "select.aircon_mode": {"s": "Cool", "a": {"options": ["Heat", "Cool", "Fan only"]}},
        "select.aircon_fan_speed": {"s": "Low", "a": {"options": ["Low", "Medium", "High"]}},
        "sensor.lockout": {"s": "0"},
        "input_boolean.thermostat": {"s": "on"},
        "input_number.setpoint": {"s": "21.0", "a": {"min": 16, "max": 30}},
        "weather.home": {"s": "rainy", "a": {"temperature": 9.4}},
        "sensor.lounge": {"s": "19.2"},
        "sensor.study": {"s": "20.1"},
        "sensor.aircon_rooms": {"s": "2", "a": {"rooms": [
            {"name": "Lounge", "sensor": "sensor.lounge"},
            {"name": "Study", "sensor": "sensor.study"}
        ]}},
        "select.aircon_thermostat_room": room_select("Average"),
        "sensor.aircon_thermostat_status": {"s": "Running"},
        "sensor.aircon_thermostat_temperature": {"s": "19.7"},
        "sensor.aircon_operation": {"s": "Cooling"},
    });
    for (k, v) in extra.as_object().unwrap() {
        all[k] = v.clone();
    }
    apply_compressed(&mut m, &json!({"a": all}));
    Arc::new(m)
}

fn app() -> App {
    let mut app = App::new(dash(), aircon(), Fonts::bitmap(), (W, H));
    app.on_ha_event(HaEvent::Entities(states(json!({}))));
    app.on_ha_event(HaEvent::Connected);
    app.render();
    app.pending_update = None;
    app
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

fn tap(app: &mut App, (x, y): (i32, i32)) -> Option<UiAction> {
    app.on_touch(TouchEvent::Down { x, y })
}

/// The hit region whose call sets `entity` to `value`.
fn hit_for(app: &App, entity: &str, value: &str) -> Rect {
    app.hits
        .iter()
        .find(|h| matches!(&h.tap, Tap::Set { entity: e, desired, .. } if e == entity && desired == value))
        .unwrap_or_else(|| panic!("no hit for {entity} = {value}"))
        .rect
}

#[test]
fn subscribes_to_every_configured_entity() {
    let app = app();
    let wanted = app.entities_wanted();
    for id in ["switch.aircon_power", "select.aircon_mode", "sensor.lockout", "input_number.setpoint", "weather.home", ROOM_SELECT, "sensor.aircon_rooms"] {
        assert!(wanted.iter().any(|w| w == id), "{id}");
    }
}

#[test]
fn rooms_come_from_the_aircon() {
    let mut app = app();
    assert_eq!(app.room_sensors(), ["sensor.lounge", "sensor.study"]);
    // Kept while the Rooms entity is unavailable (HA drops its attributes then).
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.aircon_rooms": {"s": "unavailable"}}))));
    assert_eq!(app.room_sensors().len(), 2);
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.aircon_rooms": {"s": "0", "a": {"rooms": []}}}))));
    assert!(app.room_sensors().is_empty());
    app.render();
    assert!(app.hits.iter().all(|h| !matches!(&h.tap, Tap::Set { entity, .. } if entity == ROOM_SELECT)));
}

#[test]
fn tapping_a_room_selects_it_and_tapping_again_goes_back_to_average() {
    let mut app = app();
    let lounge = hit_for(&app, ROOM_SELECT, "Lounge");
    let Some(UiAction::Call(call)) = tap(&mut app, centre(lounge)) else { panic!() };
    assert_eq!(call.data, Some(json!({"option": "Lounge"})));
    app.on_ha_event(HaEvent::Entities(states(json!({ROOM_SELECT: room_select("Lounge")}))));
    app.render();
    assert_eq!(app.selected_room().as_deref(), Some("Lounge"));
    // The selected room has the thick border.
    assert_eq!(app.canvas.pixel(lounge.x + 2, lounge.y + lounge.h / 2), BLACK);
    let again = hit_for(&app, ROOM_SELECT, "Average");
    assert_eq!(again, lounge);
    hit_for(&app, ROOM_SELECT, "Study");
}

#[test]
fn rooms_cant_be_picked_in_continuous_mode() {
    let mut app = app();
    app.on_ha_event(HaEvent::Entities(states(json!({"input_boolean.thermostat": {"s": "off"}, ROOM_SELECT: room_select("Lounge")}))));
    app.render();
    assert!(app.hits.iter().all(|h| !matches!(&h.tap, Tap::Set { entity, .. } if entity == ROOM_SELECT)));
}

#[test]
fn thermostat_controls_hide_when_ha_lacks_them() {
    let mut app = App::new(dash(), AirconConfig::default(), Fonts::bitmap(), (W, H));
    app.on_ha_event(HaEvent::Entities(states(json!({}))));
    app.on_ha_event(HaEvent::Connected);
    app.render();
    // The default switch.aircon_thermostat / number.aircon_set_point aren't in `states`.
    assert!(app.hits.iter().all(|h| !matches!(&h.tap, Tap::Set { entity, .. } if entity.contains("thermostat") && entity != ROOM_SELECT)));
    hit_for(&app, "switch.aircon_power", "off");
}

#[test]
fn idle_chip_replaces_the_lockout_countdown() {
    let mut app = app();
    app.on_ha_event(HaEvent::Entities(states(json!({
        "sensor.lockout": {"s": "161", "lc": 2.0},
        "sensor.aircon_thermostat_status": {"s": "Idle: at set point"},
    }))));
    assert_eq!(app.idle_reason().as_deref(), Some("at set point"));
    assert_eq!(app.lockout_shown(), None);
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.aircon_thermostat_status": {"s": "Running"}}))));
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lockout": {"s": "150", "lc": 3.0}}))));
    assert_eq!(app.lockout_shown(), Some(150));
}

#[test]
fn tapping_a_mode_selects_it_optimistically() {
    let mut app = app();
    let heat = hit_for(&app, "select.aircon_mode", "Heat");
    let action = tap(&mut app, centre(heat));
    let Some(UiAction::Call(call)) = action else { panic!("expected a call, got {action:?}") };
    assert_eq!((call.domain.as_str(), call.service.as_str()), ("select", "select_option"));
    assert_eq!(call.data, Some(json!({"option": "Heat"})));
    assert_eq!(app.effective("select.aircon_mode"), Some("Heat"));
    assert_eq!(app.pending_update, Some(Update::Fast));

    // HA confirms: the optimistic state goes away.
    app.on_ha_event(HaEvent::Entities(states(json!({"select.aircon_mode": {"s": "Heat", "a": {"options": ["Heat", "Cool", "Fan only"]}}}))));
    assert!(app.optimistic.is_empty());
}

#[test]
fn auto_is_disabled_when_the_select_has_no_auto_option() {
    let app = app();
    assert!(app.hits.iter().all(|h| !matches!(&h.tap, Tap::Set { desired, .. } if desired == "Auto")));
}

fn with_auto(extra: Value) -> App {
    let mut all = json!({"select.aircon_mode": {"s": "Auto", "a": {"options": ["Heat", "Cool", "Fan only", "Auto"]}}});
    for (k, v) in extra.as_object().unwrap() {
        all[k] = v.clone();
    }
    let mut app = app();
    app.on_ha_event(HaEvent::Entities(states(all)));
    app.render();
    app
}

fn has_hit(app: &App, entity: &str, value: &str) -> bool {
    app.hits.iter().any(|h| matches!(&h.tap, Tap::Set { entity: e, desired, .. } if e == entity && desired == value))
}

#[test]
fn auto_and_continuous_exclude_each_other() {
    // In Auto, Continuous can't be picked, but Thermostat and the other modes can.
    let app = with_auto(json!({}));
    assert!(!has_hit(&app, "input_boolean.thermostat", "off"));
    assert!(has_hit(&app, "input_boolean.thermostat", "on"));
    assert!(has_hit(&app, "select.aircon_mode", "Heat"));

    // In Continuous, Auto can't be picked.
    let app = with_auto(json!({
        "select.aircon_mode": {"s": "Cool", "a": {"options": ["Heat", "Cool", "Fan only", "Auto"]}},
        "input_boolean.thermostat": {"s": "off"},
    }));
    assert!(!has_hit(&app, "select.aircon_mode", "Auto"));
    assert!(has_hit(&app, "input_boolean.thermostat", "off"));
    assert!(has_hit(&app, "select.aircon_mode", "Heat"));

    // With the thermostat on, both can.
    let app = with_auto(json!({"select.aircon_mode": {"s": "Cool", "a": {"options": ["Heat", "Cool", "Fan only", "Auto"]}}}));
    assert!(has_hit(&app, "select.aircon_mode", "Auto"));
    assert!(has_hit(&app, "input_boolean.thermostat", "off"));
}

#[test]
fn auto_direction_is_read_from_its_sensor() {
    let app = with_auto(json!({"sensor.aircon_auto_direction": {"s": "Heat"}}));
    assert_eq!(app.auto_direction(), Some("Heat"));
    let app = with_auto(json!({"sensor.aircon_auto_direction": {"s": "unknown"}}));
    assert_eq!(app.auto_direction(), None);
}

#[test]
fn setpoint_steps_and_power_toggles() {
    let mut app = app();
    let plus = hit_for(&app, "input_number.setpoint", "21.5");
    let Some(UiAction::Call(call)) = tap(&mut app, centre(plus)) else { panic!() };
    assert_eq!(call.data, Some(json!({"value": 21.5})));
    app.render();
    // The next tap builds on the optimistic value.
    let plus = hit_for(&app, "input_number.setpoint", "22");
    assert!(tap(&mut app, centre(plus)).is_some());

    let power = hit_for(&app, "switch.aircon_power", "off");
    let Some(UiAction::Call(call)) = tap(&mut app, centre(power)) else { panic!() };
    assert_eq!((call.domain.as_str(), call.service.as_str()), ("switch", "turn_off"));
}

#[test]
fn continuous_mode_hides_the_setpoint() {
    let mut app = app();
    app.on_ha_event(HaEvent::Entities(states(json!({"input_boolean.thermostat": {"s": "off"}}))));
    app.render();
    assert!(app.hits.iter().all(|h| !matches!(&h.tap, Tap::Set { entity, .. } if entity == "input_number.setpoint")));
    hit_for(&app, "input_boolean.thermostat", "on");
}

#[test]
fn unavailable_aircon_shows_no_controls() {
    let mut app = app();
    app.on_ha_event(HaEvent::Entities(states(json!({"switch.aircon_power": {"s": "unavailable", "lc": 1.0}}))));
    app.render();
    assert!(app.hits.is_empty());
}

#[test]
fn lockout_counts_down_from_the_sensor_value() {
    let mut app = app();
    assert_eq!(app.lockout_shown(), None);
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lockout": {"s": "161", "lc": 2.0}}))));
    // 161 s left shows as 02:50 and changes to 02:40 in about a second.
    assert_eq!(app.lockout_shown(), Some(170));
    assert!(app.next_wakeup() <= Duration::from_millis(1100));
}

#[test]
fn sleeps_until_the_next_minute_when_nothing_else_is_due() {
    let app = app();
    let wait = app.next_wakeup().as_secs_f64();
    let to_minute = 60.0 - clock::now().rem_euclid(60.0);
    assert!((wait - to_minute).abs() < 0.2, "{wait} vs {to_minute}");
}

#[test]
fn countdown_steps() {
    assert_eq!(until_step(161.0, 10.0), 1.0);
    assert_eq!(until_step(160.0, 10.0), 10.0);
    assert_eq!(until_step(7.5, 5.0), 2.5);
}

#[test]
fn room_history_grows_with_live_updates() {
    let mut app = app();
    app.on_ha_event(HaEvent::History([("sensor.lounge".to_string(), vec![(clock::now() - 600.0, 18.5)])].into()));
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lounge": {"s": "19.6"}}))));
    let points = &app.history["sensor.lounge"];
    assert_eq!(points.first().unwrap().1, 18.5);
    assert_eq!(points.last().unwrap().1, 19.6);
}

#[test]
fn redraws_within_a_minute_are_identical() {
    // Start 5 s into a minute, then redraw 40 s later: the sparklines (real and demo) and
    // everything else must come out the same, or the e-ink refreshes for nothing.
    clock::test_offset::set(0.0);
    clock::test_offset::set(5.0 - clock::now().rem_euclid(60.0));
    let mut app = app();
    app.on_ha_event(HaEvent::History([("sensor.lounge".to_string(), vec![(clock::now() - 1800.0, 18.5)])].into()));
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lockout": {"s": "179"}}))));
    app.render();
    let first = app.canvas.clone();
    clock::test_offset::set(clock::test_offset::get() + 40.0);
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lockout": {"s": "139"}}))));
    app.on_ha_event(HaEvent::Entities(states(json!({"sensor.lockout": {"s": "138"}}))));
    app.render();
    clock::test_offset::set(0.0);
    // 138 s shows as 02:20 rather than 03:00, so only the lockout chip may differ.
    let diff = first.diff_bounds(&app.canvas).expect("the countdown changed");
    assert!(diff.y >= 480 && diff.h <= 60, "redrew more than the lockout chip: {diff:?}");
}

#[test]
fn menu_still_works() {
    let mut app = app();
    assert_eq!(tap(&mut app, centre(MENU_BUTTON)), None);
    assert!(app.menu.is_some());
    let exit = App::menu_item_rect(2);
    assert_eq!(tap(&mut app, centre(exit)), Some(UiAction::Exit));
}

#[test]
fn sleep_ignores_touches_until_woken() {
    let mut app = app();
    let power = hit_for(&app, "switch.aircon_power", "off");
    tap(&mut app, centre(MENU_BUTTON));
    assert_eq!(tap(&mut app, centre(App::menu_item_rect(1))), Some(UiAction::Sleep));
    app.set_asleep(true);
    assert_eq!(app.pending_update, Some(Update::Full));
    app.render_sleep();
    app.pending_update = None;
    assert_eq!(tap(&mut app, centre(power)), None);
    assert_eq!(tap(&mut app, centre(MENU_BUTTON)), None);
    assert!(app.menu.is_none());
    // HA updates while asleep don't ask for a redraw that would change anything.
    app.tick();
    assert!(app.next_wakeup() >= Duration::from_secs(600));
    app.wake();
    assert_eq!(app.pending_update, Some(Update::Full));
    app.on_ha_event(HaEvent::Connected);
    app.render();
    assert!(tap(&mut app, centre(power)).is_some());
}

#[test]
fn waking_screen_hides_reconnect_failures_until_connected() {
    let mut app = app();
    app.set_asleep(true);
    app.wake();
    assert!(matches!(app.screen, Screen::Waking { wifi_up: false, .. }));
    app.on_ha_event(HaEvent::Disconnected { reason: "unreachable".into(), attempt: 1, retry_in: Duration::from_secs(1) });
    assert!(matches!(app.screen, Screen::Waking { failure: Some(_), .. }));
    app.set_network_up();
    assert!(matches!(app.screen, Screen::Waking { wifi_up: true, .. }));
    app.render();
    assert!(app.hits.is_empty(), "no dashboard controls while waking");
    app.on_ha_event(HaEvent::Connected);
    assert!(matches!(app.screen, Screen::Dashboard));
}

#[test]
fn waking_screen_gives_up_after_a_while() {
    let mut app = app();
    app.wake();
    app.on_ha_event(HaEvent::Disconnected { reason: "unreachable".into(), attempt: 7, retry_in: Duration::from_secs(1) });
    assert!(app.next_wakeup() <= WAKING_TIMEOUT + Duration::from_secs(1));
    if let Screen::Waking { since, .. } = &mut app.screen {
        *since = Instant::now() - WAKING_TIMEOUT;
    }
    app.tick();
    assert!(matches!(&app.screen, Screen::Disconnected { reason, attempt: 7, .. } if reason == "unreachable"));
}

#[test]
fn disconnected_screen_renders_and_ignores_controls() {
    let mut app = app();
    let power = hit_for(&app, "switch.aircon_power", "off");
    app.on_ha_event(HaEvent::Disconnected { reason: "boom".into(), attempt: 1, retry_in: Duration::from_secs(5) });
    app.render();
    assert!(app.hits.is_empty());
    assert_eq!(tap(&mut app, centre(power)), None);
}

#[test]
fn set_call_picks_the_service_by_domain() {
    assert_eq!(set_call("input_boolean.x", "on").service, "turn_on");
    assert_eq!(set_call("input_number.x", "20.5").data, Some(json!({"value": 20.5})));
    assert_eq!(set_call("climate.x", "off").domain, "homeassistant");
}

/// Renders sample screens to raw 8-bit files for eyeballing the layout on a PC:
/// `PREVIEW_DIR=... PREVIEW_FONT=regular.ttf PREVIEW_BOLD=bold.ttf cargo test preview -- --ignored`
#[test]
#[ignore]
fn preview() {
    use std::path::Path;
    let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
    let font = |var: &str| std::env::var(var).expect(var);
    let fonts = || Fonts::from_files(Path::new(&font("PREVIEW_FONT")), Path::new(&font("PREVIEW_BOLD")));
    let dash = dash();
    let room_list: Vec<Value> = ["Lounge", "Kitchen", "Bedroom 1", "Bedroom 2"]
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"name": n, "sensor": format!("sensor.room{i}")}))
        .collect();
    let now = clock::now();
    let forecast: Vec<ForecastHour> = (0..30)
        .map(|i| ForecastHour {
            time: (now / 3600.0).floor() * 3600.0 + i as f64 * 3600.0,
            temperature: Some(8.0 + 4.0 * ((i as f64 - 6.0) / 24.0 * std::f64::consts::TAU).cos()),
            precipitation: Some(if (15..22).contains(&i) { [0.6, 1.4, 2.0, 1.8, 1.1, 0.5, 0.2][i - 15] } else { 0.0 }),
            condition: "cloudy".into(),
        })
        .collect();
    let rooms = json!({
        "sensor.room0": {"s": "19.2"}, "sensor.room1": {"s": "18.9"},
        "sensor.room2": {"s": "17.4"}, "sensor.room3": {"s": "18.0"},
        "select.aircon_mode": {"s": "Heat", "a": {"options": ["Heat", "Cool", "Fan only", "Auto"]}},
        "select.aircon_fan_speed": {"s": "High", "a": {"options": ["Low", "Medium", "High"]}},
        "weather.home": {"s": "cloudy", "a": {"temperature": 9.4}},
        "sensor.lockout": {"s": "161", "lc": 2.0},
        "sensor.aircon_rooms": {"s": "4", "a": {"rooms": room_list}},
        "select.aircon_thermostat_room": {"s": "Average", "a": {"options": ["Average", "Lounge", "Kitchen", "Bedroom 1", "Bedroom 2"]}},
        "sensor.aircon_thermostat_temperature": {"s": "18.4"},
    });
    let history: History = (0..4)
        .map(|i| {
            let id = format!("sensor.room{i}");
            let pts = (0..13).map(|k| (now - 3600.0 + k as f64 * 300.0, 16.0 + i as f64 * 0.4 + k as f64 * 0.15 + 0.1 * (k as f64 * 1.7).sin())).collect();
            (id, pts)
        })
        .collect();

    let shot = |name: &str, extra: Value, connected: bool| {
        let mut all = rooms.clone();
        for (k, v) in extra.as_object().unwrap() {
            all[k] = v.clone();
        }
        let mut app = App::new(dash.clone(), aircon(), fonts(), (W, H));
        app.set_battery(Some(45), true);
        app.on_ha_event(HaEvent::Entities(states(all)));
        app.on_ha_event(HaEvent::Forecast(forecast.clone()));
        app.on_ha_event(HaEvent::History(history.clone()));
        if connected {
            app.on_ha_event(HaEvent::Connected);
        } else {
            app.on_ha_event(HaEvent::Disconnected {
                reason: "connecting to 192.168.1.100:8123: Connection refused (os error 111)".into(),
                attempt: 5,
                retry_in: Duration::from_secs(16),
            });
        }
        if name == "sleep" {
            app.render_sleep();
        } else if name == "waking" {
            app.wake();
            app.set_network_up();
            app.render();
        } else {
            app.render();
        }
        std::fs::write(Path::new(&dir).join(format!("{name}.raw")), app.canvas.raw()).unwrap();
    };
    shot("dashboard", json!({}), true);
    shot("continuous", json!({"input_boolean.thermostat": {"s": "off"}, "sensor.lockout": {"s": "0"}}), true);
    shot(
        "room_selected",
        json!({"select.aircon_thermostat_room": {"s": "Kitchen", "a": {"options": ["Average", "Kitchen"]}}, "sensor.aircon_thermostat_temperature": {"s": "18.9"}}),
        true,
    );
    shot("idle", json!({"sensor.aircon_thermostat_status": {"s": "Idle: at set point"}, "sensor.aircon_thermostat_temperature": {"s": "21.0"}}), true);
    shot(
        "auto",
        json!({
            "select.aircon_mode": {"s": "Auto", "a": {"options": ["Heat", "Cool", "Fan only", "Auto"]}},
            "sensor.aircon_auto_direction": {"s": "Heat"},
        }),
        true,
    );
    shot("no_rooms", json!({"sensor.aircon_rooms": {"s": "0", "a": {"rooms": []}}}), true);
    shot("unavailable", json!({"switch.aircon_power": {"s": "unavailable", "lc": now - 34.0 * 60.0}}), true);
    shot("disconnected", json!({}), false);
    shot("sleep", json!({}), true);
    shot("waking", json!({}), true);
}

/// Renders the Library cover for the scriptlet (`kual/aircon-controller/icon.png`) as a PGM:
/// `PREVIEW_DIR=... PREVIEW_FONT=regular.ttf PREVIEW_BOLD=bold.ttf cargo test scriptlet_icon -- --ignored`,
/// then `magick icon.pgm icon.png`. Use the Kindle's Amazon Ember fonts to match its UI.
#[test]
#[ignore]
fn scriptlet_icon() {
    use std::path::Path;
    let dir = std::env::var("PREVIEW_DIR").expect("PREVIEW_DIR");
    let font = |var: &str| std::env::var(var).expect(var);
    let mut fonts = Fonts::from_files(Path::new(&font("PREVIEW_FONT")), Path::new(&font("PREVIEW_BOLD")));
    let (w, h) = (600, 900);
    let mut c = Canvas::new(w, h);
    c.fill(WHITE);
    c.stroke_round_rect(Rect::new(18, 18, w - 36, h - 36), 28.0, 4.0, BLACK);
    let size = 440;
    let mut y = 120;
    icons::draw_waking_house(&mut c, (w - size) / 2, y, size, BLACK);
    y += size + 60;
    let (a, d) = fonts.line_metrics(104, Weight::Bold);
    fonts.draw_centered(&mut c, "Aircon", 104, Weight::Bold, w / 2, y + a, BLACK);
    y += a + d + 12;
    let (a, _) = fonts.line_metrics(46, Weight::Regular);
    fonts.draw_centered(&mut c, "Wall Controller", 46, Weight::Regular, w / 2, y + a, DARK_GRAY);
    let mut pgm = format!("P5\n{w} {h}\n255\n").into_bytes();
    pgm.extend_from_slice(c.raw());
    std::fs::write(Path::new(&dir).join("icon.pgm"), pgm).unwrap();
}
