use crate::app_state::AppDirs;
use crate::settings::public_settings::{save_settings, PublicSettings};
use crate::{get_settings_from_dirs, open_settings_window_internal, open_tray_window_internal};

#[tauri::command]
pub fn get_settings(dirs: tauri::State<AppDirs>) -> Result<PublicSettings, String> {
    get_settings_from_dirs(dirs.inner())
}

#[tauri::command]
pub fn save_public_settings(dirs: tauri::State<AppDirs>, payload: PublicSettings) -> Result<(), String> {
    save_settings(&dirs.app_data_dir, &payload)
}

#[tauri::command]
pub fn list_audio_input_devices() -> Result<Vec<String>, String> {
    crate::audio::capture::list_input_devices()
}

#[tauri::command]
pub fn detect_system_source_device() -> Result<Option<String>, String> {
    crate::audio::capture::detect_system_source_device()
}

#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    open_settings_window_internal(&app)
}

#[tauri::command]
pub fn open_tray_window(app: tauri::AppHandle) -> Result<(), String> {
    open_tray_window_internal(&app)
}
