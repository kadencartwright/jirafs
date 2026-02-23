use std::sync::OnceLock;

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use serde_json::Value;

use crate::jira::IssueData;

pub fn render_issue_markdown(issue: &IssueData) -> String {
    let summary = redact_secrets(issue.summary.as_deref().unwrap_or("(no summary)"));
    let status = canonical_status(issue.status.as_deref());
    let issue_type = canonical_type(issue.issue_type.as_deref());
    let priority = canonical_priority(issue.priority.as_deref());
    let assignee = redact_secrets(issue.assignee.as_deref().unwrap_or("unassigned"));
    let reporter = redact_secrets(issue.reporter.as_deref().unwrap_or("unknown"));
    let labels = issue
        .labels
        .iter()
        .map(|label| redact_secrets(label))
        .collect::<Vec<_>>();
    let created_at = normalize_iso_utc(issue.created.as_deref());
    let updated_at = normalize_iso_utc(issue.updated.as_deref());
    let due_at = normalize_iso_utc(issue.due_at.as_deref());
    let description = adf_to_markdown(&issue.description);
    let (acceptance_criteria, implementation_notes) = split_acceptance_criteria(&description);

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", issue.key));
    out.push_str(&format!("project: {}\n", issue.project));
    out.push_str(&format!("type: {}\n", issue_type));
    out.push_str(&format!("status: {}\n", status));
    out.push_str(&format!("priority: {}\n", priority));
    out.push_str(&format!("assignee: {}\n", yaml_quote(&assignee)));
    out.push_str(&format!("reporter: {}\n", yaml_quote(&reporter)));
    out.push_str(&format!("labels: {}\n", yaml_array(&labels)));
    out.push_str(&format!("created_at: {}\n", yaml_opt(&created_at)));
    out.push_str(&format!("updated_at: {}\n", yaml_opt(&updated_at)));
    out.push_str(&format!("parent: {}\n", yaml_opt(&issue.parent)));
    out.push_str(&format!("epic: {}\n", yaml_opt(&issue.epic)));
    out.push_str(&format!("blocks: {}\n", yaml_array(&issue.blocks)));
    out.push_str(&format!("blocked_by: {}\n", yaml_array(&issue.blocked_by)));
    out.push_str(&format!("relates_to: {}\n", yaml_array(&issue.relates_to)));
    out.push_str(&format!("due_at: {}\n", yaml_opt(&due_at)));
    out.push_str("version: 2\n");
    out.push_str(&format!("source_url: {}\n", yaml_quote(&issue.source_url)));
    out.push_str("---\n\n");

    out.push_str("## Summary\n\n");
    out.push_str(&summary);
    out.push_str("\n\n");

    out.push_str("## Acceptance Criteria\n\n");
    if acceptance_criteria.is_empty() {
        out.push_str("- [ ] TBD\n\n");
    } else {
        for line in acceptance_criteria {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## Implementation Notes\n\n");
    if implementation_notes.trim().is_empty() {
        out.push_str("(none)\n");
    } else {
        out.push_str(implementation_notes.trim());
        out.push('\n');
    }
    if !issue.attachments.is_empty() {
        out.push('\n');
        for attachment in &issue.attachments {
            out.push_str(&format!(
                "- attachment: {} ({})\n",
                redact_secrets(&attachment.filename),
                attachment.id
            ));
        }
    }
    out.push('\n');

    out.push_str("## Test Evidence\n\n");
    out.push_str("(none yet)\n\n");

    out.push_str("## Comments\n\n");
    out.push_str(&format!(
        "{} comment(s). See `{}.comments.md`.\n",
        issue.comments.len(),
        issue.key
    ));

    out
}

pub fn render_issue_comments_markdown(issue: &IssueData) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} comments\n\n", issue.key));
    if issue.comments.is_empty() {
        out.push_str("(no comments)\n");
        return out;
    }

    for (idx, comment) in issue.comments.iter().enumerate() {
        let author = redact_secrets(comment.author_display_name.as_deref().unwrap_or("unknown"));
        let created =
            normalize_iso_utc(comment.created.as_deref()).unwrap_or_else(|| "unknown".to_string());
        let body = adf_to_markdown(&comment.body);
        out.push_str(&format!("## {}\n\n", idx + 1));
        out.push_str(&format!(
            "- id: {}\n",
            comment.id.clone().unwrap_or_default()
        ));
        out.push_str(&format!("- author: {}\n", author));
        out.push_str(&format!("- created_at: {}\n\n", created));
        if body.trim().is_empty() {
            out.push_str("(empty comment)\n\n");
        } else {
            out.push_str(body.trim());
            out.push_str("\n\n");
        }
    }

    out
}

