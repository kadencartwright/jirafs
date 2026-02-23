use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde::Deserialize;
use serde_json::Value;

use crate::logging;
use crate::metrics::Metrics;

#[derive(Debug, Clone)]
/// Lightweight issue reference returned by listing APIs.
pub struct IssueRef {
    pub key: String,
    pub updated: Option<String>,
}

#[derive(Debug, Clone)]
/// Identity payload returned by Jira's `/myself` endpoint.
pub struct JiraIdentity {
    pub account_id: Option<String>,
    pub display_name: Option<String>,
    pub email_address: Option<String>,
}

#[derive(Debug, Clone)]
/// Render-ready Jira comment payload.
pub struct IssueComment {
    pub id: Option<String>,
    pub author_display_name: Option<String>,
    pub body: Value,
    pub created: Option<String>,
}

#[derive(Debug, Clone)]
/// Metadata for a Jira issue attachment.
pub struct IssueAttachment {
    pub id: String,
    pub filename: String,
}

#[derive(Debug, Clone)]
/// Normalized Jira issue payload used by render and sync flows.
pub struct IssueData {
    pub key: String,
    pub project: String,
    pub issue_type: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub labels: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub parent: Option<String>,
    pub epic: Option<String>,
    pub blocks: Vec<String>,
    pub blocked_by: Vec<String>,
    pub relates_to: Vec<String>,
    pub due_at: Option<String>,
    pub source_url: String,
    pub attachments: Vec<IssueAttachment>,
    pub description: Value,
    pub comments: Vec<IssueComment>,
}

#[derive(Debug, thiserror::Error)]
/// Errors returned by [`JiraClient`].
pub enum JiraError {
    #[error("jira request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("jira returned HTTP {status}: {body}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("failed to decode jira response: {source}; body: {body}")]
    Decode {
        source: serde_json::Error,
        body: String,
    },
    #[error("invalid jira.base_url '{0}'")]
    InvalidBaseUrl(String),
}

#[derive(Debug)]
struct Limiter {
    max: usize,
    in_flight: Mutex<usize>,
    cv: Condvar,
}

#[derive(Debug)]
struct Permit<'a> {
    limiter: &'a Limiter,
}

impl Limiter {
    fn new(max: usize) -> Self {
        Self {
            max: max.max(1),
            in_flight: Mutex::new(0),
            cv: Condvar::new(),
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut current = lock_or_recover(&self.in_flight, "jira limiter in_flight");
        while *current >= self.max {
            current = wait_or_recover(&self.cv, current, "jira limiter wait");
        }
        *current += 1;
        Permit { limiter: self }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut current = lock_or_recover(&self.limiter.in_flight, "jira limiter in_flight");
        *current = current.saturating_sub(1);
        self.limiter.cv.notify_one();
    }
}

#[derive(Debug, Clone)]
/// Blocking Jira API client with bounded retry and request concurrency.
pub struct JiraClient {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    pub http: Client,
    max_retries: usize,
    limiter: Arc<Limiter>,
    metrics: Arc<Metrics>,
}

impl JiraClient {
    /// Creates a Jira client with default metrics and retry settings.
    ///
    /// # Errors
    /// Returns [`JiraError`] when URL normalization or HTTP client construction fails.
    pub fn new(base_url: String, email: String, api_token: String) -> Result<Self, JiraError> {
        Self::new_with_metrics(base_url, email, api_token, Arc::new(Metrics::new()))
    }

    /// Creates a Jira client with caller-provided metrics.
    ///
    /// # Errors
    /// Returns [`JiraError`] when URL normalization or HTTP client construction fails.
    pub fn new_with_metrics(
        base_url: String,
        email: String,
        api_token: String,
        metrics: Arc<Metrics>,
    ) -> Result<Self, JiraError> {
        let http = Client::builder().build()?;
        let normalized_base_url = normalize_base_url(&base_url)?;
        Ok(Self {
            base_url: normalized_base_url,
            email,
            api_token,
            http,
            max_retries: 3,
            limiter: Arc::new(Limiter::new(4)),
            metrics,
        })
    }

