# jirafs Tauri Desktop UI Implementation Plan

## Overview

Build a separate Tauri desktop wrapper for `jirafs` with a Linux system tray / macOS menubar presence, and a simple React config/status UI. The first version is intentionally operational (status + actions), not a full config editor.

## Current State Analysis

`jirafs` already has the runtime primitives we need for a control UI, but no desktop surface:

- Runtime sync state is exposed through writable/readable control files under `.sync_meta` (`src/fs.rs:272`, `src/fs.rs:359`, `src/fs.rs:368`, `src/fs.rs:712`).
- Manual resync and full resync are already implemented via writes to `.sync_meta/manual_refresh` and `.sync_meta/full_refresh` (`src/fs.rs:739`, `src/fs.rs:742`).
- Service lifecycle and path resolution are implemented through cross-platform `just` recipes (`Justfile:17`, `Justfile:68`, `Justfile:116`).
- Runtime config path precedence is deterministic and reusable (`src/config.rs:146`, `src/config.rs:156`, `src/config.rs:163`).
- There is no existing UI workspace, no Tauri crate, and no frontend toolchain in-repo.

## Desired End State

Add a new `apps/desktop` Tauri app that can run on Linux and macOS, display runtime status, show config + mount folder location, and trigger incremental/full sync actions against the mounted `.sync_meta` files.

### Key Discoveries:
- Sync state and triggers are already externally controllable via filesystem operations, so no `jirafs` core refactor is required (`src/fs.rs:272`, `src/fs.rs:737`).
- Service files contain explicit startup arguments (`--config` and mountpoint), which can be parsed to discover folder/config locations (`deploy/systemd/jirafs.service.tmpl:8`, `deploy/launchd/com.jirafs.mount.plist.tmpl:11`).
- A desktop wrapper aligns with current architecture because `jirafs` is foreground/blocking and already service-managed (`src/main.rs:389`, `Justfile:17`).

### End State Verification

1. A tray/menubar icon appears and reflects runtime state (`Stopped`, `Running`, `Syncing`, `Degraded`).
2. UI shows sync status, config path, and mount folder path.
3. UI can trigger `Resync` and `Full Resync` successfully.
4. Linux and macOS both work with platform-specific service detection.

## What We're NOT Doing

- No in-UI editing/saving of TOML config values in this iteration.
- No replacement of existing `jirafs` runtime/service architecture.
- No Windows support.
- No binary packaging/notarization/distribution workflow.
- No redesign of FUSE or sync internals.

## Implementation Approach

Create a new standalone desktop workspace under `apps/desktop`:

- **Backend (Tauri/Rust)**: discover service/runtime state, read sync metadata, trigger sync actions, and manage tray/menu behavior.
- **Frontend (React/Vite SPA)**: render status/config panels with shadcn components and Tailwind styling.
- **Contract-first**: keep a small typed IPC surface between frontend and backend.
- **Polling model**: use periodic status refresh (for example every 5s) plus immediate refresh after actions.

### CI Command Decisions (Refined for this repo)