fn split_acceptance_criteria(markdown: &str) -> (Vec<String>, String) {
    let mut criteria = Vec::new();
    let mut notes = Vec::new();

    for raw_line in markdown.lines() {
        let line = raw_line.trim_end();
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("- [ ]") || lower.starts_with("- [x]") {
            criteria.push(line.to_string());
        } else {
            notes.push(line.to_string());
        }
    }

    (criteria, notes.join("\n").trim().to_string())
}

fn canonical_status(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().to_ascii_lowercase().as_str() {
        "done" | "closed" | "resolved" => "done",
        "in review" | "review" | "qa" => "in_review",
        "blocked" => "blocked",
        "in progress" | "doing" | "active" => "in_progress",
        _ => "todo",
    }
}

fn canonical_type(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().to_ascii_lowercase().as_str() {
        "epic" => "epic",
        "story" => "story",
        "bug" => "bug",
        "sub-task" | "subtask" => "subtask",
        _ => "task",
    }
}

fn canonical_priority(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().to_ascii_lowercase().as_str() {
        "highest" | "blocker" => "p0",
        "high" => "p1",
        "medium" => "p2",
        "low" => "p3",
        _ => "p4",
    }
}

fn yaml_opt(v: &Option<String>) -> String {
    v.clone()
        .map_or_else(|| "null".to_string(), |s| yaml_quote(&s))
}

fn yaml_array(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }
    let items = values
        .iter()
        .map(|v| yaml_quote(v))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", items)
}

fn yaml_quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "\\\""))
}

fn normalize_iso_utc(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(ts) = DateTime::parse_from_rfc3339(value) {
        return Some(
            ts.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }

    if let Ok(ts) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(
            ts.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }

    if let Ok(ts) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(
            ts.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }

    None
}

fn adf_to_markdown(value: &Value) -> String {
    let markdown = adf_to_markdown_inner(value);
    redact_secrets(markdown.trim())
}

fn adf_to_markdown_inner(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(adf_to_markdown_inner)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => {
            let node_type = map.get("type").and_then(|v| v.as_str()).unwrap_or_default();

            match node_type {
                "text" => {
                    let text = map
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if let Some(link) = extract_mark_link(map.get("marks")) {
                        if !text.is_empty() {
                            return format!("[{}]({})", text, link);
                        }
                    }
                    text
                }
                "hardBreak" => "\n".to_string(),
                "paragraph" => {
                    let content = map
                        .get("content")
                        .map(adf_to_markdown_inner)
                        .unwrap_or_default();
                    format!("{}\n", content.trim())
                }
                "heading" => {
                    let content = map
                        .get("content")
                        .map(adf_to_markdown_inner)
                        .unwrap_or_default();
                    format!("{}\n", content.trim())
                }
                "mention" => {
                    let attrs = map.get("attrs").and_then(|v| v.as_object());
                    let display = attrs
                        .and_then(|a| a.get("text").and_then(|v| v.as_str()))
                        .or_else(|| {
                            attrs.and_then(|a| a.get("displayName").and_then(|v| v.as_str()))
                        })
                        .unwrap_or("unknown");
                    if display.starts_with('@') {
                        display.to_string()
                    } else {
                        format!("@{}", display)
                    }
                }
                "emoji" => map
                    .get("attrs")
                    .and_then(|v| v.as_object())
                    .and_then(|a| {
                        a.get("shortName")
                            .and_then(|v| v.as_str())
                            .or_else(|| a.get("text").and_then(|v| v.as_str()))
                    })
                    .unwrap_or(":emoji:")
                    .to_string(),
                "inlineCard" | "blockCard" => {
                    let url = map
                        .get("attrs")
                        .and_then(|v| v.as_object())
                        .and_then(|a| a.get("url").and_then(|v| v.as_str()))
                        .unwrap_or_default();
                    if url.is_empty() {
                        String::new()
                    } else {
                        format!("[{}]({})", url, url)
                    }
                }
                "media" | "file" => String::new(),
                _ => map
                    .get("content")
                    .map(adf_to_markdown_inner)
                    .or_else(|| map.get("text").map(adf_to_markdown_inner))
                    .unwrap_or_default(),
            }
        }
        _ => String::new(),
    }
}

