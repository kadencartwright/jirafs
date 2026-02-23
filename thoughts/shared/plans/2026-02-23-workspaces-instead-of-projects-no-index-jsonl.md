# Workspace-First Query Model Implementation Plan

## Overview

Replace the current project-key-centric model with a workspace-first model where each workspace is a named JQL query, remove all index and JSONL sidecar surfaces, and expose ticket discovery purely through directory traversal plus grep/glob over markdown files.

## Current State Analysis

The current architecture is strongly centered around `project` as both configuration and storage unit:

- Config requires `jira.projects` and validates non-empty project keys (`src/config.rs:24`, `src/config.rs:213`).
- CLI accepts `--jira-project` and `--jira-projects` overrides (`src/main.rs:23`, `src/main.rs:65`).
- Sync composes JQL from project key and stores per-project sync cursor (`src/warmup.rs:76`, `src/cache.rs:309`).
- FUSE namespace is rooted under `projects/` with project directories and issue files (`src/fs.rs:394`, `src/fs.rs:473`).
- Persistent cache schema includes `ticket_index` and `sync_cursor(project)` (`src/cache/persistent.rs:53`, `src/cache/persistent.rs:57`).
- JSONL artifacts are currently emitted and served (`src/render.rs:128`, `src/warmup.rs:145`, `src/fs.rs:825`).

## Desired End State

The system uses workspaces as the only query-scope primitive:

- Config is `jira.workspaces.<name>.jql` (named map).
- Sync runs per workspace and stores sync cursors by workspace name.
- FUSE hierarchy is `workspaces/<workspace>/<ISSUE>.md` (+ optional markdown comment sidecar only).
- Overlapping issue membership across multiple workspaces is explicitly supported.
- No `index.jsonl` filesystem endpoint, no persistent `ticket_index` table, and no `*.jsonl` comment sidecars.
- Ticket discovery is via filesystem globbing and text search on markdown files.

### Key Discoveries:
- Project-based query composition is hardcoded in sync JQL generation (`src/warmup.rs:76`).
- Current ticket index and path generation are project-derived and JSONL-oriented (`src/cache/persistent.rs:483`, `src/fs.rs:300`).
- Comments JSONL generation and rendering are first-class today and must be removed (`src/render.rs:128`, `src/warmup.rs:145`).

## What We're NOT Doing

- No backward-compatibility adapter for `jira.projects`.
- No migration layer that supports both projects and workspaces simultaneously.
- No retention of `tickets/index.jsonl` or any equivalent aggregate index endpoint.
- No JSONL comment sidecars (`*.comments.jsonl`).
- No schema version bridge for old SQLite structures beyond fresh-create behavior.

## Implementation Approach

Hard-cut to a single workspace abstraction in config, sync, cache, and filesystem code. Remove project-centric and JSONL/index code paths entirely to reduce conceptual duplication and maintenance complexity. Keep issue frontmatter `project` as Jira metadata only, not as grouping or query context.

## Phase 1: Config and CLI Cutover to Workspaces

### Overview
Introduce workspace-first configuration primitives and remove project-era CLI/config parsing.

### Changes Required:

#### 1. Config schema and validation
**File**: `src/config.rs`
**Changes**:
- Replace `jira.projects: Vec<String>` with `jira.workspaces: HashMap<String, WorkspaceConfig>`.
- Add `WorkspaceConfig { jql: String }` with validation for non-empty workspace names and JQL.
- Remove project-specific validation messages and tests.
- Add validation for at least one workspace.

```rust
#[derive(Debug, Deserialize)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    pub workspaces: std::collections::HashMap<String, WorkspaceConfig>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceConfig {
    pub jql: String,
}
```

#### 2. CLI flags and overrides
**File**: `src/main.rs`
**Changes**:
- Remove `--jira-project` and `--jira-projects` flags and parsing.
- Add `--jira-workspace <name=jql>` repeatable override (or explicit decision to keep CLI overrides minimal and config-only).
- Update usage text and CLI tests accordingly.

```text
--jira-workspace <name=jql> (repeatable)
```

