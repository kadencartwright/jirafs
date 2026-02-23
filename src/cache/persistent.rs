use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

use crate::jira::IssueRef;
use crate::logging;

pub type PersistentIssueRow = (String, Vec<u8>, Option<String>);
pub type PersistentSidecarRow = (String, Vec<u8>, Option<String>);

#[derive(Debug, Clone)]
/// Persisted issue markdown row.
pub struct PersistentIssue {
    pub markdown: Vec<u8>,
    pub updated: Option<String>,
}

#[derive(Debug)]
/// SQLite-backed cache for issue content and sync metadata.
pub struct PersistentCache {
    conn: Mutex<Connection>,
}

impl PersistentCache {
    /// Opens or creates the persistent cache database.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when opening or initializing SQLite fails.
    pub fn new(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
CREATE TABLE IF NOT EXISTS issues (
  issue_key TEXT PRIMARY KEY,
  markdown BLOB NOT NULL,
  updated TEXT,
  cached_at TEXT NOT NULL,
  access_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sync_cursor (
  workspace TEXT PRIMARY KEY,
  last_sync TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_issues (
  workspace TEXT NOT NULL,
  issue_key TEXT NOT NULL,
  updated TEXT,
  PRIMARY KEY(workspace, issue_key)
);

CREATE INDEX IF NOT EXISTS idx_workspace_issues_issue_key ON workspace_issues(issue_key);

CREATE TABLE IF NOT EXISTS issue_sidecars (
  issue_key TEXT PRIMARY KEY,
  comments_md BLOB NOT NULL,
  updated TEXT,
  cached_at TEXT NOT NULL
);
 ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Loads one persisted issue and increments its access counter.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when query or update execution fails.
    pub fn get_issue(&self, issue_key: &str) -> Result<Option<PersistentIssue>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt = conn.prepare("SELECT markdown, updated FROM issues WHERE issue_key = ?1")?;
        let mut rows = stmt.query(params![issue_key])?;

        if let Some(row) = rows.next()? {
            conn.execute(
                "UPDATE issues SET access_count = access_count + 1 WHERE issue_key = ?1",
                params![issue_key],
            )?;

            return Ok(Some(PersistentIssue {
                markdown: row.get(0)?,
                updated: row.get(1)?,
            }));
        }

        Ok(None)
    }

    /// Upserts one issue markdown payload.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn upsert_issue(
        &self,
        issue_key: &str,
        markdown: &[u8],
        updated: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let now = unix_epoch_seconds_string();
        let conn = lock_conn_or_recover(&self.conn);
        conn.execute(
            "
INSERT INTO issues(issue_key, markdown, updated, cached_at, access_count)
VALUES (?1, ?2, ?3, ?4, 1)
ON CONFLICT(issue_key) DO UPDATE SET
  markdown = excluded.markdown,
  updated = excluded.updated,
  cached_at = excluded.cached_at,
  access_count = issues.access_count + 1
",
            params![issue_key, markdown, updated, now],
        )?;
        Ok(())
    }

    /// Upserts multiple issue markdown payloads in one transaction.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when transaction or SQL execution fails.
    pub fn upsert_issues_batch(
        &self,
        issues: &[PersistentIssueRow],
    ) -> Result<usize, rusqlite::Error> {
        let now = unix_epoch_seconds_string();
        let mut conn = lock_conn_or_recover(&self.conn);
        let tx = conn.transaction()?;

        let mut count = 0;
        for (issue_key, markdown, updated) in issues {
            tx.execute(
                "
INSERT INTO issues(issue_key, markdown, updated, cached_at, access_count)
VALUES (?1, ?2, ?3, ?4, 1)
ON CONFLICT(issue_key) DO UPDATE SET
  markdown = excluded.markdown,
  updated = excluded.updated,
  cached_at = excluded.cached_at,
  access_count = issues.access_count + 1
",
                params![issue_key, markdown, updated, now],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    /// Reads the last sync cursor for a workspace.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn get_sync_cursor(&self, workspace: &str) -> Result<Option<String>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt = conn.prepare("SELECT last_sync FROM sync_cursor WHERE workspace = ?1")?;
        let mut rows = stmt.query(params![workspace])?;

        if let Some(row) = rows.next()? {
            return Ok(Some(row.get(0)?));
        }

        Ok(None)
    }

    /// Writes or updates the last sync cursor for a workspace.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn set_sync_cursor(&self, workspace: &str, last_sync: &str) -> Result<(), rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        conn.execute(
            "
INSERT INTO sync_cursor(workspace, last_sync)
VALUES (?1, ?2)
ON CONFLICT(workspace) DO UPDATE SET
  last_sync = excluded.last_sync
",
            params![workspace, last_sync],
        )?;
        Ok(())
    }

    /// Removes a persisted sync cursor for a workspace.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn clear_sync_cursor(&self, workspace: &str) -> Result<(), rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        conn.execute(
            "DELETE FROM sync_cursor WHERE workspace = ?1",
            params![workspace],
        )?;
        Ok(())
    }

    /// Counts persisted issues for a project key prefix.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn cached_issue_count(&self, project_prefix: &str) -> Result<usize, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let pattern = format!("{}-%", project_prefix);
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE issue_key LIKE ?1",
            params![pattern],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Returns stored markdown size in bytes for one issue.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn issue_markdown_len(&self, issue_key: &str) -> Result<Option<u64>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt = conn.prepare("SELECT length(markdown) FROM issues WHERE issue_key = ?1")?;
        let mut rows = stmt.query(params![issue_key])?;

        if let Some(row) = rows.next()? {
            let len: i64 = row.get(0)?;
            return Ok(Some(len.max(0) as u64));
        }

        Ok(None)
    }

    /// Replaces one workspace listing with issue refs.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when transaction or SQL execution fails.
    pub fn upsert_workspace_issue_refs(
        &self,
        workspace: &str,
        issue_refs: &[IssueRef],
    ) -> Result<(), rusqlite::Error> {
        let mut conn = lock_conn_or_recover(&self.conn);
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM workspace_issues WHERE workspace = ?1",
            params![workspace],
        )?;
        for issue in issue_refs {
            tx.execute(
                "INSERT INTO workspace_issues(workspace, issue_key, updated) VALUES (?1, ?2, ?3)",
                params![workspace, issue.key, issue.updated],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Lists issue refs for a workspace.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn list_workspace_issue_refs(
        &self,
        workspace: &str,
    ) -> Result<Vec<IssueRef>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt = conn.prepare(
            "SELECT issue_key, updated FROM workspace_issues WHERE workspace = ?1 ORDER BY issue_key ASC",
        )?;
        let mut rows = stmt.query(params![workspace])?;
        let mut out = Vec::new();

        while let Some(row) = rows.next()? {
            out.push(IssueRef {
                key: row.get(0)?,
                updated: row.get(1)?,
            });
        }

        Ok(out)
    }

    /// Upserts markdown comment sidecar for one issue.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn upsert_issue_sidecars(
        &self,
        issue_key: &str,
        comments_md: &[u8],
        updated: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let now = unix_epoch_seconds_string();
        let conn = lock_conn_or_recover(&self.conn);
        conn.execute(
            "
INSERT INTO issue_sidecars(issue_key, comments_md, updated, cached_at)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(issue_key) DO UPDATE SET
  comments_md = excluded.comments_md,
  updated = excluded.updated,
  cached_at = excluded.cached_at
",
            params![issue_key, comments_md, updated, now],
        )?;
        Ok(())
    }

    /// Upserts markdown comment sidecars in one transaction.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when transaction or SQL execution fails.
    pub fn upsert_issue_sidecars_batch(
        &self,
        sidecars: &[PersistentSidecarRow],
    ) -> Result<usize, rusqlite::Error> {
        let now = unix_epoch_seconds_string();
        let mut conn = lock_conn_or_recover(&self.conn);
        let tx = conn.transaction()?;

        let mut count = 0;
        for (issue_key, comments_md, updated) in sidecars {
            tx.execute(
                "
INSERT INTO issue_sidecars(issue_key, comments_md, updated, cached_at)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(issue_key) DO UPDATE SET
  comments_md = excluded.comments_md,
  updated = excluded.updated,
  cached_at = excluded.cached_at
",
                params![issue_key, comments_md, updated, now],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    /// Loads markdown comment sidecar bytes for one issue.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn get_issue_comments_md(
        &self,
        issue_key: &str,
    ) -> Result<Option<Vec<u8>>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt =
            conn.prepare("SELECT comments_md FROM issue_sidecars WHERE issue_key = ?1")?;
        let mut rows = stmt.query(params![issue_key])?;
        if let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    /// Returns markdown sidecar size in bytes for one issue.
    ///
    /// # Errors
    /// Returns [`rusqlite::Error`] when SQL execution fails.
    pub fn issue_comments_md_len(&self, issue_key: &str) -> Result<Option<u64>, rusqlite::Error> {
        let conn = lock_conn_or_recover(&self.conn);
        let mut stmt =
            conn.prepare("SELECT length(comments_md) FROM issue_sidecars WHERE issue_key = ?1")?;
        let mut rows = stmt.query(params![issue_key])?;

        if let Some(row) = rows.next()? {
            let len: i64 = row.get(0)?;
            return Ok(Some(len.max(0) as u64));
        }

        Ok(None)
    }
}

fn lock_conn_or_recover(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    match conn.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            logging::warn("recovering poisoned mutex: persistent cache connection");
            poisoned.into_inner()
        }
    }
}

