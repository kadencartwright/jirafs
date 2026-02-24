# FS Jira Rust Bootstrap Implementation Plan

## Overview

Build a Rust userspace FUSE filesystem that starts with a deterministic single-file mount (`test.md` -> `Hello World!`) and then evolves into a read-only Jira Cloud-backed markdown filesystem with practical caching and persistence.

## Current State Analysis

The repository currently contains planning context only and no Rust project scaffold or FUSE implementation code. There is no `Cargo.toml`, no `src/`, and no existing runtime/test harness.

Key constraints from the discovery conversation:
- Rust `fuser` is the preferred library for the initial Rust implementation (`discovery.md:490`).
- Jira Cloud issue freshness can be tracked from `fields.updated`, enabling conditional refresh behavior without refetching unchanged content (`discovery.md:166`).
- Adaptive TTL guidance exists and should be staged in after core read-path correctness (`discovery.md:174`).

## Desired End State

A Linux-mountable, read-only Rust FUSE filesystem with three implementation phases:
1. Local static bootstrap: one file `test.md` with `Hello World!`.
2. Jira Cloud-backed read-only project/issue markdown view (`/PROJECT/PROJECT-123.md`) with in-memory cache + TTL.
3. Persistent cache + warm-start and operational hardening for practical daily usage.

Verification at end of plan:
- The filesystem mounts and unmounts reliably.
- `cat` and `grep` over mounted files work deterministically.
- Jira API request volume is reduced by cache hits and stale-safe behavior.

### Key Discoveries:
- `fuser` is the Rust crate explicitly recommended in prior research (`discovery.md:490`).
- Jira `updated` value is a reliable freshness signal for read-mostly sync (`discovery.md:166`).
- Caching should prioritize conditional validation and multi-layer strategy (`discovery.md:101`).

## What We're NOT Doing

- No ticket write-back/edit support in this plan.
- No Windows-native filesystem support (WSL/WinFsp deferred).
- No advanced full-text index inside the filesystem process.
- No auth flows beyond Jira Cloud email + API token in this plan.

## Implementation Approach

Use an incremental, testable architecture:
- Start with a tiny deterministic filesystem to validate inode/path/read semantics.
- Introduce Jira client and markdown rendering behind internal interfaces.
- Add cache layers progressively (in-memory first, then persistence).
- Keep mount read-only and fail-safe (serve stale when refresh fails, otherwise return explicit I/O errors).

## Phase 1: Minimal FUSE Bootstrap

### Overview

Create a Rust project that mounts a read-only filesystem exposing exactly one file: `test.md` with content `Hello World!`.

### Changes Required:

#### 1. Project Scaffolding
**File**: `Cargo.toml`
**Changes**: Create crate metadata and dependencies (`fuser`, `libc`, optional lightweight CLI parser).

```toml
[package]
name = "jirafs"
version = "0.1.0"
edition = "2021"

[dependencies]
fuser = "0.17"
libc = "0.2"
```

#### 2. Minimal Filesystem Implementation
**File**: `src/main.rs`
**Changes**: Implement `Filesystem` trait methods required for `ls`, `stat`, `cat`: `lookup`, `getattr`, `readdir`, `open`, `read`; map fixed inodes for root and `test.md`.

```rust
const ROOT_INO: u64 = 1;
const TEST_INO: u64 = 2;
const TEST_NAME: &str = "test.md";
const TEST_CONTENT: &[u8] = b"Hello World!\n";
```

#### 3. Basic Operator Docs
**File**: `README.md`
**Changes**: Add prerequisites, run/mount/unmount instructions for Linux.

### Success Criteria:

#### Automated Verification:
- [ ] Build succeeds: `cargo build`
- [ ] Formatting passes: `cargo fmt --check`
- [ ] Lint passes: `cargo clippy --all-targets -- -D warnings`

#### Manual Verification:
- [ ] Mount succeeds with `cargo run -- <mountpoint>`
- [ ] `ls -la <mountpoint>` shows `test.md`
- [ ] `cat <mountpoint>/test.md` prints `Hello World!`
- [ ] Write attempt fails (`echo x > <mountpoint>/test.md`) with read-only semantics

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: Jira Cloud Read-Only Filesystem

### Overview

Replace static file data source with Jira Cloud-backed project directories and issue markdown files while preserving deterministic read-only behavior.

### Changes Required:

#### 1. Jira API Client and Configuration
**File**: `src/jira.rs`
**Changes**: Add Jira Cloud client using email + API token auth; implement issue fetch and project issue listing with pagination.

```rust
pub struct JiraClient {
    base_url: String,
    email: String,
    api_token: String,
    http: reqwest::Client,
}
```

#### 2. Filesystem Topology and Path Mapping
**File**: `src/fs.rs`
**Changes**: Implement root -> project dirs -> `PROJECT-123.md` files; stable inode mapping and robust `ENOENT` behavior.

```rust
// /
// /PROJ
// /PROJ/PROJ-123.md
```

#### 3. Markdown Rendering
**File**: `src/render.rs`
**Changes**: Render Jira issue fields into deterministic markdown sections (summary, status, assignee, updated, description, comments).

