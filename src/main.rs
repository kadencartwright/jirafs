use std::sync::Arc;
use std::time::Duration;

use fs_jira::cache::InMemoryCache;
use fs_jira::fs::JiraFuseFs;
use fs_jira::jira::JiraClient;
use fs_jira::logging;
use fs_jira::metrics::{spawn_metrics_logger, Metrics};
use fs_jira::sync_state::SyncState;
use fs_jira::warmup::sync_issues;
use fuser::{Config, MountOption};

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("missing required environment variable: {name}").into())
}

fn parse_projects(raw: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let projects: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect();

    if projects.is_empty() {
        return Err("JIRA_PROJECTS must contain at least one project key".into());
    }
    Ok(projects)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn spawn_periodic_sync(
    jira: Arc<JiraClient>,
    cache: Arc<InMemoryCache>,
    projects: Vec<String>,
    sync_budget: usize,
    sync_state: Arc<SyncState>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let check_interval = Duration::from_secs(1);
        loop {
            std::thread::sleep(check_interval);

            let manual_full_triggered = sync_state.check_and_clear_manual_full_trigger();
            let manual_triggered = sync_state.check_and_clear_manual_trigger();
            let time_for_sync = sync_state.seconds_until_next_sync() == 0;

            if (manual_full_triggered || manual_triggered || time_for_sync)
                && sync_state.mark_sync_start()
            {
                let reason = if manual_full_triggered {
                    "manual_full"
                } else if manual_triggered {
                    "manual"
                } else {
                    "periodic"
                };
                logging::info(format!("starting {} sync", reason));

                if manual_full_triggered {
                    for project in &projects {
                        cache.clear_sync_cursor(project);
                    }
                }

                let result =
                    sync_issues(&jira, &cache, &projects, sync_budget, manual_full_triggered);

                sync_state.mark_sync_complete();
                if manual_full_triggered {
                    sync_state.mark_full_sync_complete();
                }
                sync_state.mark_sync_end();

                logging::info(format!(
                    "{} sync complete: cached={} skipped={} errors={}",
                    reason,
                    result.issues_cached,
                    result.issues_skipped,
                    result.errors.len()
                ));

                if !result.errors.is_empty() {
                    for err in &result.errors {
                        logging::warn(format!("sync error: {}", err));
                    }
                }
            }
        }
    })
}

fn mount_options() -> Vec<MountOption> {
    let mut options = vec![
        MountOption::FSName("fs-jira".to_string()),
        MountOption::DefaultPermissions,
        MountOption::RO,
    ];

    if cfg!(target_os = "macos") {
        options.push(MountOption::NoAtime);
    }

    options
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    let mut args = std::env::args_os();
    let _program = args.next();
    let mountpoint = match args.next() {
        Some(path) => path,
        None => {
            return Err("usage: cargo run -- <mountpoint>".into());
        }
    };
    let mountpoint_path = std::path::PathBuf::from(&mountpoint);
    if !mountpoint_path.exists() {
        std::fs::create_dir_all(&mountpoint_path)?;
        logging::info(format!(
            "created missing mountpoint {}",
            mountpoint_path.display()
        ));
    }

    let base_url = required_env("JIRA_BASE_URL")?;
    let email = required_env("JIRA_EMAIL")?;
    let token = required_env("JIRA_API_TOKEN")?;
    let projects = parse_projects(&required_env("JIRA_PROJECTS")?)?;
    let ttl_secs = env_u64("JIRA_CACHE_TTL_SECS", 30);
    let metrics_interval_secs = env_u64("FS_JIRA_METRICS_INTERVAL_SECS", 60);
    let sync_budget = env_usize("FS_JIRA_SYNC_BUDGET", 1000);
    let sync_interval_secs = env_u64("FS_JIRA_SYNC_INTERVAL_SECS", 60);
    let metrics = Arc::new(Metrics::new());

    logging::info(format!(
        "starting fs-jira projects={} ttl={}s sync_budget={} sync_interval={}s",
        projects.join(","),
        ttl_secs,
        sync_budget,
        sync_interval_secs
    ));

    spawn_metrics_logger(
        Arc::clone(&metrics),
        Duration::from_secs(metrics_interval_secs.max(1)),
    );

    let jira = Arc::new(JiraClient::new_with_metrics(
        base_url,
        email,
        token,
        Arc::clone(&metrics),
    )?);
    logging::info(format!("using jira base url {}", jira.base_url));

    let cache = if let Ok(path) = std::env::var("FS_JIRA_CACHE_DB") {
        logging::info(format!("persistent cache enabled at {}", path));
        Arc::new(InMemoryCache::with_persistence(
            Duration::from_secs(ttl_secs),
            Duration::from_secs(ttl_secs),
            std::path::Path::new(&path),
            Arc::clone(&metrics),
        )?)
    } else {
        return Err("FS_JIRA_CACHE_DB is required for incremental sync".into());
    };

    let mut hydrated_projects = 0usize;
    for project in &projects {
        if let Some(issue_refs) = cache.list_project_issue_refs_from_persistence(project) {
            if !issue_refs.is_empty() {
                cache.upsert_project_issues(project, issue_refs);
                hydrated_projects += 1;
            }
        }
    }
    logging::info(format!(
        "hydrated {} project listings from persistent cache",
        hydrated_projects
    ));

    let sync_state = Arc::new(SyncState::new(Duration::from_secs(sync_interval_secs)));
    logging::info("initial sync will start right after mount");
    sync_state.mark_sync_complete();

    let _sync_thread = spawn_periodic_sync(
        Arc::clone(&jira),
        Arc::clone(&cache),
        projects.clone(),
        sync_budget,
        Arc::clone(&sync_state),
    );

    let fs = JiraFuseFs::new(
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
        projects.clone(),
        Arc::clone(&jira),
        Arc::clone(&cache),
        sync_budget,
        Arc::clone(&sync_state),
    );

    let mut config = Config::default();
    config.mount_options.extend(mount_options());

    logging::info(format!(
        "mounting filesystem at {}",
        mountpoint_path.display()
    ));
    fuser::mount2(fs, mountpoint_path, &config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_options_include_required_defaults() {
        let options = mount_options();

        assert!(options
            .iter()
            .any(|option| matches!(option, MountOption::FSName(name) if name == "fs-jira")));
        assert!(options.contains(&MountOption::DefaultPermissions));
        assert!(options.contains(&MountOption::RO));
        assert!(!options.contains(&MountOption::RW));
    }
}
