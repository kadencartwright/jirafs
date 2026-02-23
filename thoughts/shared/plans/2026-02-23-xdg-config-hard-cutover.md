# XDG Config Hard Cutover Implementation Plan

## Overview

Replace all runtime environment-variable configuration with a single TOML config file loaded from the XDG config location: `$XDG_CONFIG_HOME/fs-jira/config.toml` or `~/.config/fs-jira/config.toml` when `XDG_CONFIG_HOME` is unset.

This is a hard cutover: no environment-variable fallback at runtime.

## Current State Analysis

`fs-jira` currently initializes all runtime settings from environment variables in `main`, with required keys (`JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`, `JIRA_PROJECTS`) and optional tuning keys (`JIRA_CACHE_TTL_SECS`, `FS_JIRA_SYNC_BUDGET`, `FS_JIRA_SYNC_INTERVAL_SECS`, `FS_JIRA_METRICS_INTERVAL_SECS`, `FS_JIRA_CACHE_DB`) (`src/main.rs:140`, `src/main.rs:171`).

Startup also auto-loads `.env` via `dotenvy`, reinforcing env-vars as the primary source (`src/main.rs:121`).

Debug logging is independently env-driven (`FS_JIRA_DEBUG`) via `OnceLock`, not through a centralized runtime config object (`src/logging.rs:4`, `src/logging.rs:8`).

There is currently no config file parser, no typed app config model, and no XDG path resolution logic in `src/`.

## Desired End State

At process startup, `fs-jira` resolves and reads exactly one config file from the XDG config directory, validates it, and uses it as the sole configuration source for runtime behavior.

The binary fails fast with actionable error messages if the config file is missing or invalid.

### Key Discoveries:
- All required Jira credentials/project selection are currently hard-required env vars in `main` (`src/main.rs:140`).
- Persistent cache path is currently required for incremental sync behavior (`src/main.rs:171`, `src/main.rs:180`).
- Debug mode is currently disconnected from app startup config plumbing (`src/logging.rs:8`).
- Documentation currently teaches env-var setup and `.env` autoloading (`README.md:45`, `README.md:63`).

## What We're NOT Doing

- No compatibility layer for old env vars (no fallback, no precedence merge).
- No support for alternate config formats (YAML/JSON).
- No changes to Jira API behavior, sync algorithm, FUSE semantics, or cache schema.
- No background config hot-reload; config is startup-only.

## Implementation Approach

Add a dedicated `config` module with:
- typed deserializable config structs,
- XDG path resolution,
- strict validation with human-readable error messages,
- one startup load path consumed by `main` and logging setup.

Then remove direct env access and `.env` loading, replace docs/examples, and add tests around path resolution and validation failures.

## Phase 1: Build Config System and XDG Resolution

### Overview
Introduce a first-class config loader and validator that resolves `config.toml` from XDG location and produces a typed runtime config object.

### Changes Required:

#### 1. New Config Module
**File**: `src/config.rs`
**Changes**: Add typed config structs, `load()` API, path resolution, parse/validation error types.

```rust
#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub jira: JiraConfig,
    pub cache: CacheConfig,
    pub sync: SyncConfig,
    pub metrics: MetricsConfig,
    pub logging: LoggingConfig,
}

pub fn load() -> Result<AppConfig, ConfigError> {
    let path = resolve_config_path()?;
    let raw = std::fs::read_to_string(&path)?;
    let cfg: AppConfig = toml::from_str(&raw)?;
    cfg.validate()?;
    Ok(cfg)
}
```

#### 2. Wire Module into Crate
**File**: `src/lib.rs`
**Changes**: Export `config` module for reuse in binary/tests.

#### 3. Dependency Additions
**File**: `Cargo.toml`
**Changes**: Add TOML parser dependency and (optionally) XDG helper crate if used.

```toml
[dependencies]
toml = "0.8"
```

#### 4. Config Fixture for Developer Onboarding
**File**: `.config/fs-jira/config.toml.example` (or `config.example.toml` in repo root)
**Changes**: Provide canonical template for required/optional keys and defaults.

```toml
[jira]
base_url = "https://your-domain.atlassian.net"
email = "you@example.com"
api_token = "your_api_token_here"
projects = ["PROJ", "OPS"]

[cache]
db_path = "/tmp/fs-jira-cache.db"
ttl_secs = 30

[sync]
budget = 1000
interval_secs = 60

[metrics]
interval_secs = 60

[logging]
debug = false
```

### Success Criteria:

#### Automated Verification:
- [ ] Project compiles with new config module: `cargo check --locked`
- [ ] Config loader unit tests pass (parse, validate, missing keys): `cargo test --locked config::`
- [ ] Full test suite still passes: `cargo test --all-targets --all-features --locked`
- [ ] Linting passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] Config path resolves to `$XDG_CONFIG_HOME/fs-jira/config.toml` when `XDG_CONFIG_HOME` is set
- [ ] Config path resolves to `~/.config/fs-jira/config.toml` when `XDG_CONFIG_HOME` is unset
- [ ] Missing config file error message clearly states expected absolute path
- [ ] Invalid TOML and invalid semantic values produce actionable startup errors

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: Replace Env Usage with Config Object (Hard Cutover)

### Overview
Switch runtime initialization to use `AppConfig` only; remove all env-var reads and `.env` autoload behavior.

### Changes Required:

