use crate::app_state::AppDirs;
use crate::settings::public_settings::{save_settings, PublicSettings};
use crate::{get_settings_from_dirs, open_settings_window_internal, open_tray_window_internal};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

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

fn command_exists(program: &str) -> bool {
    let lookup_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    Command::new(lookup_cmd)
        .arg(program)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
pub struct TextEditorApp {
    pub id: String,
    pub name: String,
    pub icon_fallback: String,
    pub icon_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextEditorAppsResponse {
    pub apps: Vec<TextEditorApp>,
    pub default_app_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DetectedEditorApp {
    id: String,
    name: String,
    bundle_path: Option<PathBuf>,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    executable_path: Option<PathBuf>,
}

type IconCache = HashMap<String, Option<String>>;

fn icon_cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn resolve_command_path(program: &str) -> Option<PathBuf> {
    let lookup_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    let output = Command::new(lookup_cmd).arg(program).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let first = stdout.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(PathBuf::from(first))
}

fn process_exists(process_name: &str) -> bool {
    Command::new("pgrep")
        .arg("-x")
        .arg(process_name)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn macos_has_registered_app(app_name: &str) -> bool {
    Command::new("open")
        .arg("-Ra")
        .arg(app_name)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn username_from_env_or_system() -> Option<String> {
    if let Some(name) = env::var_os("USER").and_then(|v| v.into_string().ok()) {
        if !name.trim().is_empty() {
            return Some(name);
        }
    }
    let output = Command::new("id").arg("-un").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn macos_application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications")];

    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Applications"));
    }
    if let Some(user) = username_from_env_or_system() {
        dirs.push(PathBuf::from("/Users").join(user).join("Applications"));
    }
    if let Ok(users) = fs::read_dir("/Users") {
        for entry in users.flatten() {
            dirs.push(entry.path().join("Applications"));
        }
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

fn is_text_editor_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    [
        "textedit",
        "notepad",
        "sublime",
        "code",
        "cursor",
        "windsurf",
        "zed",
        "bbedit",
        "textmate",
        "coteditor",
        "nova",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn detect_text_editor_apps() -> Vec<DetectedEditorApp> {
    let mut detected = Vec::<DetectedEditorApp>::new();

    #[cfg(target_os = "macos")]
    {
        let app_dirs = macos_application_dirs();

        for (display_name, candidates) in [
            ("TextEdit", vec!["TextEdit.app"]),
            ("Visual Studio Code", vec!["Visual Studio Code.app", "Visual Studio Code - Insiders.app"]),
            ("Cursor", vec!["Cursor.app"]),
            ("Windsurf", vec!["Windsurf.app", "Codeium Windsurf.app"]),
            ("Sublime Text", vec!["Sublime Text.app"]),
            ("Zed", vec!["Zed.app"]),
        ] {
            for dir in &app_dirs {
                for candidate in &candidates {
                    let app_path = dir.join(candidate);
                    if !app_path.exists() {
                        continue;
                    }
                    detected.push(DetectedEditorApp {
                        id: app_path.to_string_lossy().to_string(),
                        name: display_name.to_string(),
                        bundle_path: Some(app_path),
                        executable_path: None,
                    });
                }
            }
        }

        for dir in app_dirs {
            if !dir.exists() {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|v| v.to_str()) != Some("app") {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|v| v.to_str()) else {
                    continue;
                };
                if !is_text_editor_name(name) {
                    continue;
                }
                detected.push(DetectedEditorApp {
                    id: path.to_string_lossy().to_string(),
                    name: name.to_string(),
                    bundle_path: Some(path),
                    executable_path: None,
                });
            }
        }

        // LaunchServices fallback: works even when app bundle location is non-standard.
        for app_name in [
            "TextEdit",
            "Visual Studio Code",
            "Visual Studio Code - Insiders",
            "Cursor",
            "Windsurf",
            "Codeium Windsurf",
            "Sublime Text",
            "Zed",
        ] {
            if !macos_has_registered_app(app_name) {
                continue;
            }
            detected.push(DetectedEditorApp {
                id: app_name.to_string(),
                name: app_name.to_string(),
                bundle_path: None,
                executable_path: None,
            });
        }

        // Running-process fallback for environments where LaunchServices lookup is restricted.
        for (display_name, process_name) in [
            ("Visual Studio Code", "Code"),
            ("Sublime Text", "Sublime Text"),
            ("Cursor", "Cursor"),
            ("Windsurf", "Windsurf"),
            ("Zed", "Zed"),
            ("TextEdit", "TextEdit"),
        ] {
            if !process_exists(process_name) {
                continue;
            }
            detected.push(DetectedEditorApp {
                id: format!("running:{display_name}"),
                name: display_name.to_string(),
                bundle_path: None,
                executable_path: None,
            });
        }
    }

    #[cfg(target_os = "windows")]
    {
        for (_id, name, exe_path) in [
            ("notepad", "Notepad", r"C:\Windows\System32\notepad.exe"),
            ("notepad_plus_plus", "Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
            ("visual_studio_code", "Visual Studio Code", r"C:\Program Files\Microsoft VS Code\Code.exe"),
            (
                "visual_studio_code",
                "Visual Studio Code",
                r"C:\Program Files (x86)\Microsoft VS Code\Code.exe",
            ),
            ("sublime_text", "Sublime Text", r"C:\Program Files\Sublime Text\sublime_text.exe"),
        ] {
            let path = PathBuf::from(exe_path);
            if path.exists() {
                detected.push(DetectedEditorApp {
                    id: path.to_string_lossy().to_string(),
                    name: name.to_string(),
                    bundle_path: None,
                    executable_path: Some(path),
                });
            }
        }
    }

    for (_id, name, command) in [
        ("visual_studio_code", "Visual Studio Code", "code"),
        ("cursor", "Cursor", "cursor"),
        ("zed", "Zed", "zed"),
        ("sublime_text", "Sublime Text", "subl"),
    ] {
        if !command_exists(command) {
            continue;
        }
        let resolved_path = resolve_command_path(command);
        detected.push(DetectedEditorApp {
            id: resolved_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| command.to_string()),
            name: name.to_string(),
            bundle_path: None,
            executable_path: resolved_path,
        });
    }

    detected
}

fn fallback_icon_for_editor(name: &str) -> String {
    let lowered = name.to_ascii_lowercase();
    if lowered.contains("code") {
        return "💠".to_string();
    }
    if lowered.contains("notepad") {
        return "📓".to_string();
    }
    if lowered.contains("sublime") {
        return "🟧".to_string();
    }
    if lowered.contains("cursor") || lowered.contains("zed") {
        return "🧩".to_string();
    }
    "📝".to_string()
}

#[cfg(target_os = "macos")]
fn macos_fallback_editor_apps() -> Vec<TextEditorApp> {
    vec![
        TextEditorApp {
            id: "TextEdit".to_string(),
            name: "TextEdit".to_string(),
            icon_fallback: "📝".to_string(),
            icon_data_url: None,
        },
        TextEditorApp {
            id: "Visual Studio Code".to_string(),
            name: "Visual Studio Code".to_string(),
            icon_fallback: "💠".to_string(),
            icon_data_url: None,
        },
        TextEditorApp {
            id: "Cursor".to_string(),
            name: "Cursor".to_string(),
            icon_fallback: "🧩".to_string(),
            icon_data_url: None,
        },
        TextEditorApp {
            id: "Windsurf".to_string(),
            name: "Windsurf".to_string(),
            icon_fallback: "🧩".to_string(),
            icon_data_url: None,
        },
        TextEditorApp {
            id: "Sublime Text".to_string(),
            name: "Sublime Text".to_string(),
            icon_fallback: "🟧".to_string(),
            icon_data_url: None,
        },
        TextEditorApp {
            id: "Zed".to_string(),
            name: "Zed".to_string(),
            icon_fallback: "🧩".to_string(),
            icon_data_url: None,
        },
    ]
}

#[cfg(target_os = "macos")]
fn macos_bundle_icon_data_url(app_path: &Path) -> Option<String> {
    let output_dir = env::temp_dir().join(format!(
        "bigecho_editor_icons_{}_{}",
        std::process::id(),
        normalize_id(app_path.to_string_lossy().as_ref())
    ));
    fs::create_dir_all(&output_dir).ok()?;
    let status = Command::new("qlmanage")
        .arg("-t")
        .arg("-s")
        .arg("64")
        .arg("-o")
        .arg(&output_dir)
        .arg(app_path)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let png_path = fs::read_dir(&output_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))?;
    let bytes = fs::read(png_path).ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[cfg(target_os = "windows")]
fn windows_executable_icon_data_url(executable_path: &Path) -> Option<String> {
    let output_dir = env::temp_dir().join(format!(
        "bigecho_editor_icons_{}_{}",
        std::process::id(),
        normalize_id(executable_path.to_string_lossy().as_ref())
    ));
    fs::create_dir_all(&output_dir).ok()?;
    let png_path = output_dir.join("icon.png");
    let exe_str = executable_path.to_string_lossy().replace('\'', "''");
    let png_str = png_path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Drawing; \
         $icon = [System.Drawing.Icon]::ExtractAssociatedIcon('{exe}'); \
         if ($null -eq $icon) {{ exit 1 }}; \
         $bmp = $icon.ToBitmap(); \
         $bmp.Save('{png}', [System.Drawing.Imaging.ImageFormat]::Png); \
         $bmp.Dispose(); \
         $icon.Dispose();",
        exe = exe_str,
        png = png_str
    );
    let status = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(script)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let bytes = fs::read(png_path).ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

fn icon_data_url_for_app(app: &DetectedEditorApp) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = app.bundle_path.as_deref() {
            return macos_bundle_icon_data_url(path);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = app.executable_path.as_deref() {
            return windows_executable_icon_data_url(path);
        }
    }
    None
}

fn cached_icon_data_url_for_app(id: &str, app: &DetectedEditorApp) -> Option<String> {
    if let Ok(cache) = icon_cache().lock() {
        if let Some(cached) = cache.get(id) {
            return cached.clone();
        }
    }
    let detected = icon_data_url_for_app(app);
    if let Ok(mut cache) = icon_cache().lock() {
        cache.insert(id.to_string(), detected.clone());
    }
    detected
}

#[tauri::command]
pub async fn list_text_editor_apps() -> Result<TextEditorAppsResponse, String> {
    tokio::task::spawn_blocking(list_text_editor_apps_blocking)
        .await
        .map_err(|e| format!("failed to join text editor detection task: {e}"))?
}

fn list_text_editor_apps_blocking() -> Result<TextEditorAppsResponse, String> {
    let mut by_id = BTreeMap::<String, TextEditorApp>::new();
    for app in detect_text_editor_apps() {
        let dedupe_key = normalize_id(&app.name);
        if by_id.contains_key(&dedupe_key) {
            continue;
        }
        by_id.insert(
            dedupe_key,
            TextEditorApp {
                id: app.id.clone(),
                name: app.name.clone(),
                icon_fallback: fallback_icon_for_editor(&app.name),
                icon_data_url: cached_icon_data_url_for_app(&app.id, &app),
            },
        );
    }
    let apps = by_id.into_values().collect::<Vec<_>>();
    #[cfg(target_os = "macos")]
    let apps = if apps.is_empty() { macos_fallback_editor_apps() } else { apps };

    let default_name = if cfg!(target_os = "macos") {
        "TextEdit"
    } else if cfg!(target_os = "windows") {
        "Notepad"
    } else {
        ""
    };
    let default_app_id = if default_name.is_empty() {
        None
    } else {
        apps.iter()
            .find(|app| app.name.eq_ignore_ascii_case(default_name))
            .map(|app| app.id.clone())
    };
    Ok(TextEditorAppsResponse { apps, default_app_id })
}

#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    open_settings_window_internal(&app)
}

#[tauri::command]
pub fn open_tray_window(app: tauri::AppHandle) -> Result<(), String> {
    open_tray_window_internal(&app)
}
