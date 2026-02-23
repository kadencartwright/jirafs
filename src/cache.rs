pub mod persistent;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::jira::IssueRef;
use crate::logging;
use crate::metrics::Metrics;
use persistent::PersistentCache;

/// Batch row for issue markdown cache upserts.
pub type IssueCacheRow = (String, Vec<u8>, Option<String>);
/// Batch row for issue comments sidecar upserts.
pub type IssueSidecarRow = (String, Vec<u8>, Option<String>);

#[derive(Debug, Clone)]
/// Cached value with TTL and source metadata.
pub struct CacheEntry<T> {
    pub value: T,
    pub cached_at: Instant,
    pub ttl: Duration,
    pub source_updated: Option<String>,
}

#[derive(Debug, Clone)]
/// Snapshot of workspace issue refs with staleness signal.
pub struct WorkspaceIssuesSnapshot {
    pub issues: Vec<IssueRef>,
    pub is_stale: bool,
}

#[derive(Debug, Clone)]
struct CachedIssue {
    markdown: Vec<u8>,
}

#[derive(Debug)]
/// In-memory issue cache with optional SQLite persistence.
pub struct InMemoryCache {
    workspace_ttl: Duration,
    issue_ttl: Duration,
    workspace_issues: Mutex<HashMap<String, CacheEntry<Vec<IssueRef>>>>,
    issue_markdown: Mutex<HashMap<String, CacheEntry<CachedIssue>>>,
    persistent: Option<PersistentCache>,
    metrics: Arc<Metrics>,
}

impl InMemoryCache {
    /// Creates an in-memory cache without persistence.
    pub fn new(workspace_ttl: Duration, issue_ttl: Duration, metrics: Arc<Metrics>) -> Self {
        Self {
            workspace_ttl,
            issue_ttl,
            workspace_issues: Mutex::new(HashMap::new()),
            issue_markdown: Mutex::new(HashMap::new()),
            persistent: None,
            metrics,
        }
    }