fn extract_mark_link(marks: Option<&Value>) -> Option<String> {
    marks?.as_array()?.iter().find_map(|mark| {
        let kind = mark
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if kind != "link" {
            return None;
        }
        mark.get("attrs")
            .and_then(|v| v.as_object())
            .and_then(|attrs| attrs.get("href"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
    })
}

fn redact_secrets(input: &str) -> String {
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    static LONG_TOKEN: OnceLock<Regex> = OnceLock::new();

    let mut out = input.to_string();
    out = BEARER
        .get_or_init(|| {
            Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}").expect("valid bearer regex")
        })
        .replace_all(&out, "Bearer [REDACTED]")
        .to_string();
    out = ASSIGNMENT
        .get_or_init(|| {
            Regex::new(r#"(?i)(api[_-]?key|token|secret|password)\s*[:=]\s*['\"]?[A-Za-z0-9._\-]{8,}['\"]?"#)
                .expect("valid assignment regex")
        })
        .replace_all(&out, "$1=[REDACTED]")
        .to_string();
    LONG_TOKEN
        .get_or_init(|| Regex::new(r"\b[A-Za-z0-9_\-]{32,}\b").expect("valid long token regex"))
        .replace_all(&out, "[REDACTED]")
        .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::jira::{IssueAttachment, IssueComment};

    #[test]
    fn renders_schema_v2_layout() {
        let issue = IssueData {
            key: "ST-100".to_string(),
            project: "ST".to_string(),
            issue_type: Some("Story".to_string()),
            summary: Some("Sync now on mount".to_string()),
            status: Some("In Progress".to_string()),
            priority: Some("High".to_string()),
            assignee: Some("Ada".to_string()),
            reporter: Some("Bob".to_string()),
            labels: vec!["sync".to_string()],
            created: Some("2026-02-21T00:00:00.000+0000".to_string()),
            updated: Some("2026-02-21T01:00:00.000+0000".to_string()),
            parent: None,
            epic: None,
            blocks: vec![],
            blocked_by: vec![],
            relates_to: vec![],
            due_at: None,
            source_url: "https://example.atlassian.net/browse/ST-100".to_string(),
            attachments: vec![IssueAttachment {
                id: "1".to_string(),
                filename: "notes.txt".to_string(),
            }],
            description: json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"- [ ] do thing"}]}]}),
            comments: vec![IssueComment {
                id: Some("10".to_string()),
                author_display_name: Some("Chad".to_string()),
                body: json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Looks good"}]}]}),
                created: Some("2026-02-21T02:00:00.000+0000".to_string()),
            }],
        };

        let rendered = render_issue_markdown(&issue);
        assert!(rendered.contains("id: ST-100"));
        assert!(rendered.contains("status: in_progress"));
        assert!(rendered.contains("## Acceptance Criteria"));
        assert!(rendered.contains("- [ ] do thing"));
        assert!(rendered.contains("## Comments"));
        assert!(rendered.contains("ST-100.comments.md"));
    }
}
