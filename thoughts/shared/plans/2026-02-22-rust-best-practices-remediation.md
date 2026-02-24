# Rust Best Practices Remediation Plan

## Overview

Remediate repository-wide Rust quality issues identified in the audit so the codebase is clippy-clean under strict settings, avoids panic-prone production paths, reduces unnecessary cloning/ownership churn, and has a maintainable public API documentation baseline.

## Current State Analysis

The codebase is functionally healthy (tests pass) but has policy and maintainability gaps against the Rust best-practices handbook.

- Strict clippy gate currently fails with 4 blocking warnings in active code paths.
- Several production code paths use `expect(...)`/`unreachable!(...)` that can panic.
- Public API docs are largely missing across exported modules/types/functions.
- A few API signatures and call sites force avoidable ownership/cloning.
- Existing tests validate behavior well in core paths but do not yet enforce docs/lint policy at CI-strength by default.

## Desired End State

Repository reaches an enforceable, reproducible quality baseline:

1. `cargo clippy --all-targets --all-features --locked -- -D warnings` passes.
2. Production code has no accidental panic points from mutex/time assumptions (explicit policy or typed error handling instead).
3. Public API surface has actionable `///` docs (including `# Errors`/`# Panics` where applicable).
4. Cache/update APIs use borrowing where ownership is not required.
5. Tests and lint commands are codified as repeatable verification steps for future changes.

### Key Discoveries:
- Blocking clippy issues exist in `src/cache.rs:293`, `src/cache/persistent.rs:304`, `src/render.rs:72`, `src/render.rs:244`.
- Panic-prone production `expect(...)` lock usage appears in core paths like `src/cache.rs:77`, `src/cache/persistent.rs:89`, `src/sync_state.rs:28`, `src/fs.rs:176`, and `src/jira.rs:104`.
- Public exports are broad from `src/lib.rs:1` but currently have no `///` coverage in exported modules.
- Ownership-heavy signatures appear in cache APIs: `src/cache.rs:241`, `src/cache.rs:261`, `src/cache.rs:293`.

## What We're NOT Doing

- No functional feature expansion (no new Jira/FUSE capabilities).
- No architectural rewrite of filesystem layout, sync model, or cache storage engine.
- No migration to async Jira client/runtime in this plan.
- No forced adoption of pedantic/nursery clippy lints as hard-fail globally (only selected practical lints and `-D warnings` baseline).

## Implementation Approach

Execute remediation in incremental quality phases ordered by delivery risk:

1. Unblock hard lint failures first (small and deterministic changes).
2. Remove panic-prone production assumptions with explicit error behavior.
3. Refactor ownership/signatures to reduce cloning and improve API clarity.
4. Add documentation baseline and codify verification workflow.

Each phase is independently testable and must preserve current runtime behavior.

## Phase 1: Stabilize Lint Gate

### Overview

Resolve all currently blocking clippy errors under `-D warnings` without broad refactoring.

### Changes Required:

#### 1. Type Complexity Aliases for Sidecar Payloads
**File**: `src/cache.rs`
**Changes**: Introduce explicit type aliases (or small structs) for sidecar tuples and use them in function signatures.

```rust
pub type IssueSidecarRow = (String, Vec<u8>, Vec<u8>, Option<String>);

pub fn upsert_issue_sidecars_batch(&self, sidecars: Vec<IssueSidecarRow>) -> usize {
    // existing body
}
```

#### 2. Matching Alias in Persistent Layer
**File**: `src/cache/persistent.rs`
**Changes**: Use the same alias or a persistent-layer equivalent for `upsert_issue_sidecars_batch` input.

```rust
pub type PersistentSidecarRow = (String, Vec<u8>, Vec<u8>, Option<String>);

pub fn upsert_issue_sidecars_batch(
    &self,
    sidecars: &[PersistentSidecarRow],
) -> Result<usize, rusqlite::Error> {
    // existing body
}
```

#### 3. String/Ownership Clippy Fixes in Render Path
**File**: `src/render.rs`
**Changes**:
- Replace single-char `push_str("\n")` with `push('\n')`.
- Remove unnecessary owned conversion in `adf_to_markdown` call chain.

```rust
out.push('\n');

fn adf_to_markdown(value: &Value) -> String {
    redact_secrets(adf_to_markdown_inner(value).trim())
}
```

### Success Criteria:

