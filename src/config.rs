use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub jira: JiraConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    pub workspaces: HashMap<String, WorkspaceConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkspaceConfig {
    pub jql: String,
}

#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    pub db_path: String,
    #[serde(default = "default_cache_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            db_path: String::new(),
            ttl_secs: default_cache_ttl_secs(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SyncConfig {
    #[serde(default = "default_sync_budget")]
    pub budget: usize,
    #[serde(default = "default_sync_interval_secs")]
    pub interval_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            budget: default_sync_budget(),
            interval_secs: default_sync_interval_secs(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_metrics_interval_secs")]
    pub interval_secs: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_metrics_interval_secs(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub debug: bool,
}

#[derive(Debug, Default)]
pub struct AppConfigOverrides {
    pub jira_base_url: Option<String>,
    pub jira_email: Option<String>,
    pub jira_api_token: Option<String>,
    pub jira_workspaces: Option<HashMap<String, WorkspaceConfig>>,
    pub cache_db_path: Option<String>,
    pub cache_ttl_secs: Option<u64>,
    pub sync_budget: Option<usize>,
    pub sync_interval_secs: Option<u64>,
    pub metrics_interval_secs: Option<u64>,
    pub logging_debug: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file not found at {path}. expected at $XDG_CONFIG_HOME/jirafs/config.toml or ~/.config/jirafs/config.toml")]
    MissingConfigFile { path: PathBuf },
    #[error("failed to resolve config path: HOME is not set and XDG_CONFIG_HOME is unset")]
    MissingHomeDirectory,
    #[error("failed to read config file at {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse TOML config at {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

pub fn load() -> Result<AppConfig, ConfigError> {
    let path = resolve_config_path()?;
    load_from(&path)
}

pub fn load_from(path: &std::path::Path) -> Result<AppConfig, ConfigError> {
    let path = path.to_path_buf();
    let raw = std::fs::read_to_string(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ConfigError::MissingConfigFile { path: path.clone() }
        } else {
            ConfigError::ReadFailed {
                path: path.clone(),
                source,
            }
        }
    })?;

    let cfg = toml::from_str::<AppConfig>(&raw).map_err(|source| ConfigError::ParseFailed {
        path: path.clone(),
        source,
    })?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn resolve_config_path() -> Result<PathBuf, ConfigError> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    resolve_config_path_from_env(xdg_config_home, home)
}

fn resolve_config_path_from_env(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, ConfigError> {
    if let Some(dir) = xdg_config_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir).join("jirafs").join("config.toml"));
    }

    let home = home
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::MissingHomeDirectory)?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("jirafs")
        .join("config.toml"))
}

impl AppConfig {
    pub fn apply_overrides(&mut self, overrides: &AppConfigOverrides) -> Result<(), ConfigError> {
        if let Some(value) = &overrides.jira_base_url {
            self.jira.base_url = value.clone();
        }
        if let Some(value) = &overrides.jira_email {
            self.jira.email = value.clone();
        }
        if let Some(value) = &overrides.jira_api_token {
            self.jira.api_token = value.clone();
        }
        if let Some(value) = &overrides.jira_workspaces {
            self.jira.workspaces = value.clone();
        }
        if let Some(value) = &overrides.cache_db_path {
            self.cache.db_path = value.clone();
        }
        if let Some(value) = overrides.cache_ttl_secs {
            self.cache.ttl_secs = value;
        }
        if let Some(value) = overrides.sync_budget {
            self.sync.budget = value;
        }
        if let Some(value) = overrides.sync_interval_secs {
            self.sync.interval_secs = value;
        }
        if let Some(value) = overrides.metrics_interval_secs {
            self.metrics.interval_secs = value;
        }
        if let Some(value) = overrides.logging_debug {
            self.logging.debug = value;
        }

        self.validate()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.jira.base_url.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "jira.base_url must not be empty".into(),
            ));
        }
        if self.jira.email.trim().is_empty() {
            return Err(ConfigError::Invalid("jira.email must not be empty".into()));
        }
        if self.jira.api_token.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "jira.api_token must not be empty".into(),
            ));
        }
        if self.jira.workspaces.is_empty() {
            return Err(ConfigError::Invalid(
                "jira.workspaces must contain at least one workspace".into(),
            ));
        }
        for (name, workspace) in &self.jira.workspaces {
            if name.trim().is_empty() {
                return Err(ConfigError::Invalid(
                    "jira.workspaces must not include empty workspace names".into(),
                ));
            }
            if workspace.jql.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "jira.workspaces.{name}.jql must not be empty"
                )));
            }
        }
        if self.cache.db_path.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "cache.db_path must not be empty".into(),
            ));
        }
        if self.cache.ttl_secs == 0 {
            return Err(ConfigError::Invalid("cache.ttl_secs must be > 0".into()));
        }
        if self.sync.budget == 0 {
            return Err(ConfigError::Invalid("sync.budget must be > 0".into()));
        }
        if self.sync.interval_secs == 0 {
            return Err(ConfigError::Invalid(
                "sync.interval_secs must be > 0".into(),
            ));
        }
        if self.metrics.interval_secs == 0 {
            return Err(ConfigError::Invalid(
                "metrics.interval_secs must be > 0".into(),
            ));
        }

        Ok(())
    }
}

