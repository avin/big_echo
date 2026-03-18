import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState } from "react";
import { formatAppStatus, formatSessionStatus } from "./status";

type PublicSettings = {
  recording_root: string;
  transcription_url: string;
  transcription_task: string;
  transcription_diarization_setting: string;
  summary_url: string;
  summary_prompt: string;
  openai_model: string;
  opus_bitrate_kbps: number;
  mic_device_name: string;
  system_device_name: string;
  artifact_opener_app: string;
  auto_run_pipeline_on_stop: boolean;
  api_call_logging_enabled: boolean;
};

type TextEditorApp = {
  id: string;
  name: string;
  icon_fallback: string;
  icon_data_url: string | null;
};

type TextEditorAppsResponse = {
  apps: TextEditorApp[];
  default_app_id: string | null;
};

type StartResponse = {
  session_id: string;
  session_dir: string;
  status: string;
};

type SessionListItem = {
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

type SessionMetaView = {
  session_id: string;
  source: string;
  custom_tag: string;
  topic: string;
  participants: string[];
};

type UiSyncStateView = {
  source: string;
  topic: string;
  is_recording: boolean;
  active_session_id: string | null;
};

type SecretSaveState = "unknown" | "updated" | "unchanged" | "error";
type PipelineUiState = { kind: "success" | "error"; text: string };
type LiveInputLevels = { mic: number; system: number };
type SettingsTab = "audiototext" | "generals" | "audio";
type DeleteTarget = { sessionId: string; force: boolean };

const fixedSources = ["slack", "zoom", "telemost", "telegram", "browser", "facetime"];
const transcriptionTaskOptions = ["transcribe", "diarize"];
const diarizationSettingOptions = ["general", "meeting", "telephonic"];
const currentWindow = getCurrentWindow();
const isSettingsWindow = currentWindow.label === "settings";
const isTrayWindow = currentWindow.label === "tray";

function isValidUrl(value: string): boolean {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

function validateSettings(settings: PublicSettings | null): string[] {
  if (!settings) return [];
  const errors: string[] = [];
  if (settings.transcription_url.trim() && !isValidUrl(settings.transcription_url.trim())) {
    errors.push("Неверный URL транскрибации");
  }
  if (settings.summary_url.trim() && !isValidUrl(settings.summary_url.trim())) {
    errors.push("Неверный URL саммари");
  }
  if (settings.opus_bitrate_kbps < 12 || settings.opus_bitrate_kbps > 128) {
    errors.push("Битрейт Opus должен быть от 12 до 128 kbps");
  }
  return errors;
}

function formatSecretSaveState(state: SecretSaveState): string {
  if (state === "updated") return "обновлён";
  if (state === "unchanged") return "не изменён";
  if (state === "error") return "ошибка";
  return "";
}

function splitParticipants(value: string): string[] {
  return value
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);
}

function parseEventPayload<T>(event: unknown): T | null {
  if (!event || typeof event !== "object") return null;
  const candidate = event as { payload?: unknown };
  const payload = candidate.payload ?? event;
  if (typeof payload === "string") {
    try {
      return JSON.parse(payload) as T;
    } catch {
      return null;
    }
  }
  if (payload && typeof payload === "object") {
    return payload as T;
  }
  return null;
}

function getErrorMessage(value: unknown): string {
  if (value instanceof Error) return value.message;
  return String(value);
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

export function App() {
  const [topic, setTopic] = useState("");
  const [participants, setParticipants] = useState("");
  const [source, setSource] = useState("slack");
  const [customTag, setCustomTag] = useState("");
  const [session, setSession] = useState<StartResponse | null>(null);
  const [lastSessionId, setLastSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState("idle");
  const [settings, setSettings] = useState<PublicSettings | null>(null);
  const [savedSettingsSnapshot, setSavedSettingsSnapshot] = useState<PublicSettings | null>(null);
  const [nexaraKey, setNexaraKey] = useState("");
  const [openaiKey, setOpenaiKey] = useState("");
  const [nexaraSecretState, setNexaraSecretState] = useState<SecretSaveState>("unknown");
  const [openaiSecretState, setOpenaiSecretState] = useState<SecretSaveState>("unknown");
  const [sessions, setSessions] = useState<SessionListItem[]>([]);
  const [sessionDetails, setSessionDetails] = useState<Record<string, SessionMetaView>>({});
  const [savedSessionDetails, setSavedSessionDetails] = useState<Record<string, SessionMetaView>>({});
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  const [textEditorApps, setTextEditorApps] = useState<TextEditorApp[]>([]);
  const [isOpenerDropdownOpen, setIsOpenerDropdownOpen] = useState(false);
  const [sessionSearchQuery, setSessionSearchQuery] = useState("");
  const [textPendingBySession, setTextPendingBySession] = useState<Record<string, boolean>>({});
  const [summaryPendingBySession, setSummaryPendingBySession] = useState<Record<string, boolean>>({});
  const [pipelineStateBySession, setPipelineStateBySession] = useState<Record<string, PipelineUiState>>({});
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [deletePendingSessionId, setDeletePendingSessionId] = useState<string | null>(null);
  const [liveLevels, setLiveLevels] = useState<LiveInputLevels>({ mic: 0, system: 0 });
  const [uiSyncReady, setUiSyncReady] = useState(isSettingsWindow);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("audiototext");
  const topicRef = useRef(topic);
  const sourceRef = useRef(source);
  const sessionRef = useRef<StartResponse | null>(session);
  const autosaveTimersRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const trayTopicAutosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const trayTopicSavedSignatureRef = useRef<string>("");
  const openerDropdownRef = useRef<HTMLDivElement | null>(null);

  const settingsErrors = useMemo(() => validateSettings(settings), [settings]);
  const canSaveSettings = Boolean(settings) && settingsErrors.length === 0;

  useEffect(() => {
    topicRef.current = topic;
    sourceRef.current = source;
    sessionRef.current = session;
  }, [topic, source, session]);

  useEffect(() => {
    if (!isTrayWindow) return;
    document.body.classList.add("tray-window-body");
    void loadSettings().catch(() => undefined);
    void loadAudioDevices().catch(() => undefined);
    document.documentElement.classList.add("tray-window-html");
    return () => {
      document.body.classList.remove("tray-window-body");
      document.documentElement.classList.remove("tray-window-html");
    };
  }, []);

  useEffect(() => {
    const onDocumentMouseDown = (event: MouseEvent) => {
      if (!isOpenerDropdownOpen) return;
      if (!openerDropdownRef.current) return;
      if (openerDropdownRef.current.contains(event.target as Node)) return;
      setIsOpenerDropdownOpen(false);
    };
    document.addEventListener("mousedown", onDocumentMouseDown);
    return () => document.removeEventListener("mousedown", onDocumentMouseDown);
  }, [isOpenerDropdownOpen]);

  useEffect(() => {
    if (isTrayWindow) return;
    void loadSettings().catch(() => undefined);
  }, []);

  useEffect(() => {
    if (isTrayWindow) return;
    if (settingsTab !== "audio") return;
    if (audioDevices.length > 0) return;
    void loadAudioDevices().catch(() => undefined);
  }, [settingsTab, audioDevices.length]);

  useEffect(() => {
    if (!isTrayWindow) return;
    let active = true;
    let inFlight = false;
    const tick = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const levels = await invoke<LiveInputLevels>("get_live_input_levels");
        if (!active) return;
        setLiveLevels({
          mic: clamp01(levels.mic),
          system: clamp01(levels.system),
        });
      } catch {
        if (!active) return;
      } finally {
        inFlight = false;
      }
    };
    void tick();
    const intervalMs = status === "recording" ? 90 : 160;
    const timer = setInterval(() => {
      void tick();
    }, intervalMs);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [status]);

  useEffect(() => {
    if (isSettingsWindow) return;
    let active = true;
    invoke<UiSyncStateView>("get_ui_sync_state")
      .then((current) => {
        if (!active) return;
        if (current.source?.trim()) setSource(current.source.trim());
        setTopic(current.topic ?? "");
        if (current.is_recording) {
          setStatus("recording");
          if (current.active_session_id) {
            setSession({
              session_id: current.active_session_id,
              session_dir: "",
              status: "recording",
            });
            setLastSessionId(current.active_session_id);
          }
        }
      })
      .catch(() => undefined)
      .finally(() => {
        if (active) setUiSyncReady(true);
      });
    return () => {
      active = false;
    };
  }, []);

  async function loadSettings() {
    const [data, availableEditorApps] = await Promise.all([
      invoke<PublicSettings>("get_settings"),
      invoke<TextEditorAppsResponse>("list_text_editor_apps").catch(() => ({ apps: [], default_app_id: null })),
    ]);
    const normalized: PublicSettings = {
      ...data,
      artifact_opener_app:
        (data as Partial<PublicSettings>).artifact_opener_app?.trim() || availableEditorApps.default_app_id || "",
    };
    setTextEditorApps(availableEditorApps.apps ?? []);
    setSettings(normalized);
    setSavedSettingsSnapshot(normalized);
  }

  async function loadAudioDevices() {
    const list = await invoke<string[]>("list_audio_input_devices");
    setAudioDevices(list);
  }

  async function autoDetectSystemSource() {
    const detected = await invoke<string | null>("detect_system_source_device");
    if (!detected) {
      setStatus("system_source_not_detected");
      return;
    }
    setSettings((prev) => (prev ? { ...prev, system_device_name: detected } : prev));
    setStatus(`system_source_detected:${detected}`);
  }

  async function saveSettings() {
    if (!settings) return;
    if (settingsErrors.length === 0) {
      await invoke("save_public_settings", { payload: settings });
      setSavedSettingsSnapshot(settings);
    }
    await saveApiKeys();
    setStatus(settingsErrors.length > 0 ? "error: исправьте настройки перед сохранением" : "settings_saved");
  }

  async function saveApiKeys() {
    let hasSecretError = false;
    let nexaraState: SecretSaveState = "unchanged";
    let openaiState: SecretSaveState = "unchanged";

    if (nexaraKey.trim()) {
      try {
        await invoke("set_api_secret", { name: "NEXARA_API_KEY", value: nexaraKey.trim() });
        nexaraState = "updated";
      } catch {
        nexaraState = "error";
        hasSecretError = true;
      }
    }

    if (openaiKey.trim()) {
      try {
        await invoke("set_api_secret", { name: "OPENAI_API_KEY", value: openaiKey.trim() });
        openaiState = "updated";
      } catch {
        openaiState = "error";
        hasSecretError = true;
      }
    }

    setNexaraSecretState(nexaraState);
    setOpenaiSecretState(openaiState);
    if (hasSecretError) {
      setStatus("error: не удалось сохранить один или несколько ключей");
    } else {
      if (nexaraState === "updated") setNexaraKey("");
      if (openaiState === "updated") setOpenaiKey("");
      setStatus("keys_saved");
    }
  }

  async function saveSettingsPatch(patch: Partial<PublicSettings>) {
    const base = settings ?? (await invoke<PublicSettings>("get_settings"));
    const next = { ...base, ...patch };
    setSettings(next);
    await invoke("save_public_settings", { payload: next });
  }

  async function startRecording(payload: { source: string; customTag?: string; topic?: string; participants?: string[] }) {
    const tags = [payload.source];
    if (payload.customTag && payload.customTag.trim()) tags.push(payload.customTag.trim());
    const res = await invoke<StartResponse>("start_recording", {
      payload: {
        tags,
        topic: payload.topic ?? "",
        participants: payload.participants ?? [],
      },
    });
    setSession(res);
    setLastSessionId(res.session_id);
    setStatus("recording");
    await loadSessions();
  }

  async function start() {
    await startRecording({
      source,
      customTag,
      topic,
      participants: splitParticipants(participants),
    });
  }

  async function startFromTray() {
    await startRecording({
      source,
      topic,
      participants: [],
    });
  }

  async function stop() {
    if (!session) return;
    await invoke<string>("stop_recording", { sessionId: session.session_id });
    setStatus("recorded");
    setSession(null);
    await loadSessions();
  }

  async function runPipeline() {
    if (!lastSessionId) return;
    await invoke<string>("run_pipeline", { sessionId: lastSessionId });
    setStatus("done");
    await loadSessions();
  }

  async function loadSessions() {
    const data = await invoke<SessionListItem[]>("list_sessions");
    setSessions(data);
    const details = await Promise.all(
      data.map(async (item) => {
        try {
          const meta = await invoke<SessionMetaView>("get_session_meta", { sessionId: item.session_id });
          return [item.session_id, meta] as const;
        } catch {
          return [
            item.session_id,
            {
              session_id: item.session_id,
              source: item.primary_tag,
              custom_tag: "",
              topic: item.topic,
              participants: [],
            },
          ] as const;
        }
      })
    );
    const map = Object.fromEntries(details);
    setSessionDetails(map);
    setSavedSessionDetails(map);
  }

  async function getText(sessionId: string) {
    setTextPendingBySession((prev) => ({ ...prev, [sessionId]: true }));
    setPipelineStateBySession((prev) => {
      const next = { ...prev };
      delete next[sessionId];
      return next;
    });
    try {
      await invoke<string>("run_transcription", { sessionId });
      setPipelineStateBySession((prev) => ({
        ...prev,
        [sessionId]: { kind: "success", text: "Text fetched successfully" },
      }));
      setStatus("transcribed");
      await loadSessions();
    } catch (err) {
      const message = getErrorMessage(err);
      setPipelineStateBySession((prev) => ({
        ...prev,
        [sessionId]: { kind: "error", text: `Get text failed: ${message}` },
      }));
      setStatus(`error: ${message}`);
    } finally {
      setTextPendingBySession((prev) => ({ ...prev, [sessionId]: false }));
    }
  }

  async function getSummary(sessionId: string) {
    setSummaryPendingBySession((prev) => ({ ...prev, [sessionId]: true }));
    setPipelineStateBySession((prev) => {
      const next = { ...prev };
      delete next[sessionId];
      return next;
    });
    try {
      await invoke<string>("run_summary", { sessionId });
      setPipelineStateBySession((prev) => ({
        ...prev,
        [sessionId]: { kind: "success", text: "Summary fetched successfully" },
      }));
      setStatus("done");
      await loadSessions();
    } catch (err) {
      const message = getErrorMessage(err);
      setPipelineStateBySession((prev) => ({
        ...prev,
        [sessionId]: { kind: "error", text: `Get summary failed: ${message}` },
      }));
      setStatus(`error: ${message}`);
    } finally {
      setSummaryPendingBySession((prev) => ({ ...prev, [sessionId]: false }));
    }
  }

  async function openSessionFolder(sessionDir: string) {
    await invoke<string>("open_session_folder", { sessionDir });
  }

  function requestDeleteSession(sessionId: string, force: boolean) {
    setDeleteTarget({ sessionId, force });
  }

  async function confirmDeleteSession() {
    if (!deleteTarget) return;
    const sessionId = deleteTarget.sessionId;
    setDeletePendingSessionId(sessionId);
    try {
      await invoke<string>("delete_session", { sessionId, force: deleteTarget.force });
      setSessions((prev) => prev.filter((item) => item.session_id !== sessionId));
      setSessionDetails((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
      setSavedSessionDetails((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
      setTextPendingBySession((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
      setSummaryPendingBySession((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
      setPipelineStateBySession((prev) => {
        const next = { ...prev };
        delete next[sessionId];
        return next;
      });
      if (lastSessionId === sessionId) {
        setLastSessionId(null);
      }
      setDeleteTarget(null);
      setStatus("session_deleted");
    } catch (err) {
      setStatus(`error: ${getErrorMessage(err)}`);
    } finally {
      setDeletePendingSessionId(null);
    }
  }

  useEffect(() => {
    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenUiSync: (() => void) | undefined;
    let unlistenUiRecording: (() => void) | undefined;

    listen("tray:start", async () => {
      try {
        await startRecording({
          source: sourceRef.current,
          topic: topicRef.current,
          participants: [],
        });
      } catch (err) {
        setStatus(`error: ${String(err)}`);
      }
    }).then((fn) => {
      unlistenStart = fn;
    });

    listen("tray:stop", async () => {
      try {
        if (!sessionRef.current) return;
        await invoke<string>("stop_recording", { sessionId: sessionRef.current.session_id });
        setStatus("recorded");
        setSession(null);
        await loadSessions();
      } catch (err) {
        setStatus(`error: ${String(err)}`);
      }
    }).then((fn) => {
      unlistenStop = fn;
    });

    listen("ui:sync", (event) => {
      const payload = parseEventPayload<{ source?: string; topic?: string }>(event);
      if (!payload) return;
      if (typeof payload.source === "string" && payload.source.trim() && payload.source !== sourceRef.current) {
        setSource(payload.source);
      }
      if (typeof payload.topic === "string" && payload.topic !== topicRef.current) {
        setTopic(payload.topic);
      }
    }).then((fn) => {
      unlistenUiSync = fn;
    });

    listen("ui:recording", (event) => {
      const payload = parseEventPayload<{ recording?: boolean; sessionId?: string | null }>(event);
      if (!payload || typeof payload.recording !== "boolean") return;
      if (payload.recording) {
        setStatus("recording");
        if (payload.sessionId) {
          setSession((prev) => prev ?? { session_id: payload.sessionId!, session_dir: "", status: "recording" });
          setLastSessionId(payload.sessionId);
        }
      } else {
        setSession(null);
        setStatus((prev) => (prev === "recording" ? "recorded" : prev));
      }
    }).then((fn) => {
      unlistenUiRecording = fn;
    });

    return () => {
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
      if (unlistenUiSync) unlistenUiSync();
      if (unlistenUiRecording) unlistenUiRecording();
    };
  }, []);

  useEffect(() => {
    if (isSettingsWindow) return;
    const recording = status === "recording";
    emit("recording:status", { recording }).catch(() => undefined);
    emit("ui:recording", { recording, sessionId: recording ? (session?.session_id ?? null) : null }).catch(() => undefined);
  }, [status, session]);

  useEffect(() => {
    if (isSettingsWindow || !uiSyncReady) return;
    invoke("set_ui_sync_state", { source, topic }).catch(() => undefined);
    emit("ui:sync", { source, topic }).catch(() => undefined);
  }, [source, topic, uiSyncReady]);

  useEffect(() => {
    if (!isTrayWindow) return;
    if (status !== "recording" || !session?.session_id) return;
    const signature = `${session.session_id}::${source}::${topic}`;
    if (signature === trayTopicSavedSignatureRef.current) return;
    if (trayTopicAutosaveTimerRef.current) clearTimeout(trayTopicAutosaveTimerRef.current);

    trayTopicAutosaveTimerRef.current = setTimeout(async () => {
      try {
        await invoke<string>("update_session_details", {
          payload: {
            session_id: session.session_id,
            source,
            custom_tag: "",
            topic: topic.trim(),
            participants: [],
          },
        });
        trayTopicSavedSignatureRef.current = signature;
      } catch {
        // Keep recorder responsive even if metadata update fails.
      }
    }, 450);

    return () => {
      if (trayTopicAutosaveTimerRef.current) clearTimeout(trayTopicAutosaveTimerRef.current);
    };
  }, [status, session?.session_id, source, topic]);

  useEffect(() => {
    const ids = Object.keys(sessionDetails);
    for (const sessionId of ids) {
      const current = sessionDetails[sessionId];
      const saved = savedSessionDetails[sessionId];
      if (!saved) continue;

      const currentSig = JSON.stringify(current);
      const savedSig = JSON.stringify(saved);
      if (currentSig === savedSig) continue;

      const existing = autosaveTimersRef.current[sessionId];
      if (existing) clearTimeout(existing);

      autosaveTimersRef.current[sessionId] = setTimeout(async () => {
        try {
          await invoke<string>("update_session_details", {
            payload: {
              session_id: sessionId,
              source: current.source,
              custom_tag: current.custom_tag,
              topic: current.topic,
              participants: current.participants,
            },
          });
          setSavedSessionDetails((prev) => ({ ...prev, [sessionId]: current }));
          setStatus("session_details_autosaved");
        } catch (err) {
          setStatus(`error: ${String(err)}`);
        }
      }, 700);
    }

    return () => {
      for (const timer of Object.values(autosaveTimersRef.current)) {
        clearTimeout(timer);
      }
    };
  }, [sessionDetails, savedSessionDetails]);

  const filteredSessions = useMemo(() => {
    const query = sessionSearchQuery.trim().toLowerCase();
    return sessions.filter((item) => {
      const detail = sessionDetails[item.session_id];
      const sourceValue = (detail?.source ?? item.primary_tag).toLowerCase();
      const customValue = (detail?.custom_tag ?? "").toLowerCase();
      const topicValue = (detail?.topic ?? item.topic ?? "").toLowerCase();
      const participantsValue = (detail?.participants ?? []).join(", ").toLowerCase();
      const pathValue = item.session_dir.toLowerCase();
      const statusValue = item.status.toLowerCase();
      const dateValue = item.display_date_ru.toLowerCase();
      if (!query) return true;
      return (
        sourceValue.includes(query) ||
        customValue.includes(query) ||
        topicValue.includes(query) ||
        participantsValue.includes(query) ||
        pathValue.includes(query) ||
        statusValue.includes(query) ||
        dateValue.includes(query)
      );
    });
  }, [sessions, sessionDetails, sessionSearchQuery]);

  function renderSettingsFields() {
    if (!settings) return null;
    const snapshot = savedSettingsSnapshot;
    const selectedOpenerApp = textEditorApps.find((app) => app.id === settings.artifact_opener_app) ?? null;
    const isDirty = (field: keyof PublicSettings) => Boolean(snapshot && settings[field] !== snapshot[field]);
    const dirtyByTab: Record<SettingsTab, boolean> = {
      audiototext:
        isDirty("transcription_url") ||
        isDirty("transcription_task") ||
        isDirty("transcription_diarization_setting") ||
        isDirty("summary_url") ||
        isDirty("summary_prompt") ||
        isDirty("openai_model") ||
        nexaraKey.trim().length > 0 ||
        openaiKey.trim().length > 0,
      generals:
        isDirty("recording_root") ||
        isDirty("artifact_opener_app") ||
        isDirty("auto_run_pipeline_on_stop") ||
        isDirty("api_call_logging_enabled"),
      audio: isDirty("opus_bitrate_kbps") || isDirty("mic_device_name") || isDirty("system_device_name"),
    };
    const tabButtons: Array<{ id: SettingsTab; label: string }> = [
      { id: "generals", label: "Generals" },
      { id: "audiototext", label: "AudioToText" },
      { id: "audio", label: "Audio" },
    ];

    return (
      <div className="settings-tabs">
        <div className="settings-tab-list" role="tablist" aria-label="Settings sections">
          {tabButtons.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              className={`settings-tab-button${settingsTab === tab.id ? " is-active" : ""}`}
              aria-selected={settingsTab === tab.id}
              onClick={() => setSettingsTab(tab.id)}
            >
              {tab.label}
              {dirtyByTab[tab.id] && <span className="settings-tab-dirty-dot" aria-hidden="true" />}
            </button>
          ))}
        </div>

        <div className="settings-tab-panel" role="tabpanel">
          {settingsTab === "audiototext" && (
            <div className="settings-subsections">
              <section className="settings-subsection">
                <h3>Транскрибация</h3>
                <div className="settings-tab-grid">
                  <label className="field">
                    Transcription URL
                    <input
                      value={settings.transcription_url}
                      onChange={(e) => setSettings({ ...settings, transcription_url: e.target.value })}
                    />
                  </label>
                  <label className="field">
                    Task
                    <select
                      value={settings.transcription_task}
                      onChange={(e) => setSettings({ ...settings, transcription_task: e.target.value })}
                    >
                      {transcriptionTaskOptions.map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="field">
                    Diarization setting
                    <select
                      value={settings.transcription_diarization_setting}
                      onChange={(e) =>
                        setSettings({ ...settings, transcription_diarization_setting: e.target.value })
                      }
                    >
                      {diarizationSettingOptions.map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="field">
                    Nexara API key
                    <input
                      type="password"
                      value={nexaraKey}
                      onChange={(e) => {
                        setNexaraKey(e.target.value);
                        setNexaraSecretState("unknown");
                      }}
                      placeholder="Stored in OS secure storage"
                    />
                  </label>
                </div>
              </section>

              <section className="settings-subsection">
                <h3>Саммари</h3>
                <div className="settings-tab-grid">
                  <label className="field">
                    Summary URL
                    <input
                      value={settings.summary_url}
                      onChange={(e) => setSettings({ ...settings, summary_url: e.target.value })}
                    />
                  </label>
                  <label className="field">
                    Summary prompt
                    <textarea
                      value={settings.summary_prompt}
                      onChange={(e) => setSettings({ ...settings, summary_prompt: e.target.value })}
                      rows={4}
                    />
                  </label>
                  <label className="field">
                    OpenAI model
                    <input
                      value={settings.openai_model}
                      onChange={(e) => setSettings({ ...settings, openai_model: e.target.value })}
                    />
                  </label>
                  <label className="field">
                    OpenAI API key
                    <input
                      type="password"
                      value={openaiKey}
                      onChange={(e) => {
                        setOpenaiKey(e.target.value);
                        setOpenaiSecretState("unknown");
                      }}
                      placeholder="Stored in OS secure storage"
                    />
                  </label>
                </div>
              </section>
            </div>
          )}

          {settingsTab === "generals" && (
            <div className="settings-tab-grid">
              <label className="field">
                Recording root
                <input
                  value={settings.recording_root}
                  onChange={(e) => setSettings({ ...settings, recording_root: e.target.value })}
                />
              </label>
              <div className="field">
                <span>Artifact opener app (optional)</span>
                <div className="opener-dropdown" ref={openerDropdownRef}>
                  <button
                    type="button"
                    className="opener-dropdown-trigger"
                    aria-label="Artifact opener app (optional)"
                    aria-haspopup="listbox"
                    aria-expanded={isOpenerDropdownOpen}
                    onClick={() => setIsOpenerDropdownOpen((prev) => !prev)}
                  >
                    {selectedOpenerApp ? (
                      <>
                        {selectedOpenerApp.icon_data_url ? (
                          <img
                            className="opener-app-icon"
                            src={selectedOpenerApp.icon_data_url}
                            alt=""
                            aria-hidden="true"
                          />
                        ) : (
                          <span className="opener-app-fallback-icon" aria-hidden="true">
                            {selectedOpenerApp.icon_fallback}
                          </span>
                        )}
                        <span>{selectedOpenerApp.name}</span>
                      </>
                    ) : (
                      <span>System default</span>
                    )}
                  </button>

                  {isOpenerDropdownOpen && (
                    <div className="opener-dropdown-menu" role="listbox" aria-label="Artifact opener app options">
                      <button
                        type="button"
                        className={`opener-dropdown-option${settings.artifact_opener_app === "" ? " is-active" : ""}`}
                        onClick={() => {
                          setSettings({ ...settings, artifact_opener_app: "" });
                          setIsOpenerDropdownOpen(false);
                        }}
                      >
                        <span>System default</span>
                      </button>
                      {textEditorApps.map((app) => (
                        <button
                          key={app.id}
                          type="button"
                          className={`opener-dropdown-option${
                            settings.artifact_opener_app === app.id ? " is-active" : ""
                          }`}
                          onClick={() => {
                            setSettings({ ...settings, artifact_opener_app: app.id });
                            setIsOpenerDropdownOpen(false);
                          }}
                        >
                          {app.icon_data_url ? (
                            <img className="opener-app-icon" src={app.icon_data_url} alt="" aria-hidden="true" />
                          ) : (
                            <span className="opener-app-fallback-icon" aria-hidden="true">
                              {app.icon_fallback}
                            </span>
                          )}
                          <span>{app.name}</span>
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              </div>
              <label className="field">
                <span>Auto-run pipeline on Stop</span>
                <input
                  type="checkbox"
                  checked={Boolean(settings.auto_run_pipeline_on_stop)}
                  onChange={(e) => setSettings({ ...settings, auto_run_pipeline_on_stop: e.target.checked })}
                />
              </label>
              <label className="field">
                <span>Enable API call logging</span>
                <input
                  type="checkbox"
                  checked={Boolean(settings.api_call_logging_enabled)}
                  onChange={(e) => setSettings({ ...settings, api_call_logging_enabled: e.target.checked })}
                />
              </label>
            </div>
          )}

          {settingsTab === "audio" && (
            <div className="settings-tab-grid">
              <label className="field">
                Opus bitrate kbps
                <input
                  type="number"
                  value={settings.opus_bitrate_kbps}
                  onChange={(e) => setSettings({ ...settings, opus_bitrate_kbps: Number(e.target.value) || 24 })}
                />
              </label>
              <label className="field">
                Mic device name
                <input
                  value={settings.mic_device_name}
                  onChange={(e) => setSettings({ ...settings, mic_device_name: e.target.value })}
                />
              </label>
              <label className="field">
                System source device name
                <input
                  value={settings.system_device_name}
                  onChange={(e) => setSettings({ ...settings, system_device_name: e.target.value })}
                />
              </label>
              <div className="button-row">
                <button className="secondary-button" onClick={autoDetectSystemSource}>
                  Auto-detect system source
                </button>
              </div>
              {audioDevices.length > 0 && (
                <div className="device-card">
                  <strong>Available input devices</strong>
                  <div className="device-list">
                    {audioDevices.map((dev) => (
                      <button
                        key={dev}
                        type="button"
                        className="secondary-button"
                        onClick={() =>
                          setSettings((prev) =>
                            prev
                              ? {
                                  ...prev,
                                  mic_device_name: prev.mic_device_name || dev,
                                  system_device_name: prev.system_device_name || dev,
                                }
                              : prev
                          )
                        }
                      >
                        {dev}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        {nexaraSecretState !== "unknown" && <div>Nexara API key: {formatSecretSaveState(nexaraSecretState)}</div>}
        {openaiSecretState !== "unknown" && <div>OpenAI API key: {formatSecretSaveState(openaiSecretState)}</div>}
        {settingsErrors.length > 0 && (
          <div className="error-list">
            {settingsErrors.map((error) => (
              <div key={error}>{error}</div>
            ))}
          </div>
        )}
        <div className="settings-actions">
          <button className="primary-button" onClick={saveSettings} disabled={!canSaveSettings}>
            Save settings
          </button>
          <button className="secondary-button" onClick={saveApiKeys}>
            Save API keys
          </button>
        </div>
      </div>
    );
  }

  if (isTrayWindow) {
    const micPct = Math.round(liveLevels.mic * 100);
    const systemPct = Math.round(liveLevels.system * 100);
    return (
      <main className="tray-shell">
        <p className="status-line">Status: {formatAppStatus(status)}</p>
        <div className="tray-meta-grid">
          <label className="field">
            Source
            <select value={source} onChange={(e) => setSource(e.target.value)}>
              {fixedSources.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            Topic (optional)
            <input value={topic} onChange={(e) => setTopic(e.target.value)} />
          </label>
        </div>
        <div className="tray-levels">
          <div className="tray-level-row">
            <span className="tray-level-name">Mic</span>
            <div className="tray-level-track" aria-label="Mic level">
              <div className="tray-level-fill" style={{ width: `${micPct}%` }} />
            </div>
            <label className="tray-level-device">
              <span className="sr-only">Mic device</span>
              <select
                aria-label="Mic device"
                value={settings?.mic_device_name ?? ""}
                onChange={(e) => {
                  void saveSettingsPatch({ mic_device_name: e.target.value }).catch((err) =>
                    setStatus(`error: ${String(err)}`)
                  );
                }}
                disabled={status === "recording"}
              >
                <option value="">Auto</option>
                {audioDevices.map((dev) => (
                  <option key={`mic-${dev}`} value={dev}>
                    {dev}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="tray-level-row">
            <span className="tray-level-name">System</span>
            <div className="tray-level-track" aria-label="System level">
              <div className="tray-level-fill" style={{ width: `${systemPct}%` }} />
            </div>
            <label className="tray-level-device">
              <span className="sr-only">System device</span>
              <select
                aria-label="System device"
                value={settings?.system_device_name ?? ""}
                onChange={(e) => {
                  void saveSettingsPatch({ system_device_name: e.target.value }).catch((err) =>
                    setStatus(`error: ${String(err)}`)
                  );
                }}
                disabled={status === "recording"}
              >
                <option value="">Auto</option>
                {audioDevices.map((dev) => (
                  <option key={`sys-${dev}`} value={dev}>
                    {dev}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>
        <div className="button-row">
          <button className="primary-button rec-button" onClick={startFromTray} disabled={status === "recording"}>
            <span className="rec-dot" />
            Rec
          </button>
          <button className="secondary-button" onClick={stop} disabled={status !== "recording"}>
            <span className="stop-square" />
            Stop
          </button>
        </div>
      </main>
    );
  }

  if (isSettingsWindow) {
    return (
      <main className="app-shell settings-shell">
        <header className="hero">
          <h1>BigEcho Settings</h1>
          <p className="status-line">Статус: {formatAppStatus(status)}</p>
        </header>
        <section className="panel">
          {renderSettingsFields()}
        </section>
      </main>
    );
  }

  return (
    <main className="app-shell">
      <header className="hero">
        <div>
          <h1>BigEcho</h1>
          <p className="status-line">Status: {formatAppStatus(status)}</p>
        </div>
      </header>

      <section className="panel">
        <h2>Settings</h2>
        {renderSettingsFields()}
      </section>

      <section className="panel">
        <h2>Sessions</h2>
        <div className="search-grid">
          <label className="field">
            Search sessions
            <input value={sessionSearchQuery} onChange={(e) => setSessionSearchQuery(e.target.value)} />
          </label>
        </div>
        <div className="button-row">
          <button className="secondary-button" onClick={loadSessions}>
            Refresh sessions
          </button>
        </div>
        <div className="sessions-grid">
          {filteredSessions.map((item) => {
            const detail = sessionDetails[item.session_id] ?? {
              session_id: item.session_id,
              source: item.primary_tag,
              custom_tag: "",
              topic: item.topic,
              participants: [],
            };
            const textPending = Boolean(textPendingBySession[item.session_id]);
            const summaryPending = Boolean(summaryPendingBySession[item.session_id]);
            const pipelineState = pipelineStateBySession[item.session_id];
            const query = sessionSearchQuery.trim().toLowerCase();
            const sourceMatch = query !== "" && detail.source.toLowerCase().includes(query);
            const customMatch = query !== "" && detail.custom_tag.toLowerCase().includes(query);
            const topicMatch = query !== "" && detail.topic.toLowerCase().includes(query);
            const participantsText = detail.participants.join(", ");
            const participantsMatch = query !== "" && participantsText.toLowerCase().includes(query);
            const pathMatch = query !== "" && item.session_dir.toLowerCase().includes(query);
            const statusMatch = query !== "" && item.status.toLowerCase().includes(query);
            return (
              <article key={item.session_id} className="session-card">
                <div className="session-header">
                  <div>
                    <strong>{detail.topic || "Без темы"}</strong> ({detail.source}) - {item.display_date_ru}
                  </div>
                  <div className="session-header-right">
                    <div className="session-labels">
                      {item.has_transcript_text && <span className="session-label session-label-text">текст</span>}
                      {item.has_summary_text && <span className="session-label session-label-summary">саммари</span>}
                    </div>
                    <button
                      type="button"
                      className="icon-button delete-session-button"
                      aria-label="Удалить сессию"
                      title="Удалить сессию"
                      onClick={() => requestDeleteSession(item.session_id, item.status === "recording")}
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path
                          d="M9 3h6l1 2h4v2H4V5h4l1-2zm1 7h2v8h-2v-8zm4 0h2v8h-2v-8zM7 10h2v8H7v-8z"
                          fill="currentColor"
                        />
                      </svg>
                    </button>
                  </div>
                </div>
                <div className={statusMatch ? "match-hit" : ""}>Status: {formatSessionStatus(item.status)}</div>
                <div>Audio: {item.audio_duration_hms}</div>
                <div className="session-path-row">
                  <div className={`session-path${pathMatch ? " match-hit" : ""}`}>{item.session_dir}</div>
                  <button className="link-button" type="button" onClick={() => void openSessionFolder(item.session_dir)}>
                    открыть
                  </button>
                </div>
                <div className="session-edit-grid">
                  <label className={`field${sourceMatch ? " match-hit" : ""}`}>
                    Source
                    <select
                      value={detail.source}
                      onChange={(e) =>
                        setSessionDetails((prev) => ({
                          ...prev,
                          [item.session_id]: { ...detail, source: e.target.value },
                        }))
                      }
                    >
                      {fixedSources.map((s) => (
                        <option key={s} value={s}>
                          {s}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className={`field${customMatch ? " match-hit" : ""}`}>
                    Custom tag
                    <input
                      value={detail.custom_tag}
                      onChange={(e) =>
                        setSessionDetails((prev) => ({
                          ...prev,
                          [item.session_id]: { ...detail, custom_tag: e.target.value },
                        }))
                      }
                    />
                  </label>
                  <label className={`field${topicMatch ? " match-hit" : ""}`}>
                    Topic
                    <input
                      value={detail.topic}
                      onChange={(e) =>
                        setSessionDetails((prev) => ({
                          ...prev,
                          [item.session_id]: { ...detail, topic: e.target.value },
                        }))
                      }
                    />
                  </label>
                  <label className={`field${participantsMatch ? " match-hit" : ""}`}>
                    Participants
                    <input
                      value={participantsText}
                      onChange={(e) =>
                        setSessionDetails((prev) => ({
                          ...prev,
                          [item.session_id]: {
                            ...detail,
                            participants: splitParticipants(e.target.value),
                          },
                        }))
                      }
                    />
                  </label>
                </div>
                <div className="button-row">
                  <button
                    className="secondary-button"
                    onClick={() => getText(item.session_id)}
                    disabled={item.status === "recording" || textPending || summaryPending}
                  >
                    {textPending ? (
                      <>
                        <span className="inline-loader" aria-hidden="true" />
                        Getting text...
                      </>
                    ) : (
                      "Get text"
                    )}
                  </button>
                  <button
                    className="secondary-button"
                    onClick={() => getSummary(item.session_id)}
                    disabled={item.status === "recording" || !item.has_transcript_text || summaryPending || textPending}
                  >
                    {summaryPending ? (
                      <>
                        <span className="inline-loader" aria-hidden="true" />
                        Getting summary...
                      </>
                    ) : (
                      "Get Summary"
                    )}
                  </button>
                  {pipelineState && (
                    <span
                      className={
                        pipelineState.kind === "error"
                          ? "retry-state retry-state-error"
                          : "retry-state retry-state-success"
                      }
                    >
                      {pipelineState.text}
                    </span>
                  )}
                </div>
              </article>
            );
          })}
          {!filteredSessions.length && <div>No sessions yet</div>}
        </div>
        {deleteTarget && (
          <div className="confirm-overlay" role="dialog" aria-modal="true" aria-label="Подтверждение удаления">
            <div className="confirm-card">
              <p>
                {deleteTarget.force
                  ? "Сессия помечена как активная. Принудительно удалить сессию и все связанные файлы?"
                  : "Удалить сессию и все связанные файлы?"}
              </p>
              <div className="button-row">
                <button
                  className="secondary-button"
                  type="button"
                  onClick={() => setDeleteTarget(null)}
                  disabled={deletePendingSessionId !== null}
                >
                  Отмена
                </button>
                <button
                  className="secondary-button danger-button"
                  type="button"
                  onClick={() => void confirmDeleteSession()}
                  disabled={deletePendingSessionId !== null}
                >
                  {deletePendingSessionId !== null ? "Удаление..." : "Удалить"}
                </button>
              </div>
            </div>
          </div>
        )}
      </section>
    </main>
  );
}
