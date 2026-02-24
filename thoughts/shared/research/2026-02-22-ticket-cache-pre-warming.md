---
date: 2026-02-22T12:00:00Z
researcher: k
git_commit: 62d9e51f8f8f85dae7e16ec09e98d4ac2b914724
branch: main
repository: jirafs
topic: "How ticket caches are pre-warmed"
tags: [research, codebase, caching, warmup, fuse, jira]
status: complete
last_updated: 2026-02-22
last_updated_by: k
---

# Research: Ticket Cache Pre-warming in jirafs

**Date**: 2026-02-22T12:00:00Z
**Researcher**: k
**Git Commit**: 62d9e51f8f8f85dae7e16ec09e98d4ac2b914724
**Branch**: main
**Repository**: jirafs

## Research Question

How does the jirafs FUSE filesystem pre-warm ticket caches at startup?

## Summary

The jirafs Rust FUSE filesystem implements a two-phase cache pre-warming strategy at startup:
1. **Project listing seeding** - Fetches all issue references for configured projects
2. **Recent issue warming** - Prefetches the most recently updated issues up to a configurable budget

The implementation uses TTL-based in-memory caching with optional SQLite persistence for durability across restarts.

## Detailed Findings

### Phase 1: Project Listing Seeding

**File**: `src/warmup.rs:6-29`

The `seed_project_listings()` function fetches issue references for each configured project at startup:

```rust
pub fn seed_project_listings(
    jira: &JiraClient,
    cache: &InMemoryCache,
    projects: &[String],
) -> usize {
    let mut seeded = 0;
    for project in projects {
        match jira.list_project_issue_refs(project) {
            Ok(items) => {
                cache.upsert_project_issues(project, items.clone());
                seeded += 1;
            }
            Err(err) => {
                logging::warn(format!("failed to seed project {}: {}", project, err));
            }
        }
    }
    seeded
}
```

This populates the project listing cache before the FUSE mount is exposed, ensuring directory listings are immediately available.

### Phase 2: Recent Issue Warmup

**File**: `src/warmup.rs:31-77`

The `warm_recent_issues()` function prefetches recently updated tickets:

```rust
pub fn warm_recent_issues(
    jira: &JiraClient,
    cache: &InMemoryCache,
    projects: &[String],
    budget: usize,
) -> usize {
    if budget == 0 {
        return 0;
    }

    let mut warmed = 0;

    for project in projects {
        if warmed >= budget {
            break;
        }

        let mut refs = match jira.list_project_issue_refs(project) {
            Ok(v) => v,
            Err(err) => {
                logging::warn(format!("warmup list failed for {}: {}", project, err));
                continue;
            }
        };

        refs.sort_by(|a, b| b.updated.cmp(&a.updated));

        for issue in refs {
            if warmed >= budget {
                break;
            }

            let warmed_issue = cache.get_issue_markdown_stale_safe(&issue.key, || {
                let full = jira.get_issue(&issue.key).map_err(|_| ())?;
                Ok::<_, ()>((render_issue_markdown(&full).into_bytes(), full.updated))
            });

            if warmed_issue.is_ok() {
                warmed += 1;
            }
        }
    }

    warmed
}
```

Key characteristics:
- Sorts issues by `updated` timestamp (most recent first)
- Uses `get_issue_markdown_stale_safe()` for cache-aware fetching
- Limited by `budget` parameter to avoid overwhelming the Jira API

### Cache Configuration

**File**: `src/main.rs:70, 114-129`

Configuration via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `JIRAFS_WARMUP_BUDGET` | 0 | Number of recent issues to prefetch |
| `JIRAFS_CACHE_DB` | (none) | Enable SQLite persistence |
| `JIRA_CACHE_TTL_SECS` | 30 | Cache TTL for both projects and issues |

```rust
let warmup_budget = env_usize("JIRAFS_WARMUP_BUDGET", 0);
// ...
if warmup_budget > 0 {
    let warmed = warm_recent_issues(&jira, &cache, &projects, warmup_budget);
    logging::info(format!("warmup loaded {} issues", warmed));
}
```

### Cache Architecture

**File**: `src/cache.rs:32-67`

The `InMemoryCache` provides a two-tier caching system:

```rust
pub struct InMemoryCache {
    project_ttl: Duration,
    issue_ttl: Duration,
    project_issues: Mutex<HashMap<String, CacheEntry<Vec<IssueRef>>>>,
    issue_markdown: Mutex<HashMap<String, CacheEntry<CachedIssue>>>,
    persistent: Option<PersistentCache>,
    metrics: Arc<Metrics>,
}
```

#### Stale-Safe Fetch Pattern

**File**: `src/cache.rs:137-231`

The `get_issue_markdown_stale_safe()` method implements sophisticated cache logic:

```rust
pub fn get_issue_markdown_stale_safe<F, E>(
    &self,
    issue_key: &str,
    fetch: F,
) -> Result<Vec<u8>, E>
where
    F: FnOnce() -> Result<(Vec<u8>, Option<String>), E>,
    E: Clone,
{
    // 1. Check in-memory cache
    // 2. If miss, check persistent cache
    // 3. If miss, call fetch()
    // 4. If fetch fails but stale exists, serve stale
    // 5. Otherwise return error
}
```

This ensures:
- Fresh cache within TTL is served immediately
- Persistent cache hydrates on restart
- Stale content is served if refresh fails (graceful degradation)

### Invocation Flow

**File**: `src/main.rs:131-137`

Startup sequence:

```rust
// 1. Seed project listings before mount
let seeded_projects = seed_project_listings(&jira, &cache, &projects);
logging::info(format!("seeded {} project listings", seeded_projects));

// 2. Warm recent issues if budget > 0
if warmup_budget > 0 {
    let warmed = warm_recent_issues(&jira, &cache, &projects, warmup_budget);
    logging::info(format!("warmup loaded {} issues", warmed));
}

// 3. Mount FUSE filesystem
fuser::mount2(fs, mountpoint_path, &config)?;
```

## Code References

- `src/warmup.rs:6-29` - Project listing seed function
- `src/warmup.rs:31-77` - Recent issues warmup function
- `src/cache.rs:137-231` - Stale-safe cache fetch with fallback
- `src/main.rs:131-137` - Startup invocation of warmup functions

## Architecture Insights

1. **Budget-limited prefetch** - The `JIRAFS_WARMUP_BUDGET` prevents overwhelming the Jira API (default 0, recommended 25)

2. **Freshness-first sorting** - Issues are sorted by `updated` timestamp so most-relevant tickets are warmed first

3. **Graceful degradation** - Stale-safe cache logic ensures filesystem remains usable even when Jira API is unavailable

4. **Two-tier persistence** - In-memory cache for speed, SQLite for durability across restarts

5. **Metrics observability** - Cache hit/miss/stale counters are tracked and logged periodically

## Historical Context (from thoughts/)

- `thoughts/shared/plans/2026-02-21-jirafs-rust-bootstrap.md:253` - Plan specifies "Prefer cache correctness and resilience over aggressive prefetch volume"

## Related Research

- `discovery.md` - Contains extensive research on FUSE caching strategies including conditional fetching, adaptive TTL, and kernel cache integration

## Open Questions

- Could the warmup strategy be enhanced with predictive prefetching based on agent access patterns?
- Should adaptive TTL based on issue status (as documented in discovery.md) be implemented?
