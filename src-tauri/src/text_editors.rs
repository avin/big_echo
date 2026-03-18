use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEditorApp {
    pub id: String,
    pub name: String,
    pub icon_fallback: String,
    pub icon_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEditorAppsResponse {
    pub apps: Vec<TextEditorApp>,
    pub default_app_id: Option<String>,
}

type IconCache = HashMap<String, Option<String>>;

fn icon_cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn default_text_editor_id() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("textedit")
    } else if cfg!(target_os = "windows") {
        Some("notepad")
    } else {
        None
    }
}

pub fn list_text_editor_apps() -> TextEditorAppsResponse {
    let mut by_id = BTreeMap::<String, TextEditorApp>::new();

    for app in detect_editor_apps_from_system() {
        let id = normalize_editor_id(&app.name);
        if id.is_empty() || by_id.contains_key(&id) {
            continue;
        }
        by_id.insert(
            id.clone(),
            TextEditorApp {
                id: id.clone(),
                icon_fallback: icon_fallback_for_editor_name(&app.name),
                icon_data_url: cached_icon_data_url_for_editor(&id, &app),
                name: app.name,
            },
        );
    }

    let mut apps: Vec<TextEditorApp> = by_id.into_values().collect();
    apps.sort_by_key(|app| priority_for_editor(&app.name.to_ascii_lowercase()));

    let default_app_id = default_text_editor_id().and_then(|expected| {
        apps.iter()
            .find(|app| app.id == expected)
            .map(|app| app.id.clone())
    });

    TextEditorAppsResponse {
        apps,
        default_app_id,
    }
}