    /// Creates an in-memory cache backed by SQLite persistence.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when opening or initializing persistence fails.
    pub fn with_persistence(
        workspace_ttl: Duration,
        issue_ttl: Duration,
        db_path: &Path,
        metrics: Arc<Metrics>,
    ) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            workspace_ttl,
            issue_ttl,
            workspace_issues: Mutex::new(HashMap::new()),
            issue_markdown: Mutex::new(HashMap::new()),
            persistent: Some(PersistentCache::new(db_path)?),
            metrics,
        })
    }

    /// Gets workspace issues from cache or via `fetch`, then caches fresh values.
    pub fn get_workspace_issues<F, E>(&self, workspace: &str, fetch: F) -> Result<Vec<IssueRef>, E>
    where
        F: FnOnce() -> Result<Vec<IssueRef>, E>,
    {
        let now = Instant::now();
        if let Some(entry) = self
            .workspace_issues
            .lock_or_recover("workspace_issues")
            .get(workspace)
            .cloned()
        {
            if now.duration_since(entry.cached_at) < entry.ttl {
                self.metrics.inc_cache_hit();
                return Ok(entry.value);
            }
        }

        self.metrics.inc_cache_miss();
        let fresh = fetch()?;
        let entry = CacheEntry {
            value: fresh.clone(),
            cached_at: now,
            ttl: self.workspace_ttl,
            source_updated: None,
        };
        self.workspace_issues
            .lock_or_recover("workspace_issues")
            .insert(workspace.to_string(), entry);
        Ok(fresh)
    }

    /// Returns a workspace issue snapshot with stale/fresh metadata.
    pub fn get_workspace_issues_snapshot(
        &self,
        workspace: &str,
    ) -> Option<WorkspaceIssuesSnapshot> {
        let now = Instant::now();
        let entry = self
            .workspace_issues
            .lock_or_recover("workspace_issues")
            .get(workspace)
            .cloned()?;

        let is_stale = now.duration_since(entry.cached_at) >= entry.ttl;
        if is_stale {
            self.metrics.inc_cache_miss();
        } else {
            self.metrics.inc_cache_hit();
        }

        Some(WorkspaceIssuesSnapshot {
            issues: entry.value,
            is_stale,
        })
    }

    /// Replaces workspace issue refs in the in-memory cache.
    pub fn upsert_workspace_issues(&self, workspace: &str, issues: Vec<IssueRef>) {
        let persisted_issues = issues.clone();
        let entry = CacheEntry {
            value: issues,
            cached_at: Instant::now(),
            ttl: self.workspace_ttl,
            source_updated: None,
        };
        self.workspace_issues
            .lock_or_recover("workspace_issues")
            .insert(workspace.to_string(), entry);

        if let Some(persistent) = &self.persistent {
            let _ = persistent.upsert_workspace_issue_refs(workspace, &persisted_issues);
        }
    }

    /// Returns issue markdown and serves stale values on refresh failure.
    pub fn get_issue_markdown_stale_safe<F, E>(
        &self,
        issue_key: &str,
        fetch: F,
    ) -> Result<Vec<u8>, E>
    where
        F: FnOnce() -> Result<(Vec<u8>, Option<String>), E>,
        E: Clone,
    {
        let now = Instant::now();
        let existing = self
            .issue_markdown
            .lock_or_recover("issue_markdown")
            .get(issue_key)
            .cloned();

        if let Some(entry) = &existing {
            if now.duration_since(entry.cached_at) < entry.ttl {
                self.metrics.inc_cache_hit();
                return Ok(entry.value.markdown.clone());
            }
        }

        if existing.is_none() {
            if let Some(persistent) = &self.persistent {
                if let Ok(Some(issue)) = persistent.get_issue(issue_key) {
                    let hydrated = CacheEntry {
                        value: CachedIssue {
                            markdown: issue.markdown.clone(),
                        },
                        cached_at: now,
                        ttl: self.issue_ttl,
                        source_updated: issue.updated,
                    };
                    self.issue_markdown
                        .lock_or_recover("issue_markdown")
                        .insert(issue_key.to_string(), hydrated);
                    self.metrics.inc_cache_hit();
                    return Ok(issue.markdown);
                }
            }
        }

        self.metrics.inc_cache_miss();
        let fetched = fetch();

        let (fresh_markdown, fresh_updated) = match fetched {
            Ok(value) => value,
            Err(err) => {
                if let Some(entry) = existing {
                    self.metrics.inc_stale_served();
                    return Ok(entry.value.markdown);
                }
                return Err(err);
            }
        };

        if let Some(mut entry) = self
            .issue_markdown
            .lock_or_recover("issue_markdown")
            .get(issue_key)
            .cloned()
        {
            if entry.source_updated == fresh_updated {
                entry.cached_at = now;
                self.issue_markdown
                    .lock_or_recover("issue_markdown")
                    .insert(issue_key.to_string(), entry.clone());
                return Ok(entry.value.markdown);
            }
        }

        let entry = CacheEntry {
            value: CachedIssue {
                markdown: fresh_markdown.clone(),
            },
            cached_at: now,
            ttl: self.issue_ttl,
            source_updated: fresh_updated.clone(),
        };
        self.issue_markdown
            .lock_or_recover("issue_markdown")
            .insert(issue_key.to_string(), entry);

        if let Some(persistent) = &self.persistent {
            let _ = persistent.upsert_issue(issue_key, &fresh_markdown, fresh_updated.as_deref());
        }

        Ok(fresh_markdown)
    }

    /// Returns in-memory markdown length in bytes for one issue.
    pub fn cached_issue_len(&self, issue_key: &str) -> Option<u64> {
        self.issue_markdown
            .lock_or_recover("issue_markdown")
            .get(issue_key)
            .map(|entry| entry.value.markdown.len() as u64)
    }

    /// Upserts one issue payload into memory and persistence.
    pub fn upsert_issue_direct(&self, issue_key: &str, markdown: &[u8], updated: Option<&str>) {
        let now = Instant::now();
        let entry = CacheEntry {
            value: CachedIssue {
                markdown: markdown.to_vec(),
            },
            cached_at: now,
            ttl: self.issue_ttl,
            source_updated: updated.map(ToString::to_string),
        };
        self.issue_markdown
            .lock_or_recover("issue_markdown")
            .insert(issue_key.to_string(), entry);

        if let Some(persistent) = &self.persistent {
            let _ = persistent.upsert_issue(issue_key, markdown, updated);
        }
    }

    /// Upserts a batch of issue payloads into memory and persistence.
    pub fn upsert_issues_batch(&self, issues: &[IssueCacheRow]) -> usize {
        let now = Instant::now();
        let mut count = 0;

        {
            let mut guard = self.issue_markdown.lock_or_recover("issue_markdown");
            for (issue_key, markdown, updated) in issues {
                let entry = CacheEntry {
                    value: CachedIssue {
                        markdown: markdown.clone(),
                    },
                    cached_at: now,
                    ttl: self.issue_ttl,
                    source_updated: updated.clone(),
                };
                guard.insert(issue_key.clone(), entry);
                count += 1;
            }
        }

        if let Some(persistent) = &self.persistent {
            let _ = persistent.upsert_issues_batch(issues);
        }

        count
    }

    /// Upserts a batch of sidecar payloads into persistence.
    pub fn upsert_issue_sidecars_batch(&self, sidecars: &[IssueSidecarRow]) -> usize {
        if let Some(persistent) = &self.persistent {
            return persistent
                .upsert_issue_sidecars_batch(sidecars)
                .unwrap_or(0);
        }
        0
    }

    /// Returns persisted sync cursor for a workspace when available.
    pub fn get_sync_cursor(&self, workspace: &str) -> Option<String> {
        self.persistent
            .as_ref()
            .and_then(|p| p.get_sync_cursor(workspace).ok().flatten())
    }

    /// Writes persisted sync cursor for a workspace when persistence is enabled.
    pub fn set_sync_cursor(&self, workspace: &str, last_sync: &str) {
        if let Some(persistent) = &self.persistent {
            let _ = persistent.set_sync_cursor(workspace, last_sync);
        }
    }

    /// Clears persisted sync cursor for a workspace when persistence is enabled.
    pub fn clear_sync_cursor(&self, workspace: &str) {
        if let Some(persistent) = &self.persistent {
            let _ = persistent.clear_sync_cursor(workspace);
        }
    }

    /// Returns persisted issue count for a project prefix.
    pub fn cached_issue_count(&self, project_prefix: &str) -> usize {
        self.persistent
            .as_ref()
            .and_then(|p| p.cached_issue_count(project_prefix).ok())
            .unwrap_or(0)
    }

    /// Reports whether persistence is configured.
    pub fn has_persistence(&self) -> bool {
        self.persistent.is_some()
    }

    /// Returns persisted issue markdown length in bytes.
    pub fn persistent_issue_len(&self, issue_key: &str) -> Option<u64> {
        self.persistent
            .as_ref()
            .and_then(|p| p.issue_markdown_len(issue_key).ok().flatten())
    }

    /// Lists persisted workspace issue refs.
    pub fn list_workspace_issue_refs_from_persistence(
        &self,
        workspace: &str,
    ) -> Option<Vec<IssueRef>> {
        self.persistent
            .as_ref()
            .and_then(|p| p.list_workspace_issue_refs(workspace).ok())
    }

    /// Returns persisted comments markdown sidecar bytes.
    pub fn persistent_comments_md(&self, issue_key: &str) -> Option<Vec<u8>> {
        self.persistent
            .as_ref()
            .and_then(|p| p.get_issue_comments_md(issue_key).ok().flatten())
    }

    /// Returns persisted comments markdown sidecar length in bytes.
    pub fn persistent_comments_md_len(&self, issue_key: &str) -> Option<u64> {
        self.persistent
            .as_ref()
            .and_then(|p| p.issue_comments_md_len(issue_key).ok().flatten())
    }
}

