use crate::errors::{
    classify_probe_failure, run_command_with_timeout, ServiceProbeError, ServiceProbeErrorKind,
};
use crate::ServiceProbe;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const SYSTEMD_UNIT_NAME: &str = "fs-jira.service";

pub fn probe_service() -> Result<ServiceProbe, ServiceProbeError> {
    let unit_path = resolve_unit_path();
    let installed = unit_path.exists();
    let (config_path, mountpoint) = if installed {
        let content = fs::read_to_string(&unit_path).map_err(|error| ServiceProbeError {
            kind: ServiceProbeErrorKind::ParseError,
            message: format!(
                "failed to read systemd unit at {}: {error}",
                unit_path.display()
            ),
        })?;
        parse_exec_start_args(&content)
    } else {
        (None, None)
    };

    let mut command = Command::new("systemctl");
    command
        .args(["--user", "is-active", SYSTEMD_UNIT_NAME])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let running_output = run_command_with_timeout(command, Duration::from_secs(2))?;
    let running = running_output.status_ok && running_output.stdout == "active";
    if !running_output.status_ok && !installed {
        let kind = classify_probe_failure(&running_output.stderr);
        if matches!(kind, ServiceProbeErrorKind::Permission) {
            return Err(ServiceProbeError {
                kind,
                message: format!(
                    "systemd probe denied while checking {}: {}",
                    SYSTEMD_UNIT_NAME, running_output.stderr
                ),
            });
        }
    }

    Ok(ServiceProbe {
        installed,
        running,
        config_path,
        mountpoint,
    })
}

fn resolve_unit_path() -> PathBuf {
    if let Some(xdg_config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config_home)
            .join("systemd")
            .join("user")
            .join(SYSTEMD_UNIT_NAME);
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("systemd")
            .join("user")
            .join(SYSTEMD_UNIT_NAME);
    }

    PathBuf::from(SYSTEMD_UNIT_NAME)
}

pub fn parse_exec_start_args(unit_content: &str) -> (Option<String>, Option<String>) {
    let mut config_path = None;
    let mut mountpoint = None;

    for line in unit_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("ExecStart=") {
            continue;
        }
        let value = trimmed.trim_start_matches("ExecStart=");
        let Some(tokens) = shlex::split(value) else {
            continue;
        };

        for (idx, token) in tokens.iter().enumerate() {
            if token == "--config" {
                config_path = tokens.get(idx + 1).cloned();
            }
        }

        mountpoint = tokens
            .iter()
            .rev()
            .find(|token| !token.starts_with('-'))
            .cloned();

        if mountpoint
            .as_deref()
            .is_some_and(|value| value.ends_with("fs-jira"))
        {
            return (config_path, mountpoint);
        }
    }

    (config_path, mountpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_systemd_exec_start_args() {
        let content = r#"
[Unit]
Description=fs-jira FUSE mount
[Service]
ExecStart=/usr/local/bin/fs-jira --config /tmp/config.toml /tmp/mount
"#;
        let (config, mountpoint) = parse_exec_start_args(content);
        assert_eq!(config.as_deref(), Some("/tmp/config.toml"));
        assert_eq!(mountpoint.as_deref(), Some("/tmp/mount"));
    }
}
