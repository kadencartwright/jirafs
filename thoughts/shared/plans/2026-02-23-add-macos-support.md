# Add macOS Support Implementation Plan

## Overview

Add first-class macOS support for `fs-jira` so contributors can build, run, mount, and unmount the filesystem on both Linux and macOS with one documented workflow and platform-aware validation.

## Current State Analysis

The core filesystem implementation is mostly platform-neutral, but project ergonomics and verification are Linux-first.

- README prerequisites explicitly require Linux and Linux package names (`README.md:7`, `README.md:9`).
- Unmount instructions are Linux-only (`fusermount3`) with no macOS path (`README.md:83`, `README.md:86`).
- Runtime mount configuration is minimal and shared (`src/main.rs:205`, `src/main.rs:206`), with no platform-specific mount behavior documented or tested.
- The project depends on `fuser` (`Cargo.toml:7`), which supports macOS via macFUSE/pkg-config in its build logic (`fuser` build script: `/home/k/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fuser-0.17.0/build.rs:16`).
- There is currently no CI workflow in-repo to assert cross-platform build health (`.github/workflows/*` is absent).

## Desired End State

`fs-jira` is treated as Linux+macOS supported with explicit install/run/unmount docs, platform-aware mount configuration decisions, and automated checks for both OS families.

### Key Discoveries:
- `fuser` already contains macOS-specific support paths and feature gates, including macFUSE compatibility (`/home/k/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fuser-0.17.0/Cargo.toml:66`, `/home/k/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fuser-0.17.0/build.rs:16`).
- Application mount path is centralized in `main`, which is the right integration point for OS-aware mount options (`src/main.rs:205`, `src/main.rs:215`).
- Filesystem semantics are already read-only by behavior (EROFS on non-control writes), reducing macOS-specific risk in the operation handlers (`src/fs.rs:779`, `src/fs.rs:883`, `src/fs.rs:932`).
- Cross-target compilation from this environment currently fails because Apple target stdlib is not installed, indicating validation setup gaps (`cargo check --target x86_64-apple-darwin` error, missing target/toolchain).

### Verification of End State

1. Contributors can follow README steps for Linux and macOS without guessing package/tool names.
2. `cargo check` passes for both Linux host target and at least one macOS target (`x86_64-apple-darwin` and/or `aarch64-apple-darwin`) in automation.
3. Runtime mount path does not rely on Linux-only assumptions and clearly documents macOS limitations/prerequisites.
4. Unmount workflow is documented and works on macOS.

## What We're NOT Doing

- No Windows support in this plan.
- No migration away from `fuser`.
- No Finder UX polishing beyond required correctness (for example, no custom icon metadata support).
- No packaging/distribution work (Homebrew tap, release notarization, installer signing).

## Implementation Approach

Deliver in three phases: documentation parity, runtime compatibility hardening, and cross-platform verification automation. Keep behavior read-only and avoid broad filesystem refactors.

## Phase 1: Document macOS Prerequisites and Operations

### Overview

Make setup and operational guidance explicitly dual-platform so macOS users can install dependencies and operate mounts without ad-hoc troubleshooting.

### Changes Required:

#### 1. Expand Prerequisites Section to Linux + macOS
**File**: `README.md`
**Changes**: Replace Linux-only requirement language with platform matrix; add macFUSE + pkg-config prerequisites and architecture notes.

```markdown
## Prerequisites

- Rust toolchain (`rustup`, `cargo`)

### Linux
- FUSE userspace headers (`libfuse3-dev` on Debian/Ubuntu)

### macOS
- macFUSE installed (`brew install macfuse`)
- `pkg-config` available (`brew install pkgconf`)
```

#### 2. Add Platform-Specific Unmount Instructions
**File**: `README.md`
**Changes**: Keep `fusermount3`/`fusermount` for Linux and add macOS unmount command path.

```markdown
### Unmount (Linux)
fusermount3 -u /tmp/fs-jira-mnt

### Unmount (macOS)
umount /tmp/fs-jira-mnt
```

### Success Criteria:

#### Automated Verification:
- [ ] README examples and command blocks are lint-clean markdown (if markdown lint exists, run it; otherwise verify with `cargo doc --no-deps` to ensure no command regressions in contributor docs flow).
- [ ] Baseline project checks still pass after doc edits: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] A macOS developer can install prerequisites from README without external notes.
- [ ] Linux developer instructions remain unchanged and still accurate.
- [ ] macOS unmount command is confirmed against a real mount.

**Implementation Note**: After this phase and automated checks pass, pause for human confirmation from a macOS machine before proceeding.

---

## Phase 2: Harden Runtime Mount Path for macOS

### Overview

Ensure the mount bootstrap path is explicit about cross-platform behavior and avoids Linux-only assumptions.

### Changes Required:

#### 1. Introduce Platform-Aware Mount Option Builder
**File**: `src/main.rs`
**Changes**: Extract mount option assembly into a helper that can append OS-specific options while retaining existing shared defaults (`FSName`, `DefaultPermissions`).

