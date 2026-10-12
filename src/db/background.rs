//! FORK: storage for background Claude sessions (`src/fork/background.rs`,
//! DESIGN.md §19.7): the short id `claude --bg` printed, and the full
//! conversation id once known. Table created by `migrations::FORK_SCHEMA`.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::params;

use super::AiwDb;

/// One stored background session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackgroundRow {
    pub bg_id: Option<String>,
    pub conversation_id: Option<String>,
}

impl AiwDb {
    /// Every stored background session, by feature session id.
    pub(crate) fn all_background_sessions(&self) -> Result<HashMap<String, BackgroundRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, bg_id, conversation_id FROM aiw_background_sessions")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                BackgroundRow {
                    bg_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                },
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, record) = row?;
            out.insert(id, record);
        }
        Ok(out)
    }

    pub(crate) fn save_background_session(
        &self,
        session_id: &str,
        record: &BackgroundRow,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_background_sessions (session_id, bg_id, conversation_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET bg_id = excluded.bg_id,
                 conversation_id = excluded.conversation_id, updated_at = excluded.updated_at",
            params![
                session_id,
                record.bg_id,
                record.conversation_id,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_background_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_background_sessions WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AiwDb, BackgroundRow};

    #[test]
    fn rows_round_trip_update_and_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let db = AiwDb::open(&path).unwrap();
            db.save_background_session(
                "s1",
                &BackgroundRow {
                    bg_id: Some("7c5d".into()),
                    conversation_id: None,
                },
            )
            .unwrap();
            db.save_background_session(
                "s1",
                &BackgroundRow {
                    bg_id: Some("7c5d".into()),
                    conversation_id: Some("conv-1".into()),
                },
            )
            .unwrap();
            db.save_background_session("s2", &BackgroundRow::default())
                .unwrap();
        }
        let db = AiwDb::open(&path).unwrap();
        let all = db.all_background_sessions().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["s1"].conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(all["s2"], BackgroundRow::default());
        db.delete_background_session("s1").unwrap();
        assert!(!db.all_background_sessions().unwrap().contains_key("s1"));
    }
}
