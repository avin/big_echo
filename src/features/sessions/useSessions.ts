import { useEffect, useMemo, useRef, useState } from "react";
import {
  DeleteTarget,
  PipelineUiState,
  SessionArtifactPreview,
  SessionListItem,
  SessionMetaView,
} from "../../appTypes";
import { getErrorMessage } from "../../lib/appUtils";
import { tauriInvoke } from "../../lib/tauri";

type SessionArtifactSearchHit = {
  transcript_match: boolean;
  summary_match: boolean;
};

type SessionArtifactReadResponse = {
  path: string;
  text: string;
};

type UseSessionsOptions = {
  setStatus: (status: string) => void;
  lastSessionId: string | null;
  setLastSessionId: (sessionId: string | null) => void;
};

function fallbackSessionMeta(item: SessionListItem): SessionMetaView {
  return {
    session_id: item.session_id,
    source: item.primary_tag,
    custom_tag: "",
    topic: item.topic,
    participants: [],
  };
}

export function useSessions({ setStatus, lastSessionId, setLastSessionId }: UseSessionsOptions) {
  const [sessions, setSessions] = useState<SessionListItem[]>([]);
  const [sessionDetails, setSessionDetails] = useState<Record<string, SessionMetaView>>({});
  const [savedSessionDetails, setSavedSessionDetails] = useState<Record<string, SessionMetaView>>({});
  const [sessionSearchQuery, setSessionSearchQuery] = useState("");
  const [sessionArtifactSearchHits, setSessionArtifactSearchHits] = useState<Record<string, SessionArtifactSearchHit>>(
    {}
  );
  const [textPendingBySession, setTextPendingBySession] = useState<Record<string, boolean>>({});
  const [summaryPendingBySession, setSummaryPendingBySession] = useState<Record<string, boolean>>({});
  const [pipelineStateBySession, setPipelineStateBySession] = useState<Record<string, PipelineUiState>>({});
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [deletePendingSessionId, setDeletePendingSessionId] = useState<string | null>(null);
  const [artifactPreview, setArtifactPreview] = useState<SessionArtifactPreview | null>(null);
  const autosaveTimersRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const artifactSearchRequestIdRef = useRef(0);

  async function loadSessions() {
    const data = await tauriInvoke<SessionListItem[]>("list_sessions");
    setSessions(data);
    const details = await Promise.all(
      data.map(async (item) => {
        try {
          const meta = await tauriInvoke<SessionMetaView>("get_session_meta", { sessionId: item.session_id });
          return [item.session_id, meta] as const;
        } catch {
          return [item.session_id, fallbackSessionMeta(item)] as const;
        }
      })
    );
    const nextDetails = Object.fromEntries(details);
    setSessionDetails(nextDetails);
    setSavedSessionDetails(nextDetails);
  }

  async function getText(sessionId: string) {
    setTextPendingBySession((prev) => ({ ...prev, [sessionId]: true }));
    setPipelineStateBySession((prev) => {
      const next = { ...prev };
      delete next[sessionId];
      return next;
    });
    try {
      await tauriInvoke<string>("run_transcription", { sessionId });
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
      await tauriInvoke<string>("run_summary", { sessionId });
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
    await tauriInvoke<string>("open_session_folder", { sessionDir });
  }

  async function openSessionArtifact(sessionId: string, artifactKind: "transcript" | "summary") {
    const query = sessionSearchQuery.trim();
    const artifactHit = sessionArtifactSearchHits[sessionId];
    const hasArtifactMatch =
      query !== "" &&
      Boolean(
        artifactKind === "transcript" ? artifactHit?.transcript_match : artifactHit?.summary_match
      );

    if (hasArtifactMatch) {
      try {
        const preview = await tauriInvoke<SessionArtifactReadResponse>("read_session_artifact", {
          sessionId,
          artifactKind,
        });
        setArtifactPreview({
          sessionId,
          artifactKind,
          path: preview.path,
          text: preview.text,
          query,
        });
      } catch (err) {
        setStatus(`error: ${getErrorMessage(err)}`);
      }
      return;
    }

    try {
      await tauriInvoke<string>("open_session_artifact", { sessionId, artifactKind });
    } catch (err) {
      setStatus(`error: ${getErrorMessage(err)}`);
    }
  }

  function requestDeleteSession(sessionId: string, force: boolean) {
    setDeleteTarget({ sessionId, force });
  }

  async function confirmDeleteSession() {
    if (!deleteTarget) return;
    const sessionId = deleteTarget.sessionId;
    setDeletePendingSessionId(sessionId);
    try {
      await tauriInvoke<string>("delete_session", { sessionId, force: deleteTarget.force });
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
    const query = sessionSearchQuery.trim();
    if (!query || sessions.length === 0) {
      setSessionArtifactSearchHits({});
      return;
    }

    const requestId = artifactSearchRequestIdRef.current + 1;
    artifactSearchRequestIdRef.current = requestId;
    const timer = setTimeout(() => {
      void tauriInvoke<Record<string, SessionArtifactSearchHit>>("search_session_artifacts", { query })
        .then((hits) => {
          if (artifactSearchRequestIdRef.current !== requestId) return;
          setSessionArtifactSearchHits(hits ?? {});
        })
        .catch(() => {
          if (artifactSearchRequestIdRef.current !== requestId) return;
          setSessionArtifactSearchHits({});
        });
    }, 180);

    return () => clearTimeout(timer);
  }, [sessionSearchQuery, sessions]);

  useEffect(() => {
    const ids = Object.keys(sessionDetails);
    for (const sessionId of ids) {
      const current = sessionDetails[sessionId];
      const saved = savedSessionDetails[sessionId];
      if (!saved) continue;

      if (JSON.stringify(current) === JSON.stringify(saved)) continue;

      const existing = autosaveTimersRef.current[sessionId];
      if (existing) clearTimeout(existing);

      autosaveTimersRef.current[sessionId] = setTimeout(async () => {
        try {
          await tauriInvoke<string>("update_session_details", {
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
  }, [savedSessionDetails, sessionDetails, setStatus]);

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
      const artifactHit = sessionArtifactSearchHits[item.session_id];
      const artifactTextMatch = Boolean(artifactHit?.transcript_match || artifactHit?.summary_match);
      if (!query) return true;
      return (
        sourceValue.includes(query) ||
        customValue.includes(query) ||
        topicValue.includes(query) ||
        participantsValue.includes(query) ||
        pathValue.includes(query) ||
        statusValue.includes(query) ||
        dateValue.includes(query) ||
        artifactTextMatch
      );
    });
  }, [sessionArtifactSearchHits, sessionDetails, sessionSearchQuery, sessions]);

  return {
    artifactPreview,
    closeArtifactPreview: () => setArtifactPreview(null),
    confirmDeleteSession,
    deletePendingSessionId,
    deleteTarget,
    filteredSessions,
    getSummary,
    getText,
    loadSessions,
    openSessionFolder,
    openSessionArtifact,
    pipelineStateBySession,
    requestDeleteSession,
    sessionArtifactSearchHits,
    sessionDetails,
    sessionSearchQuery,
    sessions,
    setDeleteTarget,
    setSessionDetails,
    setSessionSearchQuery,
    summaryPendingBySession,
    textPendingBySession,
  };
}