const fn default_cache_ttl_secs() -> u64 {
    30
}

const fn default_sync_budget() -> usize {
    1000
}

const fn default_sync_interval_secs() -> u64 {
    60
}

const fn default_metrics_interval_secs() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_path_prefers_xdg_config_home() {
        let path = resolve_config_path_from_env(
            Some(OsString::from("/tmp/xdg-home")),
            Some(OsString::from("/tmp/home")),
        )
        .expect("xdg path should resolve");

        assert_eq!(path, PathBuf::from("/tmp/xdg-home/jirafs/config.toml"));
    }

    #[test]
    fn resolve_path_falls_back_to_home_dot_config() {
        let path = resolve_config_path_from_env(None, Some(OsString::from("/tmp/home")))
            .expect("home path should resolve");

        assert_eq!(path, PathBuf::from("/tmp/home/.config/jirafs/config.toml"));
    }

    #[test]
    fn resolve_path_requires_home_when_xdg_missing() {
        let err = resolve_config_path_from_env(None, None).expect_err("resolution should fail");
        assert!(matches!(err, ConfigError::MissingHomeDirectory));
    }

    #[test]
    fn validates_rejects_empty_workspaces() {
        let raw = r#"
            [jira]
            base_url = "https://example.atlassian.net"
            email = "you@example.com"
            api_token = "token"
            [jira.workspaces]

            [cache]
            db_path = "/tmp/jirafs-cache.db"
        "#;

        let cfg: AppConfig = toml::from_str(raw).expect("toml should parse");
        let err = cfg.validate().expect_err("empty workspaces should fail");
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn validates_rejects_non_positive_values() {
        let raw = r#"
            [jira]
            base_url = "https://example.atlassian.net"
            email = "you@example.com"
            api_token = "token"

            [jira.workspaces.default]
            jql = "project = PROJ ORDER BY updated DESC"

            [cache]
            db_path = "/tmp/jirafs-cache.db"
            ttl_secs = 0

            [sync]
            budget = 0
            interval_secs = 0

            [metrics]
            interval_secs = 0
        "#;

        let cfg: AppConfig = toml::from_str(raw).expect("toml should parse");
        let err = cfg.validate().expect_err("invalid values should fail");
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn config_example_parses() {
        let raw = include_str!("../config.example.toml");
        let cfg: AppConfig = toml::from_str(raw).expect("example config should parse");
        cfg.validate().expect("example config should validate");
    }

    #[test]
    fn apply_overrides_updates_values() {
        let raw = include_str!("../config.example.toml");
        let mut cfg: AppConfig = toml::from_str(raw).expect("example config should parse");

        let overrides = AppConfigOverrides {
            jira_base_url: Some("https://override.atlassian.net".into()),
            jira_email: Some("override@example.com".into()),
            jira_api_token: Some("override-token".into()),
            jira_workspaces: Some(HashMap::from([(
                "ops".to_string(),
                WorkspaceConfig {
                    jql: "project = OPS ORDER BY updated DESC".to_string(),
                },
            )])),
            cache_db_path: Some("/tmp/override.db".into()),
            cache_ttl_secs: Some(15),
            sync_budget: Some(250),
            sync_interval_secs: Some(30),
            metrics_interval_secs: Some(20),
            logging_debug: Some(true),
        };

        cfg.apply_overrides(&overrides)
            .expect("overrides should validate");

        assert_eq!(cfg.jira.base_url, "https://override.atlassian.net");
        assert_eq!(cfg.jira.email, "override@example.com");
        assert_eq!(cfg.jira.api_token, "override-token");
        assert_eq!(cfg.jira.workspaces.len(), 1);
        assert_eq!(
            cfg.jira
                .workspaces
                .get("ops")
                .map(|workspace| workspace.jql.as_str()),
            Some("project = OPS ORDER BY updated DESC")
        );
        assert_eq!(cfg.cache.db_path, "/tmp/override.db");
        assert_eq!(cfg.cache.ttl_secs, 15);
        assert_eq!(cfg.sync.budget, 250);
        assert_eq!(cfg.sync.interval_secs, 30);
        assert_eq!(cfg.metrics.interval_secs, 20);
        assert!(cfg.logging.debug);
    }
}
