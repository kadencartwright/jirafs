# Cross-Platform Auto-Launch Service Implementation Plan

## Overview

Add first-class user-session auto-launch support for `jirafs` FUSE mounts on Linux (`systemd --user`) and macOS (`launchd` LaunchAgent), using the existing foreground runtime process and explicit service-managed startup arguments.

## Current State Analysis

`jirafs` already has stable mount/runtime primitives, but no service management surface:

- Runtime process entrypoint is CLI-first and foreground/blocking (`src/main.rs:39`, `src/main.rs:389`).
- A mountpoint positional argument is required and auto-created if missing (`src/main.rs:123`, `src/main.rs:289`).
- Config resolution depends on `XDG_CONFIG_HOME`/`HOME` unless `--config` is passed (`src/config.rs:146`, `src/config.rs:157`, `src/main.rs:272`).
- Install/run workflows are available via `just`, but no service install/enable commands exist (`Justfile:9`, `Justfile:13`, `Justfile:17`).
- README documents mount/unmount only; no systemd or launchd workflow is documented (`README.md:129`, `README.md:170`).
- Logging and metrics emit to stderr, which service managers can capture if configured (`src/logging.rs:27`, `src/metrics.rs:55`).

## Desired End State

Operators can install, enable, inspect, and remove a single user-level auto-start service instance for `jirafs` on Linux and macOS from repo-native commands. Services start at login, run as the current user, and mount by default to `~/jirafs` using explicit config and mountpoint arguments.

### Key Discoveries:
- Foreground process model maps directly to service manager supervision; no daemonization refactor needed (`src/main.rs:389`).
- Existing CLI supports explicit `--config` path, which avoids brittle service environment assumptions (`src/main.rs:53`, `src/main.rs:272`).
- Config path precedence logic already exists and should be reused in install scripts (`src/config.rs:156`, `src/config.rs:163`).
- Mount ownership derives from effective UID/GID, so user-level services are the correct default (`src/main.rs:373`, `src/fs.rs:147`).
- There are no existing `*.service` or `*.plist` artifacts, so templates can be introduced cleanly.

### Verification of End State

1. `just service-install` writes a valid user service file on Linux and macOS with resolved binary/config/mountpoint values.
2. `just service-enable` starts service successfully and mount appears at `~/jirafs`.
3. `just service-status` and `just service-logs` provide actionable runtime diagnostics.
4. `just service-disable` + `just service-uninstall` cleanly stop and remove managed service assets.

## What We're NOT Doing

- System-wide boot services (`/etc/systemd/system`, root launch daemons).
- Multi-instance service orchestration (single-instance only in this iteration).
- Runtime changes to FUSE operation handlers or sync architecture.
- Packaging/distribution work (Homebrew formula, system packages, installers).

## Implementation Approach

Add parameterized service templates and `Justfile` lifecycle recipes. Keep runtime stable by running the same binary/args path used today (`jirafs --config <path> <mountpoint>`). Favor explicit absolute paths in generated service definitions to avoid environment drift.

## Phase 1: Service Template Artifacts

### Overview

Introduce versioned templates for Linux and macOS user-session service definitions.

### Changes Required:

#### 1. Add systemd user service template
**File**: `deploy/systemd/jirafs.service.tmpl`
**Changes**: Add a template with placeholders for absolute binary path, config path, and mountpoint.

```ini
[Unit]
Description=jirafs FUSE mount
After=network-online.target

[Service]
Type=simple
ExecStart={{BIN_PATH}} --config {{CONFIG_PATH}} {{MOUNTPOINT}}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

#### 2. Add launchd LaunchAgent template
**File**: `deploy/launchd/com.jirafs.mount.plist.tmpl`
**Changes**: Add a template with `ProgramArguments`, `RunAtLoad`, and log file paths.

```xml
<key>ProgramArguments</key>
<array>
  <string>{{BIN_PATH}}</string>
  <string>--config</string>
  <string>{{CONFIG_PATH}}</string>
  <string>{{MOUNTPOINT}}</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