fn normalize_editor_id(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

#[derive(Debug, Clone)]
struct DetectedEditorApp {
    name: String,
    bundle_path: Option<PathBuf>,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    executable_path: Option<PathBuf>,
}

fn detect_editor_apps_from_system() -> Vec<DetectedEditorApp> {
    let mut apps: Vec<DetectedEditorApp> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        for dir in [
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            home_dir().join("Applications"),
        ] {
            if !dir.exists() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
                    continue;
                };
                if !ext.eq_ignore_ascii_case("app") {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
                    continue;
                };
                if is_text_editor_name(name) && macos_bundle_can_open_text(&path) {
                    apps.push(DetectedEditorApp {
                        name: name.to_string(),
                        bundle_path: Some(path),
                        executable_path: None,
                    });
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for known in [
            ("Notepad", r"C:\Windows\System32\notepad.exe"),
            (
                "Visual Studio Code",
                r"C:\Program Files\Microsoft VS Code\Code.exe",
            ),
            (
                "Visual Studio Code",
                r"C:\Program Files (x86)\Microsoft VS Code\Code.exe",
            ),
            (
                "Sublime Text",
                r"C:\Program Files\Sublime Text\sublime_text.exe",
            ),
            ("Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
        ] {
            let executable_path = PathBuf::from(known.1);
            if executable_path.exists() {
                apps.push(DetectedEditorApp {
                    name: known.0.to_string(),
                    bundle_path: None,
                    executable_path: Some(executable_path),
                });
            }
        }
    }

    for (name, command) in [
        ("Visual Studio Code", "code"),
        ("Cursor", "cursor"),
        ("Zed", "zed"),
        ("Sublime Text", "subl"),
    ] {
        if command_exists(command) {
            let executable_path = resolve_command_path(command);
            apps.push(DetectedEditorApp {
                name: name.to_string(),
                bundle_path: None,
                executable_path,
            });
        }
    }

    apps
}

fn is_text_editor_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    [
        "textedit",
        "text",
        "code",
        "sublime",
        "vim",
        "nvim",
        "nano",
        "emacs",
        "notepad",
        "bbedit",
        "coteditor",
        "textmate",
        "zed",
        "nova",
    ]
    .iter()
    .any(|part| lowered.contains(part))
}

fn command_exists(command: &str) -> bool {
    let checker = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    Command::new(checker)
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let checker = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    let output = Command::new(checker).arg(command).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let first = stdout.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(PathBuf::from(first))
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn icon_fallback_for_editor_name(name: &str) -> String {
    let lowered = name.to_ascii_lowercase();
    if lowered.contains("code") {
        return "💠".to_string();
    }
    if lowered.contains("cursor") || lowered.contains("zed") {
        return "🧩".to_string();
    }
    if lowered.contains("sublime") {
        return "🟧".to_string();
    }
    if lowered.contains("notepad") {
        return "📓".to_string();
    }
    "📝".to_string()
}

fn icon_data_url_for_editor(editor: &DetectedEditorApp) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = editor.bundle_path.as_deref() {
            return macos_bundle_icon_data_url(path);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = editor.executable_path.as_deref() {
            return windows_executable_icon_data_url(path);
        }
    }
    None
}

fn cached_icon_data_url_for_editor(app_id: &str, editor: &DetectedEditorApp) -> Option<String> {
    if let Ok(cache) = icon_cache().lock() {
        if let Some(cached) = cache.get(app_id) {
            return cached.clone();
        }
    }

    let detected = icon_data_url_for_editor(editor);

    if let Ok(mut cache) = icon_cache().lock() {
        cache.insert(app_id.to_string(), detected.clone());
    }

    detected
}

#[cfg(target_os = "macos")]
fn macos_bundle_icon_data_url(app_path: &Path) -> Option<String> {
    let output_dir = env::temp_dir().join(format!(
        "bigecho_editor_icons_{}_{}",
        std::process::id(),
        normalize_editor_id(app_path.to_string_lossy().as_ref())
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
        .find(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        })?;

    let bytes = fs::read(png_path).ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[cfg(target_os = "windows")]
fn windows_executable_icon_data_url(executable_path: &Path) -> Option<String> {
    let output_dir = env::temp_dir().join(format!(
        "bigecho_editor_icons_{}_{}",
        std::process::id(),
        normalize_editor_id(executable_path.to_string_lossy().as_ref())
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

fn priority_for_editor(name_lower: &str) -> u8 {
    if name_lower == "textedit" || name_lower == "notepad" {
        return 0;
    }
    if name_lower.contains("visual studio code") || name_lower == "vscode" {
        return 1;
    }
    if name_lower.contains("sublime") {
        return 2;
    }
    if name_lower.contains("cursor") || name_lower.contains("zed") {
        return 3;
    }
    10
}

#[cfg(target_os = "macos")]
fn macos_bundle_can_open_text(app_path: &Path) -> bool {
    let info_plist = app_path.join("Contents").join("Info.plist");
    if !info_plist.exists() {
        return false;
    }
    let output = Command::new("plutil")
        .arg("-convert")
        .arg("json")
        .arg("-o")
        .arg("-")
        .arg(&info_plist)
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return false;
    };
    let Some(doc_types) = json.get("CFBundleDocumentTypes").and_then(|v| v.as_array()) else {
        return false;
    };
    doc_types.iter().any(bundle_doc_type_supports_text)
}

#[cfg(target_os = "macos")]
fn bundle_doc_type_supports_text(doc_type: &serde_json::Value) -> bool {
    let content_types = doc_type
        .get("LSItemContentTypes")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let extensions = doc_type
        .get("CFBundleTypeExtensions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>();

    let text_types = [
        "public.plain-text",
        "public.text",
        "public.source-code",
        "net.daringfireball.markdown",
    ];
    let text_exts = [
        "txt", "md", "markdown", "log", "json", "yaml", "yml", "toml", "csv", "tsv", "xml",
    ];

    content_types
        .iter()
        .any(|t| text_types.iter().any(|text_type| t == text_type))
        || extensions
            .iter()
            .any(|ext| text_exts.iter().any(|text_ext| ext == text_ext || ext == "*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_editor_id() {
        assert_eq!(
            normalize_editor_id("Visual Studio Code"),
            "visual_studio_code"
        );
    }

    #[test]
    fn matches_text_editor_names() {
        assert!(is_text_editor_name("TextEdit"));
        assert!(is_text_editor_name("Sublime Text"));
        assert!(!is_text_editor_name("Safari"));
    }

    #[test]
    fn has_fallback_icon_for_known_editor() {
        assert_eq!(icon_fallback_for_editor_name("Visual Studio Code"), "💠");
    }
}