fn unix_epoch_seconds_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| {
            logging::warn("system clock before unix epoch; using fallback timestamp 0");
            "0".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reads_issue() {
        let db = PersistentCache::new(Path::new(":memory:")).expect("db open");
        db.upsert_issue("PROJ-1", b"hello", Some("u1"))
            .expect("upsert");

        let got = db.get_issue("PROJ-1").expect("read").expect("row present");
        assert_eq!(got.markdown, b"hello");
        assert_eq!(got.updated.as_deref(), Some("u1"));
    }

    #[test]
    fn sync_cursor_roundtrip() {
        let db = PersistentCache::new(Path::new(":memory:")).expect("db open");

        assert!(db.get_sync_cursor("default").expect("get").is_none());

        db.set_sync_cursor("default", "2026-02-22T10:00:00.000+0000")
            .expect("set");

        let cursor = db
            .get_sync_cursor("default")
            .expect("get")
            .expect("present");
        assert_eq!(cursor, "2026-02-22T10:00:00.000+0000");
    }

    #[test]
    fn workspace_refs_roundtrip() {
        let db = PersistentCache::new(Path::new(":memory:")).expect("db open");
        db.upsert_workspace_issue_refs(
            "default",
            &[
                IssueRef {
                    key: "ST-10".to_string(),
                    updated: Some("u1".to_string()),
                },
                IssueRef {
                    key: "OPS-2".to_string(),
                    updated: None,
                },
            ],
        )
        .expect("upsert refs");

        let rows = db.list_workspace_issue_refs("default").expect("list refs");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "OPS-2");
        assert_eq!(rows[1].key, "ST-10");
    }

    #[test]
    fn persists_sidecars_markdown_only() {
        let db = PersistentCache::new(Path::new(":memory:")).expect("db open");
        db.upsert_issue_sidecars("DATA-1", b"md", Some("u1"))
            .expect("upsert sidecars");

        let md = db
            .get_issue_comments_md("DATA-1")
            .expect("load md")
            .expect("present");
        assert_eq!(md, b"md");
        assert_eq!(
            db.issue_comments_md_len("DATA-1")
                .expect("md len")
                .expect("present"),
            2
        );
    }
}