#### Automated Verification:
- [ ] Strict clippy passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] Unit tests remain green: `cargo test --all-targets --all-features --locked`
- [ ] Formatting check passes: `cargo fmt --check`

#### Manual Verification:
- [ ] Mount + basic read workflow remains unchanged (`ls`, `cat` on mounted tree)
- [ ] Rendered markdown structure remains human-readable for sample tickets
- [ ] No behavior regression in sidecar file exposure (`*.comments.md`, `*.comments.jsonl`)

**Implementation Note**: After completing this phase and all automated verification passes, pause for manual confirmation before proceeding.

---

## Phase 2: Panic-Safe Production Paths

### Overview

Replace panic-prone production assumptions with explicit policy-driven handling.

### Changes Required:

#### 1. Establish Shared Internal Error Type for Operational Failures
**File**: `src/cache.rs`
**Changes**: Introduce internal error enum (`thiserror`) for cache/system failures currently masked by `expect(...)` panics.

```rust
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache lock poisoned: {0}")]
    LockPoisoned(&'static str),
    #[error("persistent cache error: {0}")]
    Persistent(#[from] rusqlite::Error),
}
```

#### 2. Replace `expect(...)` on Mutex Locks in Runtime Code
**File**: `src/cache.rs`
**Changes**: Convert lock sites to `map_err(...)`/fallback policy and propagate or degrade gracefully.

```rust
let guard = self
    .issue_markdown
    .lock()
    .map_err(|_| CacheError::LockPoisoned("issue_markdown"))?;
```

#### 3. Replace Time Assumption Panics in Persistence
**File**: `src/cache/persistent.rs`
**Changes**: Replace `duration_since(...).expect(...)` with safe fallback or typed error mapping.

```rust
let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs().to_string())
    .unwrap_or_else(|_| "0".to_string());
```

#### 4. Remove `unreachable!` in Retry Loop
**File**: `src/jira.rs`
**Changes**: Replace tail `unreachable!` with explicit terminal error return if retry loop invariants are broken.

```rust
Err(JiraError::Http {
    status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
    body: "retry loop exhausted unexpectedly".to_string(),
})
```

#### 5. Apply Same Policy to Sync State/FS/Jira Lock Usage
**File**: `src/sync_state.rs`
**Changes**: Ensure lock access paths avoid panic and use explicit defaults/failure signaling.

### Success Criteria:

#### Automated Verification:
- [ ] No production-path `expect(...)` remains in `src/cache.rs`, `src/cache/persistent.rs`, `src/fs.rs`, `src/sync_state.rs`, `src/jira.rs` (tests allowed): `rg "\.expect\(" src/{cache.rs,cache/persistent.rs,fs.rs,sync_state.rs,jira.rs}`
- [ ] All tests pass after error-path changes: `cargo test --all-targets --all-features --locked`
- [ ] Clippy remains clean under strict gate: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] Simulated transient failures do not crash process (e.g., API failure while serving stale cache)
- [ ] Sync loop continues operating after recoverable runtime errors
- [ ] FUSE mount remains responsive under degraded backend conditions

**Implementation Note**: After completing this phase and all automated verification passes, pause for manual confirmation before proceeding.

---

## Phase 3: Ownership and API Ergonomics Cleanup

### Overview

Reduce redundant cloning and make API ownership contracts explicit, especially in cache ingestion paths.

### Changes Required:

#### 1. Borrowing-Based Signatures for Cache Upserts
**File**: `src/cache.rs`
**Changes**: Update batch/direct APIs to accept slices/borrows where ownership is not needed.

```rust
pub fn upsert_issue_direct(&self, issue_key: &str, markdown: &[u8], updated: Option<&str>)

pub fn upsert_issues_batch(&self, issues: &[(String, Vec<u8>, Option<String>)]) -> usize
```

#### 2. Adjust Call Sites to Avoid Clone Churn
**File**: `src/warmup.rs`
**Changes**: Remove avoidable clones when moving values is already acceptable or when references can be passed.

```rust
cache.upsert_project_issues(project, items);
```

#### 3. Clean Minor Redundant Clones in FS/Jira Paths
**File**: `src/fs.rs`
**Changes**: Replace clone-plus-concat patterns with borrow-based checks.

```rust
if !issue_key.starts_with(&format!("{project}-")) {
    reply.error(Errno::ENOENT);
    return;
}
```

