//! FORK: where each Claude session's inbox socket is (`src/fork/messaging.rs`,
//! DESIGN.md §19.5). Table created by `migrations::FORK_SCHEMA`.
//!
//! Only the socket path and the conversation it belongs to are stored. The
//! inbox token stays in memory: `aiw.db` is not a secrets file, and on Linux
//! Claude accepts an inbox line without the auth line, so an address reloaded
//! after a restart still delivers.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::params;

use super::AiwDb;
use crate::fork::messaging::InboxAddress;

impl AiwDb {
    /// Every stored inbox, keyed by feature session id (tokens are `None`).
    pub(crate) fn all_inbox_addresses(&self) -> Result<HashMap<String, InboxAddress>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, socket, conversation_id FROM aiw_inbox_addresses")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (session_id, socket, conversation_id) = row?;
            out.insert(
                session_id,
                InboxAddress {
                    socket: PathBuf::from(socket),
                    token: None,
                    conversation_id,
                },
            );
        }
        Ok(out)
    }

    /// Save (or replace) a session's inbox. The token is not written.
    pub(crate) fn save_inbox_address(
        &self,
        session_id: &str,
        address: &InboxAddress,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_inbox_addresses (session_id, socket, conversation_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET socket = excluded.socket,
                 conversation_id = excluded.conversation_id, updated_at = excluded.updated_at",
            params![
                session_id,
                address.socket.to_string_lossy(),
                address.conversation_id,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_inbox_address(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM aiw_inbox_addresses WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_round_trip_without_the_token() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let db = AiwDb::open(&path).unwrap();
            let address = InboxAddress {
                socket: PathBuf::from("/run/cc-socks/1.sock"),
                token: Some("0123456789abcdef0123456789abcdef".into()),
                conversation_id: Some("conv-1".into()),
            };
            db.save_inbox_address("s1", &address).unwrap();
            db.save_inbox_address(
                "s1",
                &InboxAddress {
                    socket: PathBuf::from("/run/cc-socks/2.sock"),
                    ..address.clone()
                },
            )
            .unwrap();
            db.save_inbox_address("s2", &address).unwrap();
            db.delete_inbox_address("s2").unwrap();
        }
        let db = AiwDb::open(&path).unwrap();
        let all = db.all_inbox_addresses().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all["s1"].socket, PathBuf::from("/run/cc-socks/2.sock"));
        assert_eq!(all["s1"].conversation_id.as_deref(), Some("conv-1"));
        assert!(all["s1"].token.is_none(), "the token is never stored");
        let raw: String = db
            .conn
            .query_row("SELECT group_concat(socket || coalesce(conversation_id,'')) FROM aiw_inbox_addresses", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("0123456789abcdef"));
    }
}