```

#### 3. Add operator-facing service docs section anchors
**File**: `README.md`
**Changes**: Reserve an "Auto-start Services" section that links to Linux/macOS lifecycle commands added in later phases.

### Success Criteria:

#### Automated Verification:
- [ ] Template files exist in repo: `test -f deploy/systemd/jirafs.service.tmpl && test -f deploy/launchd/com.jirafs.mount.plist.tmpl`
- [ ] Existing checks remain green: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] A maintainer can inspect each template and identify where binary, config, and mountpoint are injected.
- [ ] Template defaults align with per-user service scope and do not reference root-owned paths.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the template intent is correct before proceeding to the next phase.

---

## Phase 2: Justfile Service Lifecycle Commands

### Overview

Add cross-platform `just` recipes to install/enable/start/status/logs/stop/disable/uninstall user services.

### Changes Required:

#### 1. Add common path-resolution helper logic in `Justfile`
**File**: `Justfile`
**Changes**: Resolve `BIN_PATH`, `CONFIG_PATH`, and default `MOUNTPOINT` (`~/jirafs`) in shell-safe way with explicit errors.

```bash
bin_path="$(command -v jirafs || true)"
if [ -z "$bin_path" ]; then
  echo "jirafs binary not found; run just install" >&2
  exit 1
fi
```

#### 2. Add Linux recipes (`systemd --user`)
**File**: `Justfile`
**Changes**: Add Linux-specific recipes that render template to `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/jirafs.service`, then run `systemctl --user` lifecycle commands.

```just
service-enable:
    systemctl --user daemon-reload
    systemctl --user enable --now jirafs.service
```

#### 3. Add macOS recipes (`launchd`)
**File**: `Justfile`
**Changes**: Add macOS-specific recipes that render plist to `$HOME/Library/LaunchAgents/com.jirafs.mount.plist`, then run `launchctl bootstrap/bootout/print`.

```just
service-status:
    launchctl print gui/$(id -u)/com.jirafs.mount
