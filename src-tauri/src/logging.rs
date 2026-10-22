//! A small file logger: `voxtray.log` in the config folder, replaced on each
//! launch. It records timings, state changes and errors, never the text being
//! read.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};

struct FileLogger {
    file: Mutex<File>,
    start: Instant,
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        // Only our own crate; dependencies are noisy at info level.
        m.level() <= Level::Info && m.target().starts_with("voxtray")
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if let Ok(mut f) = self.file.lock() {
            let t = self.start.elapsed().as_secs_f64();
            let _ = writeln!(f, "{t:>9.3}s {:<5} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

/// Start logging to `path`. Logging stays off if the file can't be created.
pub fn init(path: &Path) {
    let Ok(file) = File::create(path) else {
        return;
    };
    let logger = Box::leak(Box::new(FileLogger {
        file: Mutex::new(file),
        start: Instant::now(),
    }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}
