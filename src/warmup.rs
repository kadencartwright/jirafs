use std::sync::Arc;

use crate::cache::InMemoryCache;
use crate::jira::JiraClient;
use crate::logging;
use crate::render::{
    render_issue_comments_jsonl, render_issue_comments_markdown, render_issue_markdown,
};

pub fn seed_project_listings(
    jira: &JiraClient,
    cache: &InMemoryCache,
    projects: &[String],
) -> usize {
    let mut seeded = 0;
    for project in projects {
        match jira.list_project_issue_refs(project) {
            Ok(items) => {
                let count = items.len();
                cache.upsert_project_issues(project, items);
                logging::info(format!(
                    "seeded project listing for {} with {} issues",
                    project, count
                ));
                seeded += 1;
            }
            Err(err) => {
                logging::warn(format!("failed to seed project {}: {}", project, err));
            }
        }
    }
    seeded
}

pub struct SyncResult {
    pub issues_cached: usize,
    pub issues_skipped: usize,
    pub errors: Vec<String>,
}

pub fn sync_issues(
    jira: &JiraClient,
    cache: &Arc<InMemoryCache>,
    projects: &[String],
    budget: usize,
    force_full: bool,
) -> SyncResult {
    let mut result = SyncResult {
        issues_cached: 0,
        issues_skipped: 0,
        errors: Vec::new(),
    };

    if budget == 0 {
        return result;
    }

    if !cache.has_persistence() {
        result
            .errors
            .push("cache.db_path must be configured for sync".to_string());
        return result;
    }

    for project in projects {
        let cursor = if force_full {
            None
        } else {
            cache.get_sync_cursor(project)
        };

        let jql = match &cursor {
            Some(since) => {
                logging::info(format!("incremental sync for {} since {}", project, since));
                format!(
                    "project={} AND updated > \"{}\" ORDER BY updated DESC",
                    project, since
                )
            }
            None => {
                logging::info(format!("initial full sync for {}", project));
                format!("project={} ORDER BY updated DESC", project)
            }
        };

        let page_size = budget.min(100);

        match jira.search_issues_bulk(&jql, page_size) {
            Ok(issues) => {
                let latest_refs: Vec<_> = issues
                    .iter()
                    .map(|issue| crate::jira::IssueRef {
                        key: issue.key.clone(),
                        updated: issue.updated.clone(),
                    })
                    .collect();

                if cursor.is_none() {
                    cache.upsert_project_issues(project, latest_refs);
                } else {
                    let mut merged = cache
                        .get_project_issues_snapshot(project)
                        .map(|snapshot| snapshot.issues)
                        .unwrap_or_default();

                    for new_ref in latest_refs {
                        if let Some(existing) =
                            merged.iter_mut().find(|item| item.key == new_ref.key)
                        {
                            existing.updated = new_ref.updated.clone();
                        } else {
                            merged.push(new_ref);
                        }
                    }

                    merged.sort_by(|a, b| a.key.cmp(&b.key));
                    cache.upsert_project_issues(project, merged);
                }

                if issues.is_empty() {
                    logging::info(format!("sync for {}: no changes", project));
                    result.issues_skipped += 1;
                    continue;
                }

                let remaining_budget = budget.saturating_sub(result.issues_cached);
                let count = issues.len().min(remaining_budget);

                let to_cache: Vec<_> = issues
                    .iter()
                    .take(count)
                    .map(|issue| {
                        let markdown = render_issue_markdown(issue).into_bytes();
                        (issue.key.clone(), markdown, issue.updated.clone())
                    })
                    .collect();

                let sidecars: Vec<_> = issues
                    .iter()
                    .take(count)
                    .map(|issue| {
                        (
                            issue.key.clone(),
                            render_issue_comments_markdown(issue).into_bytes(),
                            render_issue_comments_jsonl(issue).into_bytes(),
                            issue.updated.clone(),
                        )
                    })
                    .collect();

                let cached = cache.upsert_issues_batch(&to_cache);
                let _ = cache.upsert_issue_sidecars_batch(&sidecars);
                result.issues_cached += cached;

                if let Some(latest) = issues.first().and_then(|i| i.updated.as_ref()) {
                    cache.set_sync_cursor(project, latest);
                    logging::info(format!("updated sync cursor for {} to {}", project, latest));
                }

                logging::info(format!("sync for {}: cached {} issues", project, cached));

                if result.issues_cached >= budget {
                    break;
                }
            }
            Err(err) => {
                let msg = format!("sync failed for {}: {}", project, err);
                logging::warn(&msg);
                result.errors.push(msg);
            }
        }
    }

    result
}
