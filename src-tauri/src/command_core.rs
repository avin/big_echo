use crate::domain::session::{SessionMeta, SessionStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineInvocation {
    Run,
    Retry,
    WorkerRetry,
    Manual,
}

pub fn validate_start_request(topic: &str, participants: &[String]) -> Result<(), String> {
    if topic.chars().count() > 200 {
        return Err("Topic is too long (max 200 chars)".to_string());
    }
    if participants.len() > 32 {
        return Err("Too many participants (max 32)".to_string());
    }
    Ok(())
}

pub fn ensure_stop_session_matches(
    active_session_id: &str,
    requested_session_id: Option<&str>,
) -> Result<(), String> {
    if let Some(requested) = requested_session_id {
        if requested != active_session_id {
            return Err("Session id mismatch".to_string());
        }
    }
    Ok(())
}

pub fn should_schedule_retry(invocation: PipelineInvocation) -> bool {
    !matches!(invocation, PipelineInvocation::WorkerRetry | PipelineInvocation::Manual)
}

pub fn mark_pipeline_audio_missing(meta: &mut SessionMeta) -> String {
    meta.status = SessionStatus::Failed;
    let msg = "Audio file is missing".to_string();
    meta.errors.push(msg.clone());
    msg
}

pub fn mark_pipeline_transcription_failed(meta: &mut SessionMeta, err: &str) -> String {
    meta.status = SessionStatus::Failed;
    let msg = format!("Transcription error: {err}");
    meta.errors.push(msg.clone());
    msg
}

pub fn mark_pipeline_summary_failed(meta: &mut SessionMeta, err: &str) -> String {
    meta.status = SessionStatus::Failed;
    let msg = format!("Summary error: {err}");
    meta.errors.push(msg.clone());
    msg
}

pub fn mark_pipeline_transcribed(meta: &mut SessionMeta) {
    meta.status = SessionStatus::Transcribed;
}

pub fn mark_pipeline_done(meta: &mut SessionMeta) {
    meta.status = SessionStatus::Done;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta() -> SessionMeta {
        SessionMeta::new(
            "session-1".to_string(),
            vec!["zoom".to_string()],
            "Weekly sync".to_string(),
            vec!["Alice".to_string()],
        )
    }

    #[test]
    fn start_validation_allows_empty_topic() {
        let participants = vec!["Alice".to_string()];
        let result = validate_start_request("", &participants);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn start_validation_allows_empty_participants() {
        let participants: Vec<String> = Vec::new();
        let result = validate_start_request("Planning", &participants);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn start_validation_rejects_too_long_topic() {
        let topic = "x".repeat(201);
        let result = validate_start_request(&topic, &[]);
        assert_eq!(result, Err("Topic is too long (max 200 chars)".to_string()));
    }

    #[test]
    fn start_validation_rejects_too_many_participants() {
        let participants: Vec<String> = (0..33).map(|i| format!("P{i}")).collect();
        let result = validate_start_request("Planning", &participants);
        assert_eq!(result, Err("Too many participants (max 32)".to_string()));
    }

    #[test]
    fn stop_validation_rejects_session_mismatch() {
        let result = ensure_stop_session_matches("session-1", Some("session-2"));
        assert_eq!(result, Err("Session id mismatch".to_string()));
    }

    #[test]
    fn worker_retry_does_not_schedule_followup_retry() {
        assert!(!should_schedule_retry(PipelineInvocation::WorkerRetry));
    }

    #[test]
    fn run_and_retry_schedule_followup_retry() {
        assert!(should_schedule_retry(PipelineInvocation::Run));
        assert!(should_schedule_retry(PipelineInvocation::Retry));
    }

    #[test]
    fn audio_missing_marks_failed_and_records_error() {
        let mut meta = sample_meta();
        let err = mark_pipeline_audio_missing(&mut meta);
        assert_eq!(err, "Audio file is missing");
        assert_eq!(meta.status, SessionStatus::Failed);
        assert!(meta.errors.iter().any(|e| e == "Audio file is missing"));
    }

    #[test]
    fn transcription_failure_adds_detailed_error() {
        let mut meta = sample_meta();
        let detail = mark_pipeline_transcription_failed(&mut meta, "timeout");
        assert_eq!(detail, "Transcription error: timeout");
        assert_eq!(meta.status, SessionStatus::Failed);
        assert!(meta.errors.iter().any(|e| e == "Transcription error: timeout"));
    }

    #[test]
    fn summary_failure_adds_detailed_error() {
        let mut meta = sample_meta();
        let detail = mark_pipeline_summary_failed(&mut meta, "upstream 500");
        assert_eq!(detail, "Summary error: upstream 500");
        assert_eq!(meta.status, SessionStatus::Failed);
        assert!(meta.errors.iter().any(|e| e == "Summary error: upstream 500"));
    }
}