- Keep root Rust checks as-is (already established in `.github/workflows/ci.yml`).
- Pin toolchains across CI and docs: Node.js `20.12.2`, Rust `1.84.0`.
- Add desktop checks as independent CI steps using explicit working directories:
  - `npm --prefix apps/desktop ci`
  - `npm --prefix apps/desktop run biome:check`
  - `npm --prefix apps/desktop run build`
  - `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
  - `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`
  - `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- For Linux CI where tray dependencies are needed, install before desktop backend checks:
  - `sudo apt-get update && sudo apt-get install -y libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
- Keep macOS CI dependency setup minimal unless compilation requires additional GUI libs.

## Phase 1: Bootstrap Desktop Workspace

### Overview

Create a new Tauri + React + TypeScript workspace with Tailwind, shadcn/ui, and Biome so implementation can proceed with a stable baseline.

### Changes Required:

#### 1. Create desktop app scaffolding
**Files**: `apps/desktop/package.json`, `apps/desktop/src/main.tsx`, `apps/desktop/src/App.tsx`, `apps/desktop/index.html`
**Changes**: Add Vite React TS SPA entrypoint and base app shell.

```json
{
  "name": "jirafs-desktop",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build",
    "biome:check": "biome check .",
    "biome:fix": "biome check --write ."
  }
}
```

#### 2. Add styling and component tooling
**Files**: `apps/desktop/tailwind.config.ts`, `apps/desktop/postcss.config.cjs`, `apps/desktop/src/index.css`, `apps/desktop/components.json`
**Changes**: Enable Tailwind and shadcn component generation/config.

#### 3. Add formatting/linting baseline
**Files**: `apps/desktop/biome.json`, `apps/desktop/tsconfig.json`, `apps/desktop/vite.config.ts`
**Changes**: Standardize TS and Biome checks for the new workspace.

#### 4. Add Tauri backend shell
**Files**: `apps/desktop/src-tauri/Cargo.toml`, `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/src-tauri/src/main.rs`, `apps/desktop/src-tauri/src/lib.rs`
**Changes**: Minimal Tauri app boot with command registration and window startup.

### Success Criteria:

#### Automated Verification:
- [ ] Frontend dependencies install cleanly in CI mode: `npm --prefix apps/desktop ci`
- [ ] Frontend lint/format checks pass: `npm --prefix apps/desktop run biome:check`
- [ ] Frontend type/build checks pass: `npm --prefix apps/desktop run build`
- [ ] Tauri crate compiles with lockfile enforcement: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] `npm --prefix apps/desktop run tauri:dev` opens desktop window on Linux.
- [ ] `npm --prefix apps/desktop run tauri:dev` opens desktop window on macOS.
- [ ] App shell renders without runtime console errors.

**Implementation Note**: After completing this phase and all automated verification passes, pause for manual confirmation before implementing status/action logic.

---

## Phase 2: Implement Backend Runtime/Service Adapter and IPC

### Overview

Implement Tauri backend commands that normalize Linux/macOS service state, resolve config/mount paths, read sync metadata, and trigger manual/full resync.

### Changes Required:

#### 1. Define shared DTOs and command contract
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**: Add serializable DTOs and command handlers, including an explicit `sync_state` contract and structured sync action results.

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum SyncStateValue {
    Stopped,
    Running,
    Syncing,
    Degraded,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum PathSource {
    ServiceArgs,
    KnownDefaults,
    ConfigResolver,
}

#[derive(serde::Serialize)]
struct AppStatusDto {
    platform: String,
    service_installed: bool,
    service_running: bool,
    sync_state: SyncStateValue,
    config_path: Option<String>,
    mountpoint: Option<String>,
    path_source: PathSource,
    sync: SyncStatusDto,
    errors: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum TriggerReason {
    Accepted,
    AlreadySyncing,
    ServiceNotRunning,
    MountpointUnavailable,
    TriggerWriteFailed,
}

#[derive(serde::Serialize)]
struct TriggerSyncResultDto {
    accepted: bool,
    reason: TriggerReason,
}

#[tauri::command]
fn get_app_status() -> Result<AppStatusDto, String> { /* ... */ }

#[tauri::command]
fn trigger_sync(kind: String) -> Result<TriggerSyncResultDto, String> { /* ... */ }
```

#### 2. Add Linux and macOS service discovery
**Files**: `apps/desktop/src-tauri/src/service_linux.rs`, `apps/desktop/src-tauri/src/service_macos.rs`
**Changes**: Query running state and parse runtime args from systemd user unit or launchd plist with explicit path precedence and probe timeouts.

Path discovery precedence (consistent on both platforms):
1. Parse service arguments (`--config` and mountpoint) from unit/plist.
2. If absent, use known service defaults (for example `~/jirafs` mountpoint).
3. If still unresolved, use runtime-equivalent config resolver logic (`src/config.rs:146`).

Include `path_source` in `AppStatusDto` (`service_args`, `known_defaults`, `config_resolver`) for debugging and UI diagnostics.

