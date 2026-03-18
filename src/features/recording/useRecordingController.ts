import { Dispatch, SetStateAction, useEffect, useRef, useState } from "react";
import { LiveInputLevels, StartResponse, UiSyncStateView } from "../../appTypes";
import { clamp01, parseEventPayload, splitParticipants } from "../../lib/appUtils";
import { tauriEmit, tauriInvoke, tauriListen } from "../../lib/tauri";

type Setter<T> = Dispatch<SetStateAction<T>>;

type UseRecordingControllerOptions = {
  isSettingsWindow: boolean;
  isTrayWindow: boolean;
  topic: string;
  setTopic: Setter<string>;
  participants: string;
  setParticipants: Setter<string>;
  source: string;
  setSource: Setter<string>;
  customTag: string;
  setCustomTag: Setter<string>;
  session: StartResponse | null;
  setSession: Setter<StartResponse | null>;
  lastSessionId: string | null;
  setLastSessionId: Setter<string | null>;
  status: string;
  setStatus: Setter<string>;
  loadSessions: () => Promise<void>;
};

export function useRecordingController({
  isSettingsWindow,
  isTrayWindow,
  topic,
  setTopic,
  participants,
  source,
  setSource,
  customTag,
  session,
  setSession,
  lastSessionId,
  setLastSessionId,
  status,
  setStatus,
  loadSessions,
}: UseRecordingControllerOptions) {
  const [liveLevels, setLiveLevels] = useState<LiveInputLevels>({ mic: 0, system: 0 });
  const [uiSyncReady, setUiSyncReady] = useState(isSettingsWindow);
  const topicRef = useRef(topic);
  const sourceRef = useRef(source);
  const sessionRef = useRef<StartResponse | null>(session);
  const trayTopicAutosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const trayTopicSavedSignatureRef = useRef<string>("");

  useEffect(() => {
    topicRef.current = topic;
    sourceRef.current = source;
    sessionRef.current = session;
  }, [session, source, topic]);

  async function startRecording(payload: { source: string; customTag?: string; topic?: string; participants?: string[] }) {
    const tags = [payload.source];
    if (payload.customTag && payload.customTag.trim()) tags.push(payload.customTag.trim());
    const response = await tauriInvoke<StartResponse>("start_recording", {
      payload: {
        tags,
        topic: payload.topic ?? "",
        participants: payload.participants ?? [],
      },
    });
    setSession(response);
    setLastSessionId(response.session_id);
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
    await tauriInvoke<string>("stop_recording", { sessionId: session.session_id });
    setStatus("recorded");
    setSession(null);
    await loadSessions();
  }

  async function runPipeline() {
    if (!lastSessionId) return;
    await tauriInvoke<string>("run_pipeline", { sessionId: lastSessionId });
    setStatus("done");
    await loadSessions();
  }

  useEffect(() => {
    if (!isTrayWindow) return;
    let active = true;
    let inFlight = false;
    const tick = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const levels = await tauriInvoke<LiveInputLevels>("get_live_input_levels");
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
  }, [isTrayWindow, status]);

  useEffect(() => {
    if (isSettingsWindow) return;
    let active = true;
    tauriInvoke<UiSyncStateView>("get_ui_sync_state")
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
  }, [isSettingsWindow, setLastSessionId, setSession, setSource, setStatus, setTopic]);

  useEffect(() => {
    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenUiSync: (() => void) | undefined;
    let unlistenUiRecording: (() => void) | undefined;

    tauriListen("tray:start", async () => {
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

    tauriListen("tray:stop", async () => {
      try {
        if (!sessionRef.current) return;
        await tauriInvoke<string>("stop_recording", { sessionId: sessionRef.current.session_id });
        setStatus("recorded");
        setSession(null);
        await loadSessions();
      } catch (err) {
        setStatus(`error: ${String(err)}`);
      }
    }).then((fn) => {
      unlistenStop = fn;
    });

    tauriListen("ui:sync", (event) => {
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

    tauriListen("ui:recording", (event) => {
      const payload = parseEventPayload<{ recording?: boolean; sessionId?: string | null }>(event);
      if (!payload || typeof payload.recording !== "boolean") return;
      if (payload.recording) {
        setStatus("recording");
        if (payload.sessionId) {
          setSession((prev) => prev ?? { session_id: payload.sessionId, session_dir: "", status: "recording" });
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
  }, [loadSessions, setLastSessionId, setSession, setSource, setStatus, setTopic]);

  useEffect(() => {
    if (isSettingsWindow) return;
    const recording = status === "recording";
    tauriEmit("recording:status", { recording }).catch(() => undefined);
    tauriEmit("ui:recording", {
      recording,
      sessionId: recording ? (session?.session_id ?? null) : null,
    }).catch(() => undefined);
  }, [isSettingsWindow, session, status]);

  useEffect(() => {
    if (isSettingsWindow || !uiSyncReady) return;
    tauriInvoke("set_ui_sync_state", { source, topic }).catch(() => undefined);
    tauriEmit("ui:sync", { source, topic }).catch(() => undefined);
  }, [isSettingsWindow, source, topic, uiSyncReady]);

  useEffect(() => {
    if (!isTrayWindow || status !== "recording" || !session?.session_id) return;
    const signature = `${session.session_id}::${source}::${topic}`;
    if (signature === trayTopicSavedSignatureRef.current) return;
    if (trayTopicAutosaveTimerRef.current) clearTimeout(trayTopicAutosaveTimerRef.current);

    trayTopicAutosaveTimerRef.current = setTimeout(async () => {
      try {
        await tauriInvoke<string>("update_session_details", {
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
  }, [isTrayWindow, session?.session_id, source, status, topic]);

  return {
    liveLevels,
    runPipeline,
    start,
    startFromTray,
    stop,
    uiSyncReady,
  };
}
