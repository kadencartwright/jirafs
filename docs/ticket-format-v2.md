# Ticket Format V2

This document defines the canonical, deterministic markdown contract for generated Jira tickets.

## Required frontmatter

```yaml
id: ST-1010
project: ST
type: story
status: in_progress
priority: p2
assignee: "Ada"
reporter: "Bob"
labels: ["sync", "agent"]
created_at: "2026-02-21T00:00:00Z"
updated_at: "2026-02-21T01:00:00Z"
```

## Optional frontmatter

```yaml
parent: "ST-1000"
epic: "ST-999"
blocks: ["ST-2000"]
blocked_by: ["ST-1500"]
relates_to: ["DEVO-42"]
due_at: "2026-03-01T00:00:00Z"
version: 2
source_url: "https://<tenant>.atlassian.net/browse/ST-1010"
```

## Enums

- `status`: `todo | in_progress | blocked | in_review | done`
- `type`: `epic | story | task | bug | subtask`
- `priority`: `p0 | p1 | p2 | p3 | p4`

## Canonical section order

1. `## Summary`
2. `## Acceptance Criteria`
3. `## Implementation Notes`
4. `## Test Evidence`
5. `## Comments`

## Rules

- Acceptance criteria entries must be markdown checkboxes only.
- ADF export artifacts must be sanitized; never emit raw ADF node names in body text.
- Rich entities should be converted to markdown-safe output:
  - links: `[label](url)`
  - mentions: `@name`
  - hard breaks: normal markdown paragraph spacing
- Attachments must use compact lines: `- attachment: <filename> (<id>)`.
- Timestamp fields must be normalized to ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
- Token-like material must be redacted before writing ticket markdown.

## Sidecars and discovery

- Main ticket files should remain concise.
- Verbose comments are emitted into sidecar files:
  - `<KEY>.comments.md`
- Ticket discovery is done through directory traversal under `workspaces/<workspace>/` and text search over markdown files.