```rust
fn mount_options() -> Vec<MountOption> {
    let mut options = vec![
        MountOption::FSName("fs-jira".to_string()),
        MountOption::DefaultPermissions,
    ];
    if cfg!(target_os = "macos") {
        options.push(MountOption::RO);
        // optional: options.push(MountOption::CUSTOM("volname=fs-jira".to_string()));
    }
    options
}
```

#### 2. Preserve Read-Only Contract in Mount Config
**File**: `src/main.rs`
**Changes**: Explicitly set read-only mount option to align kernel behavior with existing read-only handlers in `src/fs.rs`.

```rust
config.mount_options.extend(mount_options());
```

#### 3. Add Targeted Test Coverage for Option Builder
**File**: `src/main.rs` (or split helper module with unit tests)
**Changes**: Add deterministic unit tests asserting invariant options are always present and read-only option is applied.

```rust
#[test]
fn mount_options_include_fsname_and_default_permissions() {
    let options = mount_options();
    assert!(options.contains(&MountOption::FSName("fs-jira".to_string())));
    assert!(options.contains(&MountOption::DefaultPermissions));
}
```

### Success Criteria:

#### Automated Verification:
- [ ] Code compiles on Linux host: `cargo check --locked`
- [ ] Unit tests pass, including new mount-option tests: `cargo test --all-targets --all-features --locked`
- [ ] Strict lint gate passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] On macOS, mount succeeds and `ls`/`cat` over mounted files works.
- [ ] Finder/CLI write attempts fail with read-only semantics (no crash/hang).
- [ ] `.sync_meta/manual_refresh` and `.sync_meta/full_refresh` write triggers still function.

**Implementation Note**: After this phase and automated checks pass, pause for human macOS mount verification before proceeding.

---

## Phase 3: Add Cross-Platform Build Validation

### Overview

Add repeatable automation so macOS support does not regress after future changes.

### Changes Required:

#### 1. Add CI Workflow with Linux + macOS Matrix
**File**: `.github/workflows/ci.yml`
**Changes**: Create matrix for `ubuntu-latest` and `macos-latest`; run fmt/clippy/tests where feasible and at minimum `cargo check` on both.

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest]
steps:
  - uses: actions/checkout@v4
  - uses: dtolnay/rust-toolchain@stable
  - run: cargo check --locked
```

#### 2. Add Optional Cross-Target Checks
**File**: `.github/workflows/ci.yml`
**Changes**: On Linux runner, add `rustup target add x86_64-apple-darwin` and `cargo check --target x86_64-apple-darwin` as non-mount compile coverage.

```yaml
- run: rustup target add x86_64-apple-darwin
- run: cargo check --target x86_64-apple-darwin --locked
```

#### 3. Document CI Coverage Scope
**File**: `README.md`
**Changes**: Clarify that CI validates build/test parity and that mount/unmount smoke test still requires native host privileges/devices.

### Success Criteria:

#### Automated Verification:
- [ ] CI runs on Linux and macOS for pull requests.
- [ ] Linux job passes: `cargo fmt --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-targets --all-features --locked`.
- [ ] macOS job passes at least `cargo check --locked` (plus clippy/tests if environment permits).
- [ ] Cross-target compile check passes where configured: `cargo check --target x86_64-apple-darwin --locked`.

#### Manual Verification:
- [ ] A maintainer can identify from CI output whether a change broke macOS compatibility.
- [ ] Local contributor workflow on Linux remains fast and unchanged.

**Implementation Note**: After this phase and automated checks pass, pause for final human sign-off on one real macOS mount/unmount cycle.

---

## Testing Strategy

### Unit Tests:
- Add focused tests for mount option construction invariants.
- Keep tests platform-agnostic where possible; gate platform-specific assertions with `cfg(target_os = "macos")` only when necessary.

### Integration Tests:
- Continue existing Rust integration/unit coverage for cache/render/fs behavior.
- Add CI-level cross-platform compile checks to prevent OS-specific breakage in `main`/mount plumbing.

### Manual Testing Steps:
1. On macOS, install macFUSE + pkg-config and run `cargo run -- /tmp/fs-jira-mnt`.
2. Verify `ls /tmp/fs-jira-mnt`, `ls /tmp/fs-jira-mnt/projects`, and `cat` on an issue file.
3. Trigger manual sync via `.sync_meta/manual_refresh` write and verify sync metadata updates.
4. Unmount with `umount /tmp/fs-jira-mnt` and remount once.

## Performance Considerations

- No expected performance impact from documentation/CI changes.
- Mount-option helper should avoid per-request overhead (startup-only code path).
- Keep mount options minimal to avoid platform-specific side effects.

## Migration Notes

- No data or schema migration required.
- Existing Linux users continue using current commands; docs become additive.
- CI introduction may require repository settings update for GitHub Actions if currently disabled.

## References

- App mount setup: `src/main.rs:205`, `src/main.rs:215`
- Read-only behavior in filesystem handlers: `src/fs.rs:779`, `src/fs.rs:883`, `src/fs.rs:932`
- Linux-only prerequisites/unmount docs today: `README.md:7`, `README.md:86`
- `fuser` macOS mount implementation selection: `/home/k/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fuser-0.17.0/build.rs:16`
- `fuser` macOS dependency guidance: `/home/k/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fuser-0.17.0/README.md:64`