    fn request_with_retry<F>(&self, mut send: F) -> Result<Response, JiraError>
    where
        F: FnMut() -> Result<Response, reqwest::Error>,
    {
        let _permit = self.limiter.acquire();
        for attempt in 0..=self.max_retries {
            self.metrics.inc_api_request();
            let response = match send() {
                Ok(resp) => resp,
                Err(err) => {
                    logging::warn(format!(
                        "jira request transport error on attempt {}: {}",
                        attempt + 1,
                        err
                    ));
                    return Err(JiraError::Request(err));
                }
            };

            if !is_retryable(response.status()) || attempt == self.max_retries {
                if !response.status().is_success() {
                    logging::warn(format!(
                        "jira request completed with status {} after {} attempt(s)",
                        response.status(),
                        attempt + 1
                    ));
                }
                return Ok(response);
            }

            let wait = retry_after_or_backoff(&response, attempt);
            logging::debug(format!(
                "jira retryable status {} attempt {} waiting {:?}",
                response.status(),
                attempt + 1,
                wait
            ));
            self.metrics.inc_retry();
            thread::sleep(wait);
        }

        Err(JiraError::Http {
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            body: "retry loop exhausted unexpectedly".to_string(),
        })
    }

    /// Lists issue keys for a Jira project in key order.
    ///
    /// # Errors
    /// Returns [`JiraError`] when request transport, HTTP status, or decode fails.
    pub fn list_project_issue_refs(&self, project: &str) -> Result<Vec<IssueRef>, JiraError> {
        let mut start_at: usize = 0;
        let mut next_page_token: Option<String> = None;
        let max_results: usize = 50;
        let jql = format!("project={} ORDER BY key ASC", project);
        let mut all = Vec::new();

        loop {
            let url = format!("{}/rest/api/3/search/jql", self.base_url);
            let response = self.request_with_retry(|| {
                let mut query = vec![
                    ("jql", jql.clone()),
                    ("fields", "updated".to_string()),
                    ("maxResults", max_results.to_string()),
                ];

                if let Some(token) = &next_page_token {
                    query.push(("nextPageToken", token.clone()));
                } else {
                    query.push(("startAt", start_at.to_string()));
                }

                self.http
                    .get(&url)
                    .basic_auth(&self.email, Some(&self.api_token))
                    .query(&query)
                    .send()
            })?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                return Err(JiraError::Http { status, body });
            }

            let body = response.text()?;
            let payload: SearchResponse = serde_json::from_str(&body).map_err(|source| {
                let short_body = if body.len() > 1000 {
                    format!("{}...", &body[..1000])
                } else {
                    body.clone()
                };
                logging::warn(format!(
                    "failed decoding Jira search response for project {}: {}",
                    project, short_body
                ));
                JiraError::Decode {
                    source,
                    body: short_body,
                }
            })?;
            let page_issues = payload.take_issues();
            let page_count = page_issues.len();
            logging::debug(format!(
                "jira list project={} page_count={} start_at={} next_page_token_present={}",
                project,
                page_count,
                start_at,
                payload
                    .next_page_token
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
            ));

            for issue in page_issues {
                all.push(IssueRef {
                    key: issue.key,
                    updated: issue.fields.updated,
                });
            }

            if let Some(token) = payload.next_page_token {
                if token.is_empty() || payload.is_last == Some(true) {
                    break;
                }
                next_page_token = Some(token);
                continue;
            }

            start_at += page_count;
            if let Some(total) = payload.total {
                if start_at >= total {
                    break;
                }
                continue;
            }

            if payload.is_last.unwrap_or(true) || page_count == 0 {
                break;
            }
        }

        if all.is_empty() {
            logging::warn(format!(
                "jira project {} returned zero issues for jql '{}'; verify project key and Browse Project permission",
                project, jql
            ));
        }

        Ok(all)
    }

