import { useEffect, useRef, useState } from "react";
import {
  fixedSources,
  diarizationSettingOptions,
  PublicSettings,
  SessionMetaView,
  SettingsTab,
  StartResponse,
  transcriptionTaskOptions,
} from "./appTypes";
import { useRecordingController } from "./features/recording/useRecordingController";
import { useSessions } from "./features/sessions/useSessions";
import { useSettingsForm } from "./features/settings/useSettingsForm";
import { formatSecretSaveState, splitParticipants } from "./lib/appUtils";
import { getCurrentWindowLabel } from "./lib/tauri";
import { formatAppStatus, formatSessionStatus } from "./status";
import vscodeIcon from "./assets/editor-icons/vscode.svg";
import cursorIcon from "./assets/editor-icons/cursor.svg";
import sublimeIcon from "./assets/editor-icons/sublime.svg";
const currentWindowLabel = getCurrentWindowLabel();
const isSettingsWindow = currentWindowLabel === "settings";
const isTrayWindow = currentWindowLabel === "tray";
const openerUiFallback = [
  { id: "TextEdit", name: "TextEdit", icon_fallback: "📝", icon_data_url: null },
  { id: "Visual Studio Code", name: "Visual Studio Code", icon_fallback: "💠", icon_data_url: null },
  { id: "Sublime Text", name: "Sublime Text", icon_fallback: "🟧", icon_data_url: null },
  { id: "Cursor", name: "Cursor", icon_fallback: "🧩", icon_data_url: null },
  { id: "Windsurf", name: "Windsurf", icon_fallback: "🧩", icon_data_url: null },
  { id: "Zed", name: "Zed", icon_fallback: "🧩", icon_data_url: null },
] as const;

function localIconForEditor(editorName: string): string | null {
  const lowered = editorName.toLowerCase();
  if (lowered.includes("visual studio code") || lowered === "vscode") return vscodeIcon;
  if (lowered.includes("cursor")) return cursorIcon;
  if (lowered.includes("sublime")) return sublimeIcon;
  return null;
}

export function App() {
  const [topic, setTopic] = useState("");
  const [participants, setParticipants] = useState("");
  const [source, setSource] = useState("slack");
  const [customTag, setCustomTag] = useState("");
  const [session, setSession] = useState<StartResponse | null>(null);
  const [lastSessionId, setLastSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState("idle");
  const [isOpenerDropdownOpen, setIsOpenerDropdownOpen] = useState(false);
  const openerDropdownRef = useRef<HTMLDivElement | null>(null);
  const {
    audioDevices,
    autoDetectSystemSource,
    canSaveSettings,
    nexaraKey,
    nexaraSecretState,
    openaiKey,
    openaiSecretState,
    saveApiKeys,
    saveSettings,
    saveSettingsPatch,
    savedSettingsSnapshot,
    setNexaraKey,
    setNexaraSecretState,
    setOpenaiKey,
    setOpenaiSecretState,
    setSettings,
    setSettingsTab,
    settings,
    settingsErrors,
    settingsTab,
    textEditorApps,
  } = useSettingsForm({ isTrayWindow, setStatus });
  const {
    confirmDeleteSession,
    deletePendingSessionId,
    deleteTarget,
    filteredSessions,
    getSummary,
    getText,
    loadSessions,
    openSessionArtifact,
    openSessionFolder,
    pipelineStateBySession,
    requestDeleteSession,
    sessionDetails,
    sessionSearchQuery,
    sessions,
    setDeleteTarget,
    setSessionDetails,
    setSessionSearchQuery,
    summaryPendingBySession,
    textPendingBySession,
  } = useSessions({ setStatus, lastSessionId, setLastSessionId });
  const { liveLevels, start, startFromTray, stop } = useRecordingController({
    isSettingsWindow,
    isTrayWindow,
    topic,
    setTopic,
    participants,
    setParticipants,
    source,
    setSource,
    customTag,
    setCustomTag,
    session,
    setSession,
    lastSessionId,
    setLastSessionId,
    status,
    setStatus,
    loadSessions,
  });

  useEffect(() => {
    if (!isTrayWindow) return;
    document.body.classList.add("tray-window-body");
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

  function renderSettingsFields() {
    if (!settings) return null;
    const openerOptions = textEditorApps.length > 0 ? textEditorApps : openerUiFallback;
    const selectedOpenerApp = openerOptions.find((app) => app.id === settings.artifact_open_app) ?? null;
    const snapshot = savedSettingsSnapshot;
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
        isDirty("artifact_open_app") ||
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
      <div className="settings-tabs settings-layout">
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
                        {(selectedOpenerApp.icon_data_url || localIconForEditor(selectedOpenerApp.name)) ? (
                          <img
                            className="opener-app-icon"
                            src={selectedOpenerApp.icon_data_url || localIconForEditor(selectedOpenerApp.name) || ""}
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
                        className={`opener-dropdown-option${settings.artifact_open_app === "" ? " is-active" : ""}`}
                        onClick={() => {
                          setSettings({ ...settings, artifact_open_app: "" });
                          setIsOpenerDropdownOpen(false);
                        }}
                      >
                        <span>System default</span>
                      </button>
                      {openerOptions.map((editor) => (
                        <button
                          key={editor.id}
                          type="button"
                          className={`opener-dropdown-option${settings.artifact_open_app === editor.id ? " is-active" : ""}`}
                          onClick={() => {
                            setSettings({ ...settings, artifact_open_app: editor.id });
                            setIsOpenerDropdownOpen(false);
                          }}
                        >
                          {(editor.icon_data_url || localIconForEditor(editor.name)) ? (
                            <img
                              className="opener-app-icon"
                              src={editor.icon_data_url || localIconForEditor(editor.name) || ""}
                              alt=""
                              aria-hidden="true"
                            />
                          ) : (
                            <span className="opener-app-fallback-icon" aria-hidden="true">
                              {editor.icon_fallback}
                            </span>
                          )}
                          <span>{editor.name}</span>
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
      <main className="app-shell settings-shell mac-window settings-layout">
        <header className="hero mac-titlebar">
          <h1 className="mac-title">BigEcho Settings</h1>
          <p className="status-line">Статус: {formatAppStatus(status)}</p>
        </header>
        <section className="panel">
          {renderSettingsFields()}
        </section>
      </main>
    );
  }

  return (
    <main className="app-shell mac-window mac-content">
      <header className="hero mac-titlebar">
        <div>
          <h1 className="mac-title">BigEcho</h1>
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
                  <div className="session-title-block">
                    <div className="session-title-line">
                      <strong>{detail.topic || "Без темы"}</strong>
                      <span className="session-title-meta">
                        ({detail.source}) - {item.display_date_ru}
                      </span>
                      <span className="session-duration-label">{item.audio_duration_hms}</span>
                    </div>
                    <div className={statusMatch ? "session-status match-hit" : "session-status"}>
                      Status: {formatSessionStatus(item.status)}
                    </div>
                  </div>
                  <div className="session-header-right">
                    <div className="session-labels">
                      {item.has_transcript_text && (
                        <button
                          type="button"
                          className="session-label session-label-action session-label-text"
                          onClick={() => void openSessionArtifact(item.session_id, "transcript")}
                        >
                          текст
                        </button>
                      )}
                      {item.has_summary_text && (
                        <button
                          type="button"
                          className="session-label session-label-action session-label-summary"
                          onClick={() => void openSessionArtifact(item.session_id, "summary")}
                        >
                          саммари
                        </button>
                      )}
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
