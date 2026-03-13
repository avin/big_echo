use crate::domain::session::SessionMeta;
use std::fs;
use std::path::Path;

pub fn save_meta(path: &Path, meta: &SessionMeta) -> Result<(), String> {
    let body = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(path, body).map_err(|e| e.to_string())
}

pub fn load_meta(path: &Path) -> Result<SessionMeta, String> {
    let body = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}
