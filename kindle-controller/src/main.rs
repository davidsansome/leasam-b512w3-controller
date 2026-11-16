//! Aircon wall controller for a jailbroken Kindle.
//!
//! Usage:
//!   aircon-wall-controller [--config PATH]               run the UI
//!   aircon-wall-controller [--config PATH] --restore-framework
//!                                                        give the screen back to the Kindle UI
//!   aircon-wall-controller [--config PATH] --list-entities
//!                                                        print HA entity ids for the config
//!
//! The config defaults to `config.toml` in the directory above the binary (the KUAL
//! extension folder); `state.json` lives next to it.

mod backlight;
mod clock;
mod config;
mod diagnostics;
mod display;
mod framework;
mod ha;
mod input;
mod lipc;
mod logger;
mod sleep;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use log::{error, info, warn};

use config::Config;
use display::font::Fonts;
use ui::{App, UiAction};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
/// Set by the panic hook. The main loop then stops cleanly, but we exit with an error so
/// supervise.sh starts us again rather than handing back to the Kindle UI.
static PANICKED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Run,
    RestoreFramework,
    ListEntities,
}

enum AppEvent {
    Touch(input::TouchEvent),
    Ha(ha::HaEvent),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // only produced on the device
    PowerButton,
    /// The Kindle has a working network connection again.
    NetworkUp,
    /// SIGTERM / SIGINT / SIGHUP, or a panic on another thread.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Shutdown,
}

/// After a fast (black/white) refresh, redraw that area properly this much later.
const CLEANUP_DELAY: Duration = Duration::from_millis(1500);

fn main() {
    logger::init();
    let code = match parse_args().and_then(|(path, mode)| {
        let config = Config::load(&path)?;
        match mode {
            Mode::RestoreFramework => {
                framework::restore(&config.framework);
                Ok(())
            }
            Mode::ListEntities => ha::list_entities(&config.home_assistant),
            Mode::Run => run(&path, config),
        }
    }) {
        Ok(()) => 0,
        Err(e) => {
            error!("{e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn parse_args() -> Result<(PathBuf, Mode)> {
    let mut config = None;
    let mut mode = Mode::Run;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(args.next().context("--config needs a path")?)),
            "--restore-framework" => mode = Mode::RestoreFramework,
            "--list-entities" => mode = Mode::ListEntities,
            other => bail!("unknown argument {other}"),
        }
    }
    let config = match config {
        Some(c) => c,
        None => std::env::current_exe()?
            .parent()
            .and_then(Path::parent)
            .context("can't work out the default config path")?
            .join("config.toml"),
    };
    Ok((config, mode))
}

/// How the UI loop ended without an error.
enum Outcome {
    /// Exit from the menu, or SIGTERM: hand the display back to the Kindle UI.
    Exit,
    /// Reboot from the menu.
    Reboot,
}

/// The upstart job that starts the controller at boot, when it's installed.
const AUTOSTART_JOB: &str = "/etc/upstart/aircon-controller.conf";

fn run(config_path: &Path, config: Config) -> Result<()> {
    info!("aircon-wall-controller {} starting", env!("CARGO_PKG_VERSION"));
    let signals = install_signal_handlers();
    let framework_cfg = config.framework.clone();
    framework::apply(&framework_cfg);
    // On an error or panic the display is left as it is: supervise.sh starts us again, and
    // handing back in between would start X and the whole Kindle UI.
    let outcome = run_ui(config_path, config, signals)?;
    if PANICKED.load(Ordering::SeqCst) {
        bail!("a thread panicked");
    }
    match outcome {
        Outcome::Exit => framework::restore(&framework_cfg),
        Outcome::Reboot => {
            // No handing back first: that would start X and the Kindle UI on the way down.
            lipc::run("sync", &[]);
            if !lipc::run("reboot", &[]) {
                bail!("reboot failed");
            }
        }
    }
    Ok(())
}

