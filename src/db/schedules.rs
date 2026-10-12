//! FORK: scheduled prompts and commands and their run history
//! (`src/fork/schedules.rs`, DESIGN.md §19.9). Tables created by
//! `migrations::FORK_SCHEMA`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Row, params};

use super::AiwDb;
use crate::fork::schedules::{CatchUp, HISTORY_CAP, RunRecord, Schedule, ScheduleKind};

const COLUMNS: &str = "id, feature_id, session_id, target, kind, spec, text, paused, start_if_stopped, catch_up, end_at, max_runs, runs, last_fire, created_at, config_name, queued_followup";

fn parse_time(text: Option<String>) -> Option<DateTime<Utc>> {
    text.and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn row_to_schedule(row: &Row<'_>) -> rusqlite::Result<Schedule> {
    Ok(Schedule {
        id: row.get(0)?,
        feature_id: row.get(1)?,
        session_id: row.get(2)?,
        target: row.get(3)?,
        kind: ScheduleKind::parse(&row.get::<_, String>(4)?).unwrap_or_default(),
        spec: row.get(5)?,
        text: row.get(6)?,
        paused: row.get::<_, i64>(7)? != 0,
        start_if_stopped: row.get::<_, i64>(8)? != 0,
        catch_up: CatchUp::parse(&row.get::<_, String>(9)?).unwrap_or_default(),
        end_at: parse_time(row.get(10)?),
        max_runs: row.get::<_, Option<i64>>(11)?.map(|n| n.max(0) as u32),
        runs: row.get::<_, i64>(12)?.max(0) as u32,
        last_fire: parse_time(row.get(13)?),
        created_at: parse_time(Some(row.get(14)?)).unwrap_or_else(Utc::now),
        config_name: row.get(15)?,
        queued_followup: row.get(16)?,
    })
}

impl AiwDb {
    pub(crate) fn all_schedules(&self) -> Result<Vec<Schedule>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM aiw_schedules ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([], row_to_schedule)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Insert or replace a schedule.
    pub(crate) fn save_schedule(&self, s: &Schedule) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO aiw_schedules ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                 ON CONFLICT(id) DO UPDATE SET feature_id = excluded.feature_id, session_id = excluded.session_id,
                   target = excluded.target, kind = excluded.kind, spec = excluded.spec, text = excluded.text,
                   paused = excluded.paused, start_if_stopped = excluded.start_if_stopped,
                   catch_up = excluded.catch_up, end_at = excluded.end_at, max_runs = excluded.max_runs,
                   runs = excluded.runs, last_fire = excluded.last_fire, created_at = excluded.created_at,
                   config_name = excluded.config_name, queued_followup = excluded.queued_followup"
            ),
            params![
                s.id,
                s.feature_id,
                s.session_id,
                s.target,
                s.kind.as_str(),
                s.spec,
                s.text,
                s.paused as i64,
                s.start_if_stopped as i64,
                s.catch_up.as_str(),
                s.end_at.map(|t| t.to_rfc3339()),
                s.max_runs.map(i64::from),
                i64::from(s.runs),
                s.last_fire.map(|t| t.to_rfc3339()),
                s.created_at.to_rfc3339(),
                s.config_name,
                s.queued_followup,
            ],
        )?;
        Ok(())
    }

    /// Delete a schedule and its history.
    pub(crate) fn delete_schedule(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM aiw_schedule_runs WHERE schedule_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM aiw_schedules WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Record a run and keep only the newest [`HISTORY_CAP`] for the schedule.
    pub(crate) fn push_schedule_run(&self, schedule_id: &str, run: &RunRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aiw_schedule_runs (schedule_id, at, outcome, detail) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_id, run.at.to_rfc3339(), run.outcome, run.detail],
        )?;
        self.conn.execute(
            "DELETE FROM aiw_schedule_runs WHERE schedule_id = ?1 AND id NOT IN
               (SELECT id FROM aiw_schedule_runs WHERE schedule_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![schedule_id, HISTORY_CAP as i64],
        )?;
        Ok(())
    }

    /// The schedule's runs, newest first.
    pub(crate) fn schedule_runs(&self, schedule_id: &str) -> Result<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, outcome, detail FROM aiw_schedule_runs WHERE schedule_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([schedule_id], |row| {
            Ok(RunRecord {
                at: parse_time(Some(row.get(0)?)).unwrap_or_else(Utc::now),
                outcome: row.get(1)?,
                detail: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(id: &str) -> Schedule {
        Schedule {
            id: id.into(),
            feature_id: "f1".into(),
            session_id: Some("s1".into()),
            target: None,
            kind: ScheduleKind::Prompt,
            spec: "every 30m".into(),
            text: "run the tests".into(),
            paused: false,
            start_if_stopped: true,
            catch_up: CatchUp::None,
            end_at: Some(Utc::now()),
            max_runs: Some(3),
            runs: 1,
            last_fire: Some(Utc::now()),
            created_at: Utc::now(),
            config_name: None,
            queued_followup: Some(7),
        }
    }

    #[test]
    fn schedules_round_trip_update_and_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        let db = AiwDb::open(&path).unwrap();
        let mut a = schedule("a");
        db.save_schedule(&a).unwrap();
        db.save_schedule(&schedule("b")).unwrap();
        a.paused = true;
        a.runs = 2;
        a.kind = ScheduleKind::Shell;
        a.target = Some("tests".into());
        db.save_schedule(&a).unwrap();
        drop(db);
        let db = AiwDb::open(&path).unwrap();
        let all = db.all_schedules().unwrap();
        assert_eq!(all.len(), 2);
        let loaded = all.iter().find(|s| s.id == "a").unwrap();
        assert!(loaded.paused);
        assert_eq!(loaded.runs, 2);
        assert_eq!(loaded.kind, ScheduleKind::Shell);
        assert_eq!(loaded.target.as_deref(), Some("tests"));
        assert_eq!(loaded.catch_up, CatchUp::None);
        assert_eq!(loaded.max_runs, Some(3));
        assert_eq!(loaded.queued_followup, Some(7));
        assert!(loaded.start_if_stopped);
        db.push_schedule_run(
            "a",
            &RunRecord {
                at: Utc::now(),
                outcome: "ran".into(),
                detail: "timer".into(),
            },
        )
        .unwrap();
        db.delete_schedule("a").unwrap();
        assert_eq!(db.all_schedules().unwrap().len(), 1);
        assert!(
            db.schedule_runs("a").unwrap().is_empty(),
            "history goes too"
        );
    }

    #[test]
    fn history_is_capped_and_newest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = AiwDb::open(&dir.path().join("aiw.db")).unwrap();
        for i in 0..(HISTORY_CAP + 7) {
            db.push_schedule_run(
                "a",
                &RunRecord {
                    at: Utc::now(),
                    outcome: format!("run {i}"),
                    detail: String::new(),
                },
            )
            .unwrap();
        }
        db.push_schedule_run(
            "b",
            &RunRecord {
                at: Utc::now(),
                outcome: "other".into(),
                detail: String::new(),
            },
        )
        .unwrap();
        let runs = db.schedule_runs("a").unwrap();
        assert_eq!(runs.len(), HISTORY_CAP);
        assert_eq!(runs[0].outcome, format!("run {}", HISTORY_CAP + 6));
        assert_eq!(runs.last().unwrap().outcome, "run 7");
        assert_eq!(
            db.schedule_runs("b").unwrap().len(),
            1,
            "other schedules untouched"
        );
    }
}
