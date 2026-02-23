# Justfile Build, Run, and Install Implementation Plan

## Overview

Add a root `Justfile` that provides stable developer entrypoints for building, running, and installing `fs-jira`, and ensure `install` bootstraps a default TOML config in the exact runtime lookup location.

The install flow will be non-destructive: it must refuse to overwrite an existing config file.

## Current State Analysis

The repository currently documents Cargo-first workflows (`cargo build`, `cargo run`) and quality gates in README and CI, but has no task runner wrapper like `Justfile` or `Makefile` (`README.md:19`, `.github/workflows/ci.yml:34`).

Config path behavior is already explicitly implemented in Rust: prefer `$XDG_CONFIG_HOME/fs-jira/config.toml`, otherwise fallback to `$HOME/.config/fs-jira/config.toml`, and error if neither env var is usable (`src/config.rs:146`, `src/config.rs:156`, `src/config.rs:163`).

README currently shows manual config bootstrap with `mkdir -p` and `cp config.example.toml` into `~/.config/fs-jira/config.toml` (`README.md:53`, `README.md:54`).

## Desired End State

Developers can run `just build`, `just run`, and `just install` from repo root. `just install` installs the binary and creates `config.toml` at the same path used by runtime config resolution, but fails safely when `config.toml` already exists.

### Key Discoveries:
- There is no existing task-runner file to preserve; this is a net-new addition (`README.md:19`, `.github/workflows/ci.yml:34`).
- Runtime config path precedence is already unambiguous in code and tests (`src/config.rs:156`, `src/config.rs:294`).
- CLI already supports explicit config path override for advanced usage, so `install` should focus on sane defaults (`src/main.rs:53`, `src/main.rs:272`).
- Existing docs already establish `config.example.toml` as the default template to copy (`README.md:50`, `config.example.toml:1`).

## What We're NOT Doing

- Not introducing a full packaging/distribution pipeline (Homebrew, apt, release archives).
- Not changing runtime config resolution in Rust code.
- Not auto-merging or modifying existing user config files.
- Not replacing Cargo commands in CI in this iteration (CI can continue using Cargo directly).

## Implementation Approach

Add a single root `Justfile` with explicit recipes:
- `build`: compile with Cargo.
- `run`: execute the binary with a mountpoint argument and pass-through args as needed.
- `install`: perform `cargo install` from local path, resolve config destination using XDG/HOME logic matching `src/config.rs`, create directory, and copy `config.example.toml` only when missing.

Keep recipe behavior deterministic and shell-safe, with clear error messages and non-destructive install semantics.

## Phase 1: Add Justfile Skeleton and Core Build/Run Recipes

### Overview
Create the initial `Justfile`, define shell behavior, and implement ergonomic `build` and `run` recipes that mirror existing documented commands.

### Changes Required:

#### 1. Create Root Task Runner
**File**: `Justfile`
**Changes**: Add `default`, `build`, and `run` recipes with robust shell settings and sensible defaults.

```just
set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

build:
    cargo build --locked

run mountpoint="/tmp/fs-jira-mnt":
    mkdir -p "{{mountpoint}}"
    cargo run -- "{{mountpoint}}"
```

#### 2. Optional Config-Aware Run Variant
**File**: `Justfile`
**Changes**: Add a recipe that uses `--config` for explicit-path runs so local testing can mirror docs.

```just
run-with-config config_path mountpoint="/tmp/fs-jira-mnt":
    mkdir -p "{{mountpoint}}"
    cargo run -- --config "{{config_path}}" "{{mountpoint}}"
```

### Success Criteria:

#### Automated Verification:
- [ ] `Justfile` parses and lists recipes: `just --list`
- [ ] Build recipe compiles successfully: `just build`
- [ ] Run recipe command renders expected usage when mountpoint omitted/invalid conditions are simulated: `just run /tmp/fs-jira-mnt`
- [ ] Existing quality checks still pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Developer can discover commands quickly via `just` output
- [ ] `just run` creates mountpoint if absent and starts process cleanly
- [ ] Argument quoting works with mountpoint paths containing spaces
- [ ] Recipe output is understandable without reading `Justfile` internals

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: Implement Non-Destructive Install with Config Bootstrap

### Overview
Add `install` recipe that installs the binary and creates default config in the correct location while refusing to overwrite existing config.

### Changes Required:

#### 1. Add Install Recipe
**File**: `Justfile`
**Changes**: Add install steps for binary + config bootstrap with exact XDG/HOME precedence.

