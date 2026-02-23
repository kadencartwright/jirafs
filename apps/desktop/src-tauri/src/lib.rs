mod errors;
#[cfg(target_os = "linux")]
mod service_linux;
#[cfg(target_os = "macos")]
mod service_macos;
mod sync_meta;

use errors::{ServiceProbeError, ServiceProbeErrorKind};
use std::path::{Path, PathBuf};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone)]
struct ServiceProbe {
    installed: bool,
    running: bool,
    config_path: Option<String>,
    mountpoint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum SyncStateValue {
    Stopped,
    Running,
    Syncing,
    Degraded,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum PathSource {
    ServiceArgs,
    KnownDefaults,
    ConfigResolver,
}

#[derive(Debug, Clone, serde::Serialize)]
struct AppStatusDto {
    platform: String,
    service_installed: bool,
    service_running: bool,
    sync_state: SyncStateValue,
    config_path: Option<String>,
    mountpoint: Option<String>,
    path_source: PathSource,
    sync: sync_meta::SyncStatusDto,
    errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum TriggerReason {
    Accepted,
    AlreadySyncing,
    ServiceNotRunning,
    MountpointUnavailable,
    TriggerWriteFailed,
}

#[derive(Debug, Clone, serde::Serialize)]
struct TriggerSyncResultDto {
    accepted: bool,
    reason: TriggerReason,
}

#[tauri::command]
fn get_app_status(app: AppHandle) -> Result<AppStatusDto, String> {
    let status = compute_status()?;
    update_tray_tooltip(&app, &status);
    Ok(status)
}

#[tauri::command]
fn trigger_sync(app: AppHandle, kind: String) -> Result<TriggerSyncResultDto, String> {
    let trigger_kind = match kind.as_str() {
        "resync" => sync_meta::SyncTriggerKind::Resync,
        "full_resync" => sync_meta::SyncTriggerKind::FullResync,
        _ => return Err(format!("unsupported sync kind: {kind}")),
    };

    let status = compute_status()?;
    if !status.service_running {
        return Ok(TriggerSyncResultDto {
            accepted: false,
            reason: TriggerReason::ServiceNotRunning,
        });
    }

    let Some(mountpoint) = status.mountpoint.as_ref() else {
        return Ok(TriggerSyncResultDto {
            accepted: false,
            reason: TriggerReason::MountpointUnavailable,
        });
    };

    if status.sync.sync_in_progress {
        return Ok(TriggerSyncResultDto {
            accepted: false,
            reason: TriggerReason::AlreadySyncing,
        });
    }

    let mountpoint_path = PathBuf::from(mountpoint);
    if !mountpoint_path.exists() {
        return Ok(TriggerSyncResultDto {
            accepted: false,
            reason: TriggerReason::MountpointUnavailable,
        });
    }

    let result = sync_meta::trigger_sync(&mountpoint_path, trigger_kind);
    let response = match result {
        Ok(()) => TriggerSyncResultDto {
            accepted: true,
            reason: TriggerReason::Accepted,
        },
        Err(_) => TriggerSyncResultDto {
            accepted: false,
            reason: TriggerReason::TriggerWriteFailed,
        },
    };

    if let Ok(next_status) = compute_status() {
        update_tray_tooltip(&app, &next_status);
    }

    Ok(response)
}

fn compute_status() -> Result<AppStatusDto, String> {
    let mut errors = Vec::new();
    let probe = match probe_service() {
        Ok(value) => value,
        Err(error) => {
            errors.push(format_probe_error(&error));
            ServiceProbe {
                installed: !matches!(error.kind, ServiceProbeErrorKind::NotInstalled),
                running: false,
                config_path: None,
                mountpoint: None,
            }
        }
    };

    let mut path_source = PathSource::ServiceArgs;
    let mut config_path = probe.config_path.clone();
    let mut mountpoint = probe.mountpoint.clone();

    if mountpoint.is_none() {
        mountpoint = known_default_mountpoint();
        path_source = PathSource::KnownDefaults;
    }

    if config_path.is_none() {
        config_path = fs_jira::config::resolve_config_path()
            .ok()
            .map(|path| path.to_string_lossy().to_string());
        path_source = PathSource::ConfigResolver;
    }

    let sync = if probe.running {
        if let Some(path) = mountpoint.as_ref() {
            match sync_meta::read_sync_status(Path::new(path)) {
                Ok(value) => value,
                Err(error) => {
                    errors.push(error);
                    empty_sync_status()
                }
            }
        } else {
            errors.push("mountpoint is unresolved".to_string());
            empty_sync_status()
        }
    } else {
        empty_sync_status()
    };

    let sync_state = if !probe.running {
        SyncStateValue::Stopped
    } else if sync.sync_in_progress {
        SyncStateValue::Syncing
    } else if errors.is_empty() {
        SyncStateValue::Running
    } else {
        SyncStateValue::Degraded
    };

    Ok(AppStatusDto {
        platform: std::env::consts::OS.to_string(),
        service_installed: probe.installed,
        service_running: probe.running,
        sync_state,
        config_path,
        mountpoint,
        path_source,
        sync,
        errors,
    })
}

fn empty_sync_status() -> sync_meta::SyncStatusDto {
    sync_meta::SyncStatusDto {
        last_sync: None,
        last_full_sync: None,
        seconds_to_next_sync: None,
        sync_in_progress: false,
    }
}

fn known_default_mountpoint() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("fs-jira")
            .to_string_lossy()
            .to_string(),
    )
}

fn format_probe_error(error: &ServiceProbeError) -> String {
    let kind = match error.kind {
        ServiceProbeErrorKind::Permission => "permission",
        ServiceProbeErrorKind::NotInstalled => "not_installed",
        ServiceProbeErrorKind::Unreachable => "unreachable",
        ServiceProbeErrorKind::ParseError => "parse_error",
    };
    format!("service probe failed ({kind}): {}", error.message)
}

#[cfg(target_os = "linux")]
fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    service_linux::probe_service()
}

#[cfg(target_os = "macos")]
fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    service_macos::probe_service()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    Err(ServiceProbeError {
        kind: ServiceProbeErrorKind::NotInstalled,
        message: "unsupported platform".to_string(),
    })
}

fn update_tray_tooltip(app: &AppHandle, status: &AppStatusDto) {
    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = format!(
            "fs-jira: {:?} (service: {})",
            status.sync_state, status.service_running
        );
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let resync_item = MenuItem::with_id(app, "resync", "Resync", true, None::<&str>)?;
    let full_resync_item =
        MenuItem::with_id(app, "full_resync", "Full Resync", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open_item, &resync_item, &full_resync_item, &quit_item],
    )?;

    TrayIconBuilder::with_id("main")
        .tooltip("fs-jira: starting")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "resync" => {
                let _ = trigger_sync(app.clone(), "resync".to_string());
            }
            "full_resync" => {
                let _ = trigger_sync(app.clone(), "full_resync".to_string());
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            setup_tray(app.handle())?;
            if let Ok(status) = compute_status() {
                update_tray_tooltip(app.handle(), &status);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_app_status, trigger_sync])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}
