use crate::text_editors::default_text_editor_id;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicSettings {
    pub recording_root: String,
    pub artifact_open_app: String,
    pub transcription_url: String,
    pub transcription_task: String,
    pub transcription_diarization_setting: String,
    pub summary_url: String,
    pub summary_prompt: String,
    pub openai_model: String,
    pub audio_format: String,
    pub opus_bitrate_kbps: u32,
    pub mic_device_name: String,
    pub system_device_name: String,
    pub artifact_opener_app: String,
    pub auto_run_pipeline_on_stop: bool,
    pub api_call_logging_enabled: bool,
}

impl Default for PublicSettings {
    fn default() -> Self {
        Self {
            recording_root: "./recordings".to_string(),
            artifact_open_app: String::new(),
            transcription_url: String::new(),
            transcription_task: "transcribe".to_string(),
            transcription_diarization_setting: "general".to_string(),
            summary_url: String::new(),
            summary_prompt: "Есть стенограмма встречи. Подготовь краткое саммари.".to_string(),
            openai_model: "gpt-4.1-mini".to_string(),
            audio_format: "opus".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            artifact_opener_app: default_text_editor_id().unwrap_or_default().to_string(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        }
    }
}

impl PublicSettings {
    fn parse_http_url(value: &str, field: &str) -> Result<(), String> {
        let parsed = Url::parse(value).map_err(|_| format!("Invalid {field} URL"))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(format!("Invalid {field} URL"));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.audio_format.as_str(),
            "opus" | "mp3" | "m4a" | "ogg" | "wav"
        ) {
            return Err("Invalid audio format".to_string());
        }
        if !self.transcription_url.is_empty() {
            Self::parse_http_url(&self.transcription_url, "transcription")?;
        }
        if !self.summary_url.is_empty() {
            Self::parse_http_url(&self.summary_url, "summary")?;
        }
        if self.transcription_task != "transcribe" && self.transcription_task != "diarize" {
            return Err("Invalid transcription task".to_string());
        }
        if self.transcription_diarization_setting != "general"
            && self.transcription_diarization_setting != "meeting"
            && self.transcription_diarization_setting != "telephonic"
        {
            return Err("Invalid diarization setting".to_string());
        }
        if self.opus_bitrate_kbps < 12 || self.opus_bitrate_kbps > 128 {
            return Err("Opus bitrate must be between 12 and 128 kbps".to_string());
        }
        Ok(())
    }
}

pub fn settings_file_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

pub fn load_settings(app_data_dir: &Path) -> Result<PublicSettings, String> {
    let path = settings_file_path(app_data_dir);
    if !path.exists() {
        return Ok(PublicSettings::default());
    }
    let body = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

pub fn save_settings(app_data_dir: &Path, settings: &PublicSettings) -> Result<(), String> {
    settings.validate()?;
    fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
    let path = settings_file_path(app_data_dir);
    let body = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_urls() {
        let s = PublicSettings {
            transcription_url: "not-a-url".to_string(),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn accepts_valid_urls() {
        let s = PublicSettings {
            transcription_url: "https://example.com/transcribe".to_string(),
            summary_url: "https://example.com/summary".to_string(),
            ..Default::default()
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn rejects_non_http_urls() {
        let s = PublicSettings {
            transcription_url: "file:///tmp/transcribe".to_string(),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn auto_run_pipeline_on_stop_is_disabled_by_default() {
        assert!(!PublicSettings::default().auto_run_pipeline_on_stop);
    }

    #[test]
    fn api_call_logging_is_disabled_by_default() {
        assert!(!PublicSettings::default().api_call_logging_enabled);
    }

    #[test]
    fn missing_auto_run_pipeline_on_stop_uses_default() {
        let body = r#"{
            "recording_root":"./recordings",
            "artifact_open_app":"",
            "transcription_url":"",
            "transcription_task":"transcribe",
            "transcription_diarization_setting":"general",
            "summary_url":"",
            "summary_prompt":"Есть стенограмма встречи. Подготовь краткое саммари.",
            "openai_model":"gpt-4.1-mini",
            "audio_format":"opus",
            "opus_bitrate_kbps":24,
            "mic_device_name":"",
            "system_device_name":""
        }"#;
        let parsed: PublicSettings = serde_json::from_str(body).expect("settings should parse");
        assert!(!parsed.auto_run_pipeline_on_stop);
        assert!(!parsed.api_call_logging_enabled);
        assert_eq!(
            parsed.artifact_opener_app,
            default_text_editor_id().unwrap_or_default().to_string()
        );
    }

    #[test]
    fn missing_transcription_task_fields_use_defaults() {
        let body = r#"{
            "recording_root":"./recordings",
            "artifact_open_app":"",
            "transcription_url":"",
            "summary_url":"",
            "openai_model":"gpt-4.1-mini",
            "audio_format":"opus",
            "opus_bitrate_kbps":24,
            "mic_device_name":"",
            "system_device_name":""
        }"#;
        let parsed: PublicSettings = serde_json::from_str(body).expect("settings should parse");
        assert_eq!(parsed.transcription_task, "transcribe");
        assert_eq!(parsed.transcription_diarization_setting, "general");
    }

    #[test]
    fn missing_summary_prompt_uses_default() {
        let body = r#"{
            "recording_root":"./recordings",
            "artifact_open_app":"",
            "transcription_url":"",
            "transcription_task":"transcribe",
            "transcription_diarization_setting":"general",
            "summary_url":"",
            "openai_model":"gpt-4.1-mini",
            "audio_format":"opus",
            "opus_bitrate_kbps":24,
            "mic_device_name":"",
            "system_device_name":""
        }"#;
        let parsed: PublicSettings = serde_json::from_str(body).expect("settings should parse");
        assert_eq!(
            parsed.summary_prompt,
            "Есть стенограмма встречи. Подготовь краткое саммари."
        );
    }

    #[test]
    fn missing_artifact_open_app_uses_default() {
        let body = r#"{
            "recording_root":"./recordings",
            "transcription_url":"",
            "transcription_task":"transcribe",
            "transcription_diarization_setting":"general",
            "summary_url":"",
            "summary_prompt":"Есть стенограмма встречи. Подготовь краткое саммари.",
            "openai_model":"gpt-4.1-mini",
            "audio_format":"opus",
            "opus_bitrate_kbps":24,
            "mic_device_name":"",
            "system_device_name":"",
            "auto_run_pipeline_on_stop":false,
            "api_call_logging_enabled":false
        }"#;
        let parsed: PublicSettings = serde_json::from_str(body).expect("settings should parse");
        assert_eq!(parsed.artifact_open_app, "");
    }

    #[test]
    fn missing_audio_format_uses_default() {
        let body = r#"{
            "recording_root":"./recordings",
            "artifact_open_app":"",
            "transcription_url":"",
            "transcription_task":"transcribe",
            "transcription_diarization_setting":"general",
            "summary_url":"",
            "summary_prompt":"Есть стенограмма встречи. Подготовь краткое саммари.",
            "openai_model":"gpt-4.1-mini",
            "opus_bitrate_kbps":24,
            "mic_device_name":"",
            "system_device_name":"",
            "auto_run_pipeline_on_stop":false,
            "api_call_logging_enabled":false
        }"#;
        let parsed: PublicSettings = serde_json::from_str(body).expect("settings should parse");
        assert_eq!(parsed.audio_format, "opus");
    }

    #[test]
    fn rejects_unknown_audio_format() {
        let s = PublicSettings {
            audio_format: "flac".to_string(),
            ..Default::default()
        };
        assert_eq!(s.validate(), Err("Invalid audio format".to_string()));
    }
}