#### 4. In-Memory Cache Layer
**File**: `src/cache.rs`
**Changes**: Add TTL-based caches for directory listings and issue markdown payloads with freshness keyed by Jira `updated`.

```rust
pub struct CacheEntry<T> {
    pub value: T,
    pub cached_at: std::time::Instant,
    pub ttl: std::time::Duration,
    pub source_updated: Option<String>,
}
```

#### 5. Runtime Wiring
**File**: `src/main.rs`
**Changes**: Parse env vars (`JIRA_BASE_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`, `JIRA_PROJECTS`), instantiate client/cache/fs, and mount.

### Success Criteria:

#### Automated Verification:
- [ ] Unit tests pass for path parsing/inode mapping: `cargo test`
- [ ] Unit tests pass for markdown rendering snapshots: `cargo test`
- [ ] Lint and format checks pass: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
- [ ] Mocked Jira integration tests pass for pagination and cache hit/miss flows: `cargo test -- --nocapture`

#### Manual Verification:
- [ ] `ls <mountpoint>/<PROJECT>` shows issue markdown files
- [ ] `cat <mountpoint>/<PROJECT>/<KEY>.md` returns expected markdown
- [ ] `grep -R "Status:" <mountpoint>` works without mount instability
- [ ] Invalid paths return normal file-not-found behavior

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 3: Persistent Cache, Warm Start, and Operational Hardening

### Overview

Add durability and runtime resilience so the filesystem performs well in repeated read-heavy workflows and degrades gracefully under Jira/API failures.

### Changes Required:

#### 1. Persistent Cache Store
**File**: `src/cache/persistent.rs`
**Changes**: Add SQLite-backed cache for issue markdown and metadata (`updated`, cached timestamp, access stats), loaded on startup.

```rust
CREATE TABLE IF NOT EXISTS issues (
  issue_key TEXT PRIMARY KEY,
  markdown BLOB NOT NULL,
  updated TEXT,
  cached_at TEXT NOT NULL,
  access_count INTEGER NOT NULL DEFAULT 0
);
```

#### 2. Cache Policy and Stale-While-Error
**File**: `src/cache.rs`
**Changes**: Implement read policy: serve fresh, refresh on expiry, and serve stale if refresh fails and stale exists; return `EIO` only when uncached fetch fails.

#### 3. Rate Limiting + Backoff
**File**: `src/jira.rs`
**Changes**: Add bounded concurrency and 429-aware retry with `Retry-After` support.

#### 4. Warm-Start/Prefetch Hooks
**File**: `src/warmup.rs`
**Changes**: Optional startup warmup for recently updated issues in configured projects, with strict request budget.

#### 5. Observability
**File**: `src/metrics.rs`
**Changes**: Add counters for cache hit/miss/stale-served/api-requests/retries and periodic log output.

### Success Criteria:

#### Automated Verification:
- [ ] Persistent cache CRUD tests pass: `cargo test`
- [ ] Retry/backoff behavior tests pass for synthetic 429/5xx sequences: `cargo test`
- [ ] Stale-while-error behavior tests pass: `cargo test`
- [ ] Full suite passes with lint/format: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

#### Manual Verification:
- [ ] Restarting the process serves previously cached files quickly before any fresh API fetch
- [ ] Simulated Jira outage still serves stale cached content where available
- [ ] Request volume during repeated reads is visibly reduced vs uncached behavior
- [ ] Unmount/remount cycle remains stable with no cache corruption symptoms

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before considering production rollout.

---

## Testing Strategy

### Unit Tests:
- inode/path mapping and lookup behavior
- markdown rendering determinism with missing/null Jira fields
- TTL expiry logic and stale-while-error fallbacks
- persistent cache serialization/deserialization

### Integration Tests:
- mock Jira server for project listing pagination
- issue fetch lifecycle: uncached -> cached -> expired -> refreshed
- rate-limit handling with 429 and retry delays

### Manual Testing Steps:
1. Mount local filesystem and verify static bootstrap behavior.
2. Configure Jira Cloud credentials and verify project/issue tree navigation.
3. Re-run repeated `cat` and `grep` operations and confirm reduced API traffic and stable output.
4. Temporarily block Jira network access and validate stale-cache serving behavior.

## Performance Considerations

- Primary bottleneck is Jira API rate limits, not FUSE dispatch latency.
- Keep read path allocation-light (slice-based reads from cached bytes).
- Use bounded parallel fetches to avoid burst-driven throttling.
- Prefer cache correctness and resilience over aggressive prefetch volume.

## Migration Notes

- No schema/data migration required for repository bootstrapping.
- Persistent cache schema is local-only and can be recreated if corrupted.
- Future write-back support should be separate work with explicit conflict/invalidation design.

## References

- Prior research context: `discovery.md`
- Rust FUSE recommendation: `discovery.md:490`
- Jira freshness strategy (`updated`): `discovery.md:166`
- Multi-layer caching concept: `discovery.md:101`
- License baseline in repository: `LICENSE:1`
