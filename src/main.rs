use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fs_jira::cache::InMemoryCache;
use fs_jira::config::AppConfigOverrides;
use fs_jira::fs::JiraFuseFs;
use fs_jira::jira::JiraClient;
use fs_jira::logging;
use fs_jira::metrics::{spawn_metrics_logger, Metrics};
use fs_jira::sync_state::SyncState;
use fs_jira::warmup::sync_issues;
use fuser::{Config, MountOption};

const USAGE: &str = "usage: cargo run -- [flags] <mountpoint>\n\
flags:\n\
  -c, --config <path>\n\
  -h, --help\n\
  --jira-base-url <url>\n\
  --jira-email <email>\n\
  --jira-api-token <token>\n\
  --jira-workspace <name=jql> (repeatable)\n\
  --cache-db-path <path>\n\
  --cache-ttl-secs <u64>\n\
  --sync-budget <usize>\n\
  --sync-interval-secs <u64>\n\
  --metrics-interval-secs <u64>\n\
  --logging-debug <true|false>";

#[derive(Debug)]
struct CliArgs {
    mountpoint: PathBuf,
    config_path: Option<PathBuf>,
    overrides: AppConfigOverrides,
}

fn parse_cli_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<CliArgs>, String> {
    let mut iter = args.into_iter();
    let _program = iter.next();

    let mut mountpoint = None;
    let mut config_path = None;
    let mut overrides = AppConfigOverrides::default();

    while let Some(arg) = iter.next() {
        let arg_text = arg.to_string_lossy();
        match arg_text.as_ref() {
            "-h" | "--help" => {
                return Ok(None);
            }
            "-c" | "--config" => {
                config_path = Some(PathBuf::from(next_value(&mut iter, "--config")?));
            }
            "--jira-base-url" => {
                overrides.jira_base_url = Some(next_string(&mut iter, "--jira-base-url")?);
            }
            "--jira-email" => {
                overrides.jira_email = Some(next_string(&mut iter, "--jira-email")?);
            }
            "--jira-api-token" => {
                overrides.jira_api_token = Some(next_string(&mut iter, "--jira-api-token")?);
            }
            "--jira-workspace" => {
                let value = next_string(&mut iter, "--jira-workspace")?;
                let (name, jql) = parse_workspace_override(&value)?;
                overrides
                    .jira_workspaces
                    .get_or_insert_with(HashMap::new)
                    .insert(name, fs_jira::config::WorkspaceConfig { jql });
            }
            "--cache-db-path" => {
                overrides.cache_db_path = Some(next_string(&mut iter, "--cache-db-path")?);
            }
            "--cache-ttl-secs" => {
                overrides.cache_ttl_secs =
                    Some(parse_u64(&next_string(&mut iter, "--cache-ttl-secs")?)?);
            }
            "--sync-budget" => {
                overrides.sync_budget =
                    Some(parse_usize(&next_string(&mut iter, "--sync-budget")?)?);
            }
            "--sync-interval-secs" => {
                overrides.sync_interval_secs =
                    Some(parse_u64(&next_string(&mut iter, "--sync-interval-secs")?)?);
            }
            "--metrics-interval-secs" => {
                overrides.metrics_interval_secs = Some(parse_u64(&next_string(
                    &mut iter,
                    "--metrics-interval-secs",
                )?)?);
            }
            "--logging-debug" => {
                overrides.logging_debug =
                    Some(parse_bool(&next_string(&mut iter, "--logging-debug")?)?);
            }
            "--" => {
                if mountpoint.is_none() {
                    let value = iter
                        .next()
                        .ok_or_else(|| format!("missing mountpoint\n{USAGE}"))?;
                    mountpoint = Some(PathBuf::from(value));
                }
                if iter.next().is_some() {
                    return Err(format!("unexpected extra positional arguments\n{USAGE}"));
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown flag: {value}\n{USAGE}"));
            }
            _ => {
                if mountpoint.is_some() {
                    return Err(format!(
                        "unexpected extra positional argument: {arg_text}\n{USAGE}"
                    ));
                }
                mountpoint = Some(PathBuf::from(arg));
            }
        }
    }

    let mountpoint = mountpoint.ok_or_else(|| format!("missing mountpoint\n{USAGE}"))?;
    Ok(Some(CliArgs {
        mountpoint,
        config_path,
        overrides,
    }))
}

fn next_value(iter: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<OsString, String> {
    iter.next()
        .ok_or_else(|| format!("missing value for {flag}\n{USAGE}"))
}

fn next_string(iter: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<String, String> {
    let value = next_value(iter, flag)?;
    value
        .into_string()
        .map_err(|_| format!("{flag} value must be valid UTF-8"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid integer value: {value}"))
}

fn parse_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid integer value: {value}"))
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid boolean value: {value}")),
    }
}

fn parse_workspace_override(value: &str) -> Result<(String, String), String> {
    let Some((name, jql)) = value.split_once('=') else {
        return Err(format!(
            "invalid workspace override '{value}': expected <name=jql>"
        ));
    };
    let name = name.trim();
    let jql = jql.trim();
    if name.is_empty() {
        return Err("workspace name in --jira-workspace must not be empty".to_string());
    }
    if jql.is_empty() {
        return Err(format!("workspace jql for '{name}' must not be empty"));
    }
    Ok((name.to_string(), jql.to_string()))
}

fn spawn_periodic_sync(
    jira: Arc<JiraClient>,
    cache: Arc<InMemoryCache>,
    workspaces: Vec<(String, String)>,
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
                    for (workspace, _) in &workspaces {
                        cache.clear_sync_cursor(workspace);
                    }
                }

                let result = sync_issues(
                    &jira,
                    &cache,
                    &workspaces,
                    sync_budget,
                    manual_full_triggered,
                );

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
    let cli = parse_cli_args(std::env::args_os())
        .map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;

    let cli = match cli {
        Some(cli) => cli,
        None => {
            eprintln!("{USAGE}");
            return Ok(());
        }
    };

    let mut app_config = if let Some(config_path) = cli.config_path.as_deref() {
        fs_jira::config::load_from(config_path)?
    } else {
        fs_jira::config::load()?
    };

    app_config.apply_overrides(&cli.overrides)?;
    logging::init(app_config.logging.debug);

    if let Some(config_path) = cli.config_path.as_deref() {
        logging::info(format!(
            "loaded config from override path {}",
            config_path.display()
        ));
    }

    let mountpoint_path = cli.mountpoint;
    if !mountpoint_path.exists() {
        std::fs::create_dir_all(&mountpoint_path)?;
        logging::info(format!(
            "created missing mountpoint {}",
            mountpoint_path.display()
        ));
    }

    let mut workspaces: Vec<(String, String)> = app_config
        .jira
        .workspaces
        .iter()
        .map(|(name, workspace)| (name.clone(), workspace.jql.clone()))
        .collect();
    workspaces.sort_by(|a, b| a.0.cmp(&b.0));
    let ttl_secs = app_config.cache.ttl_secs;
    let metrics_interval_secs = app_config.metrics.interval_secs;
    let sync_budget = app_config.sync.budget;
    let sync_interval_secs = app_config.sync.interval_secs;
    let metrics = Arc::new(Metrics::new());

    logging::info(format!(
        "starting fs-jira workspaces={} ttl={}s sync_budget={} sync_interval={}s",
        workspaces
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(","),
        ttl_secs,
        sync_budget,
        sync_interval_secs
    ));

    spawn_metrics_logger(
        Arc::clone(&metrics),
        Duration::from_secs(metrics_interval_secs.max(1)),
    );

    let jira = Arc::new(JiraClient::new_with_metrics(
        app_config.jira.base_url,
        app_config.jira.email,
        app_config.jira.api_token,
        Arc::clone(&metrics),
    )?);
    logging::info(format!("using jira base url {}", jira.base_url));

    logging::info(format!(
        "persistent cache enabled at {}",
        app_config.cache.db_path
    ));
    let cache = Arc::new(InMemoryCache::with_persistence(
        Duration::from_secs(ttl_secs),
        Duration::from_secs(ttl_secs),
        Path::new(&app_config.cache.db_path),
        Arc::clone(&metrics),
    )?);

    let mut hydrated_workspaces = 0usize;
    for (workspace, _) in &workspaces {
        if let Some(issue_refs) = cache.list_workspace_issue_refs_from_persistence(workspace) {
            if !issue_refs.is_empty() {
                cache.upsert_workspace_issues(workspace, issue_refs);
                hydrated_workspaces += 1;
            }
        }
    }
    logging::info(format!(
        "hydrated {} workspace listings from persistent cache",
        hydrated_workspaces
    ));

    let sync_state = Arc::new(SyncState::new(Duration::from_secs(sync_interval_secs)));
    logging::info("initial sync will start right after mount");
    sync_state.mark_sync_complete();

    let _sync_thread = spawn_periodic_sync(
        Arc::clone(&jira),
        Arc::clone(&cache),
        workspaces.clone(),
        sync_budget,
        Arc::clone(&sync_state),
    );

    let fs = JiraFuseFs::new(
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
        workspaces.clone(),
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

    #[test]
    fn cli_parses_config_override_and_scalar_flags() {
        let args = vec![
            OsString::from("fs-jira"),
            OsString::from("-c"),
            OsString::from("/tmp/custom.toml"),
            OsString::from("--jira-base-url"),
            OsString::from("https://example.atlassian.net"),
            OsString::from("--sync-budget"),
            OsString::from("250"),
            OsString::from("--logging-debug"),
            OsString::from("true"),
            OsString::from("/tmp/mount"),
        ];

        let cli = parse_cli_args(args)
            .expect("cli should parse")
            .expect("expected run arguments");
        assert_eq!(cli.mountpoint, PathBuf::from("/tmp/mount"));
        assert_eq!(cli.config_path, Some(PathBuf::from("/tmp/custom.toml")));
        assert_eq!(
            cli.overrides.jira_base_url,
            Some("https://example.atlassian.net".into())
        );
        assert_eq!(cli.overrides.sync_budget, Some(250));
        assert_eq!(cli.overrides.logging_debug, Some(true));
    }

    #[test]
    fn cli_parses_repeatable_workspace_flags() {
        let args = vec![
            OsString::from("fs-jira"),
            OsString::from("--jira-workspace"),
            OsString::from("default=project in (PROJ, OPS) ORDER BY updated DESC"),
            OsString::from("--jira-workspace"),
            OsString::from("eng=project = ENG"),
            OsString::from("/tmp/mount"),
        ];

        let cli = parse_cli_args(args)
            .expect("cli should parse")
            .expect("expected run arguments");
        assert_eq!(
            cli.overrides
                .jira_workspaces
                .as_ref()
                .and_then(|workspaces| workspaces.get("default"))
                .map(|workspace| workspace.jql.as_str()),
            Some("project in (PROJ, OPS) ORDER BY updated DESC")
        );
        assert_eq!(
            cli.overrides
                .jira_workspaces
                .as_ref()
                .and_then(|workspaces| workspaces.get("eng"))
                .map(|workspace| workspace.jql.as_str()),
            Some("project = ENG")
        );
    }

    #[test]
    fn cli_help_flag_returns_help_result() {
        let args = vec![OsString::from("fs-jira"), OsString::from("--help")];
        let result = parse_cli_args(args).expect("help should parse");
        assert!(result.is_none());
    }
}
