//! FORK: storage for prompts waiting on their agent's pane and for fan-out
//! group membership (`src/fork/fan_out.rs`, DESIGN.md §19.3). Tables created
//! by `migrations::FORK_SCHEMA`.

use anyhow::Result;
use rusqlite::params;

use super::AiwDb;
use crate::project::AgentKind;

/// A stored pending prompt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredPendingPrompt {
    pub session_id: String,
    pub text: String,
    pub send: bool,
}

/// A feature's place in a fan-out group.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FanOutMember {
    pub feature_id: String,
    pub group_id: String,
    pub base_branch: String,
    pub agent: AgentKind,
    pub position: i64,
}

fn agent_from(text: &str) -> AgentKind {
    serde_json::from_value(serde_json::Value::String(text.to_string())).unwrap_or_default()
}

impl AiwDb {
    pub(crate) fn all_pending_prompts(&self) -> Result<Vec<StoredPendingPrompt>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, text, send FROM aiw_pending_prompts ORDER BY created_at, session_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StoredPendingPrompt {
                session_id: row.get(0)?,
                text: row.get(1)?,
                send: row.get::<_, i64>(2)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Save (or replace) the prompt waiting for `session_id`.
    pub(crate) fn save_pending_prompt(
        &self,
        session_id: &str,
        text: &str,
        send: bool,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_pending_prompts (session_id, text, send, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET text = excluded.text, send = excluded.send,
                 created_at = excluded.created_at",
            params![
                session_id,
                text,
                send as i64,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_pending_prompt(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_pending_prompts WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    pub(crate) fn all_fanout_members(&self) -> Result<Vec<FanOutMember>> {
        let mut stmt = self.conn.prepare(
            "SELECT feature_id, group_id, base_branch, agent, position
             FROM aiw_fanout_members ORDER BY group_id, position",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FanOutMember {
                feature_id: row.get(0)?,
                group_id: row.get(1)?,
                base_branch: row.get(2)?,
                agent: agent_from(&row.get::<_, String>(3)?),
                position: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub(crate) fn save_fanout_member(&self, member: &FanOutMember) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_fanout_members (feature_id, group_id, base_branch, agent, position)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(feature_id) DO UPDATE SET group_id = excluded.group_id,
                 base_branch = excluded.base_branch, agent = excluded.agent,
                 position = excluded.position",
            params![
                member.feature_id,
                member.group_id,
                member.base_branch,
                crate::fork::fan_out::agent_slug(&member.agent),
                member.position
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_fanout_member(&self, feature_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_fanout_members WHERE feature_id = ?1",
            [feature_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_prompts_survive_a_reopen_and_go_once_deleted() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let db = AiwDb::open(&path).unwrap();
            db.save_pending_prompt("s1", "do x\nthen y", false).unwrap();
            db.save_pending_prompt("s2", "first", true).unwrap();
            db.save_pending_prompt("s2", "replaced", true).unwrap();
        }
        let db = AiwDb::open(&path).unwrap();
        let mut all = db.all_pending_prompts().unwrap();
        all.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        assert_eq!(
            all,
            [
                StoredPendingPrompt {
                    session_id: "s1".into(),
                    text: "do x\nthen y".into(),
                    send: false
                },
                StoredPendingPrompt {
                    session_id: "s2".into(),
                    text: "replaced".into(),
                    send: true
                },
            ]
        );
        db.delete_pending_prompt("s1").unwrap();
        assert_eq!(db.all_pending_prompts().unwrap().len(), 1);
    }

    #[test]
    fn fanout_members_round_trip_in_group_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        for (i, (id, agent)) in [("f2", AgentKind::Codex), ("f1", AgentKind::Claude)]
            .into_iter()
            .enumerate()
        {
            db.save_fanout_member(&FanOutMember {
                feature_id: id.into(),
                group_id: "g".into(),
                base_branch: "auth-fix".into(),
                agent,
                position: 1 - i as i64,
            })
            .unwrap();
        }
        let members = db.all_fanout_members().unwrap();
        assert_eq!(members[0].feature_id, "f1");
        assert_eq!(members[0].agent, AgentKind::Claude);
        assert_eq!(members[1].agent, AgentKind::Codex);
        db.delete_fanout_member("f1").unwrap();
        assert_eq!(db.all_fanout_members().unwrap().len(), 1);
    }
}