trait MutexExt<T> {
    fn lock_or_recover(&self, name: &'static str) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self, name: &'static str) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                logging::warn(format!("recovering poisoned mutex: {}", name));
                poisoned.into_inner()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    fn metrics() -> Arc<Metrics> {
        Arc::new(Metrics::new())
    }

    #[test]
    fn issue_cache_hits_within_ttl() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Duration::from_secs(60), metrics());
        let calls = Arc::new(AtomicUsize::new(0));

        let c1 = Arc::clone(&calls);
        let first = cache
            .get_issue_markdown_stale_safe("PROJ-1", move || {
                c1.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>((b"v1".to_vec(), Some("u1".to_string())))
            })
            .expect("first fetch");

        let c2 = Arc::clone(&calls);
        let second = cache
            .get_issue_markdown_stale_safe("PROJ-1", move || {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>((b"v2".to_vec(), Some("u2".to_string())))
            })
            .expect("second fetch");

        assert_eq!(first, b"v1");
        assert_eq!(second, b"v1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_is_served_when_refresh_fails() {
        let cache = InMemoryCache::new(Duration::from_secs(0), Duration::from_secs(0), metrics());
        let first = cache
            .get_issue_markdown_stale_safe("PROJ-1", || {
                Ok::<_, String>((b"old".to_vec(), Some("same".to_string())))
            })
            .expect("seed cache");

        let second = cache
            .get_issue_markdown_stale_safe("PROJ-1", || {
                Err::<(Vec<u8>, Option<String>), _>("boom".to_string())
            })
            .expect("returns stale instead of error");

        assert_eq!(first, b"old");
        assert_eq!(second, b"old");
    }

    #[test]
    fn warm_starts_from_persistent_cache() {
        let cache = InMemoryCache::with_persistence(
            Duration::from_secs(60),
            Duration::from_secs(60),
            Path::new(":memory:"),
            metrics(),
        )
        .expect("cache");

        cache
            .get_issue_markdown_stale_safe("PROJ-1", || {
                Ok::<_, String>((b"persisted".to_vec(), Some("u1".to_string())))
            })
            .expect("prime persistent");

        let got = cache
            .get_issue_markdown_stale_safe("PROJ-1", || {
                Err::<(Vec<u8>, Option<String>), _>("nope".to_string())
            })
            .expect("loaded from cache");
        assert_eq!(got, b"persisted");
    }
}
