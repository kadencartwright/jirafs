# Rename `jirafs` to `jirafs` (Hard Cutover) Implementation Plan

## Overview

Perform a hard-cutover rename of all `jirafs` references to `jirafs` across runtime code, desktop app surfaces, service artifacts, CI checks, docs, and generated metadata. There will be no backward compatibility for legacy names, paths, service labels, or filenames.

## Current State Analysis

`jirafs` is currently embedded as a cross-cutting identity in:

- Rust package/crate naming and import paths (`Cargo.toml:2`, `src/main.rs:8`)
- Runtime defaults for config and mount paths (`src/config.rs:157`, `apps/desktop/src-tauri/src/lib.rs:529`)
- Service/unit/label identity across Linux/macOS (`Justfile:51`, `apps/desktop/src-tauri/src/service_linux.rs:13`, `apps/desktop/src-tauri/src/service_macos.rs:11`)
- Deployment template filenames and CI assertions (`deploy/systemd/jirafs.service.tmpl:1`, `.github/workflows/ci.yml:26`)
- Desktop product/package identifiers (`apps/desktop/src-tauri/tauri.conf.json:5`, `apps/desktop/package.json:2`)
- User-facing docs/examples and tests (`README.md:1`, `src/main.rs:492`, `src/logging.rs:82`)

Because naming is used in runtime-sensitive paths and service orchestration, this change must be phased to avoid hidden mismatches.

## Desired End State

The repository, binaries, service assets, desktop artifacts, and documentation use `jirafs` exclusively. Running a repository-wide search for `jirafs`, `jirafs`, and `JIRAFS` returns no hits outside historical git metadata.

### Key Discoveries:
- Crate/package identity and code imports are currently tied to `jirafs`/`jirafs` (`Cargo.toml:2`, `src/main.rs:8`, `apps/desktop/src-tauri/Cargo.toml:20`).
- Service lifecycle commands and probes depend on exact old unit/label names (`Justfile:72`, `apps/desktop/src-tauri/src/service_linux.rs:13`, `apps/desktop/src-tauri/src/service_macos.rs:11`).
- Config path resolution currently hardcodes `~/.config/jirafs/config.toml` and XDG equivalent (`src/config.rs:157`, `src/config.rs:165`).
- CI currently asserts old template file names and will fail after file renames unless updated (`.github/workflows/ci.yml:26`, `.github/workflows/ci.yml:27`).

## What We're NOT Doing

- No compatibility shim for old config directories (`~/.config/jirafs`), service names, or labels.
- No migration helper that copies legacy files to new paths.
- No partial aliasing where both names remain supported.
- No unrelated feature work in FUSE behavior, sync logic, or Jira API behavior.

## Implementation Approach

Apply a strict identity swap in dependency order:

1. Core naming primitives first (crate/package/import/runtime strings).
2. Service/template/CI naming next (Linux/macOS orchestration consistency).
3. Desktop package/bundle naming and launch artifacts.
4. Docs/tests/generated files cleanup.
5. End-to-end verification and final static search enforcement.

## Phase 1: Core Runtime Identity Rename

### Overview
Rename the root package/crate identity and runtime default strings to `jirafs` so downstream desktop and service code can reference the new canonical name.

### Changes Required:

#### 1. Root crate/package identity
**File**: `Cargo.toml`
**Changes**: Rename package from `jirafs` to `jirafs`.

```toml
[package]
name = "jirafs"
```

#### 2. Runtime import paths, FS name, log banner, and CLI test fixtures
**File**: `src/main.rs`
**Changes**: Update `use jirafs::...` to `use jirafs::...`; rename `MountOption::FSName("jirafs")` and all hardcoded program-name fixtures/log labels.

```rust
use jirafs::cache::InMemoryCache;

MountOption::FSName("jirafs".to_string())
```

#### 3. Config resolver and missing-config messaging
**File**: `src/config.rs`
**Changes**: Replace `jirafs` directory components with `jirafs` in XDG/HOME resolution and related error text/tests.

```rust
PathBuf::from(dir).join("jirafs").join("config.toml")
```

#### 4. Crate docs and logging test text
**Files**: `src/lib.rs`, `src/logging.rs`
**Changes**: Update remaining hardcoded references in docs/tests to `jirafs`.

### Success Criteria:

#### Automated Verification:
- [ ] Root compile succeeds with new crate identity: `cargo check --locked`
- [ ] Root tests pass after import/path/string updates: `cargo test --all-targets --all-features --locked`
- [ ] Root lint passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] Startup logs identify runtime as `jirafs`.
- [ ] Default config resolution now points to `$XDG_CONFIG_HOME/jirafs/config.toml` or `~/.config/jirafs/config.toml`.
- [ ] Mount options include `FSName("jirafs")` and mount behavior remains unchanged.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that manual testing was successful before proceeding.

---