/// Runs the UI until Exit, Reboot or a signal. Everything it opens (display, input grabs,
/// the HA connection) is released by the time it returns.
fn run_ui(config_path: &Path, config: Config, signals: Option<std::fs::File>) -> Result<Outcome> {
    let state_path = config_path.with_file_name("state.json");
    let mut display = display::Display::open(&config.display)?;
    // Held for the whole run: dropping them releases the grabs.
    let power_buttons = input::open_power_buttons(config.framework.block_power_button);
    let size = display.size();
    let fonts = Fonts::load(config.display.font.as_deref());

    let (tx, rx) = mpsc::channel();
    forward_signals(signals, tx.clone());
    let power_tx = tx.clone();
    input::watch_power_buttons(&power_buttons, move || {
        let _ = power_tx.send(AppEvent::PowerButton);
    });
    let network_tx = tx.clone();
    let _network = sleep::NetworkWatcher::spawn(move || {
        let _ = network_tx.send(AppEvent::NetworkUp);
    });
    let touch_tx = tx.clone();
    // Touches arrive in panel coordinates; the UI works in rotated (logical) ones.
    let rotation = display.rotation();
    input::spawn(&config.touch, display.physical_size(), move |e| {
        let e = match e {
            input::TouchEvent::Down { x, y } => {
                let (x, y) = rotation.to_logical(x, y);
                input::TouchEvent::Down { x, y }
            }
            input::TouchEvent::Up { x, y, held } => {
                let (x, y) = rotation.to_logical(x, y);
                input::TouchEvent::Up { x, y, held }
            }
        };
        let _ = touch_tx.send(AppEvent::Touch(e));
    })?;
    if config.dashboard.old_rooms.is_some() {
        warn!("[[dashboard.rooms]] in config.toml is no longer used: set rooms up in Home Assistant with the Aircon rooms blueprint");
    }
    let mut app = App::new(config.dashboard.clone(), config.aircon.clone(), fonts, size);
    let ha = ha::spawn(
        ha::Options {
            ha: config.home_assistant.clone(),
            device: config.device.clone(),
            state_path,
            entities: app.entities_wanted(),
            aircon: config.aircon.ids(),
            aircon_device: config.aircon.device.clone(),
            weather: config.dashboard.weather.clone(),
            history_window: ui::HISTORY_WINDOW,
        },
        move |e| {
            let _ = tx.send(AppEvent::Ha(e));
        },
    )?;

    let mut backlight = backlight::Backlight::new();
    app.set_backlight_state(backlight.is_on());
    let (percent, charging) = diagnostics::battery_status();
    app.set_battery(percent, charging);
    // Whether the backlight was on when going to sleep, to turn it back on when waking.
    let mut backlight_before_sleep = false;
    let mut outcome = Outcome::Exit;
    let mut last_battery = Instant::now();
    // The loop sleeps until an event arrives or the UI next has something time-based to
    // do (see `App::next_wakeup`), so it doesn't wake the CPU while idle.
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("shutting down");
            break;
        }
        // A fast (black/white) refresh leaves rough edges; redraw that area properly soon after.
        if display.needs_cleanup(CLEANUP_DELAY) {
            app.invalidate(display::Update::Normal);
        }
        app.tick();
        // Checked whenever we wake, which is at least once a minute for the clock.
        if last_battery.elapsed() >= Duration::from_secs(60) {
            last_battery = Instant::now();
            let (percent, charging) = diagnostics::battery_status();
            app.set_battery(percent, charging);
        }
        if let Err(e) = app.present(&mut display) {
            error!("display update failed: {e:#}");
        }
        let mut wait = app.next_wakeup();
        if let Some(cleanup) = display.cleanup_in(CLEANUP_DELAY) {
            wait = wait.min(cleanup + Duration::from_millis(10));
        }
        match rx.recv_timeout(wait) {
            Ok(AppEvent::Touch(t)) => match app.on_touch(t) {
                Some(UiAction::Call(call)) => ha.call_service(call),
                Some(UiAction::SetBacklight(on)) => {
                    backlight.set(on);
                    app.set_backlight_state(backlight.is_on());
                }
                Some(UiAction::Sleep) => {
                    info!("going to sleep");
                    app.set_asleep(true);
                    if let Err(e) = app.present(&mut display) {
                        error!("display update failed: {e:#}");
                    }
                    // Close the HA connection cleanly before the network goes.
                    ha.set_paused(true);
                    sleep::set_touch(false);
                    sleep::set_wifi(false);
                    // Turned back on when waking, if it was on now.
                    backlight_before_sleep = backlight.is_on() != Some(false);
                    if backlight_before_sleep {
                        backlight.set(false);
                        app.set_backlight_state(backlight.is_on());
                    }
                }
                Some(UiAction::Exit) => {
                    info!("exit requested from the menu");
                    let detail = match config.framework.mode {
                        config::FrameworkMode::Stop => "This takes about a minute.",
                        config::FrameworkMode::Coexist => "",
                    };
                    let _ = app.show_farewell(&mut display, "Returning to the Kindle home screen", detail);
                    break;
                }
                Some(UiAction::Reboot) => {
                    info!("reboot requested from the menu");
                    let detail = if Path::new(AUTOSTART_JOB).exists() {
                        "The controller starts again by itself."
                    } else {
                        "Start the controller again from the Library afterwards."
                    };
                    let _ = app.show_farewell(&mut display, "Rebooting", detail);
                    outcome = Outcome::Reboot;
                    break;
                }
                None => {}
            },
            Ok(AppEvent::Ha(e)) => app.on_ha_event(e),
            // Awake, the button does nothing: it's grabbed so powerd can't sleep the Kindle.
            Ok(AppEvent::PowerButton) if app.is_asleep() => {
                info!("waking up");
                sleep::set_wifi(true);
                sleep::set_touch(true);
                ha.set_paused(false);
                if backlight_before_sleep {
                    backlight.set(true);
                    app.set_backlight_state(backlight.is_on());
                }
                app.wake();
            }
            Ok(AppEvent::PowerButton) => {}
            // Also helps after an ordinary Wi-Fi dropout: no waiting out the backoff.
            Ok(AppEvent::NetworkUp) => {
                app.set_network_up();
                ha.retry_now();
            }
            Ok(AppEvent::Shutdown) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("event sources went away"),
        }
    }
    Ok(outcome)
}

