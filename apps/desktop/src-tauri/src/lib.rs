mod errors;
#[cfg(target_os = "linux")]
mod service_linux;
#[cfg(target_os = "macos")]
mod service_macos;
mod sync_meta;

use errors::{ServiceProbeError, ServiceProbeErrorKind};
use jirafs::jira::JiraClient;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State, WindowEvent};

const SESSION_LOG_CAPACITY: usize = 10_000;

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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceActionReason {
    Started,
    Restarted,
    ServiceNotInstalled,
    ActionFailed,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ServiceActionResultDto {
    ok: bool,
    reason: ServiceActionReason,
}

#[derive(Debug, Clone, serde::Serialize)]
struct LogLineDto {
    ts: Option<String>,
    source: String,
    line: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WorkspaceJqlInputDto {
    name: String,
    jql: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct WorkspaceJqlValidationDto {
    name: String,
    valid: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct LogBufferState {
    capacity: usize,
    lines: Arc<Mutex<Vec<LogLineDto>>>,
}

impl LogBufferState {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn push_line(&self, source: &str, line: String) {
        let mut guard = self
            .lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.push(LogLineDto {
            ts: None,
            source: source.to_string(),
            line,
        });

        if guard.len() > self.capacity {
            let excess = guard.len().saturating_sub(self.capacity);
            guard.drain(0..excess);
        }
    }

    fn snapshot(&self) -> Vec<LogLineDto> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[derive(Debug, Clone)]
struct DesktopState {
    logs: LogBufferState,
    shutdown: Arc<AtomicBool>,
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

#[tauri::command]
fn ensure_service_running_or_restart(app: AppHandle) -> Result<ServiceActionResultDto, String> {
    let status = compute_status()?;

    if !status.service_installed {
        return Ok(ServiceActionResultDto {
            ok: false,
            reason: ServiceActionReason::ServiceNotInstalled,
        });
    }

    let response = if status.service_running {
        match restart_service() {
            Ok(()) => ServiceActionResultDto {
                ok: true,
                reason: ServiceActionReason::Restarted,
            },
            Err(_) => ServiceActionResultDto {
                ok: false,
                reason: ServiceActionReason::ActionFailed,
            },
        }
    } else {
        match start_service() {
            Ok(()) => ServiceActionResultDto {
                ok: true,
                reason: ServiceActionReason::Started,
            },
            Err(_) => ServiceActionResultDto {
                ok: false,
                reason: ServiceActionReason::ActionFailed,
            },
        }
    };

    if let Ok(next_status) = compute_status() {
        update_tray_tooltip(&app, &next_status);
    }

    Ok(response)
}

#[tauri::command]
fn get_session_logs(state: State<DesktopState>) -> Result<Vec<LogLineDto>, String> {
    Ok(state.logs.snapshot())
}

#[tauri::command]
fn get_workspace_jql_config() -> Result<Vec<WorkspaceJqlInputDto>, String> {
    let path = resolve_effective_config_path()?;
    let config = jirafs::config::load_from(&path).map_err(|error| error.to_string())?;

    let mut rows = config
        .jira
        .workspaces
        .into_iter()
        .map(|(name, workspace)| WorkspaceJqlInputDto {
            name,
            jql: workspace.jql,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(rows)
}

#[tauri::command]
fn validate_workspace_jqls(
    workspaces: Vec<WorkspaceJqlInputDto>,
) -> Result<Vec<WorkspaceJqlValidationDto>, String> {
    validate_workspace_jqls_inner(&workspaces)
}

#[tauri::command]
fn save_workspace_jql_config(workspaces: Vec<WorkspaceJqlInputDto>) -> Result<(), String> {
    let normalized = normalize_workspace_inputs(&workspaces)?;
    let validation = validate_workspace_jqls_inner(&normalized)?;

    let failures = validation
        .iter()
        .filter(|row| !row.valid)
        .map(|row| {
            format!(
                "{}: {}",
                row.name,
                row.error
                    .clone()
                    .unwrap_or_else(|| "validation failed".to_string())
            )
        })
        .collect::<Vec<_>>();

    if !failures.is_empty() {
        return Err(format!(
            "workspace validation failed: {}",
            failures.join("; ")
        ));
    }

    let path = resolve_effective_config_path()?;
    persist_workspace_jql_config(&path, &normalized)?;
    jirafs::config::load_from(&path).map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_workspace_jqls_inner(
    workspaces: &[WorkspaceJqlInputDto],
) -> Result<Vec<WorkspaceJqlValidationDto>, String> {
    let normalized = normalize_workspace_inputs(workspaces)?;
    let path = resolve_effective_config_path()?;
    let config = jirafs::config::load_from(&path).map_err(|error| error.to_string())?;
    let jira = JiraClient::new(
        config.jira.base_url,
        config.jira.email,
        config.jira.api_token,
    )
    .map_err(|error| error.to_string())?;

    let mut results = Vec::with_capacity(normalized.len());
    for workspace in normalized {
        match jira.list_issue_refs_for_jql(&workspace.jql) {
            Ok(_) => results.push(WorkspaceJqlValidationDto {
                name: workspace.name,
                valid: true,
                error: None,
            }),
            Err(error) => results.push(WorkspaceJqlValidationDto {
                name: workspace.name,
                valid: false,
                error: Some(error.to_string()),
            }),
        }
    }

    Ok(results)
}

fn normalize_workspace_inputs(
    workspaces: &[WorkspaceJqlInputDto],
) -> Result<Vec<WorkspaceJqlInputDto>, String> {
    if workspaces.is_empty() {
        return Err("at least one workspace is required".to_string());
    }

    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        let name = workspace.name.trim().to_string();
        let jql = workspace.jql.trim().to_string();
        if name.is_empty() {
            return Err("workspace name must not be empty".to_string());
        }
        if jql.is_empty() {
            return Err(format!("workspace '{name}' jql must not be empty"));
        }
        if !seen.insert(name.clone()) {
            return Err(format!("workspace '{name}' is duplicated"));
        }

        normalized.push(WorkspaceJqlInputDto { name, jql });
    }

    Ok(normalized)
}

fn resolve_effective_config_path() -> Result<PathBuf, String> {
    let status = compute_status()?;
    if let Some(path) = status.config_path {
        return Ok(PathBuf::from(path));
    }
    jirafs::config::resolve_config_path().map_err(|error| error.to_string())
}

fn persist_workspace_jql_config(
    path: &Path,
    workspaces: &[WorkspaceJqlInputDto],
) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read config file for workspace update at {}: {}",
            path.display(),
            error
        )
    })?;

    let mut document = toml::from_str::<toml::Value>(&raw)
        .map_err(|error| format!("failed to parse config TOML for workspace update: {error}"))?;
    let Some(root_table) = document.as_table_mut() else {
        return Err("config root is not a TOML table".to_string());
    };

    if !root_table.contains_key("jira") {
        root_table.insert(
            "jira".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let Some(jira_table) = root_table
        .get_mut("jira")
        .and_then(toml::Value::as_table_mut)
    else {
        return Err("config jira section is not a TOML table".to_string());
    };

    let mut workspaces_table = toml::map::Map::new();
    let mut sorted = workspaces.to_vec();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    for workspace in sorted {
        let mut row = toml::map::Map::new();
        row.insert("jql".to_string(), toml::Value::String(workspace.jql));
        workspaces_table.insert(workspace.name, toml::Value::Table(row));
    }
    jira_table.insert(
        "workspaces".to_string(),
        toml::Value::Table(workspaces_table),
    );

    let updated = toml::to_string_pretty(&document)
        .map_err(|error| format!("failed to serialize updated config TOML: {error}"))?;

    std::fs::write(path, updated).map_err(|error| {
        format!(
            "failed to write updated workspace config at {}: {}",
            path.display(),
            error
        )
    })
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
        config_path = jirafs::config::resolve_config_path()
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
            .join("jirafs")
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

#[cfg(target_os = "linux")]
fn start_service() -> Result<(), ServiceProbeError> {
    service_linux::start_service()
}

#[cfg(target_os = "linux")]
fn restart_service() -> Result<(), ServiceProbeError> {
    service_linux::restart_service()
}

#[cfg(target_os = "linux")]
fn spawn_session_log_collector(logs: LogBufferState, shutdown: Arc<AtomicBool>) {
    service_linux::spawn_log_collector(logs, shutdown);
}

#[cfg(target_os = "macos")]
fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    service_macos::probe_service()
}

#[cfg(target_os = "macos")]
fn start_service() -> Result<(), ServiceProbeError> {
    service_macos::start_service()
}

#[cfg(target_os = "macos")]
fn restart_service() -> Result<(), ServiceProbeError> {
    service_macos::restart_service()
}

#[cfg(target_os = "macos")]
fn spawn_session_log_collector(logs: LogBufferState, shutdown: Arc<AtomicBool>) {
    service_macos::spawn_log_collector(logs, shutdown);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    Err(ServiceProbeError {
        kind: ServiceProbeErrorKind::NotInstalled,
        message: "unsupported platform".to_string(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn start_service() -> Result<(), ServiceProbeError> {
    Err(ServiceProbeError {
        kind: ServiceProbeErrorKind::NotInstalled,
        message: "unsupported platform".to_string(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn restart_service() -> Result<(), ServiceProbeError> {
    Err(ServiceProbeError {
        kind: ServiceProbeErrorKind::NotInstalled,
        message: "unsupported platform".to_string(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn spawn_session_log_collector(_logs: LogBufferState, _shutdown: Arc<AtomicBool>) {}

fn update_tray_tooltip(app: &AppHandle, status: &AppStatusDto) {
    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = format!(
            "jirafs: {:?} (service: {})",
            status.sync_state, status.service_running
        );
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let start_item = MenuItem::with_id(
        app,
        "start_or_restart_service",
        "Start/Restart Service",
        true,
        None::<&str>,
    )?;
    let resync_item = MenuItem::with_id(app, "resync", "Resync", true, None::<&str>)?;
    let full_resync_item =
        MenuItem::with_id(app, "full_resync", "Full Resync", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &start_item,
            &resync_item,
            &full_resync_item,
            &quit_item,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .tooltip("jirafs: starting")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "resync" => {
                let _ = trigger_sync(app.clone(), "resync".to_string());
            }
            "start_or_restart_service" => {
                let _ = ensure_service_running_or_restart(app.clone());
            }
            "full_resync" => {
                let _ = trigger_sync(app.clone(), "full_resync".to_string());
            }
            "quit" => {
                let state = app.state::<DesktopState>();
                state.shutdown.store(true, Ordering::Relaxed);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn setup_window_behavior(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let window_clone = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_clone.hide();
            }
        });
    }
}

pub fn run() {
    let desktop_state = DesktopState {
        logs: LogBufferState::new(SESSION_LOG_CAPACITY),
        shutdown: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .manage(desktop_state)
        .setup(|app| {
            setup_tray(app.handle())?;
            setup_window_behavior(app.handle());

            let state = app.state::<DesktopState>();
            spawn_session_log_collector(state.logs.clone(), state.shutdown.clone());

            if let Ok(status) = compute_status() {
                update_tray_tooltip(app.handle(), &status);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            trigger_sync,
            ensure_service_running_or_restart,
            get_session_logs,
            get_workspace_jql_config,
            validate_workspace_jqls,
            save_workspace_jql_config
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_workspace_inputs_rejects_duplicate_names() {
        let values = vec![
            WorkspaceJqlInputDto {
                name: "ops".to_string(),
                jql: "project = OPS".to_string(),
            },
            WorkspaceJqlInputDto {
                name: "ops".to_string(),
                jql: "project = OPS ORDER BY updated DESC".to_string(),
            },
        ];

        let err = normalize_workspace_inputs(&values).expect_err("duplicates should fail");
        assert!(err.contains("duplicated"));
    }

    #[test]
    fn persist_workspace_jql_rewrites_only_workspace_map() {
        let tmp =
            std::env::temp_dir().join(format!("jirafs-workspaces-{}.toml", std::process::id()));

        let raw = r#"
[jira]
base_url = "https://example.atlassian.net"
email = "you@example.com"
api_token = "token"

[jira.workspaces.default]
jql = "project = TEST ORDER BY updated DESC"

[cache]
db_path = "/tmp/cache.db"

[sync]
budget = 10
interval_secs = 60

[metrics]
interval_secs = 60

[logging]
debug = false
"#;

        std::fs::write(&tmp, raw).expect("seed config");
        let workspaces = vec![WorkspaceJqlInputDto {
            name: "ops".to_string(),
            jql: "project = OPS ORDER BY updated DESC".to_string(),
        }];

        persist_workspace_jql_config(&tmp, &workspaces).expect("workspace update should succeed");
        let loaded = jirafs::config::load_from(&tmp).expect("updated config should parse");

        assert_eq!(loaded.cache.db_path, "/tmp/cache.db");
        assert_eq!(loaded.sync.budget, 10);
        assert_eq!(loaded.metrics.interval_secs, 60);
        assert!(!loaded.logging.debug);
        assert_eq!(loaded.jira.workspaces.len(), 1);
        assert_eq!(
            loaded
                .jira
                .workspaces
                .get("ops")
                .map(|workspace| workspace.jql.as_str()),
            Some("project = OPS ORDER BY updated DESC")
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn log_buffer_keeps_recent_lines() {
        let state = LogBufferState::new(2);
        state.push_line("journalctl", "one".to_string());
        state.push_line("journalctl", "two".to_string());
        state.push_line("journalctl", "three".to_string());

        let rows = state.snapshot();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].line, "two");
        assert_eq!(rows[1].line, "three");
    }
}