## Phase 2: Service Artifacts and OS Integration Rename

### Overview
Rename all service artifacts and service-management references so Linux and macOS lifecycle operations target `jirafs` consistently.

### Changes Required:

#### 1. Rename service template files and internal labels
**Files**: `deploy/systemd/jirafs.service.tmpl`, `deploy/launchd/com.jirafs.mount.plist.tmpl`
**Changes**:
- Rename systemd template file to `deploy/systemd/jirafs.service.tmpl` and update unit description text.
- Rename launchd template file to `deploy/launchd/com.jirafs.mount.plist.tmpl` and update label + log filenames.

```ini
Description=jirafs FUSE mount
```

```xml
<string>com.jirafs.mount</string>
<string>__HOME_DIR__/Library/Logs/jirafs.log</string>
```

#### 2. Update Justfile service/install paths and binary names
**File**: `Justfile`
**Changes**: Replace all old binary/unit/label/path/app names and generated desktop launcher values with `jirafs` equivalents, including:
- `command -v jirafs`
- `jirafs.service`
- `com.jirafs.mount.plist`
- `~/jirafs`
- desktop launcher/app bundle names using `jirafs-desktop`

#### 3. Update desktop service probes and parsers
**Files**: `apps/desktop/src-tauri/src/service_linux.rs`, `apps/desktop/src-tauri/src/service_macos.rs`
**Changes**: Replace constants, parser heuristics, log file names, and test fixtures to match new unit/label/binary names.

```rust
const SYSTEMD_UNIT_NAME: &str = "jirafs.service";
const LAUNCHD_LABEL: &str = "com.jirafs.mount";
```

#### 4. Update CI service artifact checks
**File**: `.github/workflows/ci.yml`
**Changes**: Adjust sanity-check paths to renamed template files.

### Success Criteria:

#### Automated Verification:
- [ ] CI artifact paths match new names: `test -f deploy/systemd/jirafs.service.tmpl && test -f deploy/launchd/com.jirafs.mount.plist.tmpl`
- [ ] `just --list` still exposes service lifecycle recipes: `just --list`
- [ ] Desktop backend compiles/tests with new service constants: `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] `just service-install` generates `jirafs` service artifacts on Linux/macOS.
- [ ] `just service-enable`, `service-status`, `service-logs`, `service-uninstall` all target `jirafs` names and work as expected.
- [ ] macOS logs are written/read from `~/Library/Logs/jirafs.log` and `~/Library/Logs/jirafs.err.log`.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that manual testing was successful before proceeding.

---

## Phase 3: Desktop Product and Package Identity Rename

### Overview
Rename desktop package/product identifiers to `jirafs` so build artifacts, app launchers, and UI branding are consistent with runtime changes.

### Changes Required:

#### 1. Rename desktop Rust and npm package identities
**Files**: `apps/desktop/src-tauri/Cargo.toml`, `apps/desktop/package.json`
**Changes**:
- Rename desktop crate/package (`jirafs-desktop` -> `jirafs-desktop`).
- Update root dependency key (`jirafs` -> `jirafs`).
- Update library crate symbol name as needed (`jirafs_desktop_lib` -> `jirafs_desktop_lib`).

#### 2. Update Tauri metadata and window/product labels
**File**: `apps/desktop/src-tauri/tauri.conf.json`
**Changes**: Replace product title and app identifier with `jirafs` values.

```json
{
  "productName": "jirafs Desktop",
  "identifier": "com.jirafs.desktop"
}
```

#### 3. Update desktop UI titles/labels
**Files**: `apps/desktop/index.html`, `apps/desktop/src/App.tsx`
**Changes**: Replace user-facing `jirafs` labels with `jirafs`.

#### 4. Update desktop backend references to root crate and defaults
**Files**: `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/main.rs`, `apps/desktop/src-tauri/src/sync_meta.rs`
**Changes**:
- Replace `jirafs::...` usage with `jirafs::...`.
- Rename hardcoded default mount folder and temp filename prefixes to `jirafs`.

### Success Criteria:

#### Automated Verification:
- [ ] Desktop frontend builds with renamed package identity: `npm --prefix apps/desktop run build`
- [ ] Desktop frontend lint passes: `npm --prefix apps/desktop run biome:check`
- [ ] Desktop backend check passes: `cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Desktop backend clippy/tests pass: `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings && cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`

#### Manual Verification:
- [ ] Desktop app title, tray tooltip, and product naming show `jirafs` only.
- [ ] Linux launcher/macOS app bundle names and identifiers use `jirafs` values.
- [ ] Desktop app still probes service status and sync metadata correctly after rename.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that manual testing was successful before proceeding.

---

## Phase 4: Documentation, Tests, and Generated Metadata Cleanup

### Overview
Remove remaining `jirafs` references from docs, test fixtures, and generated lock metadata so the repository reflects hard cutover.

### Changes Required:

