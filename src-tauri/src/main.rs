mod audio;
mod command_core;
mod domain;
mod pipeline;
mod settings;
mod storage;

use chrono::{DateTime, Local};
use command_core::{
    ensure_stop_session_matches, mark_pipeline_audio_missing, mark_pipeline_done,
    mark_pipeline_summary_failed, mark_pipeline_transcribed, mark_pipeline_transcription_failed,
    should_schedule_retry, validate_start_request, PipelineInvocation,
};
use domain::session::{SessionArtifacts, SessionMeta, SessionStatus};
use serde::{Deserialize, Serialize};
use settings::public_settings::{load_settings, save_settings, PublicSettings};
use settings::secret_store::{get_secret, set_secret};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use storage::fs_layout::{build_session_relative_dir, summary_name, transcript_name};
use storage::session_store::{load_meta, save_meta};
use storage::sqlite_repo::{
    add_event, clear_retry_job, delete_session as repo_delete_session, fetch_due_retry_jobs, get_meta_path,
    get_session_dir, list_sessions as repo_list_sessions, schedule_retry_job, upsert_session, SessionListItem,
};
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Listener, Manager, PhysicalPosition, Position, Theme, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_global_shortcut::{Builder as GlobalShortcutBuilder, ShortcutState};
use uuid::Uuid;

