use crate::app_state::{AppDirs, AppState, LiveInputLevelsView, SessionMetaView, UiSyncStateView, UpdateSessionDetailsRequest};
use crate::domain::session::SessionMeta;
use crate::storage::session_store::{load_meta, save_meta};
use crate::storage::sqlite_repo::{
    add_event, delete_session as repo_delete_session, get_meta_path, get_session_dir,
    list_sessions as repo_list_sessions, upsert_session, SessionListItem,
};
use crate::{get_settings_from_dirs, set_tray_indicator_from_state};
use std::fs;
use std::path::{Path, PathBuf};

fn open_path_in_file_manager(path: &Path, preferred_app: Option<&str>) -> Result<(), String> {
    let target = path
        .to_str()
        .ok_or_else(|| "Path contains invalid UTF-8".to_string())?
        .trim();
    if target.is_empty() {
        return Err("Path is empty".to_string());
    }

    let preferred_app = preferred_app.map(str::trim).filter(|v| !v.is_empty());
    if let Some(app) = preferred_app {
        let status = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(target)
                .status()
                .map_err(|e| e.to_string())?
        } else {
            std::process::Command::new(app)
                .arg(target)
                .status()
                .map_err(|e| e.to_string())?
        };

        if status.success() {
            return Ok(());
        }
        return Err(format!("failed to open path with preferred app: exit status {status}"));
    }

    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .arg(target)
            .status()
            .map_err(|e| e.to_string())?
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
            .arg(target)
            .status()
            .map_err(|e| e.to_string())?
    } else {
        std::process::Command::new("xdg-open")
            .arg(target)
            .status()
            .map_err(|e| e.to_string())?
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to open session folder: exit status {status}"))
    }
}

fn resolve_artifact_path(session_dir: &Path, meta: &SessionMeta, artifact_kind: &str) -> Result<PathBuf, String> {
    let file_name = match artifact_kind {
        "transcript" => &meta.artifacts.transcript_file,
        "summary" => &meta.artifacts.summary_file,
        _ => return Err("Unsupported artifact kind".to_string()),
    };
    Ok(session_dir.join(file_name))
}

