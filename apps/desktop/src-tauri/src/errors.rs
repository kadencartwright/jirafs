use serde::Serialize;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceProbeErrorKind {
    Permission,
    NotInstalled,
    Unreachable,
    ParseError,
}

#[derive(Debug, Clone)]
pub struct ServiceProbeError {
    pub kind: ServiceProbeErrorKind,
    pub message: String,
}

#[derive(Debug)]
pub struct CommandOutput {
    pub status_ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<CommandOutput, ServiceProbeError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| ServiceProbeError {
        kind: ServiceProbeErrorKind::Unreachable,
        message: format!("failed to spawn command: {error}"),
    })?;

    let exit_status = child
        .wait_timeout(timeout)
        .map_err(|error| ServiceProbeError {
            kind: ServiceProbeErrorKind::Unreachable,
            message: format!("failed while waiting for command: {error}"),
        })?
        .ok_or_else(|| {
            let _ = child.kill();
            ServiceProbeError {
                kind: ServiceProbeErrorKind::Unreachable,
                message: format!("service probe timed out after {}s", timeout.as_secs()),
            }
        })?;

    let output = child
        .wait_with_output()
        .map_err(|error| ServiceProbeError {
            kind: ServiceProbeErrorKind::Unreachable,
            message: format!("failed to collect command output: {error}"),
        })?;

    Ok(CommandOutput {
        status_ok: exit_status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

pub fn classify_probe_failure(stderr: &str) -> ServiceProbeErrorKind {
    let lowered = stderr.to_ascii_lowercase();
    if lowered.contains("permission") || lowered.contains("access denied") {
        ServiceProbeErrorKind::Permission
    } else if lowered.contains("not found")
        || lowered.contains("could not be found")
        || lowered.contains("no such file")
    {
        ServiceProbeErrorKind::NotInstalled
    } else {
        ServiceProbeErrorKind::Unreachable
    }
}