const MAX_PIPELINE_RETRY_ATTEMPTS: i64 = 4;
const RETRY_WORKER_POLL_SECONDS: u64 = 20;
const LIVE_LEVELS_IDLE_POLL_MS: u64 = 260;
const TRAY_ICON_ID: &str = "bigecho-tray";
const REC_HOTKEY: &str = "CmdOrCtrl+Shift+R";
const STOP_HOTKEY: &str = "CmdOrCtrl+Shift+S";
const APP_ICON_LIGHT_BYTES: &[u8] = include_bytes!("../icons/app-icon-light.png");
const APP_ICON_DARK_BYTES: &[u8] = include_bytes!("../icons/app-icon-dark.png");
const TRAY_IDLE_LIGHT_BYTES: &[u8] = include_bytes!("../icons/tray-idle-light.png");
const TRAY_IDLE_DARK_BYTES: &[u8] = include_bytes!("../icons/tray-idle-dark.png");
const TRAY_REC_DARK_BYTES: &[u8] = include_bytes!("../icons/tray-rec-dark.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayIconVariant {
    IdleLight,
    IdleDark,
    RecDark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipelineMode {
    Full,
    TranscriptionOnly,
    SummaryOnly,
}

struct AppState {
    active_session: Mutex<Option<SessionMeta>>,
    active_capture: Mutex<Option<audio::capture::ContinuousCapture>>,
    ui_sync: Mutex<UiSyncState>,
    live_levels: audio::capture::SharedLevels,
    tray_app: Mutex<Option<AppHandle>>,
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
struct AppDirs {
    app_data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiSyncState {
    source: String,
    topic: String,
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
struct UiSyncStateView {
    source: String,
    topic: String,
    is_recording: bool,
    active_session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LiveInputLevelsView {
    mic: f32,
    system: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartRecordingRequest {
    tags: Vec<String>,
    topic: String,
    participants: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartRecordingResponse {
    session_id: String,
    session_dir: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateSessionDetailsRequest {
    session_id: String,
    source: String,
    custom_tag: String,
    topic: String,
    participants: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionMetaView {
    session_id: String,
    source: String,
    custom_tag: String,
    topic: String,
    participants: Vec<String>,
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))
}

fn root_recordings_dir(app_data_dir: &std::path::Path, settings: &PublicSettings) -> Result<PathBuf, String> {
    let root = PathBuf::from(&settings.recording_root);
    if root.is_absolute() {
        Ok(root)
    } else {
        Ok(app_data_dir.join(root))
    }
}

fn file_has_non_empty_text(path: &std::path::Path) -> bool {
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

fn audio_duration_hms(meta: &SessionMeta) -> String {
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

fn get_settings_from_dirs(dirs: &AppDirs) -> Result<PublicSettings, String> {
    load_settings(&dirs.app_data_dir)
}

fn should_auto_run_pipeline_after_stop(settings: &PublicSettings) -> bool {
    settings.auto_run_pipeline_on_stop
        && !settings.transcription_url.trim().is_empty()
        && !settings.summary_url.trim().is_empty()
}

fn should_intercept_close_to_tray(window_label: &str) -> bool {
    window_label == "main"
}

fn should_start_hidden_on_launch(value: Option<&str>, default_hidden: bool) -> bool {
    match value {
        None => default_hidden,
        Some(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        }
    }
}

fn should_start_hidden_on_launch_from_env() -> bool {
    let env_value = std::env::var("BIGECHO_START_HIDDEN").ok();
    let default_hidden = !cfg!(debug_assertions);
    should_start_hidden_on_launch(env_value.as_deref(), default_hidden)
}

fn register_close_to_tray_for_main(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        let label = main_window.label().to_string();
        let window_for_event = main_window.clone();
        main_window.on_window_event(move |event| {
            if let tauri::WindowEvent::ThemeChanged(theme) = event {
                let app = window_for_event.app_handle();
                let _ = apply_app_icons_for_theme(&app, theme.clone());
                let state = app.state::<AppState>();
                let _ = set_tray_indicator(&app, is_recording_active(state.inner()));
            }
            if !should_intercept_close_to_tray(&label) {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_for_event.hide();
            }
        });
    }
}

fn toggle_main_window_visibility(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().map_err(|e| e.to_string())? {
            window.hide().map_err(|e| e.to_string())?;
        } else {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn should_show_context_menu_on_left_click(platform: &str) -> bool {
    platform == "windows"
}

fn should_toggle_tray_popover_on_left_click(platform: &str) -> bool {
    platform == "macos"
}

fn should_hide_tray_popover_on_focus_lost(platform: &str, focused: bool) -> bool {
    platform == "macos" && !focused
}

fn position_tray_popover(window: &tauri::WebviewWindow, anchor: PhysicalPosition<f64>) -> Result<(), String> {
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let x = (anchor.x.round() as i32) - (size.width as i32 / 2);
    let y = (anchor.y.round() as i32) + 12;
    window
        .set_position(Position::Physical(PhysicalPosition::new(x, y)))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn toggle_tray_window_visibility(app: &AppHandle, anchor: Option<PhysicalPosition<f64>>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("tray") {
        if window.is_visible().map_err(|e| e.to_string())? {
            window.hide().map_err(|e| e.to_string())?;
        } else {
            if let Some(anchor) = anchor {
                let _ = position_tray_popover(&window, anchor);
            }
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    open_tray_window_internal(app)?;
    if let Some(window) = app.get_webview_window("tray") {
        if let Some(anchor) = anchor {
            let _ = position_tray_popover(&window, anchor);
        }
    }
    Ok(())
}

fn choose_tray_icon_variant(theme: Theme, is_recording: bool) -> TrayIconVariant {
    if is_recording {
        return TrayIconVariant::RecDark;
    }
    match theme {
        Theme::Dark => TrayIconVariant::IdleDark,
        _ => TrayIconVariant::IdleLight,
    }
}

#[cfg(target_os = "macos")]
fn build_macos_app_menu(app: &tauri::App) -> Result<Menu<tauri::Wry>, String> {
    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let app_submenu = Submenu::with_items(
        app,
        pkg_info.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None::<&str>, Some(about_metadata)).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &MenuItem::with_id(app, "app_settings", "Settings", true, Some("CmdOrCtrl+," as &str))
                .map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::services(app, None::<&str>).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::hide(app, None::<&str>).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::hide_others(app, None::<&str>).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::quit(app, None::<&str>).map_err(|e| e.to_string())?,
        ],
    )
    .map_err(|e| e.to_string())?;

    let window_menu = Submenu::with_id_and_items(
        app,
        tauri::menu::WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None::<&str>).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::maximize(app, None::<&str>).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::close_window(app, None::<&str>).map_err(|e| e.to_string())?,
        ],
    )
    .map_err(|e| e.to_string())?;

    let help_menu = Submenu::with_id_and_items(
        app,
        tauri::menu::HELP_SUBMENU_ID,
        "Help",
        true,
        &[],
    )
    .map_err(|e| e.to_string())?;

    Menu::with_items(
        app,
        &[
            &app_submenu,
            &Submenu::with_items(
                app,
                "File",
                true,
                &[&PredefinedMenuItem::close_window(app, None::<&str>).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None::<&str>).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::redo(app, None::<&str>).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::cut(app, None::<&str>).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::copy(app, None::<&str>).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::paste(app, None::<&str>).map_err(|e| e.to_string())?,
                    &PredefinedMenuItem::select_all(app, None::<&str>).map_err(|e| e.to_string())?,
                ],
            )
            .map_err(|e| e.to_string())?,
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None::<&str>).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?,
            &window_menu,
            &help_menu,
        ],
    )
    .map_err(|e| e.to_string())
}

fn tray_icon_bytes(variant: TrayIconVariant) -> &'static [u8] {
    match variant {
        TrayIconVariant::IdleLight => TRAY_IDLE_LIGHT_BYTES,
        TrayIconVariant::IdleDark => TRAY_IDLE_DARK_BYTES,
        TrayIconVariant::RecDark => TRAY_REC_DARK_BYTES,
    }
}

fn app_icon_bytes(theme: Theme) -> &'static [u8] {
    match theme {
        Theme::Dark => APP_ICON_DARK_BYTES,
        _ => APP_ICON_LIGHT_BYTES,
    }
}

fn load_png_icon(bytes: &'static [u8]) -> Result<tauri::image::Image<'static>, String> {
    tauri::image::Image::from_bytes(bytes)
        .map(|image| image.to_owned())
        .map_err(|e| format!("failed to decode icon: {e}"))
}

fn resolve_system_theme(app: &AppHandle) -> Theme {
    for label in ["main", "tray", "settings"] {
        if let Some(window) = app.get_webview_window(label) {
            if let Ok(theme) = window.theme() {
                return theme;
            }
        }
    }
    Theme::Light
}

fn apply_app_icons_for_theme(app: &AppHandle, theme: Theme) -> Result<(), String> {
    let icon = load_png_icon(app_icon_bytes(theme))?;
    for label in ["main", "tray", "settings"] {
        if let Some(window) = app.get_webview_window(label) {
            window.set_icon(icon.clone()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn is_recording_active(state: &AppState) -> bool {
    state
        .active_session
        .lock()
        .map(|session| session.is_some())
        .unwrap_or(false)
}

fn set_tray_indicator(app: &AppHandle, is_recording: bool) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        let tooltip = if is_recording { "BigEcho REC" } else { "BigEcho IDLE" };
        tray.set_tooltip(Some(tooltip)).map_err(|e| e.to_string())?;
        let theme = resolve_system_theme(app);
        let icon = load_png_icon(tray_icon_bytes(choose_tray_icon_variant(theme, is_recording)))?;
        tray.set_icon(Some(icon)).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        tray.set_icon_as_template(false).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn set_tray_indicator_from_state(state: &AppState, is_recording: bool) {
    let app_handle = state
        .tray_app
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(app) = app_handle {
        let _ = set_tray_indicator(&app, is_recording);
    }
}

fn focus_main_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn open_settings_window_internal(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = apply_app_icons_for_theme(app, resolve_system_theme(app));
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("BigEcho Settings")
        .inner_size(720.0, 620.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = apply_app_icons_for_theme(app, resolve_system_theme(app));
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

fn open_tray_window_internal(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("tray") {
        let _ = apply_app_icons_for_theme(app, resolve_system_theme(app));
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(app, "tray", WebviewUrl::App("index.html".into()))
        .title("BigEcho Recorder")
        .inner_size(460.0, 244.0)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .decorations(false)
            .shadow(false)
            .transparent(true)
            .visible_on_all_workspaces(true);
    }

    let window = builder.build().map_err(|e| e.to_string())?;
    let window_for_event = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(focused) = event {
            if should_hide_tray_popover_on_focus_lost(std::env::consts::OS, *focused) {
                let _ = window_for_event.hide();
            }
        }
    });
    let _ = apply_app_icons_for_theme(app, resolve_system_theme(app));
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

fn stop_active_recording_internal(
    dirs: &AppDirs,
    state: &AppState,
    session_id: Option<&str>,
    app: Option<&AppHandle>,
) -> Result<String, String> {
    let mut guard = state
        .active_session
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;

    let mut meta = guard
        .take()
        .ok_or_else(|| "No active recording session".to_string())?;

    ensure_stop_session_matches(&meta.session_id, session_id)?;

    meta.status = SessionStatus::Recorded;
    meta.ended_at_iso = Some(Local::now().to_rfc3339());

    let settings = get_settings_from_dirs(dirs)?;
    let started_at: DateTime<Local> = DateTime::parse_from_rfc3339(&meta.started_at_iso)
        .map_err(|e| e.to_string())?
        .with_timezone(&Local);
    let rel_dir = build_session_relative_dir(&meta.primary_tag, started_at);
    let abs_dir = root_recordings_dir(&dirs.app_data_dir, &settings)?.join(&rel_dir);

    save_meta(&abs_dir.join("meta.json"), &meta)?;
    let data_dir = dirs.app_data_dir.clone();
    upsert_session(&data_dir, &meta, &abs_dir, &abs_dir.join("meta.json"))?;
    add_event(&data_dir, &meta.session_id, "recording_stopped", "Audio capture stopped")?;

    let mut cap_guard = state
        .active_capture
        .lock()
        .map_err(|_| "capture state lock poisoned".to_string())?;
    if let Some(capture) = cap_guard.take() {
        let artifacts = capture.stop_and_take_artifacts()?;
        let write_res = audio::opus_writer::write_mixed_raw_i16_to_opus(
            &abs_dir.join("audio.opus"),
            &artifacts.mic_path,
            artifacts.mic_rate,
            artifacts.system_path.as_ref(),
            artifacts.system_rate,
            settings.opus_bitrate_kbps,
        );
        audio::capture::cleanup_artifacts(&artifacts);
        write_res?;
    } else {
        audio::opus_writer::write_pcm_opus(
            &abs_dir.join("audio.opus"),
            48_000,
            &[],
            settings.opus_bitrate_kbps,
        )?;
    }
    state.live_levels.reset();

    if let Some(app) = app {
        let _ = set_tray_indicator(app, false);
    } else {
        set_tray_indicator_from_state(state, false);
    }

    if should_auto_run_pipeline_after_stop(&settings) {
        let dirs_for_pipeline = dirs.clone();
        let session_id = meta.session_id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = run_pipeline_core(
                dirs_for_pipeline,
                &session_id,
                PipelineInvocation::Run,
                PipelineMode::Full,
            )
            .await;
        });
    }

    Ok("recorded".to_string())
}

#[tauri::command]
fn get_settings(dirs: tauri::State<AppDirs>) -> Result<PublicSettings, String> {
    get_settings_from_dirs(dirs.inner())
}

#[tauri::command]
fn save_public_settings(dirs: tauri::State<AppDirs>, payload: PublicSettings) -> Result<(), String> {
    save_settings(&dirs.app_data_dir, &payload)
}

#[tauri::command]
fn list_audio_input_devices() -> Result<Vec<String>, String> {
    audio::capture::list_input_devices()
}

#[tauri::command]
fn detect_system_source_device() -> Result<Option<String>, String> {
    audio::capture::detect_system_source_device()
}

#[tauri::command]
fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    open_settings_window_internal(&app)
}

#[tauri::command]
fn open_tray_window(app: tauri::AppHandle) -> Result<(), String> {
    open_tray_window_internal(&app)
}

fn open_path_in_file_manager(path: &str) -> Result<(), String> {
    let target = path.trim();
    if target.is_empty() {
        return Err("Session directory is empty".to_string());
    }
    let status = if cfg!(target_os = "macos") {
        Command::new("open")
            .arg(target)
            .status()
            .map_err(|e| e.to_string())?
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
            .arg(target)
            .status()
            .map_err(|e| e.to_string())?
    } else {
        Command::new("xdg-open")
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

fn append_api_call_log_line(session_dir: &std::path::Path, event_type: &str, detail: &str) -> Result<(), String> {
    let log_path = session_dir.join("api_calls.txt");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let timestamp = chrono::Local::now().to_rfc3339();
    writeln!(file, "{timestamp} | {event_type} | {detail}").map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_session_folder(session_dir: String) -> Result<String, String> {
    open_path_in_file_manager(&session_dir)?;
    Ok("opened".to_string())
}

fn remove_session_catalog(path: &std::path::Path) -> Result<(), String> {
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
fn delete_session(
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
fn list_sessions(dirs: tauri::State<AppDirs>) -> Result<Vec<SessionListItem>, String> {
    let mut sessions = repo_list_sessions(&dirs.app_data_dir)?;
    for item in &mut sessions {
        let Ok(Some(meta_path)) = get_meta_path(&dirs.app_data_dir, &item.session_id) else {
            continue;
        };
        let Ok(meta) = load_meta(&meta_path) else {
            continue;
        };

        let session_dir = PathBuf::from(&item.session_dir);
        let transcript_ok = file_has_non_empty_text(&session_dir.join(&meta.artifacts.transcript_file));
        let summary_ok = file_has_non_empty_text(&session_dir.join(&meta.artifacts.summary_file));
        item.audio_duration_hms = audio_duration_hms(&meta);

        item.has_transcript_text = transcript_ok
            && !matches!(meta.status, SessionStatus::Recording | SessionStatus::Recorded);
        item.has_summary_text =
            summary_ok && matches!(meta.status, SessionStatus::Summarized | SessionStatus::Done);
    }
    Ok(sessions)
}

#[tauri::command]
fn get_ui_sync_state(state: tauri::State<AppState>) -> Result<UiSyncStateView, String> {
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
fn get_live_input_levels(state: tauri::State<AppState>) -> Result<LiveInputLevelsView, String> {
    let levels = state.live_levels.snapshot();
    Ok(LiveInputLevelsView {
        mic: levels.mic,
        system: levels.system,
    })
}

#[tauri::command]
fn set_ui_sync_state(
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
fn get_session_meta(dirs: tauri::State<AppDirs>, session_id: String) -> Result<SessionMetaView, String> {
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
fn update_session_details(dirs: tauri::State<AppDirs>, payload: UpdateSessionDetailsRequest) -> Result<String, String> {
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

#[tauri::command]
fn set_api_secret(dirs: tauri::State<AppDirs>, name: String, value: String) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Secret name must not be empty".to_string());
    }
    set_secret(&dirs.app_data_dir, name.trim(), value.trim())
}

#[tauri::command]
fn get_api_secret(dirs: tauri::State<AppDirs>, name: String) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("Secret name must not be empty".to_string());
    }
    get_secret(&dirs.app_data_dir, name.trim())
}

#[tauri::command]
fn start_recording(
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
    fs::create_dir_all(&abs_dir).map_err(|e| e.to_string())?;

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

    // Placeholder artifacts for pipeline integration
    fs::write(abs_dir.join(&meta.artifacts.transcript_file), "").map_err(|e| e.to_string())?;
    fs::write(abs_dir.join(&meta.artifacts.summary_file), "").map_err(|e| e.to_string())?;

    let system_source = if settings.system_device_name.trim().is_empty() {
        audio::capture::detect_system_source_device()?
    } else {
        Some(settings.system_device_name.clone())
    };

    let capture = audio::capture::ContinuousCapture::start(
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
fn stop_recording(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
    session_id: String,
) -> Result<String, String> {
    stop_active_recording_internal(dirs.inner(), state.inner(), Some(session_id.as_str()), None)
}

#[tauri::command]
fn stop_active_recording(
    dirs: tauri::State<AppDirs>,
    state: tauri::State<AppState>,
) -> Result<String, String> {
    stop_active_recording_internal(dirs.inner(), state.inner(), None, None)
}

#[tauri::command]
async fn run_pipeline(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Run,
        PipelineMode::Full,
    )
    .await
}

#[tauri::command]
async fn retry_pipeline(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Retry,
        PipelineMode::Full,
    )
    .await
}

#[tauri::command]
async fn run_transcription(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Manual,
        PipelineMode::TranscriptionOnly,
    )
    .await
}

#[tauri::command]
async fn run_summary(dirs: tauri::State<'_, AppDirs>, session_id: String) -> Result<String, String> {
    run_pipeline_core(
        dirs.inner().clone(),
        &session_id,
        PipelineInvocation::Manual,
        PipelineMode::SummaryOnly,
    )
    .await
}

async fn run_pipeline_core(
    dirs: AppDirs,
    session_id: &str,
    invocation: PipelineInvocation,
    mode: PipelineMode,
) -> Result<String, String> {
    let settings = get_settings_from_dirs(&dirs)?;
    let data_dir = dirs.app_data_dir.clone();
    let meta_path = get_meta_path(&data_dir, session_id)?.ok_or_else(|| "Session not found".to_string())?;
    let mut meta = load_meta(&meta_path)?;
    let session_dir = meta_path
        .parent()
        .ok_or_else(|| "Invalid session directory".to_string())?;
    let api_logging_enabled = settings.api_call_logging_enabled;
    let log_session_id = meta.session_id.clone();
    let log_session_dir = session_dir.to_path_buf();
    let log_api_call = |event_type: &str, detail: String| {
        if api_logging_enabled {
            let _ = add_event(&data_dir, &log_session_id, event_type, &detail);
            let _ = append_api_call_log_line(&log_session_dir, event_type, &detail);
        }
    };

    let audio_path = session_dir.join(&meta.artifacts.audio_file);
    if !audio_path.exists() {
        let detail = mark_pipeline_audio_missing(&mut meta);
        save_meta(&meta_path, &meta)?;
        upsert_session(&data_dir, &meta, session_dir, &meta_path)?;
        add_event(&data_dir, &meta.session_id, "pipeline_failed", &detail)?;
        if should_schedule_retry(invocation) {
            schedule_retry_for_session(&data_dir, &meta.session_id, &detail)?;
        }
        return Err(detail);
    }

    let (nexara_key, nexara_key_lookup_err) = match get_secret(&dirs.app_data_dir, "NEXARA_API_KEY") {
        Ok(value) => (value, None),
        Err(err) => (String::new(), Some(err)),
    };
    let openai_key = get_secret(&dirs.app_data_dir, "OPENAI_API_KEY").unwrap_or_default();

    let needs_transcription = matches!(mode, PipelineMode::Full | PipelineMode::TranscriptionOnly);
    let needs_summary = matches!(mode, PipelineMode::Full | PipelineMode::SummaryOnly);

    let mut transcript: Option<String> = None;
    if needs_transcription {
        log_api_call(
            "api_transcription_request",
            format!(
                "url={} task={} diarization_setting={}",
                settings.transcription_url.trim(),
                settings.transcription_task.trim(),
                settings.transcription_diarization_setting.trim()
            ),
        );
        let transcribed = match pipeline::transcribe_audio(&settings, &nexara_key, &audio_path).await {
            Ok(text) => text,
            Err(err) => {
                log_api_call("api_transcription_error", format!("error={err}"));
                let err = if err.contains("No token specified") {
                    if let Some(keyring_err) = nexara_key_lookup_err.as_ref() {
                        format!("{err}. keyring lookup error for NEXARA_API_KEY: {keyring_err}")
                    } else if nexara_key.trim().is_empty() {
                        format!("{err}. NEXARA_API_KEY is empty")
                    } else {
                        err
                    }
                } else {
                    err
                };
                let detail = mark_pipeline_transcription_failed(&mut meta, &err);
                save_meta(&meta_path, &meta)?;
                upsert_session(&data_dir, &meta, session_dir, &meta_path)?;
                add_event(&data_dir, &meta.session_id, "pipeline_failed", &detail)?;
                if should_schedule_retry(invocation) {
                    schedule_retry_for_session(&data_dir, &meta.session_id, &detail)?;
                }
                return Err(err);
            }
        };
        log_api_call(
            "api_transcription_success",
            format!("transcript_chars={}", transcribed.chars().count()),
        );
        fs::write(session_dir.join(&meta.artifacts.transcript_file), &transcribed).map_err(|e| e.to_string())?;
        mark_pipeline_transcribed(&mut meta);
        save_meta(&meta_path, &meta)?;
        upsert_session(&data_dir, &meta, session_dir, &meta_path)?;
        add_event(&data_dir, &meta.session_id, "transcribed", "Transcript created")?;
        transcript = Some(transcribed);
    }

    if needs_summary {
        let transcript_for_summary = if let Some(text) = transcript {
            text
        } else {
            let transcript_path = session_dir.join(&meta.artifacts.transcript_file);
            let text = fs::read_to_string(&transcript_path).map_err(|_| "Transcript file is missing".to_string())?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err("Transcript file is empty".to_string());
            }
            trimmed.to_string()
        };

        log_api_call(
            "api_summary_request",
            format!(
                "url={} model={} prompt_chars={}",
                settings.summary_url.trim(),
                settings.openai_model.trim(),
                settings.summary_prompt.trim().chars().count()
            ),
        );
        let summary = match pipeline::summarize_text(&settings, &openai_key, &transcript_for_summary).await {
            Ok(text) => text,
            Err(err) => {
                log_api_call("api_summary_error", format!("error={err}"));
                let detail = mark_pipeline_summary_failed(&mut meta, &err);
                save_meta(&meta_path, &meta)?;
                upsert_session(&data_dir, &meta, session_dir, &meta_path)?;
                add_event(&data_dir, &meta.session_id, "pipeline_failed", &detail)?;
                if should_schedule_retry(invocation) {
                    schedule_retry_for_session(&data_dir, &meta.session_id, &detail)?;
                }
                return Err(err);
            }
        };
        log_api_call(
            "api_summary_success",
            format!("summary_chars={}", summary.chars().count()),
        );
        fs::write(session_dir.join(&meta.artifacts.summary_file), &summary).map_err(|e| e.to_string())?;
        mark_pipeline_done(&mut meta);
        save_meta(&meta_path, &meta)?;
        upsert_session(&data_dir, &meta, session_dir, &meta_path)?;
        add_event(&data_dir, &meta.session_id, "pipeline_done", "Summary created")?;
    }

    if matches!(mode, PipelineMode::Full) {
        clear_retry_job(&data_dir, &meta.session_id)?;
        return Ok("done".to_string());
    }
    if matches!(mode, PipelineMode::TranscriptionOnly) {
        return Ok("transcribed".to_string());
    }
    Ok("done".to_string())
}

fn schedule_retry_for_session(data_dir: &PathBuf, session_id: &str, error: &str) -> Result<(), String> {
    match schedule_retry_job(data_dir, session_id, error, MAX_PIPELINE_RETRY_ATTEMPTS)? {
        Some(attempt) => {
            add_event(
                data_dir,
                session_id,
                "pipeline_retry_scheduled",
                &format!("Attempt {} scheduled due to: {}", attempt, error),
            )?;
        }
        None => {
            add_event(
                data_dir,
                session_id,
                "pipeline_retry_exhausted",
                "Retry attempts exhausted",
            )?;
        }
    }
    Ok(())
}

async fn process_retry_jobs_once(dirs: &AppDirs, now_epoch: i64, limit: usize) -> Result<(), String> {
    let data_dir = dirs.app_data_dir.clone();
    let jobs = fetch_due_retry_jobs(&data_dir, now_epoch, limit)?;
    for job in jobs {
        let session_id = job.session_id.clone();
        let result = run_pipeline_core(
            dirs.clone(),
            &session_id,
            PipelineInvocation::WorkerRetry,
            PipelineMode::Full,
        )
        .await;
        if result.is_ok() {
            clear_retry_job(&data_dir, &session_id)?;
            add_event(
                &data_dir,
                &session_id,
                "pipeline_retry_success",
                "Retry succeeded",
            )?;
        } else if let Err(err) = result {
            schedule_retry_for_session(&data_dir, &session_id, &err)?;
        }
    }
    Ok(())
}

fn spawn_retry_worker(dirs: AppDirs) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(RETRY_WORKER_POLL_SECONDS)).await;
            let now = chrono::Utc::now().timestamp();
            let _ = process_retry_jobs_once(&dirs, now, 10).await;
        }
    });
}

fn spawn_live_levels_worker(app: AppHandle, dirs: AppDirs) {
    tauri::async_runtime::spawn(async move {
        loop {
            let recording_active = {
                let state = app.state::<AppState>();
                is_recording_active(state.inner())
            };
            if !recording_active {
                let settings = get_settings_from_dirs(&dirs).ok();
                let (mic_name, system_name) = if let Some(settings) = settings {
                    let mic = settings.mic_device_name.trim().to_string();
                    let system = settings.system_device_name.trim().to_string();
                    (
                        if mic.is_empty() { None } else { Some(mic) },
                        if system.is_empty() { None } else { Some(system) },
                    )
                } else {
                    (None, None)
                };
                let probe_result = tauri::async_runtime::spawn_blocking(move || {
                    audio::capture::probe_levels(mic_name.as_deref(), system_name.as_deref())
                })
                .await
                .ok()
                .and_then(Result::ok);

                let state = app.state::<AppState>();
                if let Some(levels) = probe_result {
                    state.live_levels.set_mic(levels.mic);
                    state.live_levels.set_system(levels.system);
                } else {
                    state.live_levels.reset();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(LIVE_LEVELS_IDLE_POLL_MS)).await;
        }
    });
}

fn parse_recording_flag(payload: &str) -> bool {
    fn from_value(value: &serde_json::Value) -> Option<bool> {
        value.get("recording").and_then(|f| f.as_bool())
    }

    let parsed = serde_json::from_str::<serde_json::Value>(payload).ok();
    match parsed {
        Some(serde_json::Value::Object(_)) => parsed.as_ref().and_then(from_value).unwrap_or(false),
        Some(serde_json::Value::String(inner)) => serde_json::from_str::<serde_json::Value>(&inner)
            .ok()
            .as_ref()
            .and_then(from_value)
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod ipc_runtime_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use storage::session_store::{load_meta, save_meta};
    use storage::sqlite_repo::{
        fetch_due_retry_jobs, list_session_events, list_sessions, schedule_retry_job, upsert_session,
    };
    use serde_json::json;
    use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;

    #[test]
    fn close_to_tray_intercepts_only_main_window() {
        assert!(should_intercept_close_to_tray("main"));
        assert!(!should_intercept_close_to_tray("settings"));
    }

    #[test]
    fn start_hidden_env_policy_respects_debug_default() {
        assert!(!should_start_hidden_on_launch(None, false));
        assert!(should_start_hidden_on_launch(None, true));
        assert!(should_start_hidden_on_launch(Some("1"), false));
        assert!(should_start_hidden_on_launch(Some("true"), false));
        assert!(!should_start_hidden_on_launch(Some("0"), true));
        assert!(!should_start_hidden_on_launch(Some("false"), true));
    }

    #[test]
    fn tray_variant_depends_on_theme_and_recording_status() {
        assert_eq!(
            choose_tray_icon_variant(Theme::Light, false),
            TrayIconVariant::IdleLight
        );
        assert_eq!(
            choose_tray_icon_variant(Theme::Dark, false),
            TrayIconVariant::IdleDark
        );
        assert_eq!(
            choose_tray_icon_variant(Theme::Light, true),
            TrayIconVariant::RecDark
        );
        assert_eq!(
            choose_tray_icon_variant(Theme::Dark, true),
            TrayIconVariant::RecDark
        );
    }

    #[test]
    fn tray_popover_autoclose_policy_is_platform_specific() {
        assert!(should_hide_tray_popover_on_focus_lost("macos", false));
        assert!(!should_hide_tray_popover_on_focus_lost("macos", true));
        assert!(!should_hide_tray_popover_on_focus_lost("windows", false));
    }

    #[test]
    fn parse_recording_flag_supports_object_and_nested_json_string() {
        assert!(parse_recording_flag(r#"{"recording":true}"#));
        assert!(parse_recording_flag(r#""{\"recording\":true}""#));
        assert!(!parse_recording_flag(r#"{"recording":false}"#));
    }

    #[test]
    fn audio_duration_is_formatted_as_hh_mm_ss() {
        let mut meta = SessionMeta::new(
            "s-duration".to_string(),
            vec!["slack".to_string()],
            "".to_string(),
            vec![],
        );
        meta.started_at_iso = "2026-03-11T10:00:00+03:00".to_string();
        meta.ended_at_iso = Some("2026-03-11T11:02:03+03:00".to_string());
        assert_eq!(audio_duration_hms(&meta), "01:02:03");
    }

    #[test]
    fn tray_left_click_policy_is_platform_specific() {
        assert!(should_show_context_menu_on_left_click("windows"));
        assert!(!should_show_context_menu_on_left_click("macos"));
        assert!(should_toggle_tray_popover_on_left_click("macos"));
        assert!(!should_toggle_tray_popover_on_left_click("windows"));
        assert!(!should_toggle_tray_popover_on_left_click("linux"));
    }

    #[test]
    fn auto_pipeline_after_stop_requires_toggle_and_urls() {
        let disabled = PublicSettings::default();
        assert!(!should_auto_run_pipeline_after_stop(&disabled));

        let no_urls = PublicSettings {
            auto_run_pipeline_on_stop: true,
            ..Default::default()
        };
        assert!(!should_auto_run_pipeline_after_stop(&no_urls));

        let ready = PublicSettings {
            transcription_url: "https://example.com/transcribe".to_string(),
            summary_url: "https://example.com/summary".to_string(),
            summary_prompt: "Есть стенограмма встречи. Подготовь краткое саммари.".to_string(),
            auto_run_pipeline_on_stop: true,
            ..Default::default()
        };
        assert!(should_auto_run_pipeline_after_stop(&ready));
    }

    fn invoke_request(cmd: &str, body: serde_json::Value) -> InvokeRequest {
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().expect("valid test url"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        }
    }

    fn extract_err_string(err: serde_json::Value) -> String {
        match err {
            serde_json::Value::String(v) => v,
            other => other.to_string(),
        }
    }

    fn extract_ok_json(body: InvokeResponseBody) -> serde_json::Value {
        match body {
            InvokeResponseBody::Json(v) => {
                serde_json::from_str(&v).expect("json response should be valid")
            }
            InvokeResponseBody::Raw(v) => {
                serde_json::from_slice(v.as_ref()).expect("raw body should be valid json")
            }
        }
    }

    fn build_test_app() -> (tauri::App<tauri::test::MockRuntime>, std::path::PathBuf) {
        let mut ctx = mock_context(noop_assets());
        ctx.config_mut().identifier = "dev.bigecho.tests".to_string();
        let app_data_dir = std::env::temp_dir().join(format!("bigecho_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&app_data_dir).expect("create app-data");
        let dirs = AppDirs {
            app_data_dir: app_data_dir.clone(),
        };

        let app = mock_builder()
            .manage(AppState::default())
            .manage(dirs)
            .invoke_handler(tauri::generate_handler![
                get_settings,
                save_public_settings,
                list_sessions,
                get_live_input_levels,
                open_session_folder,
                delete_session,
                get_session_meta,
                update_session_details,
                start_recording,
                stop_recording,
                stop_active_recording,
                run_pipeline,
                retry_pipeline,
                run_transcription,
                run_summary
            ])
            .build(ctx)
            .expect("failed to build test app");
        (app, app_data_dir)
    }

    fn spawn_mock_pipeline_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let request_line = read_http_request_line(&mut stream);
                let body = if request_line.contains("/transcribe") {
                    r#"{"text":"mock transcript"}"#
                } else if request_line.contains("/summary") {
                    r#"{"choices":[{"message":{"content":"mock summary"}}]}"#
                } else {
                    r#"{"error":"not found"}"#
                };
                write_http_json_response(&mut stream, body);
            }
        });
        format!("http://{addr}")
    }

    fn read_http_request_line(stream: &mut TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read request line");

        let mut content_length = 0usize;
        loop {
            let mut header_line = String::new();
            reader
                .read_line(&mut header_line)
                .expect("read header line");
            if header_line == "\r\n" {
                break;
            }
            let lower = header_line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("content-length:") {
                content_length = rest.trim().parse::<usize>().unwrap_or(0);
            }
        }
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("read request body");
        }
        line
    }

    fn write_http_json_response(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        stream.flush().expect("flush response");
    }

    fn seed_pipeline_ready_session(app_data_dir: &std::path::Path, session_id: &str, base_url: &str) {
        let settings = PublicSettings {
            recording_root: app_data_dir.join("recordings").to_string_lossy().to_string(),
            transcription_url: format!("{base_url}/transcribe"),
            transcription_task: "transcribe".to_string(),
            transcription_diarization_setting: "general".to_string(),
            summary_url: format!("{base_url}/summary"),
            summary_prompt: "Есть стенограмма встречи. Подготовь краткое саммари.".to_string(),
            openai_model: "gpt-4.1-mini".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        };
        save_settings(app_data_dir, &settings).expect("save settings");

        let session_dir = app_data_dir.join("sessions").join(session_id);
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let meta_path = session_dir.join("meta.json");
        let mut meta = SessionMeta::new(
            session_id.to_string(),
            vec!["zoom".to_string()],
            "Weekly sync".to_string(),
            vec!["Alice".to_string()],
        );
        meta.artifacts.audio_file = "audio.opus".to_string();
        meta.artifacts.transcript_file = "transcript.txt".to_string();
        meta.artifacts.summary_file = "summary.txt".to_string();
        save_meta(&meta_path, &meta).expect("save meta");
        std::fs::write(session_dir.join("audio.opus"), b"OggS").expect("write audio fixture");
        upsert_session(app_data_dir, &meta, &session_dir, &meta_path).expect("upsert session");
    }

    fn seed_pipeline_missing_audio_session(app_data_dir: &std::path::Path, session_id: &str, base_url: &str) {
        let settings = PublicSettings {
            recording_root: app_data_dir.join("recordings").to_string_lossy().to_string(),
            transcription_url: format!("{base_url}/transcribe"),
            transcription_task: "transcribe".to_string(),
            transcription_diarization_setting: "general".to_string(),
            summary_url: format!("{base_url}/summary"),
            summary_prompt: "Есть стенограмма встречи. Подготовь краткое саммари.".to_string(),
            openai_model: "gpt-4.1-mini".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        };
        save_settings(app_data_dir, &settings).expect("save settings");

        let session_dir = app_data_dir.join("sessions").join(session_id);
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let meta_path = session_dir.join("meta.json");
        let mut meta = SessionMeta::new(
            session_id.to_string(),
            vec!["zoom".to_string()],
            "Weekly sync".to_string(),
            vec!["Alice".to_string()],
        );
        meta.artifacts.audio_file = "audio.opus".to_string();
        meta.artifacts.transcript_file = "transcript.txt".to_string();
        meta.artifacts.summary_file = "summary.txt".to_string();
        save_meta(&meta_path, &meta).expect("save meta");
        upsert_session(app_data_dir, &meta, &session_dir, &meta_path).expect("upsert session");
    }

    #[test]
    fn invoke_start_allows_empty_topic_and_participants() {
        let (app, _) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let response = get_ipc_response(
            &webview,
            invoke_request(
                "start_recording",
                json!({
                    "payload": {"tags":["zoom"], "topic":"", "participants":[]}
                }),
            ),
        );
        let out = extract_ok_json(response.expect("command must succeed"));
        let parsed: StartRecordingResponse = serde_json::from_value(out).expect("parse response");
        assert!(!parsed.session_id.is_empty());
        assert_eq!(parsed.status, "recording");
    }

    #[test]
    fn invoke_update_session_details_persists_values() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");

        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-details", &base_url);

        let update_response = get_ipc_response(
            &webview,
            invoke_request(
                "update_session_details",
                json!({
                    "payload": {
                        "session_id":"session-details",
                        "source":"telegram",
                        "custom_tag":"client-a",
                        "topic":"",
                        "participants":["Alice", "Bob"]
                    }
                }),
            ),
        );
        let update_out = extract_ok_json(update_response.expect("update should succeed"));
        assert_eq!(update_out, serde_json::Value::String("updated".to_string()));

        let get_response = get_ipc_response(
            &webview,
            invoke_request("get_session_meta", json!({ "sessionId":"session-details" })),
        );
        let get_out = extract_ok_json(get_response.expect("get should succeed"));
        let details: SessionMetaView = serde_json::from_value(get_out).expect("parse details");
        assert_eq!(details.source, "telegram");
        assert_eq!(details.custom_tag, "client-a");
        assert_eq!(details.topic, "");
        assert_eq!(details.participants, vec!["Alice".to_string(), "Bob".to_string()]);
    }

    #[test]
    fn invoke_stop_rejects_without_active_session() {
        let (app, _) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let response = get_ipc_response(
            &webview,
            invoke_request("stop_recording", json!({ "sessionId":"missing-session" })),
        );
        let err = response.expect_err("command must fail");
        assert_eq!(extract_err_string(err), "No active recording session");
    }

    #[test]
    fn invoke_pipeline_rejects_unknown_session() {
        let (app, _) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let response = get_ipc_response(
            &webview,
            invoke_request("run_pipeline", json!({ "sessionId":"missing-session" })),
        );
        let err = response.expect_err("command must fail");
        assert_eq!(extract_err_string(err), "Session not found");
    }

    #[test]
    fn invoke_pipeline_success_writes_transcript_and_summary() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-success", &base_url);
        let mut settings = load_settings(&app_data_dir).expect("load settings");
        settings.api_call_logging_enabled = true;
        save_settings(&app_data_dir, &settings).expect("save settings");

        let response = get_ipc_response(
            &webview,
            invoke_request("run_pipeline", json!({ "sessionId":"session-success" })),
        )
        .expect("run_pipeline should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("done string"),
            "done".to_string()
        );

        let session_dir = app_data_dir.join("sessions").join("session-success");
        let transcript = std::fs::read_to_string(session_dir.join("transcript.txt"))
            .expect("read transcript");
        let summary = std::fs::read_to_string(session_dir.join("summary.txt"))
            .expect("read summary");
        assert_eq!(transcript, "mock transcript");
        assert_eq!(summary, "mock summary");
        let api_log = std::fs::read_to_string(session_dir.join("api_calls.txt"))
            .expect("read api_calls.txt");
        assert!(api_log.contains("api_transcription_request"));
        assert!(api_log.contains("api_transcription_success"));
        assert!(api_log.contains("api_summary_request"));
        assert!(api_log.contains("api_summary_success"));

        let meta = load_meta(&session_dir.join("meta.json")).expect("load meta");
        assert_eq!(meta.status, SessionStatus::Done);

        let listed = list_sessions(&app_data_dir).expect("list sessions");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, "done");

        let due_retry = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch retry jobs");
        assert!(due_retry.is_empty());

        let events = list_session_events(&app_data_dir, "session-success").expect("load events");
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "api_transcription_request")
        );
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "api_transcription_success")
        );
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "api_summary_request")
        );
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "api_summary_success")
        );
    }

    #[test]
    fn invoke_retry_pipeline_success_writes_transcript_and_summary() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-retry-success", &base_url);

        let response = get_ipc_response(
            &webview,
            invoke_request("retry_pipeline", json!({ "sessionId":"session-retry-success" })),
        )
        .expect("retry_pipeline should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("done string"),
            "done".to_string()
        );

        let session_dir = app_data_dir.join("sessions").join("session-retry-success");
        let transcript = std::fs::read_to_string(session_dir.join("transcript.txt"))
            .expect("read transcript");
        let summary = std::fs::read_to_string(session_dir.join("summary.txt"))
            .expect("read summary");
        assert_eq!(transcript, "mock transcript");
        assert_eq!(summary, "mock summary");

        let listed = list_sessions(&app_data_dir).expect("list sessions");
        assert_eq!(listed[0].status, "done");
    }

    #[test]
    fn invoke_run_transcription_writes_only_transcript() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-get-text", &base_url);

        let response = get_ipc_response(
            &webview,
            invoke_request("run_transcription", json!({ "sessionId":"session-get-text" })),
        )
        .expect("run_transcription should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("transcribed string"),
            "transcribed".to_string()
        );

        let session_dir = app_data_dir.join("sessions").join("session-get-text");
        let transcript = std::fs::read_to_string(session_dir.join("transcript.txt"))
            .expect("read transcript");
        assert_eq!(transcript, "mock transcript");
        assert!(!session_dir.join("summary.txt").exists());

        let listed = list_sessions(&app_data_dir).expect("list sessions");
        assert_eq!(listed[0].status, "transcribed");
    }

    #[test]
    fn invoke_run_summary_from_existing_transcript() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-summary-only", &base_url);
        let session_dir = app_data_dir.join("sessions").join("session-summary-only");
        std::fs::write(session_dir.join("transcript.txt"), "existing transcript").expect("write transcript");

        let response = get_ipc_response(
            &webview,
            invoke_request("run_summary", json!({ "sessionId":"session-summary-only" })),
        )
        .expect("run_summary should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("done string"),
            "done".to_string()
        );

        let summary = std::fs::read_to_string(session_dir.join("summary.txt"))
            .expect("read summary");
        assert_eq!(summary, "mock summary");
    }

    #[test]
    fn invoke_retry_pipeline_audio_missing_schedules_retry_job() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");

        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_missing_audio_session(&app_data_dir, "session-retry-failed", &base_url);

        let response = get_ipc_response(
            &webview,
            invoke_request("retry_pipeline", json!({ "sessionId":"session-retry-failed" })),
        );
        let err = response.expect_err("retry_pipeline should fail");
        assert_eq!(extract_err_string(err), "Audio file is missing");

        let due_retry = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch retry jobs");
        assert_eq!(due_retry.len(), 1);
        assert_eq!(due_retry[0].session_id, "session-retry-failed");
        assert_eq!(due_retry[0].attempts, 1);
    }

    #[test]
    fn invoke_delete_session_removes_catalog_and_db_record() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-delete", &base_url);

        let session_dir = app_data_dir.join("sessions").join("session-delete");
        assert!(session_dir.exists());
        assert_eq!(list_sessions(&app_data_dir).expect("list sessions").len(), 1);

        let response = get_ipc_response(
            &webview,
            invoke_request("delete_session", json!({ "sessionId":"session-delete" })),
        )
        .expect("delete_session should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("deleted string"),
            "deleted".to_string()
        );

        assert!(!session_dir.exists());
        assert!(list_sessions(&app_data_dir).expect("list sessions").is_empty());
        assert!(list_session_events(&app_data_dir, "session-delete")
            .expect("load events")
            .is_empty());
    }

    #[test]
    fn invoke_delete_session_force_allows_active_session_cleanup() {
        let (app, app_data_dir) = build_test_app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview should be created");
        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-delete-active", &base_url);

        {
            let state = app.state::<AppState>();
            let mut active = state.active_session.lock().expect("active session lock");
            *active = Some(SessionMeta::new(
                "session-delete-active".to_string(),
                vec!["zoom".to_string()],
                "Broken active".to_string(),
                vec![],
            ));
        }

        let blocked = get_ipc_response(
            &webview,
            invoke_request("delete_session", json!({ "sessionId":"session-delete-active" })),
        )
        .expect_err("delete_session without force should fail");
        assert_eq!(
            extract_err_string(blocked),
            "Cannot delete active recording session"
        );

        let response = get_ipc_response(
            &webview,
            invoke_request(
                "delete_session",
                json!({ "sessionId":"session-delete-active", "force": true }),
            ),
        )
        .expect("forced delete_session should succeed");
        assert_eq!(
            response.deserialize::<String>().expect("deleted string"),
            "deleted".to_string()
        );

        let state = app.state::<AppState>();
        let active = state.active_session.lock().expect("active session lock");
        assert!(active.is_none());
        assert!(
            !app_data_dir
                .join("sessions")
                .join("session-delete-active")
                .exists()
        );
    }

    #[test]
    fn retry_worker_exhausts_attempts_and_clears_job() {
        let app_data_dir = std::env::temp_dir().join(format!("bigecho_worker_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&app_data_dir).expect("create app data");
        let dirs = AppDirs {
            app_data_dir: app_data_dir.clone(),
        };

        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_missing_audio_session(&app_data_dir, "session-worker-exhaust", &base_url);

        for _ in 0..MAX_PIPELINE_RETRY_ATTEMPTS {
            let _ = schedule_retry_job(
                &app_data_dir,
                "session-worker-exhaust",
                "seed retry",
                MAX_PIPELINE_RETRY_ATTEMPTS,
            )
            .expect("seed retry attempt");
        }

        let initial_jobs = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch initial jobs");
        assert_eq!(initial_jobs.len(), 1);
        assert_eq!(initial_jobs[0].attempts, MAX_PIPELINE_RETRY_ATTEMPTS);

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let result = rt.block_on(run_pipeline_core(
            dirs.clone(),
            "session-worker-exhaust",
            PipelineInvocation::WorkerRetry,
            PipelineMode::Full,
        ));
        let err = result.expect_err("worker run should fail without audio");
        assert_eq!(err, "Audio file is missing");

        schedule_retry_for_session(&app_data_dir, "session-worker-exhaust", &err)
            .expect("schedule followup retry");
        let final_jobs = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch final jobs");
        assert!(final_jobs.is_empty());

        let events =
            list_session_events(&app_data_dir, "session-worker-exhaust").expect("load event log");
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "pipeline_retry_exhausted")
        );
    }

    #[test]
    fn retry_worker_process_once_handles_partial_failures() {
        let app_data_dir = std::env::temp_dir().join(format!("bigecho_worker_mix_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&app_data_dir).expect("create app data");
        let dirs = AppDirs {
            app_data_dir: app_data_dir.clone(),
        };

        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_ready_session(&app_data_dir, "session-worker-ok", &base_url);
        seed_pipeline_missing_audio_session(&app_data_dir, "session-worker-fail", &base_url);

        schedule_retry_job(&app_data_dir, "session-worker-ok", "seed retry", MAX_PIPELINE_RETRY_ATTEMPTS)
            .expect("schedule ok");
        schedule_retry_job(
            &app_data_dir,
            "session-worker-fail",
            "seed retry",
            MAX_PIPELINE_RETRY_ATTEMPTS,
        )
        .expect("schedule fail");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(process_retry_jobs_once(&dirs, i64::MAX, 10))
            .expect("process retry jobs");

        let ok_meta = load_meta(&app_data_dir.join("sessions").join("session-worker-ok").join("meta.json"))
            .expect("load ok meta");
        let fail_meta = load_meta(&app_data_dir.join("sessions").join("session-worker-fail").join("meta.json"))
            .expect("load fail meta");
        assert_eq!(ok_meta.status, SessionStatus::Done);
        assert_eq!(fail_meta.status, SessionStatus::Failed);

        let due_jobs = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch jobs");
        assert_eq!(due_jobs.len(), 1);
        assert_eq!(due_jobs[0].session_id, "session-worker-fail");
        assert_eq!(due_jobs[0].attempts, 2);

        let ok_events = list_session_events(&app_data_dir, "session-worker-ok").expect("ok events");
        let fail_events = list_session_events(&app_data_dir, "session-worker-fail").expect("fail events");
        assert!(
            ok_events
                .iter()
                .any(|e| e.event_type == "pipeline_retry_success")
        );
        assert!(
            fail_events
                .iter()
                .any(|e| e.event_type == "pipeline_retry_scheduled")
        );
    }

    #[test]
    fn retry_worker_process_once_respects_limit() {
        let app_data_dir = std::env::temp_dir().join(format!("bigecho_worker_limit_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&app_data_dir).expect("create app data");
        let dirs = AppDirs {
            app_data_dir: app_data_dir.clone(),
        };

        let base_url = spawn_mock_pipeline_server();
        seed_pipeline_missing_audio_session(&app_data_dir, "session-limit-a", &base_url);
        seed_pipeline_missing_audio_session(&app_data_dir, "session-limit-b", &base_url);

        schedule_retry_job(&app_data_dir, "session-limit-a", "seed retry", MAX_PIPELINE_RETRY_ATTEMPTS)
            .expect("schedule a");
        schedule_retry_job(&app_data_dir, "session-limit-b", "seed retry", MAX_PIPELINE_RETRY_ATTEMPTS)
            .expect("schedule b");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(process_retry_jobs_once(&dirs, i64::MAX, 1))
            .expect("process retry jobs");

        let due_jobs = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch jobs");
        assert_eq!(due_jobs.len(), 2);
        let attempts = due_jobs.iter().map(|j| j.attempts).collect::<Vec<_>>();
        assert!(attempts.contains(&1));
        assert!(attempts.contains(&2));

        let events_a = list_session_events(&app_data_dir, "session-limit-a").expect("events a");
        let events_b = list_session_events(&app_data_dir, "session-limit-b").expect("events b");
        let scheduled_count = events_a
            .iter()
            .chain(events_b.iter())
            .filter(|e| e.event_type == "pipeline_retry_scheduled")
            .count();
        assert_eq!(scheduled_count, 1);
    }
}

fn main() {
    let builder = tauri::Builder::default();
    let builder = builder.setup(|app| {
        let data_dir = app_data_dir(&app.handle())?;
        app.manage(AppDirs {
            app_data_dir: data_dir.clone(),
        });
        register_close_to_tray_for_main(&app.handle());
        let _ = apply_app_icons_for_theme(&app.handle(), resolve_system_theme(&app.handle()));
        if let Ok(mut tray_app) = app.state::<AppState>().tray_app.lock() {
            *tray_app = Some(app.handle().clone());
        }
        if should_start_hidden_on_launch_from_env() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        spawn_retry_worker(AppDirs {
            app_data_dir: data_dir.clone(),
        });
        spawn_live_levels_worker(
            app.handle().clone(),
            AppDirs {
                app_data_dir: data_dir.clone(),
            },
        );
        #[cfg(target_os = "macos")]
        {
            let app_menu = build_macos_app_menu(app)?;
            app.set_menu(app_menu).map_err(|e| e.to_string())?;
            app.on_menu_event(|app, event| {
                if event.id().as_ref() == "app_settings" {
                    let _ = open_settings_window_internal(app);
                }
            });
        }

        let open_item = MenuItem::with_id(app, "open", "Open BigEcho", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let recorder_item = MenuItem::with_id(app, "recorder", "Recorder", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let toggle_item = MenuItem::with_id(app, "toggle", "Show/Hide BigEcho", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let start_item = MenuItem::with_id(app, "start", "Start Recording", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let stop_item = MenuItem::with_id(app, "stop", "Stop Recording", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
            .map_err(|e| e.to_string())?;

        let menu = Menu::with_items(
            app,
            &[&open_item, &recorder_item, &toggle_item, &start_item, &stop_item, &settings_item, &quit_item],
        )
            .map_err(|e| e.to_string())?;

        let initial_tray_icon = load_png_icon(tray_icon_bytes(choose_tray_icon_variant(
            resolve_system_theme(&app.handle()),
            false,
        )))?;
        let left_click_context_menu = should_show_context_menu_on_left_click(std::env::consts::OS);

        TrayIconBuilder::with_id(TRAY_ICON_ID)
            .icon(initial_tray_icon)
            .menu(&menu)
            .tooltip("BigEcho IDLE")
            .show_menu_on_left_click(left_click_context_menu)
            .on_menu_event(|tray, event| {
                let app = tray.app_handle();
                match event.id().as_ref() {
                    "open" => {
                        let _ = focus_main_window(app);
                    }
                    "toggle" => {
                        let _ = toggle_main_window_visibility(app);
                    }
                    "recorder" => {
                        let _ = open_tray_window_internal(app);
                    }
                    "start" => {
                        let _ = focus_main_window(app);
                        let _ = app.emit("tray:start", ());
                    }
                    "stop" => {
                        let _ = app.emit("tray:stop", ());
                        let state = app.state::<AppState>();
                        let dirs = app.state::<AppDirs>();
                        let _ = stop_active_recording_internal(dirs.inner(), state.inner(), None, Some(app));
                    }
                    "settings" => {
                        let _ = open_settings_window_internal(app);
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                }
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } = event
                {
                    if should_toggle_tray_popover_on_left_click(std::env::consts::OS) {
                        let _ = toggle_tray_window_visibility(tray.app_handle(), Some(position));
                    }
                }
            })
            .build(app)
            .map_err(|e| e.to_string())?;
        let _ = set_tray_indicator(&app.handle(), false);

        let app_handle = app.handle().clone();
        let _status_listener = app.listen("recording:status", move |event: tauri::Event| {
            let recording = parse_recording_flag(event.payload());
            let _ = set_tray_indicator(&app_handle, recording);
        });
        let app_handle = app.handle().clone();
        let _ui_recording_listener = app.listen("ui:recording", move |event: tauri::Event| {
            let recording = parse_recording_flag(event.payload());
            let _ = set_tray_indicator(&app_handle, recording);
        });
        Ok(())
    });

    builder
        .plugin(
            GlobalShortcutBuilder::new()
                .with_shortcuts([REC_HOTKEY, STOP_HOTKEY])
                .expect("failed to register global shortcuts")
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let shortcut_text = shortcut.to_string();
                    if shortcut_text == REC_HOTKEY {
                        let _ = app.emit("tray:start", ());
                    } else if shortcut_text == STOP_HOTKEY {
                        let _ = app.emit("tray:stop", ());
                        let state = app.state::<AppState>();
                        let dirs = app.state::<AppDirs>();
                        let _ = stop_active_recording_internal(dirs.inner(), state.inner(), None, Some(app));
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_public_settings,
            list_audio_input_devices,
            detect_system_source_device,
            open_settings_window,
            open_tray_window,
            open_session_folder,
            delete_session,
            list_sessions,
            get_ui_sync_state,
            set_ui_sync_state,
            get_live_input_levels,
            get_session_meta,
            update_session_details,
            set_api_secret,
            get_api_secret,
            start_recording,
            stop_recording,
            stop_active_recording,
            run_pipeline,
            retry_pipeline,
            run_transcription,
            run_summary
        ])
        .run(tauri::generate_context!())
        .expect("error while running bigecho app");
}
