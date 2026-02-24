# jirafs Desktop: Restart, Logs, Tray Minimize, and Workspace JQL Editor Implementation Plan

## Overview

Add four operator features to the existing desktop control panel: a single dynamic Start/Restart action, in-app service log viewing for logs observed since app launch, minimize-to-tray behavior, and a structured UI for editing `jira.workspaces.<name>.jql` with Jira API validation before saving.

## Current State Analysis

The current desktop app already has strong status/action foundations but no log viewer or config editor:

- Service status polling and action wiring already exist (`apps/desktop/src/hooks/use-app-status.ts:19`, `apps/desktop/src/components/actions-card.tsx:77`).
- Desktop backend currently exposes `get_app_status`, `trigger_sync`, and `start_user_service` only (`apps/desktop/src-tauri/src/lib.rs:83`, `apps/desktop/src-tauri/src/lib.rs:90`, `apps/desktop/src-tauri/src/lib.rs:147`).
- Cross-platform service start commands are implemented in platform modules (`apps/desktop/src-tauri/src/service_linux.rs:58`, `apps/desktop/src-tauri/src/service_macos.rs:45`).
- Tray menu is present (Open/Start/Resync/Full Resync/Quit), but no close/minimize interception exists (`apps/desktop/src-tauri/src/lib.rs:329`, `apps/desktop/src-tauri/src/lib.rs:376`).
- Logs are available via OS facilities and repo recipes, not via UI (`Justfile:127`, `deploy/launchd/com.jirafs.mount.plist.tmpl:21`).
- Config schema already supports workspace JQL and validates non-empty entries (`src/config.rs:25`, `src/config.rs:230`), but desktop UI only displays paths (`apps/desktop/src/components/path-card.tsx:27`).
- Jira JQL execution path already exists and can be reused for validation (`src/jira.rs:222`, `src/warmup.rs:94`).

## Desired End State

Desktop UI supports:

1. One primary service button that shows `Start` when stopped and `Restart` when running.
2. A logs panel showing all service logs accumulated since the desktop app started.
3. Minimize-to-tray behavior when user closes the window, with restore from tray `Open`.
4. A structured form to manage only `jira.workspaces.<name>.jql`, validating each JQL against Jira API before save.

### Key Discoveries:
- Existing frontend action architecture already maps backend reason enums into user-friendly messages, which can be extended for restart semantics (`apps/desktop/src/components/actions-card.tsx:31`).
- Service stop/restart operations are available as shell-level patterns in repo workflow (`Justfile:83`, `Justfile:94`) and can be mirrored in Tauri backend.
- Jira client provides authenticated endpoints and robust error mapping suitable for pre-save validation (`src/jira.rs:546`, `src/jira.rs:576`, `src/jira.rs:74`).
- Config load/validate lifecycle is centralized and should remain source-of-truth (`src/config.rs:125`, `src/config.rs:205`).

### End State Verification

1. Service action button label/state changes correctly as service state changes.
2. Logs panel is empty at app boot and then accumulates service logs from that moment onward.
3. Closing the window hides it to tray; tray `Open` restores and focuses window.
4. Workspace JQL form rejects invalid JQL with clear errors and only persists validated entries.

## What We're NOT Doing

- Editing Jira credentials (`jira.base_url`, `jira.email`, `jira.api_token`) in desktop UI.
- Building historical log replay from before desktop app launch.
- Changing `jirafs` FUSE, sync, or cache core behavior.
- Adding Windows support.
- Replacing service install/enable workflows in `Justfile`.

## Implementation Approach

Use a command-contract extension strategy:

- Extend Tauri backend commands and DTOs in `apps/desktop/src-tauri/src/lib.rs`.
- Keep frontend thin: invoke wrappers + stateful hooks + focused cards/components.
- Reuse existing platform modules for service lifecycle and add explicit stop/restart methods.
- Reuse `src/config.rs` and `src/jira.rs` from desktop backend for config parsing/writing and JQL validation.
- Keep manual control safe: pre-validate all workspace JQL changes before persistence.

## Phase 1: Service Action Unification (Start/Restart)

### Overview
Add backend support for restart semantics and replace the static Start button with a dynamic Start/Restart control driven by service state.

### Changes Required:

#### 1. Service lifecycle backend extension
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**: Add command and reason enum for unified service action:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceActionReason {
    Started,
    Restarted,
    AlreadyRunningRestarted,
    ServiceNotInstalled,
    ActionFailed,
}

