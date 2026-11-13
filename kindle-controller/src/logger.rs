//! Minimal stderr logger with UTC timestamps. The launcher script redirects stderr to a file.
//! Level comes from `AIRCON_LOG` (error / warn / info / debug / trace), default info.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

struct Logger;

static LOGGER: Logger = Logger;

pub fn init() {
    let level = std::env::var("AIRCON_LOG")
        .ok()
        .and_then(|v| v.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let _ = writeln!(
            std::io::stderr().lock(),
            "{} {:<5} [{}] {}",
            timestamp(),
            record.level(),
            record.target().trim_start_matches("aircon_wall_controller::"),
            record.args()
        );
    }

    fn flush(&self) {}
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let (y, m, d) = crate::clock::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}
