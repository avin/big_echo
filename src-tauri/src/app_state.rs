use crate::audio;
use crate::domain::session::SessionMeta;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::AppHandle;

pub struct AppState {
    pub active_session: Mutex<Option<SessionMeta>>,
    pub active_capture: Mutex<Option<audio::capture::ContinuousCapture>>,
    pub ui_sync: Mutex<UiSyncState>,
    pub live_levels: audio::capture::SharedLevels,
    pub tray_app: Mutex<Option<AppHandle>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_session: Mutex::new(None),
            active_capture: Mutex::new(None),
            ui_sync: Mutex::new(UiSyncState::default()),
            live_levels: audio::capture::SharedLevels::new(),
            tray_app: Mutex::new(None),
        }
    }
}

#[derive(Clone)]
pub struct AppDirs {
    pub app_data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSyncState {
    pub source: String,
    pub topic: String,
}

impl Default for UiSyncState {
    fn default() -> Self {
        Self {
            source: "slack".to_string(),
            topic: String::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UiSyncStateView {
    pub source: String,
    pub topic: String,
    pub is_recording: bool,
    pub active_session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LiveInputLevelsView {
    pub mic: f32,
    pub system: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartRecordingRequest {
    pub tags: Vec<String>,
    pub topic: String,
    pub participants: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartRecordingResponse {
    pub session_id: String,
    pub session_dir: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateSessionDetailsRequest {
    pub session_id: String,
    pub source: String,
    pub custom_tag: String,
    pub topic: String,
    pub participants: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionMetaView {
    pub session_id: String,
    pub source: String,
    pub custom_tag: String,
    pub topic: String,
    pub participants: Vec<String>,
}