#### 1. Update docs and examples
**Files**: `README.md`, `.env.example`, `config.example.toml`
**Changes**: Replace all command/path/name examples and environment-key mapping examples with `jirafs` naming.

#### 2. Update historical planning/research docs in `thoughts/shared`
**Files**: `thoughts/shared/plans/*.md`, `thoughts/shared/research/*.md` (files containing `jirafs`)
**Changes**: Replace historical references where requested by scope (user requested removing every reference).

#### 3. Regenerate lockfiles after package renames
**Files**: `Cargo.lock`, `apps/desktop/src-tauri/Cargo.lock`, `apps/desktop/package-lock.json`
**Changes**: Rebuild lockfiles so package names are synchronized with renamed crate/package metadata.

### Success Criteria:

#### Automated Verification:
- [ ] Root lockfile is updated consistently with new package name: `cargo check --locked`
- [ ] Desktop lockfiles are consistent: `npm --prefix apps/desktop ci && cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] No `jirafs` style references remain in tracked files: `rg "jirafs|jirafs|JIRAFS" .`

#### Manual Verification:
- [ ] README onboarding flow works end-to-end using only `jirafs` names.
- [ ] New contributors do not encounter legacy `jirafs` commands/paths in docs.
- [ ] Historical/internal docs no longer mention old name where scope requires full removal.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that manual testing was successful before proceeding.

---

## Phase 5: Final Hard-Cutover Validation

### Overview
Run full project validation and targeted runtime smoke tests to confirm the rename is complete and operational.

### Changes Required:

#### 1. Full quality-gate run
**Files**: repo-wide
**Changes**: No code changes; execute final validation matrix.

#### 2. Service lifecycle smoke tests on host OS
**Files**: repo-wide
**Changes**: Validate `just` service commands and mount behavior with new names.

#### 3. Rename completeness audit
**Files**: repo-wide
**Changes**: Final static search and spot checks for template names, labels, binary names, and path defaults.

### Success Criteria:

#### Automated Verification:
- [ ] Root quality gates pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`
- [ ] Desktop quality gates pass: `npm --prefix apps/desktop run biome:check && npm --prefix apps/desktop run build && cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings && cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Service artifact sanity check passes under new filenames: `test -f deploy/systemd/jirafs.service.tmpl && test -f deploy/launchd/com.jirafs.mount.plist.tmpl`
- [ ] Rename completeness search is clean: `rg "jirafs|jirafs|JIRAFS" .`

#### Manual Verification:
- [ ] `just install` installs `jirafs` CLI and desktop launcher artifacts.
- [ ] `just service-install && just service-enable` starts the new `jirafs` service and mounts successfully.
- [ ] Desktop app can start/restart service and trigger sync actions after rename.
- [ ] End-user visible surfaces (logs, tray labels, app names, docs) show only `jirafs`.

**Implementation Note**: After completing this phase and all automated verification passes, pause for final human sign-off confirming hard cutover acceptance.

## Testing Strategy

### Unit Tests:
- Update all string/fixture assertions that include binary names, FS names, unit labels, and default paths.
- Keep existing behavior tests intact while replacing name constants.

### Integration Tests:
- Service parser tests for Linux/macOS must parse `jirafs` unit/label values.
- Desktop backend command tests should validate renamed defaults and service constants.
- Root startup/config tests should validate new XDG/HOME target paths under `jirafs`.

### Manual Testing Steps:
1. Run `just install` and confirm CLI + desktop binaries are named with `jirafs`.
2. Run `just service-install` and inspect generated unit/plist filenames and labels.
3. Run `just service-enable` and verify mount appears at default `~/jirafs`.
4. Open desktop app and confirm status/tray labels and service actions function.
5. Run final `rg "jirafs|jirafs|JIRAFS" .` and confirm no code/docs references remain.

## Performance Considerations

- Rename-only changes should not alter runtime performance materially.
- Revalidating service/log path constants is important to avoid runtime polling/log tail failures caused by renamed files.
- Lockfile regeneration may change dependency ordering but should not alter dependency graph intent.

## Migration Notes

- This is a strict hard cutover: old service files, config directories, and binary names are intentionally unsupported.
- Operators must reinstall/recreate service artifacts under the new names (`jirafs.service`, `com.jirafs.mount.plist`).
- Existing configs under `~/.config/jirafs/` must be manually moved to `~/.config/jirafs/` before runtime use.

## References

- Root package identity: `Cargo.toml:2`
- Runtime import/name usage: `src/main.rs:8`
- Config path resolution: `src/config.rs:157`
- Linux service unit usage: `Justfile:51`
- macOS launchd label usage: `deploy/launchd/com.jirafs.mount.plist.tmpl:6`
- Desktop dependency on root crate: `apps/desktop/src-tauri/Cargo.toml:20`
- CI service artifact assertions: `.github/workflows/ci.yml:26`