fn remove_session_catalog(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        fs::remove_file(path).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn open_session_folder(session_dir: String) -> Result<String, String> {
    let target = PathBuf::from(session_dir);
    open_path_in_file_manager(&target, None)?;
    Ok("opened".to_string())
}

#[tauri::command]
pub fn open_session_artifact(
    dirs: tauri::State<AppDirs>,
    session_id: String,
    artifact_kind: String,
) -> Result<String, String> {
    let meta_path = get_meta_path(&dirs.app_data_dir, &session_id)?
        .ok_or_else(|| "Session not found".to_string())?;
    let meta = load_meta(&meta_path)?;
    let session_dir = meta_path
        .parent()
        .ok_or_else(|| "Invalid session directory".to_string())?;
    let artifact_path = resolve_artifact_path(session_dir, &meta, artifact_kind.trim())?;
    if !artifact_path.exists() {
        return Err("Artifact file not found".to_string());
    }

    let settings = get_settings_from_dirs(dirs.inner())?;
    open_path_in_file_manager(&artifact_path, Some(&settings.artifact_open_app))?;
    Ok("opened".to_string())
}

#[tauri::command]
pub fn delete_session(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
    session_id: String,
    force: Option<bool>,
) -> Result<String, String> {
    let force_delete = force.unwrap_or(false);
    let active_session_id = state
        .active_session
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?
        .as_ref()
        .map(|meta| meta.session_id.clone());
    if active_session_id.as_deref() == Some(session_id.as_str()) {
        if !force_delete {
            return Err("Cannot delete active recording session".to_string());
        }
        let mut capture_guard = state
            .active_capture
            .lock()
            .map_err(|_| "capture state lock poisoned".to_string())?;
        if let Some(capture) = capture_guard.take() {
            let _ = capture.stop_and_take_artifacts();
        }
        drop(capture_guard);
        let mut session_guard = state
            .active_session
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        *session_guard = None;
        state.live_levels.reset();
        set_tray_indicator_from_state(state.inner(), false);
    }

    let session_dir =
        get_session_dir(&dirs.app_data_dir, &session_id)?.ok_or_else(|| "Session not found".to_string())?;
    remove_session_catalog(&session_dir)?;
    let deleted = repo_delete_session(&dirs.app_data_dir, &session_id)?;
    if !deleted {
        return Err("Session not found".to_string());
    }
    Ok("deleted".to_string())
}

#[tauri::command]
pub fn list_sessions(dirs: tauri::State<AppDirs>) -> Result<Vec<SessionListItem>, String> {
    repo_list_sessions(&dirs.app_data_dir)
}

#[tauri::command]
pub fn get_ui_sync_state(state: tauri::State<AppState>) -> Result<UiSyncStateView, String> {
    let ui = state
        .ui_sync
        .lock()
        .map_err(|_| "ui state lock poisoned".to_string())?
        .clone();
    let active = state
        .active_session
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let active_session_id = active.as_ref().map(|s| s.session_id.clone());
    Ok(UiSyncStateView {
        source: ui.source,
        topic: ui.topic,
        is_recording: active.is_some(),
        active_session_id,
    })
}

#[tauri::command]
pub fn get_live_input_levels(state: tauri::State<AppState>) -> Result<LiveInputLevelsView, String> {
    let levels = state.live_levels.snapshot();
    Ok(LiveInputLevelsView {
        mic: levels.mic,
        system: levels.system,
    })
}

#[tauri::command]
pub fn set_ui_sync_state(
    state: tauri::State<AppState>,
    source: String,
    topic: String,
) -> Result<String, String> {
    let mut ui = state
        .ui_sync
        .lock()
        .map_err(|_| "ui state lock poisoned".to_string())?;
    if !source.trim().is_empty() {
        ui.source = source.trim().to_string();
    }
    ui.topic = topic;
    Ok("updated".to_string())
}

#[tauri::command]
pub fn get_session_meta(dirs: tauri::State<AppDirs>, session_id: String) -> Result<SessionMetaView, String> {
    let meta_path = get_meta_path(&dirs.app_data_dir, &session_id)?
        .ok_or_else(|| "Session not found".to_string())?;
    let meta = load_meta(&meta_path)?;
    let custom_tag = meta
        .tags
        .iter()
        .skip(1)
        .find(|v| !v.trim().is_empty())
        .cloned()
        .unwrap_or_default();
    Ok(SessionMetaView {
        session_id: meta.session_id,
        source: meta.primary_tag,
        custom_tag,
        topic: meta.topic,
        participants: meta.participants,
    })
}

#[tauri::command]
pub fn update_session_details(dirs: tauri::State<AppDirs>, payload: UpdateSessionDetailsRequest) -> Result<String, String> {
    let meta_path = get_meta_path(&dirs.app_data_dir, &payload.session_id)?
        .ok_or_else(|| "Session not found".to_string())?;
    let mut meta = load_meta(&meta_path)?;

    let source = if payload.source.trim().is_empty() {
        meta.primary_tag.clone()
    } else {
        payload.source.trim().to_string()
    };
    let custom_tag = payload.custom_tag.trim().to_string();
    let mut tags = vec![source.clone()];
    if !custom_tag.is_empty() {
        tags.push(custom_tag.clone());
    }

    meta.primary_tag = source;
    meta.tags = tags;
    meta.topic = payload.topic.trim().to_string();
    meta.participants = payload
        .participants
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();

    let session_dir = meta_path
        .parent()
        .ok_or_else(|| "Invalid session directory".to_string())?;
    save_meta(&meta_path, &meta)?;
    upsert_session(&dirs.app_data_dir, &meta, session_dir, &meta_path)?;
    add_event(
        &dirs.app_data_dir,
        &meta.session_id,
        "session_details_updated",
        "Source/topic/participants updated",
    )?;
    Ok("updated".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::SessionArtifacts;

    fn sample_meta() -> SessionMeta {
        let mut meta = SessionMeta::new(
            "s1".to_string(),
            vec!["slack".to_string()],
            "Topic".to_string(),
            vec![],
        );
        meta.artifacts = SessionArtifacts {
            audio_file: "audio.opus".to_string(),
            transcript_file: "transcript.txt".to_string(),
            summary_file: "summary.txt".to_string(),
            meta_file: "meta.json".to_string(),
        };
        meta
    }

    #[test]
    fn resolves_transcript_artifact_path() {
        let dir = PathBuf::from("/tmp/s1");
        let path = resolve_artifact_path(&dir, &sample_meta(), "transcript").expect("path");
        assert_eq!(path, PathBuf::from("/tmp/s1/transcript.txt"));
    }

    #[test]
    fn rejects_unknown_artifact_kind() {
        let dir = PathBuf::from("/tmp/s1");
        let result = resolve_artifact_path(&dir, &sample_meta(), "audio");
        assert_eq!(result, Err("Unsupported artifact kind".to_string()));
    }
}