#[tauri::command]
fn ensure_service_running_or_restart(app: AppHandle) -> Result<ServiceActionResultDto, String> {
    // if running -> restart, else start
}
```

#### 2. Platform-specific stop/restart support
**Files**: `apps/desktop/src-tauri/src/service_linux.rs`, `apps/desktop/src-tauri/src/service_macos.rs`
**Changes**:
- Linux: add `restart_service()` using `systemctl --user restart jirafs.service`.
- macOS: add `restart_service()` via `launchctl kickstart -k gui/<uid>/com.jirafs.mount`.
- Keep timeout/error classification behavior aligned with `run_command_with_timeout` (`apps/desktop/src-tauri/src/errors.rs:29`).

#### 3. Frontend command/types update
**Files**: `apps/desktop/src/lib/tauri.ts`, `apps/desktop/src/types.ts`, `apps/desktop/src/hooks/use-app-status.ts`, `apps/desktop/src/components/actions-card.tsx`
**Changes**:
- Replace `startUserService()` action usage with unified action.
- Compute button label from `status.service_running`.
- Keep disable/busy behavior consistent with existing action wrapper (`apps/desktop/src/components/actions-card.tsx:58`).

### Success Criteria:

#### Automated Verification:
- [ ] Desktop frontend type/build checks pass: `npm --prefix apps/desktop run build`
- [ ] Desktop frontend lint passes: `npm --prefix apps/desktop run biome:check`
- [ ] Tauri backend tests pass: `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Tauri backend compiles cleanly: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] With service stopped, button label is `Start` and action starts service.
- [ ] With service running, button label is `Restart` and action restarts service.
- [ ] User receives clear result messages for start/restart outcomes.
- [ ] Tray tooltip/state remains accurate after action completion.

**Implementation Note**: After this phase and automated checks pass, pause for manual confirmation on both service states before continuing.

---

## Phase 2: In-App Logs Since UI Start

### Overview
Introduce a desktop-session log stream so UI displays all service log lines observed from app launch time onward.

### Changes Required:

#### 1. Backend log collector state
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**:
- Add app-managed shared log buffer state (e.g., `Arc<Mutex<Vec<LogLineDto>>>`) initialized in `run()` setup.
- Spawn platform-specific log reader thread during setup.
- Provide `#[tauri::command] fn get_session_logs(...)` returning accumulated lines (and optionally cursor-based incremental fetch).

```rust
#[derive(Debug, Clone, serde::Serialize)]
struct LogLineDto {
    ts: Option<String>,
    source: String,
    line: String,
}

#[tauri::command]
fn get_session_logs(state: tauri::State<LogBufferState>) -> Result<Vec<LogLineDto>, String> {
    // returns all lines captured since UI startup
}
```

#### 2. Platform log tail readers
**Files**: `apps/desktop/src-tauri/src/service_linux.rs`, `apps/desktop/src-tauri/src/service_macos.rs`
**Changes**:
- Linux: spawn `journalctl --user -u jirafs.service -f --output=short-iso` reader and append lines.
- macOS: tail both `~/Library/Logs/jirafs.log` and `~/Library/Logs/jirafs.err.log` with follow mode.
- Add graceful shutdown behavior when desktop exits.

#### 3. Frontend logs panel
**Files**: `apps/desktop/src/lib/tauri.ts`, `apps/desktop/src/types.ts`, `apps/desktop/src/App.tsx`, `apps/desktop/src/components/logs-card.tsx`, `apps/desktop/src/hooks/use-app-status.ts`
**Changes**:
- Add typed `getSessionLogs()` wrapper.
- Add polling cadence for log refresh (separate from status poll if needed).
- Render scrollable monospaced logs panel with clear empty/loading/error states.

### Success Criteria:

#### Automated Verification:
- [ ] Frontend build/lint checks pass: `npm --prefix apps/desktop run biome:check && npm --prefix apps/desktop run build`
- [ ] Tauri tests pass (including log-buffer unit tests): `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Tauri compile passes: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] On app launch, log panel starts empty (or from reader start point) and grows over time.
- [ ] Triggering sync/service actions produces new lines visible in panel.
- [ ] Logs persist in UI memory while app remains open.
- [ ] Restarting desktop app resets session logs and starts fresh capture.

**Implementation Note**: After this phase and automated checks pass, pause for manual confirmation that log behavior matches "since UI start" semantics.

---

## Phase 3: Minimize to Tray Behavior

### Overview
Change close-window behavior so desktop app stays resident in tray/menubar and can be reopened from tray.

### Changes Required:

#### 1. Window close interception
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**:
- Register window event handler to intercept close requests.
- Prevent process exit on close and hide window instead.
- Keep explicit tray `Quit` menu behavior (`apps/desktop/src-tauri/src/lib.rs:365`) as the termination path.

#### 2. Tray behavior polish
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**:
- Ensure tray `Open` reliably shows and focuses hidden window (`apps/desktop/src-tauri/src/lib.rs:351`).
- Optional menu copy update to reflect hidden/running state.

### Success Criteria:

#### Automated Verification:
- [ ] Tauri backend compiles and tests pass: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml && cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] Clicking window close hides app instead of quitting.
- [ ] Tray icon remains active after close.
- [ ] Tray `Open` restores and focuses window.
- [ ] Tray `Quit` exits app and terminates background log readers.

**Implementation Note**: After this phase and automated checks pass, pause for manual confirmation of tray lifecycle before config editor work proceeds.

---

## Phase 4: Structured Workspace JQL Editor with Jira Validation

### Overview
Add a UI form that edits only `jira.workspaces.<name>.jql`, validates candidate queries against Jira API before save, and then persists updated TOML config.

### Changes Required:

#### 1. Backend config read/write commands (workspace-only)
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**:
- Add command to load config and return workspace list:
  - resolve path from status/config resolver (`apps/desktop/src-tauri/src/lib.rs:206`, `src/config.rs:146`)
  - parse with `jirafs::config::load_from` (`src/config.rs:125`)
- Add save command to update only `jira.workspaces` map and preserve other config keys.
- Ensure post-write parse/validate succeeds via existing config validation path (`src/config.rs:205`).

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WorkspaceJqlInputDto {
    name: String,
    jql: String,
}

#[tauri::command]
fn get_workspace_jql_config(...) -> Result<Vec<WorkspaceJqlInputDto>, String> { /* ... */ }

#[tauri::command]
fn save_workspace_jql_config(..., workspaces: Vec<WorkspaceJqlInputDto>) -> Result<(), String> { /* ... */ }
```

