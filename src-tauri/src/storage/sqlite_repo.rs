use crate::domain::session::{SessionMeta, SessionStatus};
use crate::storage::session_store::load_meta;
use chrono::DateTime;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListItem {
    pub session_id: String,
    pub status: String,
    pub primary_tag: String,
    pub topic: String,
    pub display_date_ru: String,
    pub started_at_iso: String,
    pub session_dir: String,
    pub audio_duration_hms: String,
    pub has_transcript_text: bool,
    pub has_summary_text: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryJob {
    pub session_id: String,
    pub attempts: i64,
    pub next_run_epoch: i64,
    pub last_error: String,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub id: i64,
    pub session_id: String,
    pub at_iso: String,
    pub event_type: String,
    pub detail: String,
}

fn db_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("bigecho.sqlite3")
}

fn file_has_non_empty_text(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return false,
    };
    !content.trim().is_empty()
}

fn format_hms(total_seconds: i64) -> String {
    let safe = total_seconds.max(0);
    let hours = safe / 3600;
    let minutes = (safe % 3600) / 60;
    let seconds = safe % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub(crate) fn audio_duration_hms(meta: &SessionMeta) -> String {
    let started = match DateTime::parse_from_rfc3339(&meta.started_at_iso) {
        Ok(value) => value,
        Err(_) => return "00:00:00".to_string(),
    };
    let ended = match meta
        .ended_at_iso
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    {
        Some(value) => value,
        None => return "00:00:00".to_string(),
    };
    format_hms(ended.signed_duration_since(started).num_seconds())
}

fn open(app_data_dir: &Path) -> Result<Connection, String> {
    let path = db_path(app_data_dir);
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            primary_tag TEXT NOT NULL,
            topic TEXT NOT NULL,
            display_date_ru TEXT NOT NULL,
            started_at_iso TEXT NOT NULL,
            session_dir TEXT NOT NULL,
            meta_path TEXT NOT NULL,
            updated_at_iso TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            at_iso TEXT NOT NULL,
            event_type TEXT NOT NULL,
            detail TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pipeline_retry_jobs (
            session_id TEXT PRIMARY KEY,
            attempts INTEGER NOT NULL,
            next_run_epoch INTEGER NOT NULL,
            last_error TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

pub fn upsert_session(
    app_data_dir: &Path,
    meta: &SessionMeta,
    session_dir: &Path,
    meta_path: &Path,
) -> Result<(), String> {
    let conn = open(app_data_dir)?;
    conn.execute(
        "
        INSERT INTO sessions (
            session_id, status, primary_tag, topic, display_date_ru,
            started_at_iso, session_dir, meta_path, updated_at_iso
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(session_id) DO UPDATE SET
            status=excluded.status,
            primary_tag=excluded.primary_tag,
            topic=excluded.topic,
            display_date_ru=excluded.display_date_ru,
            started_at_iso=excluded.started_at_iso,
            session_dir=excluded.session_dir,
            meta_path=excluded.meta_path,
            updated_at_iso=excluded.updated_at_iso
        ",
        params![
            meta.session_id,
            format!("{:?}", meta.status).to_lowercase(),
            meta.primary_tag,
            meta.topic,
            meta.display_date_ru,
            meta.started_at_iso,
            session_dir.to_string_lossy().to_string(),
            meta_path.to_string_lossy().to_string(),
            chrono::Local::now().to_rfc3339(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn add_event(app_data_dir: &Path, session_id: &str, event_type: &str, detail: &str) -> Result<(), String> {
    let conn = open(app_data_dir)?;
    conn.execute(
        "INSERT INTO session_events(session_id, at_iso, event_type, detail) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, chrono::Local::now().to_rfc3339(), event_type, detail],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
pub fn list_session_events(app_data_dir: &Path, session_id: &str) -> Result<Vec<SessionEvent>, String> {
    let conn = open(app_data_dir)?;
    let mut stmt = conn
        .prepare(
            "
            SELECT id, session_id, at_iso, event_type, detail
            FROM session_events
            WHERE session_id = ?1
            ORDER BY id ASC
            ",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![session_id], |row| {
            Ok(SessionEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                at_iso: row.get(2)?,
                event_type: row.get(3)?,
                detail: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn list_sessions(app_data_dir: &Path) -> Result<Vec<SessionListItem>, String> {
    let conn = open(app_data_dir)?;
    let mut stmt = conn
        .prepare(
            "
            SELECT session_id, status, primary_tag, topic, display_date_ru, started_at_iso, session_dir
            FROM sessions
            ORDER BY started_at_iso DESC
            ",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SessionListItem {
                session_id: row.get(0)?,
                status: row.get(1)?,
                primary_tag: row.get(2)?,
                topic: row.get(3)?,
                display_date_ru: row.get(4)?,
                started_at_iso: row.get(5)?,
                session_dir: row.get(6)?,
                audio_duration_hms: "00:00:00".to_string(),
                has_transcript_text: false,
                has_summary_text: false,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        let mut item = row.map_err(|e| e.to_string())?;
        if let Some(meta_path) = get_meta_path(app_data_dir, &item.session_id)? {
            if let Ok(meta) = load_meta(&meta_path) {
                let session_dir = PathBuf::from(&item.session_dir);
                let transcript_ok =
                    file_has_non_empty_text(&session_dir.join(&meta.artifacts.transcript_file));
                let summary_ok =
                    file_has_non_empty_text(&session_dir.join(&meta.artifacts.summary_file));
                item.audio_duration_hms = audio_duration_hms(&meta);
                item.has_transcript_text =
                    transcript_ok && !matches!(meta.status, SessionStatus::Recording | SessionStatus::Recorded);
                item.has_summary_text =
                    summary_ok && matches!(meta.status, SessionStatus::Summarized | SessionStatus::Done);
            }
        }
        out.push(item);
    }
    Ok(out)
}

pub fn get_meta_path(app_data_dir: &Path, session_id: &str) -> Result<Option<PathBuf>, String> {
    let conn = open(app_data_dir)?;
    let mut stmt = conn
        .prepare("SELECT meta_path FROM sessions WHERE session_id=?1")
        .map_err(|e| e.to_string())?;

    let mut rows = stmt.query(params![session_id]).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let p: String = row.get(0).map_err(|e| e.to_string())?;
        return Ok(Some(PathBuf::from(p)));
    }
    Ok(None)
}

pub fn get_session_dir(app_data_dir: &Path, session_id: &str) -> Result<Option<PathBuf>, String> {
    let conn = open(app_data_dir)?;
    let mut stmt = conn
        .prepare("SELECT session_dir FROM sessions WHERE session_id=?1")
        .map_err(|e| e.to_string())?;

    let mut rows = stmt.query(params![session_id]).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let p: String = row.get(0).map_err(|e| e.to_string())?;
        return Ok(Some(PathBuf::from(p)));
    }
    Ok(None)
}

pub fn delete_session(app_data_dir: &Path, session_id: &str) -> Result<bool, String> {
    let conn = open(app_data_dir)?;
    conn.execute(
        "DELETE FROM pipeline_retry_jobs WHERE session_id=?1",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM session_events WHERE session_id=?1", params![session_id])
        .map_err(|e| e.to_string())?;
    let deleted = conn
        .execute("DELETE FROM sessions WHERE session_id=?1", params![session_id])
        .map_err(|e| e.to_string())?;
    Ok(deleted > 0)
}

pub fn schedule_retry_job(
    app_data_dir: &Path,
    session_id: &str,
    last_error: &str,
    max_attempts: i64,
) -> Result<Option<i64>, String> {
    let conn = open(app_data_dir)?;

    let current_attempts: i64 = conn
        .query_row(
            "SELECT attempts FROM pipeline_retry_jobs WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let attempts = current_attempts + 1;
    if attempts > max_attempts {
        conn.execute(
            "DELETE FROM pipeline_retry_jobs WHERE session_id=?1",
            params![session_id],
        )
        .map_err(|e| e.to_string())?;
        return Ok(None);
    }

    let delay_seconds = match attempts {
        1 => 30,
        2 => 120,
        3 => 600,
        _ => 1800,
    };
    let next_run_epoch = chrono::Utc::now().timestamp() + delay_seconds;

    conn.execute(
        "
        INSERT INTO pipeline_retry_jobs(session_id, attempts, next_run_epoch, last_error)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(session_id) DO UPDATE SET
            attempts=excluded.attempts,
            next_run_epoch=excluded.next_run_epoch,
            last_error=excluded.last_error
        ",
        params![session_id, attempts, next_run_epoch, last_error],
    )
    .map_err(|e| e.to_string())?;

    Ok(Some(attempts))
}

pub fn fetch_due_retry_jobs(
    app_data_dir: &Path,
    now_epoch: i64,
    limit: usize,
) -> Result<Vec<RetryJob>, String> {
    let conn = open(app_data_dir)?;
    let mut stmt = conn
        .prepare(
            "
            SELECT session_id, attempts, next_run_epoch, last_error
            FROM pipeline_retry_jobs
            WHERE next_run_epoch <= ?1
            ORDER BY next_run_epoch ASC, session_id ASC
            LIMIT ?2
            ",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![now_epoch, limit as i64], |row| {
            Ok(RetryJob {
                session_id: row.get(0)?,
                attempts: row.get(1)?,
                next_run_epoch: row.get(2)?,
                last_error: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn clear_retry_job(app_data_dir: &Path, session_id: &str) -> Result<(), String> {
    let conn = open(app_data_dir)?;
    conn.execute(
        "DELETE FROM pipeline_retry_jobs WHERE session_id=?1",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::{SessionMeta, SessionStatus};
    use crate::storage::session_store::save_meta;
    use chrono::{Duration, Local};

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn list_sessions_enriches_derived_fields_from_meta_and_files() {
        let dir = temp_dir();
        let session_dir = dir.path().join("sessions").join("s-derived");
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let meta_path = session_dir.join("meta.json");

        let started = Local::now() - Duration::seconds(90);
        let ended = started + Duration::seconds(90);

        let mut meta = SessionMeta::new(
            "s-derived".to_string(),
            vec!["zoom".to_string()],
            "Weekly sync".to_string(),
            vec!["Alice".to_string()],
        );
        meta.started_at_iso = started.to_rfc3339();
        meta.ended_at_iso = Some(ended.to_rfc3339());
        meta.status = SessionStatus::Done;
        meta.artifacts.transcript_file = "transcript.txt".to_string();
        meta.artifacts.summary_file = "summary.txt".to_string();

        save_meta(&meta_path, &meta).expect("save meta");
        std::fs::write(session_dir.join("transcript.txt"), "mock transcript").expect("write transcript");
        std::fs::write(session_dir.join("summary.txt"), "mock summary").expect("write summary");
        upsert_session(dir.path(), &meta, &session_dir, &meta_path).expect("upsert session");

        let sessions = list_sessions(dir.path()).expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].audio_duration_hms, "00:01:30");
        assert!(sessions[0].has_transcript_text);
        assert!(sessions[0].has_summary_text);
    }

    #[test]
    fn schedules_retry_with_incremental_attempts_and_delays() {
        let dir = temp_dir();
        let session_id = "s-retry-1";
        let max_attempts = 4;

        let a1 = schedule_retry_job(dir.path(), session_id, "err1", max_attempts)
            .expect("schedule #1")
            .expect("attempt #1");
        let a2 = schedule_retry_job(dir.path(), session_id, "err2", max_attempts)
            .expect("schedule #2")
            .expect("attempt #2");

        assert_eq!(a1, 1);
        assert_eq!(a2, 2);

        let now = chrono::Utc::now().timestamp();
        let due_now = fetch_due_retry_jobs(dir.path(), now, 10).expect("fetch due now");
        assert!(due_now.is_empty());

        let due_future =
            fetch_due_retry_jobs(dir.path(), now + 10_000, 10).expect("fetch due in future");
        assert_eq!(due_future.len(), 1);
        assert_eq!(due_future[0].session_id, session_id);
        assert_eq!(due_future[0].attempts, 2);
        assert_eq!(due_future[0].last_error, "err2");
    }

    #[test]
    fn stops_scheduling_after_max_attempts() {
        let dir = temp_dir();
        let session_id = "s-retry-2";
        let max_attempts = 2;

        let r1 = schedule_retry_job(dir.path(), session_id, "e1", max_attempts).expect("r1");
        let r2 = schedule_retry_job(dir.path(), session_id, "e2", max_attempts).expect("r2");
        let r3 = schedule_retry_job(dir.path(), session_id, "e3", max_attempts).expect("r3");

        assert_eq!(r1, Some(1));
        assert_eq!(r2, Some(2));
        assert_eq!(r3, None);

        let due =
            fetch_due_retry_jobs(dir.path(), chrono::Utc::now().timestamp() + 100_000, 10)
                .expect("fetch due");
        assert!(due.iter().all(|j| j.session_id != session_id));
    }

    #[test]
    fn clears_retry_job() {
        let dir = temp_dir();
        let session_id = "s-retry-3";

        let _ = schedule_retry_job(dir.path(), session_id, "e1", 4).expect("schedule");
        clear_retry_job(dir.path(), session_id).expect("clear");

        let due =
            fetch_due_retry_jobs(dir.path(), chrono::Utc::now().timestamp() + 100_000, 10)
                .expect("fetch due");
        assert!(due.iter().all(|j| j.session_id != session_id));
    }
}
