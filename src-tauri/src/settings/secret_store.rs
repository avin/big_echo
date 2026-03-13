use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SERVICE_NAME: &str = "BigEcho";
const FALLBACK_FILE_NAME: &str = "secrets.local.json";

fn fallback_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(FALLBACK_FILE_NAME)
}

fn load_fallback_map(path: &Path) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let obj = json
        .as_object()
        .ok_or_else(|| "Invalid fallback secret file format".to_string())?;
    let mut out = HashMap::new();
    for (k, v) in obj {
        if let Some(text) = v.as_str() {
            out.insert(k.clone(), text.to_string());
        }
    }
    Ok(out)
}

fn save_fallback_map(path: &Path, map: &HashMap<String, String>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(path, raw).map_err(|e| e.to_string())
}

pub fn set_secret(app_data_dir: &Path, name: &str, value: &str) -> Result<(), String> {
    let mut keyring_err: Option<String> = None;
    match keyring::Entry::new(SERVICE_NAME, name).map_err(|e| e.to_string()) {
        Ok(entry) => {
            if let Err(err) = entry.set_password(value) {
                keyring_err = Some(err.to_string());
            }
        }
        Err(err) => keyring_err = Some(err),
    }

    let path = fallback_path(app_data_dir);
    let mut map = load_fallback_map(&path)?;
    map.insert(name.to_string(), value.to_string());
    save_fallback_map(&path, &map)?;

    if let Some(err) = keyring_err {
        eprintln!("warning: failed to write keyring secret {name}: {err}");
    }
    Ok(())
}

pub fn get_secret(app_data_dir: &Path, name: &str) -> Result<String, String> {
    let keyring_result = keyring::Entry::new(SERVICE_NAME, name)
        .map_err(|e| e.to_string())
        .and_then(|entry| entry.get_password().map_err(|e| e.to_string()));
    if let Ok(value) = keyring_result {
        return Ok(value);
    }

    let path = fallback_path(app_data_dir);
    let map = load_fallback_map(&path)?;
    if let Some(value) = map.get(name) {
        return Ok(value.clone());
    }

    Err(keyring_result
        .err()
        .unwrap_or_else(|| "No matching entry found in secure storage".to_string()))
}
