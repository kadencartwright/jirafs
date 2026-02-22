use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct PersistentIssue {
    pub markdown: Vec<u8>,
    pub updated: Option<String>,
}

#[derive(Debug)]
pub struct PersistentCache {
    conn: Mutex<Connection>,
}

impl PersistentCache {
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
",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_issue(&self, issue_key: &str) -> Result<Option<PersistentIssue>, rusqlite::Error> {
        let conn = self.conn.lock().expect("persistent cache mutex poisoned");
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

    pub fn upsert_issue(
        &self,
        issue_key: &str,
        markdown: &[u8],
        updated: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before unix epoch")
            .as_secs()
            .to_string();
        let conn = self.conn.lock().expect("persistent cache mutex poisoned");
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
}
