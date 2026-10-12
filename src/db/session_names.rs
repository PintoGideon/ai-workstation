//! FORK: storage for the `--name` each Claude session was launched with
//! (`src/fork/session_identity.rs`, DESIGN.md §19.2). Table created by
//! `migrations::FORK_SCHEMA`.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::params;

use super::AiwDb;

impl AiwDb {
    /// Every stored name, keyed by feature session id.
    pub(crate) fn all_session_names(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, name FROM aiw_session_names")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut names = HashMap::new();
        for row in rows {
            let (id, name): (String, String) = row?;
            names.insert(id, name);
        }
        Ok(names)
    }

    /// Save names, replacing any stored for the same sessions.
    pub(crate) fn save_session_names(&self, names: &[(String, String)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (session_id, name) in names {
            tx.execute(
                "INSERT INTO aiw_session_names (session_id, name) VALUES (?1, ?2)
                 ON CONFLICT(session_id) DO UPDATE SET name = excluded.name",
                params![session_id, name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn delete_session_name(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_session_names WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AiwDb;

    #[test]
    fn names_round_trip_and_survive_a_reopen() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let db = AiwDb::open(&path).unwrap();
            db.save_session_names(&[
                ("s1".into(), "auth-claude-1".into()),
                ("s2".into(), "auth-claude-2".into()),
            ])
            .unwrap();
            db.save_session_names(&[("s1".into(), "renamed".into())])
                .unwrap();
            db.delete_session_name("s2").unwrap();
        }
        let db = AiwDb::open(&path).unwrap();
        let names = db.all_session_names().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names["s1"], "renamed");
    }
}
