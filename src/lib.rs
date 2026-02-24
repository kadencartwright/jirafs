//! `jirafs` exposes cache, Jira API, rendering, and FUSE filesystem modules.
//! It provides a read-only Jira-backed filesystem interface.

/// In-memory cache and persistent cache integration.
pub mod cache;
/// Runtime configuration loading and validation.
pub mod config;
/// FUSE filesystem implementation that serves Jira content.
pub mod fs;
/// Jira API client and issue data models.
pub mod jira;
/// Logging helpers used throughout the crate.
pub mod logging;
/// Runtime metrics counters.
pub mod metrics;
/// Markdown and sidecar renderers for Jira payloads.
pub mod render;
/// Sync scheduling and trigger state.
pub mod sync_state;
/// Startup seeding and sync routines.
pub mod warmup;
