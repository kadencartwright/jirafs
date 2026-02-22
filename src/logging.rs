use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static DEBUG_ENABLED: OnceLock<bool> = OnceLock::new();

fn debug_enabled() -> bool {
    *DEBUG_ENABLED.get_or_init(|| {
        std::env::var("FS_JIRA_DEBUG")
            .ok()
            .map(|v| {
                let normalized = v.trim().to_ascii_lowercase();
                normalized == "1" || normalized == "true" || normalized == "yes"
            })
            .unwrap_or(false)
    })
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
