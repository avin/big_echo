use crate::settings::public_settings::PublicSettings;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use std::path::Path;

fn format_diarize_segments(body: &serde_json::Value) -> Option<String> {
    let task = body
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if task != "diarize" {
        return None;
    }
    let segments = body.get("segments").and_then(|v| v.as_array())?;
    let mut lines: Vec<String> = Vec::new();
    for segment in segments {
        let text = segment
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let speaker = segment
            .get("speaker")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("speaker_unknown");
        lines.push(format!("{speaker}: {text}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n\n"))
    }
}

pub async fn transcribe_audio(
    settings: &PublicSettings,
    api_key: &str,
    audio_path: &Path,
) -> Result<String, String> {
    if settings.transcription_url.trim().is_empty() {
        return Err("Transcription URL is not configured".to_string());
    }

    let data = std::fs::read(audio_path).map_err(|e| e.to_string())?;
    let file_name = audio_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.opus")
        .to_string();
    let mime = crate::audio::file_writer::mime_type_for_audio_path(audio_path);

    let part = reqwest::multipart::Part::bytes(data)
        .file_name(file_name)
        .mime_str(mime)
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("task", settings.transcription_task.trim().to_string())
        .text(
            "diarization_setting",
            settings
                .transcription_diarization_setting
                .trim()
                .to_string(),
        )
        .text("model", "whisper-1")
        .text("response_format", "json");

    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", api_key.trim());
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&bearer).map_err(|e| e.to_string())?,
    );

    let client = reqwest::Client::new();
    let res = client
        .post(&settings.transcription_url)
        .headers(headers)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let detail = body.trim();
        if detail.is_empty() {
            return Err(format!("transcription failed with status {status}"));
        }
        return Err(format!(
            "transcription failed with status {status}: {}",
            detail.chars().take(280).collect::<String>()
        ));
    }

    let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    if let Some(formatted) = format_diarize_segments(&body) {
        return Ok(formatted);
    }
    Ok(body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

pub async fn summarize_text(
    settings: &PublicSettings,
    api_key: &str,
    transcript: &str,
) -> Result<String, String> {
    if settings.summary_url.trim().is_empty() {
        return Err("Summary URL is not configured".to_string());
    }

    let summary_prompt = if settings.summary_prompt.trim().is_empty() {
        "Есть стенограмма встречи. Подготовь краткое саммари."
    } else {
        settings.summary_prompt.trim()
    };

    let payload = json!({
      "model": settings.openai_model,
      "temperature": 0.2,
      "messages": [
        {"role": "system", "content": summary_prompt},
        {"role": "user", "content": transcript}
      ]
    });

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if !api_key.trim().is_empty() {
        let bearer = format!("Bearer {}", api_key.trim());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer).map_err(|e| e.to_string())?,
        );
    }

    let client = reqwest::Client::new();
    let res = client
        .post(&settings.summary_url)
        .headers(headers)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("summary failed with status {}", res.status()));
    }

    let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::public_settings::PublicSettings;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut data = Vec::new();
        let mut buf = [0_u8; 4096];
        let mut header_end = None;
        let mut content_length = 0_usize;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("set timeout");
        loop {
            let n = stream.read(&mut buf).expect("read request");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            if header_end.is_none() {
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let end = pos + 4;
                    header_end = Some(end);
                    let headers = String::from_utf8_lossy(&data[..end]);
                    for line in headers.lines() {
                        let lower = line.to_ascii_lowercase();
                        if let Some(rest) = lower.strip_prefix("content-length:") {
                            content_length = rest.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
            }
            if let Some(end) = header_end {
                let body_len = data.len().saturating_sub(end);
                if body_len >= content_length {
                    break;
                }
            }
        }
        data
    }

    #[test]
    fn transcribe_audio_sends_nexara_quickstart_form_fields() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let req = read_http_request(&mut stream);
            let req_str = String::from_utf8_lossy(&req).to_string();
            let body = r#"{"text":"ok"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
            req_str
        });

        let tmp_path =
            std::env::temp_dir().join(format!("bigecho_pipeline_{}.opus", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, b"fake-opus").expect("write temp opus");
        let settings = PublicSettings {
            recording_root: "./recordings".to_string(),
            artifact_open_app: String::new(),
            transcription_url: format!("http://{addr}/api/v1/audio/transcriptions"),
            transcription_task: "diarize".to_string(),
            transcription_diarization_setting: "meeting".to_string(),
            summary_url: "https://example.com/summary".to_string(),
            summary_prompt: String::new(),
            openai_model: "gpt-4.1-mini".to_string(),
            audio_format: "opus".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            artifact_opener_app: String::new(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        };

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(transcribe_audio(&settings, "nx-test-key", &tmp_path))
            .expect("transcribe ok");
        assert_eq!(out, "ok");

        let req_str = server.join().expect("join");
        let req_lower = req_str.to_ascii_lowercase();
        assert!(req_lower.contains("authorization: bearer nx-test-key"));
        assert!(req_str.contains("name=\"response_format\""));
        assert!(req_str.contains("\r\njson\r\n"));
        assert!(req_str.contains("name=\"model\""));
        assert!(req_str.contains("\r\nwhisper-1\r\n"));
        assert!(req_str.contains("name=\"task\""));
        assert!(req_str.contains("\r\ndiarize\r\n"));
        assert!(req_str.contains("name=\"diarization_setting\""));
        assert!(req_str.contains("\r\nmeeting\r\n"));
    }

    #[test]
    fn transcribe_audio_formats_diarize_segments_as_speaker_transcript() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _req = read_http_request(&mut stream);
            let body = r#"{
                "task":"diarize",
                "language":"ru",
                "duration":12.3,
                "text":"fallback text",
                "segments":[
                    {"start":0,"end":1.1,"speaker":"speaker_0","text":"Привет"},
                    {"start":1.2,"end":2.2,"speaker":"speaker_1","text":"И тебе привет"}
                ]
            }"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
        });

        let tmp_path =
            std::env::temp_dir().join(format!("bigecho_pipeline_{}.opus", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, b"fake-opus").expect("write temp opus");
        let settings = PublicSettings {
            recording_root: "./recordings".to_string(),
            artifact_open_app: String::new(),
            transcription_url: format!("http://{addr}/api/v1/audio/transcriptions"),
            transcription_task: "diarize".to_string(),
            transcription_diarization_setting: "meeting".to_string(),
            summary_url: "https://example.com/summary".to_string(),
            summary_prompt: String::new(),
            openai_model: "gpt-4.1-mini".to_string(),
            audio_format: "opus".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            artifact_opener_app: String::new(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        };

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(transcribe_audio(&settings, "nx-test-key", &tmp_path))
            .expect("transcribe ok");
        assert_eq!(out, "speaker_0: Привет\n\nspeaker_1: И тебе привет");

        server.join().expect("join");
    }

    #[test]
    fn summarize_text_sends_configured_system_prompt() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let req = read_http_request(&mut stream);
            let req_str = String::from_utf8_lossy(&req).to_string();
            let body = r#"{"choices":[{"message":{"content":"ok summary"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
            req_str
        });

        let settings = PublicSettings {
            recording_root: "./recordings".to_string(),
            artifact_open_app: String::new(),
            transcription_url: "https://example.com/transcribe".to_string(),
            transcription_task: "transcribe".to_string(),
            transcription_diarization_setting: "general".to_string(),
            summary_url: format!("http://{addr}/summary"),
            summary_prompt: "Сделай саммари: решения, риски, action items".to_string(),
            openai_model: "gpt-4.1-mini".to_string(),
            audio_format: "opus".to_string(),
            opus_bitrate_kbps: 24,
            mic_device_name: String::new(),
            system_device_name: String::new(),
            artifact_opener_app: String::new(),
            auto_run_pipeline_on_stop: false,
            api_call_logging_enabled: false,
        };

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let out = rt
            .block_on(summarize_text(
                &settings,
                "openai-test-key",
                "meeting transcript",
            ))
            .expect("summary ok");
        assert_eq!(out, "ok summary");

        let req_str = server.join().expect("join");
        let body = req_str
            .split("\r\n\r\n")
            .nth(1)
            .expect("http request body should exist");
        let payload: serde_json::Value = serde_json::from_str(body).expect("valid json payload");
        assert_eq!(
            payload["messages"][0]["content"].as_str(),
            Some("Сделай саммари: решения, риски, action items")
        );
    }
}
