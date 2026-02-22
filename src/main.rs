use std::sync::Arc;
use std::time::Duration;

use fs_jira::cache::InMemoryCache;
use fs_jira::fs::JiraFuseFs;
use fs_jira::jira::JiraClient;
use fs_jira::logging;
use fs_jira::metrics::{spawn_metrics_logger, Metrics};
use fs_jira::warmup::{seed_project_listings, warm_recent_issues};
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
    let warmup_budget = env_usize("FS_JIRA_WARMUP_BUDGET", 0);
    let metrics = Arc::new(Metrics::new());

    logging::info(format!(
        "starting fs-jira projects={} ttl={}s warmup_budget={}",
        projects.join(","),
        ttl_secs,
        warmup_budget
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

    match jira.get_myself() {
        Ok(me) => {
            logging::info(format!(
                "jira identity display_name={:?} account_id={:?} email={:?}",
                me.display_name, me.account_id, me.email_address
            ));
        }
        Err(err) => logging::warn(format!("failed jira identity probe: {}", err)),
    }

    match jira.list_visible_projects() {
        Ok(project_keys) => {
            logging::info(format!(
                "jira visible projects count={} sample={:?}",
                project_keys.len(),
                project_keys.iter().take(10).collect::<Vec<_>>()
            ));
        }
        Err(err) => logging::warn(format!("failed visible projects probe: {}", err)),
    }

    let cache = if let Ok(path) = std::env::var("FS_JIRA_CACHE_DB") {
        logging::info(format!("persistent cache enabled at {}", path));
        Arc::new(InMemoryCache::with_persistence(
            Duration::from_secs(ttl_secs),
            Duration::from_secs(ttl_secs),
            std::path::Path::new(&path),
            Arc::clone(&metrics),
        )?)
    } else {
        logging::info("persistent cache disabled");
        Arc::new(InMemoryCache::new(
            Duration::from_secs(ttl_secs),
            Duration::from_secs(ttl_secs),
            Arc::clone(&metrics),
        ))
    };

    let seeded_projects = seed_project_listings(&jira, &cache, &projects);
    logging::info(format!("seeded {} project listings", seeded_projects));

    if warmup_budget > 0 {
        let warmed = warm_recent_issues(&jira, &cache, &projects, warmup_budget);
        logging::info(format!("warmup loaded {} issues", warmed));
    }

    let fs = JiraFuseFs::new(
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
        projects,
        jira,
        cache,
    );

    let mut config = Config::default();
    config.mount_options.extend([
        MountOption::RO,
        MountOption::FSName("fs-jira".to_string()),
        MountOption::DefaultPermissions,
    ]);

    fuser::mount2(fs, mountpoint_path, &config)?;
    Ok(())
}
