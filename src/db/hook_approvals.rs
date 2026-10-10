//! FORK: storage for approved repository lifecycle hooks
//! (`src/fork/hook_approval.rs`). Table created by `migrations::FORK_SCHEMA`.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::AiwDb;

impl AiwDb {
    /// Whether this exact hook content was approved for `repo` + `hook`.
    pub(crate) fn hook_approval_exists(
        &self,
        repo: &str,
        hook: &str,
        sha256: &str,
    ) -> Result<bool> {
        let found = self
            .conn
            .query_row(
                "SELECT 1 FROM aiw_hook_approvals WHERE repo = ?1 AND hook = ?2 AND sha256 = ?3",
                params![repo, hook, sha256],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Remember the user's approval of this exact hook content.
    pub(crate) fn record_hook_approval(
        &self,
        repo: &str,
        hook: &str,
        sha256: &str,
        command: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_hook_approvals (repo, hook, sha256, command, approved_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(repo, hook, sha256) DO UPDATE SET
                 command = excluded.command,
                 approved_at = excluded.approved_at",
            params![repo, hook, sha256, command],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AiwDb;

    #[test]
    fn approvals_round_trip_and_are_pinned_to_the_hash() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        assert!(!db.hook_approval_exists("/r", "on_start", "aa").unwrap());
        db.record_hook_approval("/r", "on_start", "aa", "make dev")
            .unwrap();
        db.record_hook_approval("/r", "on_start", "aa", "make dev")
            .unwrap();
        assert!(db.hook_approval_exists("/r", "on_start", "aa").unwrap());
        assert!(!db.hook_approval_exists("/r", "on_start", "bb").unwrap());
        assert!(!db.hook_approval_exists("/r", "on_stop", "aa").unwrap());
        assert!(!db.hook_approval_exists("/other", "on_start", "aa").unwrap());

        // Reopening (re-running the schema) keeps approvals.
        drop(db);
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        assert!(db.hook_approval_exists("/r", "on_start", "aa").unwrap());
    }
}
