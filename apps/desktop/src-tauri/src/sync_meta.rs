use std::fs;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncStatusDto {
    pub last_sync: Option<String>,
    pub last_full_sync: Option<String>,
    pub seconds_to_next_sync: Option<u64>,
    pub sync_in_progress: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum SyncTriggerKind {
    Resync,
    FullResync,
}

pub fn read_sync_status(mountpoint: &Path) -> Result<SyncStatusDto, String> {
    let base = mountpoint.join(".sync_meta");

    let last_sync = read_optional_trimmed(base.join("last_sync"));
    let last_full_sync = read_optional_trimmed(base.join("last_full_sync"));
    let seconds_to_next_sync = read_optional_trimmed(base.join("seconds_to_next_sync"))
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok());

    let manual_refresh = read_optional_trimmed(base.join("manual_refresh")).unwrap_or_default();
    let full_refresh = read_optional_trimmed(base.join("full_refresh")).unwrap_or_default();
    let sync_in_progress =
        manual_refresh.contains("sync in progress") || full_refresh.contains("sync in progress");

    if last_sync.is_none() && last_full_sync.is_none() && seconds_to_next_sync.is_none() {
        return Err("sync metadata files are unavailable".to_string());
    }

    Ok(SyncStatusDto {
        last_sync,
        last_full_sync,
        seconds_to_next_sync,
        sync_in_progress,
    })
}

pub fn trigger_sync(mountpoint: &Path, kind: SyncTriggerKind) -> Result<(), String> {
    let base = mountpoint.join(".sync_meta");
    let file_name = match kind {
        SyncTriggerKind::Resync => "manual_refresh",
        SyncTriggerKind::FullResync => "full_refresh",
    };
    fs::write(base.join(file_name), "1\n")
        .map_err(|error| format!("failed writing trigger file '{file_name}': {error}"))
}

fn read_optional_trimmed(path: impl AsRef<Path>) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parse_sync_status_when_files_exist() {
        let root = fixture_dir();
        let meta = root.join(".sync_meta");
        std::fs::create_dir_all(&meta).expect("create meta dir");
        std::fs::write(meta.join("last_sync"), "10 seconds ago\n").expect("write last_sync");
        std::fs::write(meta.join("last_full_sync"), "never\n").expect("write last_full_sync");
        std::fs::write(meta.join("seconds_to_next_sync"), "4\n").expect("write seconds");
        std::fs::write(meta.join("manual_refresh"), "sync in progress\n")
            .expect("write manual_refresh");
        std::fs::write(meta.join("full_refresh"), "write '1' or 'true'\n")
            .expect("write full_refresh");

        let status = read_sync_status(&root).expect("status should parse");
        assert_eq!(status.last_sync.as_deref(), Some("10 seconds ago"));
        assert_eq!(status.last_full_sync.as_deref(), Some("never"));
        assert_eq!(status.seconds_to_next_sync, Some(4));
        assert!(status.sync_in_progress);
    }

    fn fixture_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time moved backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jirafs-desktop-sync-meta-{unique}"));
        std::fs::create_dir_all(&path).expect("create fixture path");
        path
    }
}