#### 3. Add sync metadata reader and trigger writer
**File**: `apps/desktop/src-tauri/src/sync_meta.rs`
**Changes**: Read `.sync_meta/{last_sync,last_full_sync,seconds_to_next_sync,manual_refresh}` and write `1` to `manual_refresh`/`full_refresh`.

#### 4. Add robust error classification
**File**: `apps/desktop/src-tauri/src/errors.rs`
**Changes**: Map path/service/probe failures into stable buckets with timeout support.

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceProbeErrorKind {
    Permission,
    NotInstalled,
    Unreachable,
    ParseError,
}
```

All service probe operations should enforce a bounded timeout (for example 2 seconds) and return one of these buckets.

### Success Criteria:

#### Automated Verification:
- [ ] Backend unit tests pass: `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Parsing tests cover unit/plist argument extraction.
- [ ] Backend compiles with no warnings: `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`

#### Manual Verification:
- [ ] With service running, `get_app_status` returns running=true and non-empty config/mount paths.
- [ ] With service stopped, status returns running=false and actions are rejected with clear errors.
- [ ] `sync_state` always returns one of `stopped|running|syncing|degraded` and matches backend reality.
- [ ] `path_source` accurately reports whether paths came from service args, defaults, or config resolver.
- [ ] Triggering incremental/full sync is idempotent and returns structured `{accepted, reason}` responses.
- [ ] Triggering incremental/full sync writes to the expected `.sync_meta` files when `accepted=true`.

**Implementation Note**: After this phase, pause for human confirmation that status/action responses are accurate on at least one real host.

---

## Phase 3: Build React Config/Status UI

### Overview

Create the simple SPA interface showing sync status and paths, plus resync/full-resync controls.

### Changes Required:

#### 1. Add typed frontend API layer
**Files**: `apps/desktop/src/lib/tauri.ts`, `apps/desktop/src/types.ts`
**Changes**: Add typed `invoke` wrappers and DTO definitions shared across UI components.

#### 2. Implement status/config layout
**Files**: `apps/desktop/src/App.tsx`, `apps/desktop/src/components/status-card.tsx`, `apps/desktop/src/components/path-card.tsx`
**Changes**: Render service/sync state badges, last sync info, and folder/config paths.

#### 3. Implement resync action controls
**Files**: `apps/desktop/src/components/actions-card.tsx`, `apps/desktop/src/components/full-resync-dialog.tsx`
**Changes**: Add `Resync` and `Full Resync` (confirm dialog), disabled while sync in progress.

#### 4. Implement polling and optimistic refresh
**File**: `apps/desktop/src/hooks/use-app-status.ts`
**Changes**: Poll `get_app_status` on interval and refresh immediately after actions.

### Success Criteria:

#### Automated Verification:
- [ ] Frontend lint/format checks pass: `npm --prefix apps/desktop run biome:check`
- [ ] Frontend build passes: `npm --prefix apps/desktop run build`
- [ ] TypeScript compile passes with no type errors (included in build script): `npm --prefix apps/desktop run build`

#### Manual Verification:
- [ ] UI shows current sync status and updates without manual reload.
- [ ] `Resync` action triggers incremental sync and UI transitions to syncing/running states.
- [ ] `Full Resync` requires confirmation and then triggers full sync.
- [ ] Config path and mount folder location render correctly.
- [ ] First-run/no-service-installed state is explicit and actionable, with sync actions disabled.

**Implementation Note**: After this phase, pause for manual UX verification (button states, errors, and confirm flow) before tray integration polish.

---

## Phase 4: Tray/Menubar Status and Integration Polish

### Overview

Add system tray/menubar status behavior, expose quick actions, and finalize docs + CI checks for the desktop workspace.

### Changes Required:

#### 1. Implement tray/menubar state indicator
**Files**: `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/icons/*`
**Changes**: Add status-driven icon/tooltip updates for `Stopped`, `Running`, `Syncing`, and `Degraded`.

#### 2. Add tray menu actions
**File**: `apps/desktop/src-tauri/src/lib.rs`
**Changes**: Add `Open`, `Resync`, `Full Resync`, and `Quit` tray menu commands.