#### 2. Backend Jira JQL validation command
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**:
- Add `validate_workspace_jqls` command that:
  - builds `JiraClient` from existing loaded config (`src/jira.rs:144`)
  - validates each workspace JQL via lightweight listing call (`src/jira.rs:222`)
  - returns structured per-workspace success/error output.
- Validation must run before save; save rejects if any query fails.

#### 3. Frontend structured form
**Files**: `apps/desktop/src/lib/tauri.ts`, `apps/desktop/src/types.ts`, `apps/desktop/src/components/workspaces-card.tsx`, `apps/desktop/src/App.tsx`, `apps/desktop/src/hooks/use-app-status.ts`
**Changes**:
- Add editable table/list for workspace name + JQL rows.
- Support add/remove workspace rows.
- Add "Validate" and "Save" flow with inline error reporting.
- Keep scope strictly to `workspaces.<name>.jql` and explicitly show credentials are reused from config.

### Success Criteria:

#### Automated Verification:
- [ ] Frontend lint/build checks pass: `npm --prefix apps/desktop run biome:check && npm --prefix apps/desktop run build`
- [ ] Backend tests pass (workspace parsing/validation/save): `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Backend compile/check passes: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Root project tests remain green: `cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Existing workspaces/JQL load into form accurately.
- [ ] Invalid JQL is rejected with workspace-specific error details.
- [ ] Valid JQL passes validation and can be saved.
- [ ] Saved config preserves non-workspace sections (`cache`, `sync`, `metrics`, `logging`, Jira credentials).
- [ ] Desktop status/actions continue to work after config save.

**Implementation Note**: After this phase and automated checks pass, pause for final manual verification of config safety and validation UX before rollout.

---

## Testing Strategy

### Unit Tests:
- Service lifecycle tests for unified start/restart decision behavior in desktop backend.
- Linux/macOS command-assembly tests for restart/log-tail command invocations.
- Log buffer tests for append ordering, bounded memory policy (if implemented), and read consistency.
- Config mutation tests proving only `jira.workspaces` changes while other TOML sections remain intact.
- JQL validation tests with mocked Jira responses for success, auth failure, and invalid-query failure.

### Integration Tests:
- Desktop backend command tests covering: status -> action -> status transitions.
- End-to-end command contract tests for `get_session_logs`, workspace validation, and workspace save.

### Manual Testing Steps:
1. Start desktop app with service stopped; verify action shows `Start` and starts service.
2. With service running, verify action switches to `Restart` and restart succeeds.
3. Trigger sync/full sync and confirm new log lines appear in logs panel.
4. Close window; confirm app remains in tray and can be reopened.
5. Edit a workspace JQL to invalid syntax; confirm validation blocks save.
6. Fix JQL, validate, save, and confirm config changes survive app restart.

## Performance Considerations

- Log collection should avoid unbounded growth; apply a practical in-memory cap (for example 5k-20k lines) while preserving "since UI start" semantics within cap.
- Keep status polling at current cadence (`apps/desktop/src/hooks/use-app-status.ts:9`) unless logs polling requires independent tuning.
- Jira validation should use lightweight listing endpoints and avoid full issue hydration.

## Migration Notes

- No migration needed for existing CLI/service users.
- Desktop users get additive capabilities; current service files and config schema remain valid.
- If workspace save writes normalized TOML ordering/format, document this as expected non-functional change.

## References

- Desktop command surface: `apps/desktop/src-tauri/src/lib.rs:83`
- Current action UI: `apps/desktop/src/components/actions-card.tsx:77`
- Status polling hook: `apps/desktop/src/hooks/use-app-status.ts:19`
- Linux service start: `apps/desktop/src-tauri/src/service_linux.rs:58`
- macOS service start: `apps/desktop/src-tauri/src/service_macos.rs:45`
- Tray menu/open/quit: `apps/desktop/src-tauri/src/lib.rs:329`
- Service log recipes: `Justfile:127`
- launchd log file paths: `deploy/launchd/com.jirafs.mount.plist.tmpl:21`
- Config load/validate: `src/config.rs:125`
- Jira JQL listing API call: `src/jira.rs:222`