#### 3. Example config and docs
**File**: `config.example.toml`
**Changes**:
- Replace `projects = [..]` with workspace blocks under `jira.workspaces`.

```toml
[jira.workspaces.default]
jql = "project in (PROJ, OPS) ORDER BY updated DESC"
```

### Success Criteria:

#### Automated Verification:
- [ ] Config parses with workspace-only schema: `cargo test --locked config_example_parses`
- [ ] CLI parser tests pass with workspace flags: `cargo test --locked cli_parses`
- [ ] Full test suite still passes after config cutover: `cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Starting with a workspace-only TOML succeeds without project fields.
- [ ] Invalid workspace config (empty name/JQL) yields clear validation errors.
- [ ] Help text no longer mentions project flags.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: Workspace-Native Sync and Cache Model

### Overview
Move all list/sync logic from project keys to workspace names + JQL, with overlap support.

### Changes Required:

#### 1. Jira list/sync entrypoints
**File**: `src/jira.rs`
**Changes**:
- Add/replace listing API to accept arbitrary JQL instead of `project` key-only helper.
- Ensure deterministic ordering in workspace JQL execution where needed.

```rust
pub fn list_issue_refs_for_jql(&self, jql: &str) -> Result<Vec<IssueRef>, JiraError>
```

#### 2. Warmup and periodic sync
**File**: `src/warmup.rs`
**Changes**:
- Replace project loops with workspace loops (`(workspace_name, jql)`).
- Build incremental query per workspace using cursor keyed by workspace.
- Remove JSONL sidecar generation; keep markdown comment sidecar only.

```rust
let scoped_jql = match cursor {
    Some(since) => format!("({}) AND updated > \"{}\" ORDER BY updated DESC", base_jql, since),
    None => format!("({})", base_jql),
};
```

#### 3. In-memory cache structures
**File**: `src/cache.rs`
**Changes**:
- Rename `project_issues` cache map to `workspace_issues`.
- Rename accessors to workspace terminology.
- Keep issue markdown cache keyed by issue key; allow same issue in multiple workspace listings.
- Remove ticket-index accessors and JSONL-specific methods.

#### 4. Persistent cache schema
**File**: `src/cache/persistent.rs`
**Changes**:
- Change `sync_cursor(project,last_sync)` to `sync_cursor(workspace,last_sync)`.
- Remove `ticket_index` table and related queries/helpers.
- Remove `comments_jsonl` column and APIs from sidecar storage.
- Keep `issues` table unchanged for issue markdown reuse.

```sql
CREATE TABLE IF NOT EXISTS sync_cursor (
  workspace TEXT PRIMARY KEY,
  last_sync TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS issue_sidecars (
  issue_key TEXT PRIMARY KEY,
  comments_md BLOB NOT NULL,
  updated TEXT,
  cached_at TEXT NOT NULL
);
```

### Success Criteria:

#### Automated Verification:
- [ ] Workspace sync tests pass (including overlap behavior): `cargo test --locked sync`
- [ ] Persistent cache tests pass without ticket_index/jsonl dependencies: `cargo test --locked persistent`
- [ ] Full suite passes after cache/schema refactor: `cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Two workspaces with overlapping JQL both contain the same issue key.
- [ ] Incremental sync updates per workspace without cross-workspace cursor corruption.
- [ ] No JSONL sidecar files are created or persisted.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 3: Filesystem Namespace Refactor and Index Removal

### Overview
Expose workspace directories directly and remove all index/jsonl filesystem artifacts.

### Changes Required:

#### 1. Root and directory structure
**File**: `src/fs.rs`
**Changes**:
- Replace `projects` root node with `workspaces` root node.
- Replace `Node::Project` with `Node::Workspace` and rename inode helpers.
- Remove `tickets/` directory and `index.jsonl` node handling entirely.

```text
/
  .sync_meta/
  workspaces/
    <workspace>/
      <ISSUE>.md
      <ISSUE>.comments.md
```

#### 2. Sidecar handling
**File**: `src/fs.rs`
**Changes**:
- Remove `IssueFileKind::CommentsJsonl` support.
- Remove read/getattr/lookup paths for `.comments.jsonl` files.

#### 3. Main runtime wiring
**File**: `src/main.rs`
**Changes**:
- Pass workspace definitions into FUSE and sync threads instead of project list.
- Remove hydration paths tied to project-based index/persistence APIs.

### Success Criteria:

#### Automated Verification:
- [ ] Filesystem unit tests pass with workspace inode semantics: `cargo test --locked fs`
- [ ] Build succeeds after node and enum removals: `cargo build --locked`
- [ ] Clippy passes with no dead paths from index/jsonl code: `cargo clippy --all-targets --all-features --locked -- -D warnings`

#### Manual Verification:
- [ ] Mounted FS shows `workspaces/` and no `tickets/` directory.
- [ ] Workspace dirs contain `.md` and `.comments.md` files only.
- [ ] `grep -R` over mount works for discovery without index helpers.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 4: Rendering, Docs, and Contract Updates

### Overview
Align generated markdown contract and documentation with workspace-first, markdown-only discovery.

### Changes Required:

#### 1. Render and comments contract
**File**: `src/render.rs`
**Changes**:
- Remove `render_issue_comments_jsonl` function and tests.
- Update comments section text to reference markdown sidecar only.

#### 2. Documentation updates
**File**: `README.md`
**Changes**:
- Replace project config/flags/examples with workspace JQL examples.
- Remove index/jsonl references and examples.

**File**: `docs/ticket-format-v2.md`
**Changes**:
- Remove `.comments.jsonl` and `tickets/index.jsonl` contract references.
- Keep `project` frontmatter field as issue metadata, not FS grouping.

### Success Criteria:

#### Automated Verification:
- [ ] Render tests pass without JSONL comments renderer: `cargo test --locked render`
- [ ] README/config examples remain parse-valid where applicable: `cargo test --locked config_example_parses`
- [ ] Final quality gates pass: `cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-targets --all-features --locked`

#### Manual Verification:
- [ ] Docs and runtime behavior match (`workspaces` vocabulary only).
- [ ] No user-visible mention of project-based grouping as a primary concept.
- [ ] No JSONL sidecar/index files appear in mounted tree.

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Testing Strategy

### Unit Tests:
- Config validation for workspace map, empty JQL, empty workspace set.
- CLI parsing for workspace overrides and removed project flags.
- Sync query construction for full/incremental workspace runs.
- Persistent cache behavior for workspace cursor keys and markdown-only sidecars.
- Filesystem inode and path tests for `workspaces/` and markdown-only sidecars.

### Integration Tests:
- End-to-end mount smoke test with two workspaces and overlapping issue keys.
- Full sync + incremental sync cycle with cursor update per workspace.
- Assert absence of `tickets/index.jsonl` and `*.comments.jsonl` across mounted tree.

### Manual Testing Steps:
1. Mount with two workspaces defined by JQL and verify directory structure under `workspaces/`.
2. Confirm an overlapping issue appears in both workspace directories.
3. Run `grep -R "<term>" <mountpoint>/workspaces` and `glob` searches to confirm discoverability without index files.
4. Trigger manual sync and verify workspace cursors advance correctly.

## Performance Considerations

- Removing index generation avoids extra serialization and read-time aggregation for `index.jsonl`.
- Duplicate issue appearance across workspaces increases directory-entry fanout but should not duplicate issue markdown blobs in the issue-key cache.
- Workspace JQL quality directly affects sync volume; recommend documenting expected ordering/filters for operational consistency.

## Migration Notes

- This is an intentional hard cutover with zero backward-compatibility handling.
- Existing configs using `jira.projects` will fail validation until converted to `jira.workspaces.<name>.jql`.
- Existing SQLite databases with old tables are not a compatibility target; safe path is fresh cache DB recreation during rollout.

## References

- Project-bound config model: `src/config.rs:24`
- Project-bound sync query construction: `src/warmup.rs:76`
- FUSE project namespace and tickets index endpoint: `src/fs.rs:394`
- Persistent ticket index and project cursor schema: `src/cache/persistent.rs:53`
- JSONL sidecar generation: `src/render.rs:128`