#### 3. Document developer workflow
**Files**: `README.md`, optionally `Justfile`
**Changes**: Add desktop app run/build instructions, cross-platform prerequisites, and explicit pinned toolchain versions that match CI.

Pin versions in docs to prevent local/CI drift:
- Node.js `20.12.2`
- Rust `1.84.0`

#### 4. Add CI checks for desktop workspace
**File**: `.github/workflows/ci.yml`
**Changes**: Add frontend build/lint and Tauri backend compile/test checks with explicit dependency setup for Linux tray builds.

```yaml
# Patch sketch for .github/workflows/ci.yml
jobs:
  checks:
    env:
      NODE_VERSION: '20.12.2'
      RUST_VERSION: '1.84.0'
    steps:
      - uses: actions/checkout@v4

      - name: Install Linux runtime deps
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libfuse3-dev pkg-config

      - name: Install Linux tray deps for Tauri
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev

      - name: Install macOS dependencies
        if: runner.os == 'macOS'
        run: brew install macfuse pkgconf

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: ${{ env.RUST_VERSION }}
          components: rustfmt, clippy

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: ${{ env.NODE_VERSION }}
          cache: 'npm'
          cache-dependency-path: apps/desktop/package-lock.json

      - name: Desktop frontend checks
        run: |
          npm --prefix apps/desktop ci
          npm --prefix apps/desktop run biome:check
          npm --prefix apps/desktop run build

      - name: Desktop Tauri backend checks
        run: |
          cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
          cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
          cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
```

### Success Criteria:

#### Automated Verification:
- [ ] Desktop frontend checks run in CI: `npm --prefix apps/desktop ci && npm --prefix apps/desktop run biome:check && npm --prefix apps/desktop run build`
- [ ] Tauri backend checks run in CI: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml && cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings && cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Linux runner installs tray dependencies before backend checks: `sudo apt-get update && sudo apt-get install -y libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
- [ ] README pins Node/Rust versions equal to CI (`20.12.2` and `1.84.0`).
- [ ] Existing root project quality gates remain green: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Linux tray icon appears and menu actions work.
- [ ] macOS menubar icon appears and menu actions work.
- [ ] Tray status reflects sync transitions while actions run.
- [ ] App is usable from tray without requiring a persistent visible window.

**Implementation Note**: After this phase, pause for final human sign-off on Linux and macOS behavior before any packaging follow-up work.

---

## Testing Strategy

### Unit Tests:
- Service parser tests for systemd ExecStart and launchd ProgramArguments extraction.
- Sync metadata parse tests for missing/malformed values.
- Status reducer tests mapping probe results to `Stopped/Running/Syncing/Degraded`.
- Service probe timeout/error-bucket tests for `permission|not_installed|unreachable|parse_error` classification.

### Integration Tests:
- Backend command integration tests for `get_app_status` and `trigger_sync` with fixture data/mocked paths.
- Frontend component tests for action-disabled states and error banners (if test harness is added).

### Manual Testing Steps:
1. Start `jirafs` service and verify status fields populate.
2. Trigger `Resync`; verify sync-in-progress state and subsequent completion state.
3. Trigger `Full Resync`; verify confirmation gate and completion state.
4. Stop service; verify UI moves to stopped/degraded state and actions fail gracefully.
5. Validate tray/menubar behavior on both Linux and macOS.

## Performance Considerations

- Polling interval should remain moderate (for example 5 seconds) to avoid unnecessary command/file churn.
- Sync action calls are constant-time file writes and should not add measurable runtime overhead.
- Keep service probing lightweight and cache recent state where practical.

## Migration Notes

- No migration needed for existing `jirafs` runtime users.
- Desktop app is additive and optional.
- Existing `just` service workflows remain source of truth for install/enable/disable.

## References

- Runtime sync control files: `src/fs.rs:272`
- Manual/full sync triggers: `src/fs.rs:737`
- Service management recipes: `Justfile:17`
- Config path precedence: `src/config.rs:146`
- systemd service template: `deploy/systemd/jirafs.service.tmpl:8`
- launchd service template: `deploy/launchd/com.jirafs.mount.plist.tmpl:11`