    /// Fetches one Jira issue by key.
    ///
    /// # Errors
    /// Returns [`JiraError`] when request transport, HTTP status, or decode fails.
    pub fn get_issue(&self, issue_key: &str) -> Result<IssueData, JiraError> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, issue_key);
        let response = self.request_with_retry(|| {
            self.http
                .get(&url)
                .basic_auth(&self.email, Some(&self.api_token))
                .query(&[(
                    "fields",
                    "summary,status,issuetype,priority,assignee,reporter,labels,created,updated,description,comment,parent,attachment,duedate,issuelinks",
                )])
                .send()
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(JiraError::Http { status, body });
        }

        let payload: IssueResponse = response.json()?;
        let comments = payload
            .fields
            .comment
            .map(|c| {
                c.comments
                    .into_iter()
                    .map(|comment| IssueComment {
                        id: comment.id,
                        author_display_name: comment.author.and_then(|a| a.display_name),
                        body: comment.body,
                        created: comment.created,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let project = issue_key
            .split_once('-')
            .map(|(project, _)| project.to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let (blocks, blocked_by, relates_to) = categorize_links(payload.fields.issue_links.clone());

        Ok(IssueData {
            key: payload.key,
            project,
            issue_type: payload.fields.issue_type.and_then(|t| t.name),
            summary: payload.fields.summary,
            status: payload.fields.status.and_then(|s| s.name),
            priority: payload.fields.priority.and_then(|p| p.name),
            assignee: payload.fields.assignee.and_then(|a| a.display_name),
            reporter: payload.fields.reporter.and_then(|a| a.display_name),
            labels: payload.fields.labels,
            created: payload.fields.created,
            updated: payload.fields.updated,
            parent: payload.fields.parent.and_then(|p| p.key),
            epic: None,
            blocks,
            blocked_by,
            relates_to,
            due_at: payload.fields.due_date,
            source_url: format!("{}/browse/{}", self.base_url, issue_key),
            attachments: payload
                .fields
                .attachment
                .into_iter()
                .map(|a| IssueAttachment {
                    id: a.id,
                    filename: a.filename,
                })
                .collect(),
            description: payload.fields.description.unwrap_or(Value::Null),
            comments,
        })
    }

    /// Executes a Jira JQL search and returns hydrated issue payloads.
    ///
    /// # Errors
    /// Returns [`JiraError`] when request transport, HTTP status, or decode fails.
    pub fn search_issues_bulk(
        &self,
        jql: &str,
        max_results: usize,
    ) -> Result<Vec<IssueData>, JiraError> {
        let mut all = Vec::new();
        let mut start_at: usize = 0;
        let mut next_page_token: Option<String> = None;

        loop {
            let url = format!("{}/rest/api/3/search/jql", self.base_url);
            let response = self.request_with_retry(|| {
                let mut query = vec![
                    ("jql", jql.to_string()),
                    (
                        "fields",
                        "summary,status,issuetype,priority,assignee,reporter,labels,created,updated,description,comment,parent,attachment,duedate,issuelinks".to_string(),
                    ),
                    ("maxResults", max_results.to_string()),
                ];

                if let Some(token) = &next_page_token {
                    query.push(("nextPageToken", token.clone()));
                } else {
                    query.push(("startAt", start_at.to_string()));
                }

                self.http
                    .get(&url)
                    .basic_auth(&self.email, Some(&self.api_token))
                    .query(&query)
                    .send()
            })?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                return Err(JiraError::Http { status, body });
            }

            let body = response.text()?;
            let payload: BulkSearchResponse = serde_json::from_str(&body).map_err(|source| {
                logging::warn(format!("failed decoding bulk search response: {}", source));
                JiraError::Decode {
                    source,
                    body: body.chars().take(500).collect(),
                }
            })?;

            let page_issues = payload.take_issues();
            let page_count = page_issues.len();

            for issue in page_issues {
                let comments = issue
                    .fields
                    .comment
                    .map(|c| {
                        c.comments
                            .into_iter()
                            .map(|comment| IssueComment {
                                id: comment.id,
                                author_display_name: comment.author.and_then(|a| a.display_name),
                                body: comment.body,
                                created: comment.created,
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let issue_key = issue.key.clone();
                let project = issue_key
                    .split_once('-')
                    .map(|(project, _)| project.to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                let (blocks, blocked_by, relates_to) =
                    categorize_links(issue.fields.issue_links.clone());

                all.push(IssueData {
                    key: issue_key.clone(),
                    project,
                    issue_type: issue.fields.issue_type.and_then(|t| t.name),
                    summary: issue.fields.summary,
                    status: issue.fields.status.and_then(|s| s.name),
                    priority: issue.fields.priority.and_then(|p| p.name),
                    assignee: issue.fields.assignee.and_then(|a| a.display_name),
                    reporter: issue.fields.reporter.and_then(|a| a.display_name),
                    labels: issue.fields.labels,
                    created: issue.fields.created,
                    updated: issue.fields.updated.clone(),
                    parent: issue.fields.parent.and_then(|p| p.key),
                    epic: None,
                    blocks,
                    blocked_by,
                    relates_to,
                    due_at: issue.fields.due_date,
                    source_url: format!("{}/browse/{}", self.base_url, issue_key),
                    attachments: issue
                        .fields
                        .attachment
                        .into_iter()
                        .map(|a| IssueAttachment {
                            id: a.id,
                            filename: a.filename,
                        })
                        .collect(),
                    description: issue.fields.description.unwrap_or(Value::Null),
                    comments,
                });
            }

            if let Some(token) = payload.next_page_token {
                if token.is_empty() || payload.is_last == Some(true) {
                    break;
                }
                next_page_token = Some(token);
                continue;
            }

            start_at += page_count;
            if let Some(total) = payload.total {
                if start_at >= total {
                    break;
                }
                continue;
            }

            if payload.is_last.unwrap_or(true) || page_count == 0 {
                break;
            }
        }

        Ok(all)
    }

    /// Fetches the authenticated Jira user.
    ///
    /// # Errors
    /// Returns [`JiraError`] when request transport, HTTP status, or decode fails.
    pub fn get_myself(&self) -> Result<JiraIdentity, JiraError> {
        let url = format!("{}/rest/api/3/myself", self.base_url);
        let response = self.request_with_retry(|| {
            self.http
                .get(&url)
                .basic_auth(&self.email, Some(&self.api_token))
                .send()
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(JiraError::Http { status, body });
        }

        let body = response.text()?;
        let payload: MyselfResponse =
            serde_json::from_str(&body).map_err(|source| JiraError::Decode { source, body })?;

        Ok(JiraIdentity {
            account_id: payload.account_id,
            display_name: payload.display_name,
            email_address: payload.email_address,
        })
    }

    /// Lists project keys visible to the authenticated Jira user.
    ///
    /// # Errors
    /// Returns [`JiraError`] when request transport, HTTP status, or decode fails.
    pub fn list_visible_projects(&self) -> Result<Vec<String>, JiraError> {
        let url = format!("{}/rest/api/3/project/search", self.base_url);
        let response = self.request_with_retry(|| {
            self.http
                .get(&url)
                .basic_auth(&self.email, Some(&self.api_token))
                .query(&[("maxResults", "100")])
                .send()
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(JiraError::Http { status, body });
        }

        let body = response.text()?;
        let payload: ProjectSearchResponse =
            serde_json::from_str(&body).map_err(|source| JiraError::Decode { source, body })?;
        Ok(payload.values.into_iter().map(|p| p.key).collect())
    }
}

fn normalize_base_url(raw: &str) -> Result<String, JiraError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(JiraError::InvalidBaseUrl(raw.to_string()));
    }

    let mut candidate = trimmed.to_string();

    if candidate.starts_with("https://https//") {
        candidate = candidate.replacen("https://https//", "https://", 1);
    } else if candidate.starts_with("http://http//") {
        candidate = candidate.replacen("http://http//", "http://", 1);
    }

    if candidate.starts_with("https//") {
        candidate = format!("https://{}", candidate.trim_start_matches("https//"));
    } else if candidate.starts_with("http//") {
        candidate = format!("http://{}", candidate.trim_start_matches("http//"));
    } else if !candidate.starts_with("https://") && !candidate.starts_with("http://") {
        candidate = format!("https://{candidate}");
    }

    let parsed =
        reqwest::Url::parse(&candidate).map_err(|_| JiraError::InvalidBaseUrl(raw.to_string()))?;
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_after_or_backoff(response: &Response, attempt: usize) -> Duration {
    if let Some(header) = response.headers().get("Retry-After") {
        if let Ok(value) = header.to_str() {
            if let Ok(seconds) = value.parse::<u64>() {
                return Duration::from_secs(seconds.min(30));
            }
        }
    }

    let seconds = 1_u64 << attempt.min(4);
    Duration::from_secs(seconds)
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, name: &'static str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            logging::warn(format!("recovering poisoned mutex: {}", name));
            poisoned.into_inner()
        }
    }
}

fn wait_or_recover<'a, T>(
    cv: &Condvar,
    guard: std::sync::MutexGuard<'a, T>,
    name: &'static str,
) -> std::sync::MutexGuard<'a, T> {
    match cv.wait(guard) {
        Ok(next) => next,
        Err(poisoned) => {
            logging::warn(format!(
                "recovering poisoned mutex after condvar wait: {}",
                name
            ));
            poisoned.into_inner()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    #[serde(rename = "maxResults", default)]
    _max_results: Option<usize>,
    #[serde(default)]
    total: Option<usize>,
    #[serde(rename = "isLast", default)]
    is_last: Option<bool>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
    #[serde(default)]
    issues: Vec<SearchIssue>,
    #[serde(default)]
    values: Vec<SearchIssue>,
}

impl SearchResponse {
    fn take_issues(&self) -> Vec<SearchIssue> {
        if !self.issues.is_empty() {
            return self.issues.clone();
        }
        self.values.clone()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct SearchIssue {
    key: String,
    fields: SearchFields,
}

#[derive(Debug, Deserialize, Clone)]
struct SearchFields {
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkSearchResponse {
    #[serde(rename = "maxResults", default)]
    _max_results: Option<usize>,
    #[serde(default)]
    total: Option<usize>,
    #[serde(rename = "isLast", default)]
    is_last: Option<bool>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
    #[serde(default)]
    issues: Vec<BulkSearchIssue>,
    #[serde(default)]
    values: Vec<BulkSearchIssue>,
}

impl BulkSearchResponse {
    fn take_issues(&self) -> Vec<BulkSearchIssue> {
        if !self.issues.is_empty() {
            return self.issues.clone();
        }
        self.values.clone()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct BulkSearchIssue {
    key: String,
    fields: IssueFields,
}

#[derive(Debug, Deserialize)]
struct IssueResponse {
    key: String,
    fields: IssueFields,
}

#[derive(Debug, Deserialize, Clone)]
struct IssueFields {
    summary: Option<String>,
    status: Option<StatusObj>,
    #[serde(rename = "issuetype")]
    issue_type: Option<IssueTypeObj>,
    priority: Option<PriorityObj>,
    assignee: Option<UserObj>,
    reporter: Option<UserObj>,
    #[serde(default)]
    labels: Vec<String>,
    created: Option<String>,
    updated: Option<String>,
    description: Option<Value>,
    comment: Option<CommentContainer>,
    parent: Option<ParentIssueRef>,
    #[serde(default)]
    attachment: Vec<AttachmentObj>,
    #[serde(rename = "duedate")]
    due_date: Option<String>,
    #[serde(rename = "issuelinks", default)]
    issue_links: Vec<IssueLinkObj>,
}

#[derive(Debug, Deserialize, Clone)]
struct StatusObj {
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct IssueTypeObj {
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct PriorityObj {
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UserObj {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct CommentContainer {
    comments: Vec<CommentObj>,
}

#[derive(Debug, Deserialize, Clone)]
struct CommentObj {
    id: Option<String>,
    author: Option<UserObj>,
    body: Value,
    created: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ParentIssueRef {
    key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AttachmentObj {
    id: String,
    filename: String,
}

#[derive(Debug, Deserialize, Clone)]
struct IssueLinkObj {
    #[serde(rename = "type")]
    link_type: Option<IssueLinkTypeObj>,
    #[serde(rename = "outwardIssue")]
    outward_issue: Option<LinkedIssueObj>,
    #[serde(rename = "inwardIssue")]
    inward_issue: Option<LinkedIssueObj>,
}

#[derive(Debug, Deserialize, Clone)]
struct IssueLinkTypeObj {
    inward: Option<String>,
    outward: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct LinkedIssueObj {
    key: String,
}

fn categorize_links(links: Vec<IssueLinkObj>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut blocks = Vec::new();
    let mut blocked_by = Vec::new();
    let mut relates_to = Vec::new();

    for link in links {
        if let Some(outward) = link.outward_issue {
            let relation = link
                .link_type
                .as_ref()
                .and_then(|t| t.outward.clone().or_else(|| t.name.clone()))
                .unwrap_or_else(|| "relates to".to_string())
                .to_lowercase();
            if relation.contains("block") {
                blocks.push(outward.key);
            } else {
                relates_to.push(outward.key);
            }
        }

        if let Some(inward) = link.inward_issue {
            let relation = link
                .link_type
                .as_ref()
                .and_then(|t| t.inward.clone().or_else(|| t.name.clone()))
                .unwrap_or_else(|| "relates to".to_string())
                .to_lowercase();
            if relation.contains("block") {
                blocked_by.push(inward.key);
            } else {
                relates_to.push(inward.key);
            }
        }
    }

    blocks.sort();
    blocks.dedup();
    blocked_by.sort();
    blocked_by.dedup();
    relates_to.sort();
    relates_to.dedup();

    (blocks, blocked_by, relates_to)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MyselfResponse {
    account_id: Option<String>,
    display_name: Option<String>,
    email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectSearchResponse {
    #[serde(default)]
    values: Vec<ProjectInfo>,
}

#[derive(Debug, Deserialize)]
struct ProjectInfo {
    key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    #[test]
    fn paginates_project_issue_listing() {
        let server = MockServer::start();

        let _page_1 = server.mock(|when, then| {
            when.method(GET)
                .path("/rest/api/3/search/jql")
                .query_param("startAt", "0")
                .query_param("maxResults", "50");
            then.status(200).json_body_obj(&serde_json::json!({
                "startAt": 0,
                "maxResults": 50,
                "total": 2,
                "issues": [
                    {"key": "PROJ-1", "fields": {"updated": "2026-02-20T00:00:00.000+0000"}}
                ]
            }));
        });

        let _page_2 = server.mock(|when, then| {
            when.method(GET)
                .path("/rest/api/3/search/jql")
                .query_param("startAt", "1")
                .query_param("maxResults", "50");
            then.status(200).json_body_obj(&serde_json::json!({
                "startAt": 1,
                "maxResults": 50,
                "total": 2,
                "issues": [
                    {"key": "PROJ-2", "fields": {"updated": "2026-02-21T00:00:00.000+0000"}}
                ]
            }));
        });

        let client = JiraClient::new(server.base_url(), "e".into(), "t".into()).expect("client");
        let items = client
            .list_project_issue_refs("PROJ")
            .expect("list should succeed");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "PROJ-1");
        assert_eq!(items[1].key, "PROJ-2");
    }

    #[test]
    fn retries_on_429_then_succeeds() {
        use tiny_http::{Header, Response, Server, StatusCode};

        let server = Server::http("127.0.0.1:0").expect("server start");
        let addr = format!("http://{}", server.server_addr());
        std::thread::spawn(move || {
            let mut requests = server.incoming_requests();

            if let Some(req) = requests.next() {
                let response = Response::empty(StatusCode(429))
                    .with_header(Header::from_bytes("Retry-After", "0").expect("header"));
                let _ = req.respond(response);
            }

            if let Some(req) = requests.next() {
                let body = serde_json::json!({
                    "key": "PROJ-1",
                    "fields": {
                        "summary": "S",
                        "status": {"name": "Open"},
                        "assignee": {"displayName": "A"},
                        "updated": "2026-02-21T00:00:00.000+0000",
                        "description": null,
                        "comment": {"comments": []}
                    }
                })
                .to_string();
                let response = Response::from_string(body)
                    .with_status_code(StatusCode(200))
                    .with_header(
                        Header::from_bytes("Content-Type", "application/json").expect("header"),
                    );
                let _ = req.respond(response);
            }
        });

        let client = JiraClient::new(addr, "e".into(), "t".into()).expect("client");
        let issue = client.get_issue("PROJ-1").expect("eventually succeeds");
        assert_eq!(issue.key, "PROJ-1");
    }

    #[test]
    fn normalizes_common_base_url_typos() {
        let a = normalize_base_url("https//worshipinitiative.atlassian.net").expect("normalize");
        assert_eq!(a, "https://worshipinitiative.atlassian.net");

        let b = normalize_base_url("https://https//worshipinitiative.atlassian.net")
            .expect("normalize");
        assert_eq!(b, "https://worshipinitiative.atlassian.net");

        let c = normalize_base_url("worshipinitiative.atlassian.net/").expect("normalize");
        assert_eq!(c, "https://worshipinitiative.atlassian.net");
    }
}
