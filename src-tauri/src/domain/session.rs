use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Recording,
    Recorded,
    Transcribed,
    Summarized,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifacts {
    pub audio_file: String,
    pub transcript_file: String,
    pub summary_file: String,
    pub meta_file: String,
}

impl Default for SessionArtifacts {
    fn default() -> Self {
        Self {
            audio_file: "audio.opus".to_string(),
            transcript_file: "transcript.txt".to_string(),
            summary_file: "summary.txt".to_string(),
            meta_file: "meta.json".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub created_at_iso: String,
    pub started_at_iso: String,
    pub ended_at_iso: Option<String>,
    pub display_date_ru: String,
    pub tags: Vec<String>,
    pub primary_tag: String,
    pub topic: String,
    pub participants: Vec<String>,
    pub status: SessionStatus,
    pub artifacts: SessionArtifacts,
    pub errors: Vec<String>,
}

impl SessionMeta {
    pub fn new(session_id: String, tags: Vec<String>, topic: String, participants: Vec<String>) -> Self {
        let now = Local::now();
        let primary_tag = tags
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());

        Self {
            session_id,
            created_at_iso: now.to_rfc3339(),
            started_at_iso: now.to_rfc3339(),
            ended_at_iso: None,
            display_date_ru: format_ru_date(now),
            tags,
            primary_tag,
            topic,
            participants,
            status: SessionStatus::Recording,
            artifacts: SessionArtifacts::default(),
            errors: vec![],
        }
    }
}

pub fn format_ru_date(dt: DateTime<Local>) -> String {
    dt.format("%d.%m.%Y").to_string()
}
