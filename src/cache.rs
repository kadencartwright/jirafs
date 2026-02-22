pub mod persistent;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::jira::IssueRef;
use crate::metrics::Metrics;
use persistent::PersistentCache;

#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub value: T,
    pub cached_at: Instant,
    pub ttl: Duration,
    pub source_updated: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectIssuesSnapshot {
    pub issues: Vec<IssueRef>,
    pub is_stale: bool,
}

#[derive(Debug, Clone)]
struct CachedIssue {
    markdown: Vec<u8>,
}

#[derive(Debug)]
pub struct InMemoryCache {
    project_ttl: Duration,
    issue_ttl: Duration,
    project_issues: Mutex<HashMap<String, CacheEntry<Vec<IssueRef>>>>,
    issue_markdown: Mutex<HashMap<String, CacheEntry<CachedIssue>>>,
    persistent: Option<PersistentCache>,
    metrics: Arc<Metrics>,
}

impl InMemoryCache {
    pub fn new(project_ttl: Duration, issue_ttl: Duration, metrics: Arc<Metrics>) -> Self {
        Self {
            project_ttl,
            issue_ttl,
            project_issues: Mutex::new(HashMap::new()),
            issue_markdown: Mutex::new(HashMap::new()),
            persistent: None,
            metrics,
        }
    }

    pub fn with_persistence(
        project_ttl: Duration,
        issue_ttl: Duration,
        db_path: &Path,
        metrics: Arc<Metrics>,
    ) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            project_ttl,
            issue_ttl,
            project_issues: Mutex::new(HashMap::new()),
            issue_markdown: Mutex::new(HashMap::new()),
            persistent: Some(PersistentCache::new(db_path)?),
            metrics,
        })
    }

    pub fn get_project_issues<F, E>(&self, project: &str, fetch: F) -> Result<Vec<IssueRef>, E>
    where
        F: FnOnce() -> Result<Vec<IssueRef>, E>,
    {
        let now = Instant::now();
        if let Some(entry) = self
            .project_issues
            .lock()
            .expect("project cache mutex poisoned")
            .get(project)
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
            ttl: self.project_ttl,
            source_updated: None,
        };
        self.project_issues
            .lock()
            .expect("project cache mutex poisoned")
            .insert(project.to_string(), entry);
        Ok(fresh)
    }

    pub fn get_project_issues_snapshot(&self, project: &str) -> Option<ProjectIssuesSnapshot> {
        let now = Instant::now();
        let entry = self
            .project_issues
            .lock()
            .expect("project cache mutex poisoned")
            .get(project)
            .cloned()?;

        let is_stale = now.duration_since(entry.cached_at) >= entry.ttl;
        if is_stale {
            self.metrics.inc_cache_miss();
        } else {
            self.metrics.inc_cache_hit();
        }

        Some(ProjectIssuesSnapshot {
            issues: entry.value,
            is_stale,
        })
    }

    pub fn upsert_project_issues(&self, project: &str, issues: Vec<IssueRef>) {
        let entry = CacheEntry {
            value: issues,
            cached_at: Instant::now(),
            ttl: self.project_ttl,
            source_updated: None,
        };
        self.project_issues
            .lock()
            .expect("project cache mutex poisoned")
            .insert(project.to_string(), entry);
    }

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
            .lock()
            .expect("issue cache mutex poisoned")
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
                        .lock()
                        .expect("issue cache mutex poisoned")
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
            .lock()
            .expect("issue cache mutex poisoned")
            .get(issue_key)
            .cloned()
        {
            if entry.source_updated == fresh_updated {
                entry.cached_at = now;
                self.issue_markdown
                    .lock()
                    .expect("issue cache mutex poisoned")
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
            .lock()
            .expect("issue cache mutex poisoned")
            .insert(issue_key.to_string(), entry);

        if let Some(persistent) = &self.persistent {
            let _ = persistent.upsert_issue(issue_key, &fresh_markdown, fresh_updated.as_deref());
        }

        Ok(fresh_markdown)
    }

    pub fn cached_issue_len(&self, issue_key: &str) -> Option<u64> {
        self.issue_markdown
            .lock()
            .expect("issue cache mutex poisoned")
            .get(issue_key)
            .map(|entry| entry.value.markdown.len() as u64)
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
