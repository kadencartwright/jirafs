use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init(debug: bool) {
    DEBUG_ENABLED.store(debug, Ordering::Relaxed);
}

fn debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

fn ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn redacted(message: &str) -> String {
    static TOML_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    static ESCAPED_TOML_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    static CLI_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    static JSON_TOKEN_RE: OnceLock<Regex> = OnceLock::new();

    let toml_token_re = TOML_TOKEN_RE.get_or_init(|| {
        Regex::new(r#"(?i)(api_token\s*=\s*["'])([^"']+)(["'])"#).expect("valid toml token regex")
    });
    let cli_token_re = CLI_TOKEN_RE.get_or_init(|| {
        Regex::new(r"(?i)(--jira-api-token\s+)(\S+)").expect("valid cli token regex")
    });
    let escaped_toml_token_re = ESCAPED_TOML_TOKEN_RE.get_or_init(|| {
        Regex::new(r#"(?i)(api_token\s*=\s*\\+")([^"]+)(\\+")"#)
            .expect("valid escaped toml token regex")
    });
    let json_token_re = JSON_TOKEN_RE.get_or_init(|| {
        Regex::new(r#"(?i)(\"api_token\"\s*:\s*\")(.*?)(\")"#).expect("valid json token regex")
    });

    let masked_toml = toml_token_re.replace_all(message, "$1***REDACTED***$3");
    let masked_escaped_toml = escaped_toml_token_re.replace_all(&masked_toml, "$1***REDACTED***$3");
    let masked_cli = cli_token_re.replace_all(&masked_escaped_toml, "$1***REDACTED***");
    let masked_json = json_token_re.replace_all(&masked_cli, "$1***REDACTED***$3");
    masked_json.into_owned()
}

pub fn debug(message: impl AsRef<str>) {
    if debug_enabled() {
        eprintln!("[{}][DEBUG] {}", ts(), redacted(message.as_ref()));
    }
}

pub fn info(message: impl AsRef<str>) {
    eprintln!("[{}][INFO] {}", ts(), redacted(message.as_ref()));
}

pub fn warn(message: impl AsRef<str>) {
    eprintln!("[{}][WARN] {}", ts(), redacted(message.as_ref()));
}

pub fn error(message: impl AsRef<str>) {
    eprintln!("[{}][ERROR] {}", ts(), redacted(message.as_ref()));
}

#[cfg(test)]
mod tests {
    use super::redacted;

    #[test]
    fn redacts_toml_api_token() {
        let input = r#"api_token = "secret-token""#;
        let output = redacted(input);
        assert_eq!(output, r#"api_token = "***REDACTED***""#);
    }

    #[test]
    fn redacts_cli_api_token() {
        let input = "jirafs --jira-api-token supersecret /tmp/mnt";
        let output = redacted(input);
        assert_eq!(output, "jirafs --jira-api-token ***REDACTED*** /tmp/mnt");
    }

    #[test]
    fn redacts_escaped_toml_api_token() {
        let input = r#"raw: Some(\"api_token = \\\"secret-token\\\"\")"#;
        let output = redacted(input);
        assert!(output.contains("***REDACTED***"));
        assert!(!output.contains("secret-token"));
    }
}
