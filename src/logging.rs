use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init(debug: bool) {
    DEBUG_ENABLED.store(debug, Ordering::Relaxed);
}

fn debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

fn ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn debug(message: impl AsRef<str>) {
    if debug_enabled() {
        eprintln!("[{}][DEBUG] {}", ts(), message.as_ref());
    }
}

pub fn info(message: impl AsRef<str>) {
    eprintln!("[{}][INFO] {}", ts(), message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    eprintln!("[{}][WARN] {}", ts(), message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    eprintln!("[{}][ERROR] {}", ts(), message.as_ref());
}
