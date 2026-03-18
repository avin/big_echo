use crate::app_state::{AppDirs, AppState, StartRecordingRequest, StartRecordingResponse};
use crate::command_core::{validate_start_request, PipelineInvocation};
use crate::domain::session::{SessionArtifacts, SessionMeta};
use crate::services::pipeline_runner::{run_pipeline_core, PipelineMode};
use crate::settings::secret_store::{get_secret, set_secret};
use crate::storage::fs_layout::{build_session_relative_dir, summary_name, transcript_name};
use crate::storage::session_store::save_meta;
use crate::storage::sqlite_repo::{add_event, upsert_session};
use crate::{get_settings_from_dirs, root_recordings_dir, set_tray_indicator_from_state, stop_active_recording_internal};
use chrono::{DateTime, Local};
use uuid::Uuid;

#[tauri::command]
pub fn set_api_secret(dirs: tauri::State<AppDirs>, name: String, value: String) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Secret name must not be empty".to_string());
    }
    set_secret(&dirs.app_data_dir, name.trim(), value.trim())
}

#[tauri::command]
pub fn get_api_secret(dirs: tauri::State<AppDirs>, name: String) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("Secret name must not be empty".to_string());
    }
    get_secret(&dirs.app_data_dir, name.trim())
}

#[tauri::command]
pub fn start_recording(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
    payload: StartRecordingRequest,
) -> Result<StartRecordingResponse, String> {
    validate_start_request(&payload.topic, &payload.participants)?;

    let mut guard = state
        .active_session
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;

    if guard.is_some() {
        return Err("Recording already active".to_string());
    }

    let session_id = Uuid::new_v4().to_string();
    let source_from_payload = payload
        .tags
        .first()
        .cloned()
        .unwrap_or_else(|| "zoom".to_string());
    let topic_from_payload = payload.topic.clone();
    let meta = SessionMeta::new(
        session_id.clone(),
        payload.tags,
        payload.topic,
        payload.participants,
    );

    let settings = get_settings_from_dirs(dirs.inner())?;
    let started_at: DateTime<Local> = DateTime::parse_from_rfc3339(&meta.started_at_iso)
        .map_err(|e| e.to_string())?
        .with_timezone(&Local);

    let rel_dir = build_session_relative_dir(&meta.primary_tag, started_at);
    let abs_dir = root_recordings_dir(&dirs.app_data_dir, &settings)?.join(&rel_dir);
    std::fs::create_dir_all(&abs_dir).map_err(|e| e.to_string())?;

    let mut meta = meta;
    meta.artifacts = SessionArtifacts {
        audio_file: "audio.opus".to_string(),
        transcript_file: transcript_name(started_at),
        summary_file: summary_name(started_at),
        meta_file: "meta.json".to_string(),
    };

    save_meta(&abs_dir.join("meta.json"), &meta)?;
    let data_dir = dirs.app_data_dir.clone();
    upsert_session(&data_dir, &meta, &abs_dir, &abs_dir.join("meta.json"))?;
    add_event(&data_dir, &meta.session_id, "recording_started", "Session created")?;

    std::fs::write(abs_dir.join(&meta.artifacts.transcript_file), "").map_err(|e| e.to_string())?;
    std::fs::write(abs_dir.join(&meta.artifacts.summary_file), "").map_err(|e| e.to_string())?;

    let system_source = if settings.system_device_name.trim().is_empty() {
        crate::audio::capture::detect_system_source_device()?
    } else {
        Some(settings.system_device_name.clone())
    };

    let capture = crate::audio::capture::ContinuousCapture::start(
        if settings.mic_device_name.trim().is_empty() {
            None
        } else {
            Some(settings.mic_device_name.clone())
        },
        system_source,
        state.live_levels.clone(),
    )?;
    let mut cap_guard = state
        .active_capture
        .lock()
        .map_err(|_| "capture state lock poisoned".to_string())?;
    *cap_guard = Some(capture);

    *guard = Some(meta.clone());
    if let Ok(mut ui) = state.ui_sync.lock() {
        ui.source = source_from_payload;
        ui.topic = topic_from_payload;
    }
    set_tray_indicator_from_state(state.inner(), true);
    Ok(StartRecordingResponse {
        session_id,
        session_dir: abs_dir.to_string_lossy().to_string(),
        status: "recording".to_string(),
    })
}

#[tauri::command]
pub fn stop_recording(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
    session_id: String,
) -> Result<String, String> {
    stop_active_recording_internal(dirs.inner(), state.inner(), Some(session_id.as_str()), None)
}

#[tauri::command]
pub fn stop_active_recording(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
) -> Result<String, String> {
    stop_active_recording_internal(dirs.inner(), state.inner(), None, None)
}

#[tauri::command]
pub async fn run_pipeline(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Run,
        PipelineMode::Full,
    )
    .await
}

#[tauri::command]
pub async fn retry_pipeline(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Retry,
        PipelineMode::Full,
    )
    .await
}

#[tauri::command]
pub async fn run_transcription(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Manual,
        PipelineMode::TranscriptionOnly,
    )
    .await
}

#[tauri::command]
pub async fn run_summary(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Manual,
        PipelineMode::SummaryOnly,
    )
    .await
}
