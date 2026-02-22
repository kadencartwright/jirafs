use serde_json::Value;

use crate::jira::IssueData;

pub fn render_issue_markdown(issue: &IssueData) -> String {
    let title_summary = issue
        .summary
        .clone()
        .unwrap_or_else(|| "(no summary)".to_string());
    let status = issue
        .status
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let assignee = issue
        .assignee
        .clone()
        .unwrap_or_else(|| "Unassigned".to_string());
    let updated = issue
        .updated
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let description = extract_text(&issue.description);

    let mut out = String::new();
    out.push_str(&format!("# {} - {}\n\n", issue.key, title_summary));
    out.push_str(&format!("- Status: {}\n", status));
    out.push_str(&format!("- Assignee: {}\n", assignee));
    out.push_str(&format!("- Updated: {}\n\n", updated));
    out.push_str("## Description\n\n");
    if description.trim().is_empty() {
        out.push_str("(no description)\n\n");
    } else {
        out.push_str(description.trim());
        out.push_str("\n\n");
    }

    out.push_str("## Comments\n\n");
    if issue.comments.is_empty() {
        out.push_str("(no comments)\n");
        return out;
    }

    for (idx, comment) in issue.comments.iter().enumerate() {
        let author = comment
            .author_display_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let created = comment
            .created
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let body = extract_text(&comment.body);

        out.push_str(&format!("### Comment {}\n\n", idx + 1));
        out.push_str(&format!("- Author: {}\n", author));
        out.push_str(&format!("- Created: {}\n\n", created));
        if body.trim().is_empty() {
            out.push_str("(empty comment)\n\n");
        } else {
            out.push_str(body.trim());
            out.push_str("\n\n");
        }
    }

    out
}

fn extract_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(extract_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => {
            if let Some(Value::String(text)) = map.get("text") {
                return text.clone();
            }

            if let Some(content) = map.get("content") {
                return extract_text(content);
            }

            map.values()
                .map(extract_text)
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::jira::{IssueComment, IssueData};

    #[test]
    fn renders_markdown_deterministically() {
        let issue = IssueData {
            key: "PROJ-123".to_string(),
            summary: Some("Fix cache invalidation".to_string()),
            status: Some("In Progress".to_string()),
            assignee: Some("Ada".to_string()),
            updated: Some("2026-02-21T00:00:00.000+0000".to_string()),
            description: json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Line one"}]}]}),
            comments: vec![IssueComment {
                author_display_name: Some("Bob".to_string()),
                body: json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Looks good"}]}]}),
                created: Some("2026-02-21T01:00:00.000+0000".to_string()),
            }],
        };

        let expected = "# PROJ-123 - Fix cache invalidation\n\n- Status: In Progress\n- Assignee: Ada\n- Updated: 2026-02-21T00:00:00.000+0000\n\n## Description\n\nLine one\n\n## Comments\n\n### Comment 1\n\n- Author: Bob\n- Created: 2026-02-21T01:00:00.000+0000\n\nLooks good\n\n";
        assert_eq!(render_issue_markdown(&issue), expected);
    }

    #[test]
    fn renders_missing_fields_consistently() {
        let issue = IssueData {
            key: "PROJ-1".to_string(),
            summary: None,
            status: None,
            assignee: None,
            updated: None,
            description: Value::Null,
            comments: vec![],
        };

        let expected = "# PROJ-1 - (no summary)\n\n- Status: Unknown\n- Assignee: Unassigned\n- Updated: Unknown\n\n## Description\n\n(no description)\n\n## Comments\n\n(no comments)\n";
        assert_eq!(render_issue_markdown(&issue), expected);
    }
}
