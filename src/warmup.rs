use crate::cache::InMemoryCache;
use crate::jira::JiraClient;
use crate::logging;
use crate::render::render_issue_markdown;

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
                logging::info(format!(
                    "seeded project listing for {} with {} issues",
                    project,
                    items.len()
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
            } else {
                logging::warn(format!("warmup issue fetch failed for {}", issue.key));
            }
        }
    }

    warmed
}
