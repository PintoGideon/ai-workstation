//! FORK: storage for queued follow-ups (`src/app/fork_followups.rs`, P7).
//! Table created by `migrations::FORK_SCHEMA`: one row per queued
//! instruction, ordered by `position` within a session.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::AiwDb;
use crate::fork::followups::Followup;

impl AiwDb {
    /// Every queue, keyed by session id, in delivery order.
    pub(crate) fn all_followups(&self) -> Result<HashMap<String, Vec<Followup>>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, position, text, created_at FROM aiw_followups
             ORDER BY session_id, position, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Followup {
                id: row.get(0)?,
                session_id: row.get(1)?,
                position: row.get(2)?,
                text: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut queues: HashMap<String, Vec<Followup>> = HashMap::new();
        for followup in rows {
            let followup = followup?;
            queues
                .entry(followup.session_id.clone())
                .or_default()
                .push(followup);
        }
        Ok(queues)
    }

    /// Append `text` to the session's queue.
    pub(crate) fn push_followup(&self, session_id: &str, text: &str) -> Result<Followup> {
        let position: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM aiw_followups WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO aiw_followups (session_id, position, text, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, position, text, created_at],
        )?;
        Ok(Followup {
            id: self.conn.last_insert_rowid(),
            session_id: session_id.to_string(),
            position,
            text: text.to_string(),
            created_at,
        })
    }

    pub(crate) fn delete_followup(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM aiw_followups WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Rewrite the session's order to `ids` (first delivered first).
    pub(crate) fn reorder_followups(&self, session_id: &str, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE aiw_followups SET position = ?1 WHERE id = ?2 AND session_id = ?3",
                params![position as i64, id, session_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove every queued follow-up of `session_id`.
    pub(crate) fn clear_followups(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_followups WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AiwDb;

    #[test]
    fn queues_round_trip_in_order_and_survive_a_reopen() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        let a = db.push_followup("s1", "first").unwrap();
        let b = db.push_followup("s1", "second").unwrap();
        db.push_followup("s2", "other").unwrap();
        assert_eq!(a.position, 0);
        assert_eq!(b.position, 1);

        db.reorder_followups("s1", &[b.id, a.id]).unwrap();
        let queues = db.all_followups().unwrap();
        let texts: Vec<&str> = queues["s1"].iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, ["second", "first"]);
        assert_eq!(queues["s2"].len(), 1);

        drop(db);
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        let queues = db.all_followups().unwrap();
        assert_eq!(queues["s1"].len(), 2, "persisted across restarts");

        db.delete_followup(b.id).unwrap();
        assert_eq!(db.all_followups().unwrap()["s1"].len(), 1);
        db.clear_followups("s1").unwrap();
        assert!(!db.all_followups().unwrap().contains_key("s1"));
    }
}