```just
install:
    cargo install --path . --locked
    if [ -n "${XDG_CONFIG_HOME:-}" ]; then
      config_dir="${XDG_CONFIG_HOME}/fs-jira";
    elif [ -n "${HOME:-}" ]; then
      config_dir="${HOME}/.config/fs-jira";
    else
      echo "failed to resolve config path: HOME is not set and XDG_CONFIG_HOME is unset" >&2;
      exit 1;
    fi
    mkdir -p "$config_dir"
    config_path="$config_dir/config.toml"
    if [ -e "$config_path" ]; then
      echo "refusing to overwrite existing config: $config_path" >&2;
      exit 1;
    fi
    cp config.example.toml "$config_path"
```

#### 2. Optional Explicit Bootstrap Helper
**File**: `Justfile`
**Changes**: Add a helper recipe (e.g., `install-config`) if separation improves usability and testability of config bootstrap logic.

#### 3. Align README with New Entry Points
**File**: `README.md`
**Changes**: Add `just` usage section and update setup steps to prefer `just install` while preserving raw Cargo alternatives.

### Success Criteria:

#### Automated Verification:
- [ ] Install recipe installs binary from local source: `just install` (in clean test env)
- [ ] Config path respects XDG precedence when set: `XDG_CONFIG_HOME=/tmp/xdg-test just install`
- [ ] Install fails safely if config already exists: `just install` run twice should fail second time with explicit refusal
- [ ] `config.example.toml` remains valid for runtime parser: `cargo test --locked config_example_parses`

#### Manual Verification:
- [ ] `which fs-jira` (or equivalent) resolves installed binary after install
- [ ] Generated config file appears at expected location and contains example content
- [ ] Existing custom config is preserved because overwrite is refused
- [ ] Error message clearly tells user why install failed and where config exists

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 3: Documentation and Regression Hardening

### Overview
Finalize docs and add lightweight regression coverage for the new workflow so contributors have one canonical command surface.

### Changes Required:

#### 1. README Workflow Consolidation
**File**: `README.md`
**Changes**: Document `just build`, `just run`, and `just install`; include non-overwrite behavior note for config bootstrap.

#### 2. CI/Contributor Guidance Alignment
**File**: `.github/workflows/ci.yml` (optional) and contributor docs
**Changes**: Optionally keep CI cargo-native but document how local `just` maps to CI quality gates.

#### 3. Guardrail Notes in Justfile
**File**: `Justfile`
**Changes**: Add concise echo statements for key install outcomes (created config path vs refused overwrite) to improve operator clarity.

### Success Criteria:

#### Automated Verification:
- [ ] README command examples execute successfully on a fresh clone
- [ ] All quality gates pass after docs/recipe changes: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`
- [ ] `just --list` output includes all intended recipes with clear names

#### Manual Verification:
- [ ] New contributor can complete build/run/install using README without tribal knowledge
- [ ] Install refusal path is intuitive when config already exists
- [ ] Mount workflow still behaves as expected after switching to `just` commands
- [ ] macOS and Linux users can follow equivalent steps with minimal divergence

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Testing Strategy

### Unit Tests:
- Keep existing config path resolution tests as source of truth for destination behavior (`src/config.rs:283`, `src/config.rs:294`, `src/config.rs:302`).
- Add/retain tests ensuring `config.example.toml` parses successfully (`src/config.rs:354`).

### Integration Tests:
- Run end-to-end local command flow: `just build`, `just run <mountpoint>`, `just install`.
- Validate install behavior under both environment branches (`XDG_CONFIG_HOME` set/unset).
- Validate refusal semantics when `config.toml` already exists.

### Manual Testing Steps:
1. In a clean shell, run `just install` and confirm binary install + config creation.
2. Re-run `just install` and confirm it exits non-zero with overwrite refusal message.
3. Set `XDG_CONFIG_HOME=/tmp/fs-jira-xdg`, run `just install`, verify `/tmp/fs-jira-xdg/fs-jira/config.toml`.
4. Run `just run /tmp/fs-jira-mnt` and verify startup proceeds with installed/default config expectations.

## Performance Considerations

- `Justfile` introduces no runtime overhead for mounted filesystem operations.
- Install-time config bootstrap is single-file I/O and negligible.
- Refusal-on-existing-config avoids expensive merge/parse logic and lowers risk of accidental regressions.

## Migration Notes

- Existing Cargo workflows remain valid; `just` is an ergonomics layer, not a breaking replacement.
- Teams can adopt `just` incrementally while preserving CI cargo commands.
- Users with an existing config file will now see an explicit install refusal and must opt into manual update/merge.

## References

- Runtime config resolution logic: `src/config.rs:146`
- XDG precedence implementation: `src/config.rs:156`
- HOME fallback implementation: `src/config.rs:163`
- Config path documentation: `README.md:45`
- Existing manual config bootstrap docs: `README.md:53`
- Cargo build docs: `README.md:22`
- Cargo run docs: `README.md:106`