### Success Criteria:

#### Automated Verification:
- [ ] Clone hotspots reduced and signatures updated without breakage: `cargo test --all-targets --all-features --locked`
- [ ] Clippy strict gate still passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] No new API regressions in callers: `cargo check --all-targets --all-features --locked`

#### Manual Verification:
- [ ] End-to-end sync behavior still updates listings and sidecars correctly
- [ ] Cache hit/stale behavior remains equivalent to pre-refactor behavior
- [ ] No user-visible latency regression on common `cat`/`ls` flows

**Implementation Note**: After completing this phase and all automated verification passes, pause for manual confirmation before proceeding.

---

## Phase 4: Public API Documentation and Quality Guardrails

### Overview

Introduce maintainable API docs and codify lint/test commands as project guardrails.

### Changes Required:

#### 1. Add Public API Rustdoc Coverage
**File**: `src/lib.rs`
**Changes**: Add crate/module-level docs and ensure exported modules are documented.

```rust
//! jirafs exposes cache, Jira API, rendering, and FUSE filesystem modules.
//! It provides a read-only Jira-backed filesystem interface.
```

#### 2. Add Item-Level Docs on Public Types/Functions
**File**: `src/jira.rs`
**Changes**: Add `///` for `IssueData`, `JiraClient`, and public API methods including `# Errors` where returning `Result`.

```rust
/// Fetches one Jira issue by key.
///
/// # Errors
/// Returns [`JiraError`] when request transport, HTTP, or decode fails.
pub fn get_issue(&self, issue_key: &str) -> Result<IssueData, JiraError> { ... }
```

#### 3. Add Docs for Cache/Persistence Public Surface
**File**: `src/cache/persistent.rs`
**Changes**: Add `# Errors` and `# Panics` sections where applicable until panic paths are eliminated.

#### 4. Codify Verification Commands for Contributors
**File**: `README.md`
**Changes**: Add explicit quality gate commands (`fmt`, `clippy`, `test`) and expected usage.

### Success Criteria:

#### Automated Verification:
- [ ] Documentation builds cleanly: `cargo doc --no-deps`
- [ ] Lint/test gate remains green: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`
- [ ] Public API docs added for exported modules/types/functions in `src/lib.rs` surface

#### Manual Verification:
- [ ] Rustdoc output is readable and reflects intended usage
- [ ] New contributors can run documented quality commands successfully
- [ ] Error behavior is understandable from docs without reading implementation internals

**Implementation Note**: After completing this phase and all automated verification passes, pause for manual confirmation before declaring the remediation complete.

---

## Testing Strategy

### Unit Tests:
- Add focused tests for error-path behavior in cache/persistent/jira retry logic.
- Add/expand tests for updated ownership-oriented cache method signatures.
- Keep behavior-specific naming convention consistent (e.g., `function_should_x_when_y`).

### Integration Tests:
- Validate stale-safe serving when Jira fetch fails.
- Validate sync cursor and sidecar persistence flow across restarts.
- Validate FUSE read path remains stable after error handling refactors.

### Manual Testing Steps:
1. Mount filesystem and verify normal read paths under healthy Jira backend.
2. Trigger manual/full sync and inspect updated metadata files in `.sync_meta`.
3. Simulate API failure and confirm stale content behavior without crash.
4. Restart process with persistent DB and verify warm/hydrated behavior.

## Performance Considerations

- Avoid introducing extra heap allocations while fixing lint/doc issues.
- Prefer borrowing in hot cache paths to reduce clone overhead.
- Preserve bounded retry/backoff behavior to avoid API pressure spikes.
- Keep lock hold durations minimal when touching mutex-protected maps.

## Migration Notes

- No data schema migration required for functional behavior changes in this remediation.
- Signature changes in internal APIs may require synchronized updates across call sites in same PR.
- If panic policy changes alter logging/error surfaces, update operational runbooks accordingly.

## References

- Prior bootstrap implementation plan: `thoughts/shared/plans/2026-02-21-jirafs-rust-bootstrap.md`
- Warmup/cache behavior research: `thoughts/shared/research/2026-02-22-ticket-cache-pre-warming.md`
- Audit-derived blocking lint findings: `src/cache.rs:293`, `src/cache/persistent.rs:304`, `src/render.rs:72`, `src/render.rs:244`
- Export surface requiring docs baseline: `src/lib.rs:1`
