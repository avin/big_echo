import { act, renderHook, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

const { invokeMock, emitMock, listenMock, listeners } = vi.hoisted(() => ({
  listeners: new Map<string, (payload?: unknown) => void | Promise<void>>(),
  emitMock: vi.fn(async () => undefined),
  invokeMock: vi.fn(async (cmd: string) => {
    if (cmd === "get_ui_sync_state") {
      return { source: "slack", topic: "", is_recording: false, active_session_id: null };
    }
    if (cmd === "start_recording") {
      return { session_id: "s1", session_dir: "/tmp/s1", status: "recording" };
    }
    if (cmd === "stop_recording") {
      return "recorded";
    }
    if (cmd === "get_live_input_levels") {
      return { mic: 0.2, system: 0.5 };
    }
    if (cmd === "update_session_details") {
      return "updated";
    }
    if (cmd === "run_pipeline") {
      return "done";
    }
    return null;
  }),
  listenMock: vi.fn(async (event: string, handler: (payload?: unknown) => void | Promise<void>) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

vi.mock("../../lib/tauri", () => ({
  tauriEmit: emitMock,
  tauriInvoke: invokeMock,
  tauriListen: listenMock,
}));

import { useRecordingController } from "./useRecordingController";

describe("useRecordingController", () => {
  it("hydrates ui sync state and starts recording through the tauri adapter", async () => {
    const loadSessions = vi.fn(async () => undefined);

    const { result } = renderHook(() => {
      const [topic, setTopic] = useState("");
      const [participants, setParticipants] = useState("");
      const [source, setSource] = useState("slack");
      const [customTag, setCustomTag] = useState("");
      const [session, setSession] = useState(null);
      const [lastSessionId, setLastSessionId] = useState<string | null>(null);
      const [status, setStatus] = useState("idle");

      return useRecordingController({
        isSettingsWindow: false,
        isTrayWindow: false,
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
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_ui_sync_state");
    });

    await act(async () => {
      await result.current.start();
    });

    expect(invokeMock).toHaveBeenCalledWith("start_recording", {
      payload: {
        tags: ["slack"],
        topic: "",
        participants: [],
      },
    });
    expect(loadSessions).toHaveBeenCalled();
  });
});
