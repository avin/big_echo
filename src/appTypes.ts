export type PublicSettings = {
  recording_root: string;
  artifact_open_app: string;
  transcription_url: string;
  transcription_task: string;
  transcription_diarization_setting: string;
  summary_url: string;
  summary_prompt: string;
  openai_model: string;
  opus_bitrate_kbps: number;
  mic_device_name: string;
  system_device_name: string;
  auto_run_pipeline_on_stop: boolean;
  api_call_logging_enabled: boolean;
};

export type StartResponse = {
  session_id: string;
  session_dir: string;
  status: string;
};

export type SessionListItem = {
  session_id: string;
  status: string;
  primary_tag: string;
  topic: string;
  display_date_ru: string;
  started_at_iso: string;
  session_dir: string;
  audio_duration_hms: string;
  has_transcript_text: boolean;
  has_summary_text: boolean;
};

export type SessionMetaView = {
  session_id: string;
  source: string;
  custom_tag: string;
  topic: string;
  participants: string[];
};

export type UiSyncStateView = {
  source: string;
  topic: string;
  is_recording: boolean;
  active_session_id: string | null;
};

export type SecretSaveState = "unknown" | "updated" | "unchanged" | "error";
export type PipelineUiState = { kind: "success" | "error"; text: string };
export type LiveInputLevels = { mic: number; system: number };
export type SettingsTab = "audiototext" | "generals" | "audio";
export type DeleteTarget = { sessionId: string; force: boolean };

export type TextEditorAppOption = {
  id: string;
  name: string;
  icon_fallback: string;
  icon_data_url: string | null;
};

export type TextEditorAppsResponse = {
  apps: TextEditorAppOption[];
  default_app_id: string | null;
};

export const fixedSources = ["slack", "zoom", "telemost", "telegram", "browser", "facetime"];
export const transcriptionTaskOptions = ["transcribe", "diarize"];
export const diarizationSettingOptions = ["general", "meeting", "telephonic"];
