//! FORK: storage for user feature groups and notes (`src/fork/organize.rs`,
//! DESIGN.md §19.4). Tables created by `migrations::FORK_SCHEMA`.

use anyhow::Result;
use rusqlite::params;

use super::AiwDb;
use crate::fork::organize::{FeatureGroup, NoteTarget};

impl AiwDb {
    pub(crate) fn all_feature_groups(&self) -> Result<Vec<FeatureGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, name, color, position, collapsed
             FROM aiw_feature_groups ORDER BY project_id, position, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FeatureGroup {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                color: row.get(3)?,
                position: row.get(4)?,
                collapsed: row.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Save (or replace) a group.
    pub(crate) fn save_feature_group(&self, group: &FeatureGroup) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_feature_groups (id, project_id, name, color, position, collapsed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET project_id = excluded.project_id,
                 name = excluded.name, color = excluded.color,
                 position = excluded.position, collapsed = excluded.collapsed",
            params![
                group.id,
                group.project_id,
                group.name,
                group.color,
                group.position,
                group.collapsed as i64
            ],
        )?;
        Ok(())
    }

    /// Delete a group and every membership in it, in one transaction.
    pub(crate) fn delete_feature_group(&self, group_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM aiw_feature_group_members WHERE group_id = ?1",
            [group_id],
        )?;
        tx.execute("DELETE FROM aiw_feature_groups WHERE id = ?1", [group_id])?;
        tx.commit()?;
        Ok(())
    }

    /// `(feature_id, group_id)` for every grouped feature.
    pub(crate) fn all_feature_group_members(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT feature_id, group_id FROM aiw_feature_group_members")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Put a feature in a group (`Some`) or take it out (`None`).
    pub(crate) fn set_feature_group(&self, feature_id: &str, group_id: Option<&str>) -> Result<()> {
        match group_id {
            Some(group_id) => {
                self.conn.execute(
                    "INSERT INTO aiw_feature_group_members (feature_id, group_id) VALUES (?1, ?2)
                     ON CONFLICT(feature_id) DO UPDATE SET group_id = excluded.group_id",
                    params![feature_id, group_id],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM aiw_feature_group_members WHERE feature_id = ?1",
                    [feature_id],
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn all_notes(&self) -> Result<Vec<(NoteTarget, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, target_id, text FROM aiw_notes")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, id, text) = row?;
            if let Some(target) = NoteTarget::from_parts(&kind, id) {
                out.push((target, text));
            }
        }
        Ok(out)
    }

    /// Save a note (`Some`) or delete it (`None`).
    pub(crate) fn set_note(&self, target: &NoteTarget, text: Option<&str>) -> Result<()> {
        match text {
            Some(text) => {
                self.conn.execute(
                    "INSERT INTO aiw_notes (kind, target_id, text, updated_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(kind, target_id) DO UPDATE SET text = excluded.text,
                         updated_at = excluded.updated_at",
                    params![
                        target.kind(),
                        target.id(),
                        text,
                        chrono::Utc::now().to_rfc3339()
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM aiw_notes WHERE kind = ?1 AND target_id = ?2",
                    params![target.kind(), target.id()],
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, AiwDb) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        (dir, db)
    }

    fn group(id: &str, position: i64) -> FeatureGroup {
        FeatureGroup {
            id: id.into(),
            project_id: "p1".into(),
            name: format!("group {id}"),
            color: "cyan".into(),
            position,
            collapsed: false,
        }
    }

    #[test]
    fn groups_and_memberships_round_trip_and_delete_together() {
        let (_dir, db) = db();
        db.save_feature_group(&group("b", 1)).unwrap();
        db.save_feature_group(&group("a", 0)).unwrap();
        let mut renamed = group("a", 0);
        renamed.name = "Backend".into();
        renamed.collapsed = true;
        renamed.color = "red".into();
        db.save_feature_group(&renamed).unwrap();
        db.set_feature_group("f1", Some("a")).unwrap();
        db.set_feature_group("f2", Some("b")).unwrap();
        db.set_feature_group("f2", Some("a")).unwrap();
        db.set_feature_group("f3", Some("b")).unwrap();
        db.set_feature_group("f3", None).unwrap();

        let groups = db.all_feature_groups().unwrap();
        assert_eq!(
            groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(groups[0], renamed);
        let mut members = db.all_feature_group_members().unwrap();
        members.sort();
        assert_eq!(
            members,
            vec![("f1".into(), "a".into()), ("f2".into(), "a".into())]
        );

        db.delete_feature_group("a").unwrap();
        assert_eq!(db.all_feature_groups().unwrap().len(), 1);
        assert!(db.all_feature_group_members().unwrap().is_empty());
    }

    #[test]
    fn notes_save_replace_and_delete() {
        let (_dir, db) = db();
        let feature = NoteTarget::Feature("f1".into());
        let session = NoteTarget::Session("f1".into());
        db.set_note(&feature, Some("first")).unwrap();
        db.set_note(&feature, Some("second\nline")).unwrap();
        db.set_note(&session, Some("session note")).unwrap();
        let mut notes = db.all_notes().unwrap();
        notes.sort_by(|a, b| a.0.kind().cmp(b.0.kind()));
        assert_eq!(
            notes,
            vec![
                (feature.clone(), "second\nline".to_string()),
                (session.clone(), "session note".to_string()),
            ]
        );
        db.set_note(&feature, None).unwrap();
        assert_eq!(
            db.all_notes().unwrap(),
            vec![(session, "session note".to_string())]
        );
    }
}
