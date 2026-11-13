//! Getting the stock Kindle UI ("the framework") out of our way, and putting it back.
//!
//! * `coexist`: the framework keeps running. We hide its status bar (pillow), and SIGSTOP
//!   the processes that could draw over us: by default the `awesome` window manager (so no
//!   new windows get mapped), the `cvm` Java reader if present, `blanket` (screensaver and
//!   setup screens) and `Xorg` itself, which otherwise repaints existing windows over us.
//! * `stop`: stop the whole GUI job (`lab126_gui`: reader, browser, window manager, memory
//!   monitor...) with Upstart, and start it again afterwards. The X server is a separate
//!   job, so the pause list is still applied to whatever is left (Xorg). Saves about half
//!   the Kindle's idle CPU and their memory, but handing back takes longer.
//!
//! Either way, powerd is told not to start the screensaver while we run.
//!
//! `restore` is safe to call more than once. The launcher script calls it again after the
//! controller exits (`--restore-framework`), so a crash can't leave the Kindle frozen.

use log::info;

use crate::config::{FrameworkConfig, FrameworkMode};
use crate::lipc::{run, set as lipc_set};

/// Takes the framework out of the way on creation and restores it on drop.
pub struct Guard {
    cfg: FrameworkConfig,
}

impl Guard {
    pub fn take(cfg: &FrameworkConfig) -> Self {
        apply(cfg);
        Self { cfg: cfg.clone() }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore(&self.cfg);
    }
}

/// Wakes the Kindle if powerd already had it in screensaver mode when we started (the
/// touchscreen is off in that mode). From then on `preventScreenSaver` stops the timeout,
/// and the power button is grabbed in `input::grab_power_buttons`, so it can't sleep again.
fn ensure_awake() {
    match crate::lipc::get_string("com.lab126.powerd", "state").as_deref() {
        Some("active") | None => {}
        Some(state) => {
            info!("Kindle is in powerd state {state:?}; waking it so touch works");
            lipc_set("com.lab126.powerd", "wakeUp", "1");
        }
    }
}

pub fn apply(cfg: &FrameworkConfig) {
    info!("taking over the display ({:?} mode)", cfg.mode);
    ensure_awake();
    lipc_set("com.lab126.powerd", "preventScreenSaver", "1");
    match cfg.mode {
        FrameworkMode::Coexist => {
            lipc_set("com.lab126.pillow", "disableEnablePillow", "disable");
            for p in &cfg.pause_processes {
                run("killall", &["-STOP", p]);
            }
        }
        FrameworkMode::Stop => {
            // Newer firmware calls the job lab126_gui; older firmware calls it framework.
            // A requested stop ends with RESULT=ok, so lab126_gui_monitor doesn't restart it.
            if !run("stop", &["lab126_gui"]) {
                run("stop", &["framework"]);
            }
            // The X server is a separate job and keeps running: pause it (and anything else
            // in the list that's still around) so it can't repaint.
            for p in &cfg.pause_processes {
                run("killall", &["-STOP", p]);
            }
        }
    }
}

pub fn restore(cfg: &FrameworkConfig) {
    info!("handing the display back to the Kindle UI");
    match cfg.mode {
        FrameworkMode::Coexist => {
            for p in cfg.pause_processes.iter().rev() {
                run("killall", &["-CONT", p]);
            }
            lipc_set("com.lab126.pillow", "disableEnablePillow", "enable");
            // The framework doesn't know we drew over it, so make it repaint by going home.
            lipc_set("com.lab126.appmgrd", "start", "app://com.lab126.booklet.home");
        }
        FrameworkMode::Stop => {
            for p in cfg.pause_processes.iter().rev() {
                run("killall", &["-CONT", p]);
            }
            if !run("start", &["lab126_gui"]) {
                run("start", &["framework"]);
            }
        }
    }
    lipc_set("com.lab126.powerd", "preventScreenSaver", "0");
}