```

#### 4. Add safe idempotency and guardrails
**File**: `Justfile`
**Changes**: Ensure reruns replace managed artifacts cleanly, reject unsupported OS, and preserve clear error messages.

### Success Criteria:

#### Automated Verification:
- [ ] Recipe list contains service commands: `just --list`
- [ ] Linux syntax path renders and reload command succeeds on Linux host: `just service-install && just service-enable`
- [ ] macOS syntax path renders and bootstrap command succeeds on macOS host: `just service-install && just service-enable`
- [ ] Existing quality gates still pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Default install flow mounts at `~/jirafs` when no mountpoint argument is passed.
- [ ] `just service-status` shows a running unit/agent after enable.
- [ ] `just service-logs` shows runtime startup lines (including mount path) from stderr-backed logs.
- [ ] Re-running install with same inputs is predictable and does not create duplicate service names.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that enable/status/logs workflows are acceptable before proceeding to the next phase.

---

## Phase 3: Documentation and Troubleshooting Guide

### Overview

Document full operator workflow and service-specific caveats so setup is reproducible on both platforms.

### Changes Required:

#### 1. Add Linux/macOS service lifecycle sections
**File**: `README.md`
**Changes**: Document install/enable/status/logs/disable/uninstall command sequences for both OSes.

#### 2. Document path and environment behavior
**File**: `README.md`
**Changes**: Explain config resolution (`XDG_CONFIG_HOME`/`HOME`), default mountpoint (`~/jirafs`), and recommendation to avoid `/tmp` for service mounts.

#### 3. Add troubleshooting runbook
**File**: `README.md`
**Changes**: Add quick fixes for missing config, missing binary, stale mountpoint, and manual unmount commands (`fusermount3 -u` or `umount`).

### Success Criteria:

#### Automated Verification:
- [ ] README includes service section and platform-specific command examples: `grep -n "Auto-start Services" README.md`
- [ ] README examples align with available recipes: `just --list`

#### Manual Verification:
- [ ] New contributor can install and start auto-launch service from README without external notes.
- [ ] Contributor can identify log location and service manager commands for their OS.
- [ ] Stop/uninstall flow leaves no active user service and no stale managed unit/plist artifact.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that docs are clear and complete before proceeding to the next phase.

---

## Phase 4: Validation and Regression Coverage

### Overview

Add lightweight checks so service workflow regressions are caught early.

### Changes Required:

#### 1. Add CI sanity checks for template existence and docs references
**File**: `.github/workflows/ci.yml`
**Changes**: Add a fast step to verify service template files exist and `just --list` includes service commands.

#### 2. Add focused tests for path-resolution shell logic where practical
**File**: `Justfile` (and optional shell helper under `scripts/`)
**Changes**: If logic grows complex, extract path rendering to script and test with simple fixture env setups.

#### 3. Confirm runtime behavior unchanged
**File**: `src/main.rs` (no functional change expected)
**Changes**: Keep runtime mount pipeline untouched; only verify commands still invoke existing CLI contract.

### Success Criteria:

#### Automated Verification:
- [ ] CI validates template presence and command surface: `test -f deploy/systemd/jirafs.service.tmpl && test -f deploy/launchd/com.jirafs.mount.plist.tmpl && just --list`
- [ ] Full project quality gates pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Linux host login restart confirms user service remount behavior.
- [ ] macOS logout/login confirms LaunchAgent remount behavior.
- [ ] Manual unmount + service restart recovers mount without editing service files.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for final human sign-off on one full login-cycle validation per OS.

---

## Testing Strategy

### Unit Tests:
- Keep existing CLI/config tests as source of truth for runtime startup contracts (`src/main.rs:398`, `src/main.rs:410`, `src/config.rs:283`).
- If path rendering moves to script/helper, add deterministic tests for XDG/HOME precedence and mountpoint expansion.

### Integration Tests:
- Validate lifecycle recipes on Linux and macOS hosts:
  - `just service-install`
  - `just service-enable`
  - `just service-status`
  - `just service-disable`
  - `just service-uninstall`
- Validate mount readability post-start (`ls ~/jirafs`, `ls ~/jirafs/workspaces`).

### Manual Testing Steps:
1. Run `just install` and confirm `jirafs` is on `PATH`.
2. Run `just service-install` with defaults and inspect rendered service file contents.
3. Run `just service-enable`, then verify mount at `~/jirafs` and read one issue markdown file.
4. Restart user session and verify mount auto-returns.
5. Run `just service-disable && just service-uninstall` and verify no active service remains.

## Performance Considerations

- Service support is control-plane only; no expected change to request-path performance in FUSE handlers.
- `Restart=on-failure`/`KeepAlive` should use conservative restart behavior to avoid rapid loops during transient config/network errors.
- Logging remains stderr-based; ensure service log retention settings are acceptable for long-running sessions.

## Migration Notes

- Existing `just run` and raw `cargo run` workflows remain valid and unchanged.
- Existing config files remain valid; service flow only standardizes startup transport.
- Users currently mounting under `/tmp` can continue, but default service path is now `~/jirafs` for stability across sessions.

## References

- CLI contract and required mountpoint: `src/main.rs:17`, `src/main.rs:123`
- Config loading and path precedence: `src/main.rs:272`, `src/config.rs:146`, `src/config.rs:157`, `src/config.rs:163`
- Foreground mount lifecycle: `src/main.rs:389`
- Existing run/install recipes: `Justfile:9`, `Justfile:13`, `Justfile:17`
- Existing mount/unmount docs: `README.md:129`, `README.md:170`
- Logging/metrics emission to stderr: `src/logging.rs:27`, `src/metrics.rs:55`