/// Write end of the pipe that wakes the main loop on a signal or panic.
#[cfg(target_os = "linux")]
static WAKE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Sets SHUTDOWN and wakes the main loop. Async-signal-safe.
#[cfg(target_os = "linux")]
fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        // SAFETY: writing to a pipe is async-signal-safe; the byte's address is valid.
        unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
    }
}

/// Installs handlers for SIGTERM, SIGINT and SIGHUP (and a panic hook) that ask the main
/// loop to shut down cleanly (after a signal, the Kindle UI is restored). Returns the pipe's read end,
/// for `forward_signals`.
#[cfg(target_os = "linux")]
fn install_signal_handlers() -> Option<std::fs::File> {
    use std::os::fd::FromRawFd;

    extern "C" fn on_signal(_: libc::c_int) {
        request_shutdown();
    }
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe() fills in two fds, which we then own.
    let reader = if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
        WAKE_FD.store(fds[1], Ordering::SeqCst);
        Some(unsafe { std::fs::File::from_raw_fd(fds[0]) })
    } else {
        None
    };
    // SAFETY: the handler only touches an atomic and writes to a pipe.
    unsafe {
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        PANICKED.store(true, Ordering::SeqCst);
        request_shutdown();
    }));
    reader
}

/// Turns bytes on the signal pipe into `AppEvent::Shutdown` for the main loop.
#[cfg(target_os = "linux")]
fn forward_signals(reader: Option<std::fs::File>, tx: mpsc::Sender<AppEvent>) {
    use std::io::Read;
    let Some(mut reader) = reader else { return };
    let _ = std::thread::Builder::new().name("signals".into()).spawn(move || {
        let mut buf = [0u8; 1];
        while reader.read(&mut buf).is_ok_and(|n| n > 0) {
            if tx.send(AppEvent::Shutdown).is_err() {
                break;
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn install_signal_handlers() -> Option<std::fs::File> {
    None
}

#[cfg(not(target_os = "linux"))]
fn forward_signals(_: Option<std::fs::File>, _: mpsc::Sender<AppEvent>) {}
