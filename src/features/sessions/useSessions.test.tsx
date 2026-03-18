import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(async (cmd: string, args?: unknown) => {
    if (cmd === "list_sessions") {
      return [
        {
          session_id: "s1",
          status: "recorded",
          primary_tag: "zoom",
          topic: "Weekly sync",
          display_date_ru: "11.03.2026",
          started_at_iso: "2026-03-11T10:00:00+03:00",
          session_dir: "/tmp/s1",
          audio_duration_hms: "00:15:20",
          has_transcript_text: false,
          has_summary_text: false,
        },
      ];
    }
    if (cmd === "get_session_meta") {
      return {
        session_id: "s1",
        source: "zoom",
        custom_tag: "client-a",
        topic: "Weekly sync",
        participants: ["Alice"],
      };
    }
    return args ?? null;
  }),
}));

vi.mock("../../lib/tauri", () => ({
  tauriInvoke: invokeMock,
}));

import { useSessions } from "./useSessions";

describe("useSessions", () => {
  it("loads sessions and meta details through the tauri adapter", async () => {
    const setStatus = vi.fn();
    const setLastSessionId = vi.fn();
    const { result } = renderHook(() =>
      useSessions({ setStatus, lastSessionId: null, setLastSessionId })
    );

    await act(async () => {
      await result.current.loadSessions();
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_sessions");
      expect(invokeMock).toHaveBeenCalledWith("get_session_meta", { sessionId: "s1" });
      expect(result.current.sessions).toHaveLength(1);
      expect(result.current.sessionDetails.s1?.custom_tag).toBe("client-a");
    });
  });
});