#### 1. Main Startup Refactor
**File**: `src/main.rs`
**Changes**: Replace `required_env`, `env_u64`, `env_usize`, and direct env reads with config object fields; remove `dotenvy::dotenv()`.

```rust
let app_config = fs_jira::config::load()?;

let jira = Arc::new(JiraClient::new_with_metrics(
    app_config.jira.base_url.clone(),
    app_config.jira.email.clone(),
    app_config.jira.api_token.clone(),
    Arc::clone(&metrics),
)?);

let cache = Arc::new(InMemoryCache::with_persistence(
    Duration::from_secs(app_config.cache.ttl_secs),
    Duration::from_secs(app_config.cache.ttl_secs),
    std::path::Path::new(&app_config.cache.db_path),
    Arc::clone(&metrics),
)?);
```

#### 2. Logging Config Plumbing
**File**: `src/logging.rs`
**Changes**: Replace env-based debug detection with explicit initialization API (e.g., `logging::init(debug: bool)`).

```rust
pub fn init(debug: bool) {
    let _ = DEBUG_ENABLED.set(debug);
}
```

#### 3. Remove Env Dependency from Runtime
**File**: `Cargo.toml`
**Changes**: Remove `dotenvy` dependency once no longer used.

### Success Criteria:

#### Automated Verification:
- [ ] No direct env-var reads remain in runtime modules (except tests if intentionally scoped): `rg "std::env::var" src`
- [ ] Binary compiles after env helper removal: `cargo check --locked`
- [ ] Tests pass after startup refactor: `cargo test --all-targets --all-features --locked`
- [ ] Linting passes after refactor: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] Running without config file fails immediately with clear guidance
- [ ] Running with valid config mounts successfully and serves project directories
- [ ] `logging.debug = true` enables debug logs; `false` suppresses them
- [ ] Existing mount/unmount flow still behaves as before

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 3: Documentation, Migration Guidance, and Regression Coverage

### Overview
Update repository documentation and tests so new users configure via `config.toml`, and existing users can migrate off env vars without ambiguity.

### Changes Required:

#### 1. User Documentation Rewrite
**File**: `README.md`
**Changes**: Replace env export instructions with XDG config setup, include example file and migration notes.

#### 2. Example Configuration
**File**: `.env.example` (replace or deprecate), `config.example.toml`
**Changes**: Remove env template as canonical setup; provide TOML example as canonical.

#### 3. Regression Tests
**File**: `src/config.rs` tests and/or `src/main.rs` tests
**Changes**: Add tests for missing required fields, empty projects, invalid numeric values, and path resolution branches.

### Success Criteria:

#### Automated Verification:
- [ ] README instructions align with actual startup behavior (no `.env` references)
- [ ] Config example parses cleanly in test: `cargo test --locked config_example_parses`
- [ ] Full quality gates pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Fresh setup from README works end-to-end using only `config.toml`
- [ ] Existing user can migrate env vars to TOML using documented key mapping
- [ ] Error/help text is sufficient to recover from common mistakes (bad path, malformed TOML, missing keys)
- [ ] Team member unfamiliar with project can run mount flow from docs without additional tribal knowledge

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Testing Strategy

### Unit Tests:
- Config path resolution (`XDG_CONFIG_HOME` set vs unset).
- TOML deserialize + semantic validation (required Jira keys, non-empty projects, positive intervals).
- Logging initialization semantics (`debug` on/off).

### Integration Tests:
- Startup path that loads config and constructs Jira/cache/sync objects.
- Startup failure snapshots for missing file and malformed TOML.
- Mount option behavior remains unchanged after startup refactor.

### Manual Testing Steps:
1. Create `~/.config/fs-jira/config.toml` from example and run `cargo run -- /tmp/fs-jira-mnt`.
2. Validate filesystem output for configured projects and issue markdown retrieval.
3. Toggle `logging.debug` and confirm debug log visibility changes.
4. Temporarily break config (e.g., remove `jira.api_token`) and verify fail-fast error quality.

## Performance Considerations

- Config file is read once at startup; negligible runtime overhead.
- Centralized typed config can reduce repetitive parsing overhead currently done through per-key env reads.
- No expected impact on sync throughput, cache efficiency, or FUSE serving latency.

## Migration Notes

- Hard cutover means existing shell-based env setup immediately stops working.
- Provide explicit key mapping in docs:
  - `JIRA_BASE_URL` -> `jira.base_url`
  - `JIRA_EMAIL` -> `jira.email`
  - `JIRA_API_TOKEN` -> `jira.api_token`
  - `JIRA_PROJECTS` -> `jira.projects` (array)
  - `JIRA_CACHE_TTL_SECS` -> `cache.ttl_secs`
  - `FS_JIRA_CACHE_DB` -> `cache.db_path`
  - `FS_JIRA_SYNC_BUDGET` -> `sync.budget`
  - `FS_JIRA_SYNC_INTERVAL_SECS` -> `sync.interval_secs`
  - `FS_JIRA_METRICS_INTERVAL_SECS` -> `metrics.interval_secs`
  - `FS_JIRA_DEBUG` -> `logging.debug`
- Remove `.env` recommendation entirely from docs to avoid split-brain configuration.

## References

- Existing startup env parsing: `src/main.rs:121`
- Required env vars and defaults: `src/main.rs:140`
- Persistent cache env requirement: `src/main.rs:171`
- Logging env debug flag: `src/logging.rs:8`
- Current env-based docs: `README.md:45`
