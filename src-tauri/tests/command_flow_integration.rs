use bigecho::command_core::{
    mark_pipeline_audio_missing, should_schedule_retry, PipelineInvocation,
};
use bigecho::domain::session::{SessionMeta, SessionStatus};
use bigecho::storage::session_store::{load_meta, save_meta};
use bigecho::storage::sqlite_repo::{
    clear_retry_job, fetch_due_retry_jobs, list_sessions, schedule_retry_job, upsert_session,
};
use tempfile::tempdir;

fn prepare_session() -> (tempfile::TempDir, SessionMeta, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempdir().expect("temp dir");
    let app_data_dir = temp.path().join("app-data");
    std::fs::create_dir_all(&app_data_dir).expect("app-data dir");

    let session_dir = temp.path().join("sessions").join("s1");
    std::fs::create_dir_all(&session_dir).expect("session dir");
    let meta_path = session_dir.join("meta.json");

    let meta = SessionMeta::new(
        "session-1".to_string(),
        vec!["zoom".to_string()],
        "Weekly sync".to_string(),
        vec!["Alice".to_string()],
    );

    save_meta(&meta_path, &meta).expect("save meta");
    upsert_session(&app_data_dir, &meta, &session_dir, &meta_path).expect("upsert");
    (temp, meta, app_data_dir, meta_path)
}

#[test]
fn run_invocation_audio_missing_marks_failed_and_schedules_retry() {
    let (_temp, mut meta, app_data_dir, meta_path) = prepare_session();
    let session_dir = meta_path.parent().expect("parent").to_path_buf();

    let detail = mark_pipeline_audio_missing(&mut meta);
    save_meta(&meta_path, &meta).expect("save failed meta");
    upsert_session(&app_data_dir, &meta, &session_dir, &meta_path).expect("upsert failed meta");

    if should_schedule_retry(PipelineInvocation::Run) {
        schedule_retry_job(&app_data_dir, &meta.session_id, &detail, 4).expect("schedule retry");
    }

    let reloaded = load_meta(&meta_path).expect("load meta");
    assert_eq!(reloaded.status, SessionStatus::Failed);
    assert!(reloaded.errors.iter().any(|e| e == "Audio file is missing"));

    let listed = list_sessions(&app_data_dir).expect("list sessions");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, "failed");

    let due = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch due retry");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].session_id, meta.session_id);
}

#[test]
fn worker_retry_audio_missing_marks_failed_without_new_retry_job() {
    let (_temp, mut meta, app_data_dir, meta_path) = prepare_session();
    let session_dir = meta_path.parent().expect("parent").to_path_buf();

    let detail = mark_pipeline_audio_missing(&mut meta);
    save_meta(&meta_path, &meta).expect("save failed meta");
    upsert_session(&app_data_dir, &meta, &session_dir, &meta_path).expect("upsert failed meta");

    if should_schedule_retry(PipelineInvocation::WorkerRetry) {
        schedule_retry_job(&app_data_dir, &meta.session_id, &detail, 4).expect("schedule retry");
    }

    let due = fetch_due_retry_jobs(&app_data_dir, i64::MAX, 10).expect("fetch due retry");
    assert!(due.is_empty());

    clear_retry_job(&app_data_dir, &meta.session_id).expect("clear retry job should be no-op");
}
