import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

const { listeners, invokeMock } = vi.hoisted(() => ({
  listeners: new Map<string, (payload?: unknown) => void | Promise<void>>(),
  invokeMock: vi.fn(async (cmd: string) => {
    if (cmd === "get_ui_sync_state") {
      return { source: "slack", topic: "", is_recording: false, active_session_id: null };
    }
    if (cmd === "set_ui_sync_state") {
      return "updated";
    }
    if (cmd === "start_recording") {
      return { session_id: "s1", session_dir: "/tmp/s1", status: "recording" };
    }
    if (cmd === "stop_recording") {
      return "recorded";
    }
    if (cmd === "list_sessions") {
      return [];
    }
    if (cmd === "get_settings") {
      return {
        recording_root: "./recordings",
        artifact_open_app: "",
        transcription_url: "",
        transcription_task: "transcribe",
        transcription_diarization_setting: "general",
        summary_url: "",
        summary_prompt: "",
        openai_model: "gpt-4.1-mini",
        opus_bitrate_kbps: 24,
        mic_device_name: "",
        system_device_name: "",
        auto_run_pipeline_on_stop: false,
        api_call_logging_enabled: false,
      };
    }
    return null;
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(async () => undefined),
  listen: vi.fn(async (event: string, handler: (payload?: unknown) => void | Promise<void>) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main", hide: vi.fn(async () => undefined) }),
}));

import { App } from "./App";

describe("App main window", () => {
  it("syncs source/topic from shared ui events and uses it on tray start", async () => {
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_ui_sync_state");
      expect(listeners.has("ui:sync")).toBe(true);
      expect(listeners.has("ui:recording")).toBe(true);
      expect(listeners.has("tray:start")).toBe(true);
    });

    await act(async () => {
      await listeners.get("ui:sync")?.({
        payload: JSON.stringify({ source: "telegram", topic: "Q1 planning" }),
      });
    });

    await act(async () => {
      await listeners.get("tray:start")?.();
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_recording", {
        payload: {
          tags: ["telegram"],
          topic: "Q1 planning",
          participants: [],
        },
      });
    });
  });

  it("registers tray listeners and reacts to tray start/stop", async () => {
    render(<App />);

    expect(screen.getByRole("main")).toHaveClass("mac-window");
    expect(screen.getByRole("main")).toHaveClass("mac-content");
    await waitFor(() => {
      expect(screen.getByRole("tablist", { name: "Settings sections" }).parentElement).toHaveClass(
        "settings-layout"
      );
    });
    expect(screen.getByText("BigEcho")).toBeInTheDocument();
    expect(screen.queryByText("Recording")).not.toBeInTheDocument();
    expect(screen.queryByText("При закрытии окно сворачивается в трей")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(listeners.has("tray:start")).toBe(true);
      expect(listeners.has("tray:stop")).toBe(true);
    });

    await act(async () => {
      await listeners.get("tray:start")?.();
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_recording", expect.any(Object));
    });

    await act(async () => {
      await listeners.get("tray:stop")?.();
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("stop_recording", expect.any(Object));
    });
  });

  it("starts recording from tray start event with default source", async () => {
    render(<App />);

    await waitFor(() => {
      expect(listeners.has("tray:start")).toBe(true);
    });

    await act(async () => {
      await listeners.get("tray:start")?.();
    });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_recording", {
        payload: {
          tags: ["slack"],
          topic: "",
          participants: [],
        },
      });
    });
  });

  it("autosaves session details after edit", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s2",
            status: "recorded",
            primary_tag: "zoom",
            topic: "Initial topic",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T10:00:00+03:00",
            session_dir: "/tmp/s2",
            audio_duration_hms: "01:02:03",
            has_transcript_text: false,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s2",
          source: "zoom",
          custom_tag: "alpha",
          topic: "Initial topic",
          participants: ["Alice"],
        };
      }
      if (cmd === "update_session_details") {
        return "updated";
      }
      if (cmd === "start_recording") {
        return { session_id: "s1", session_dir: "/tmp/s1", status: "recording" };
      }
      if (cmd === "stop_recording") {
        return "recorded";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_session_meta", { sessionId: "s2" });
    });
    expect(screen.getByText("01:02:03")).toBeInTheDocument();

    const editableTopic = screen.getByDisplayValue("Initial topic");
    await user.clear(editableTopic);
    await user.type(editableTopic, "Edited topic");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_session_details", {
        payload: {
          session_id: "s2",
          source: "zoom",
          custom_tag: "alpha",
          topic: "Edited topic",
          participants: ["Alice"],
        },
      });
    }, { timeout: 3000 });
  });

  it("calls transcription command from Get text and keeps Get Summary disabled without text", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s3",
            status: "recorded",
            primary_tag: "slack",
            topic: "Retry me",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T11:00:00+03:00",
            session_dir: "/tmp/s3",
            audio_duration_hms: "00:15:20",
            has_transcript_text: false,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s3",
          source: "slack",
          custom_tag: "",
          topic: "Retry me",
          participants: [],
        };
      }
      if (cmd === "run_transcription") {
        expect(args).toEqual({ sessionId: "s3" });
        return "transcribed";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_session_meta", { sessionId: "s3" });
    });

    const getTextButton = screen.getByRole("button", { name: "Get text" });
    const getSummaryButton = screen.getByRole("button", { name: "Get Summary" });
    expect(getTextButton).toBeEnabled();
    expect(getSummaryButton).toBeDisabled();
    await user.click(getTextButton);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("run_transcription", { sessionId: "s3" });
    });
  });

  it("searches sessions by one query and highlights matched fields", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "get_settings") {
        return {
          recording_root: "./recordings",
          artifact_open_app: "",
          transcription_url: "",
          transcription_task: "transcribe",
          transcription_diarization_setting: "general",
          summary_url: "",
          summary_prompt: "",
          openai_model: "gpt-4.1-mini",
          opus_bitrate_kbps: 24,
          mic_device_name: "",
          system_device_name: "",
          auto_run_pipeline_on_stop: false,
          api_call_logging_enabled: false,
        };
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s4",
            status: "recorded",
            primary_tag: "zoom",
            topic: "Budget planning",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T10:00:00+03:00",
            session_dir: "/tmp/project-alpha/s4",
            audio_duration_hms: "00:12:11",
            has_transcript_text: false,
            has_summary_text: false,
          },
          {
            session_id: "s5",
            status: "recorded",
            primary_tag: "slack",
            topic: "Roadmap",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T11:00:00+03:00",
            session_dir: "/tmp/project-beta/s5",
            audio_duration_hms: "00:10:09",
            has_transcript_text: false,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        if ((args as { sessionId?: string } | undefined)?.sessionId === "s5") {
          return {
            session_id: "s5",
            source: "slack",
            custom_tag: "project-beta",
            topic: "Roadmap",
            participants: ["Bob"],
          };
        }
        return {
          session_id: "s4",
          source: "zoom",
          custom_tag: "project-alpha",
          topic: "Budget planning",
          participants: ["Alice"],
        };
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(screen.getByText("/tmp/project-alpha/s4")).toBeInTheDocument();
      expect(screen.getByText("/tmp/project-beta/s5")).toBeInTheDocument();
    });

    await user.type(screen.getByLabelText("Search sessions"), "project-alpha");

    expect(screen.getByDisplayValue("Budget planning")).toBeInTheDocument();
    expect(screen.queryByDisplayValue("Roadmap")).not.toBeInTheDocument();
    expect(screen.getByText("/tmp/project-alpha/s4")).toHaveClass("match-hit");
  });

  it("opens session folder from session row link", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "get_settings") {
        return {
          recording_root: "./recordings",
          artifact_open_app: "",
          transcription_url: "",
          transcription_task: "transcribe",
          transcription_diarization_setting: "general",
          summary_url: "",
          summary_prompt: "",
          openai_model: "gpt-4.1-mini",
          opus_bitrate_kbps: 24,
          mic_device_name: "",
          system_device_name: "",
          auto_run_pipeline_on_stop: false,
          api_call_logging_enabled: false,
        };
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s6",
            status: "recorded",
            primary_tag: "zoom",
            topic: "Open folder",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T11:00:00+03:00",
            session_dir: "/tmp/s6",
            audio_duration_hms: "00:03:30",
            has_transcript_text: false,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s6",
          source: "zoom",
          custom_tag: "",
          topic: "Open folder",
          participants: [],
        };
      }
      if (cmd === "open_session_folder") {
        expect(args).toEqual({ sessionDir: "/tmp/s6" });
        return "opened";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(screen.getByText("/tmp/s6")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "открыть" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("open_session_folder", { sessionDir: "/tmp/s6" });
    });
  });

  it("deletes session only after confirmation", async () => {
    const user = userEvent.setup();
    let sessions = [
      {
        session_id: "s7",
        status: "recorded",
        primary_tag: "zoom",
        topic: "Delete me",
        display_date_ru: "11.03.2026",
        started_at_iso: "2026-03-11T11:30:00+03:00",
        session_dir: "/tmp/s7",
        audio_duration_hms: "00:05:10",
        has_transcript_text: false,
        has_summary_text: false,
      },
    ];
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "get_settings") {
        return {
          recording_root: "./recordings",
          artifact_open_app: "",
          transcription_url: "",
          transcription_task: "transcribe",
          transcription_diarization_setting: "general",
          summary_url: "",
          summary_prompt: "",
          openai_model: "gpt-4.1-mini",
          opus_bitrate_kbps: 24,
          mic_device_name: "",
          system_device_name: "",
          auto_run_pipeline_on_stop: false,
          api_call_logging_enabled: false,
        };
      }
      if (cmd === "list_sessions") {
        return sessions;
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s7",
          source: "zoom",
          custom_tag: "",
          topic: "Delete me",
          participants: [],
        };
      }
      if (cmd === "delete_session") {
        expect(args).toEqual({ sessionId: "s7", force: false });
        sessions = [];
        return "deleted";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(screen.getByText("/tmp/s7")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Удалить сессию" }));
    expect(screen.getByText("Удалить сессию и все связанные файлы?")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Отмена" }));
    expect(invokeMock).not.toHaveBeenCalledWith("delete_session", { sessionId: "s7", force: false });
    expect(screen.getByText("/tmp/s7")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Удалить сессию" }));
    await user.click(screen.getByRole("button", { name: "Удалить" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("delete_session", { sessionId: "s7", force: false });
    });
    await waitFor(() => {
      expect(screen.queryByText("/tmp/s7")).not.toBeInTheDocument();
    });
  });

  it("forces delete for recording session from confirmation dialog", async () => {
    const user = userEvent.setup();
    let sessions = [
      {
        session_id: "s8",
        status: "recording",
        primary_tag: "slack",
        topic: "Stuck recording",
        display_date_ru: "11.03.2026",
        started_at_iso: "2026-03-11T12:40:00+03:00",
        session_dir: "/tmp/s8",
        audio_duration_hms: "00:00:00",
        has_transcript_text: false,
        has_summary_text: false,
      },
    ];
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "list_sessions") {
        return sessions;
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s8",
          source: "slack",
          custom_tag: "",
          topic: "Stuck recording",
          participants: [],
        };
      }
      if (cmd === "delete_session") {
        expect(args).toEqual({ sessionId: "s8", force: true });
        sessions = [];
        return "deleted";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(screen.getByText("/tmp/s8")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Удалить сессию" }));
    expect(
      screen.getByText("Сессия помечена как активная. Принудительно удалить сессию и все связанные файлы?")
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Удалить" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("delete_session", { sessionId: "s8", force: true });
    });
  });

  it("shows text loader and success message for session", async () => {
    const user = userEvent.setup();
    let resolveText: (() => void) | null = null;

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s4",
            status: "recorded",
            primary_tag: "slack",
            topic: "Retry loading",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T12:00:00+03:00",
            session_dir: "/tmp/s4",
            has_transcript_text: false,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s4",
          source: "slack",
          custom_tag: "",
          topic: "Retry loading",
          participants: [],
        };
      }
      if (cmd === "run_transcription") {
        return new Promise<string>((resolve) => {
          resolveText = () => resolve("transcribed");
        });
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_session_meta", { sessionId: "s4" });
    });

    await user.click(screen.getByRole("button", { name: "Get text" }));
    expect(screen.getByRole("button", { name: "Getting text..." })).toBeDisabled();

    act(() => {
      resolveText?.();
    });

    await waitFor(() => {
      expect(screen.getByText("Text fetched successfully")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Get text" })).toBeEnabled();
    });
  });

  it("shows summary error message for session", async () => {
    const user = userEvent.setup();

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s5",
            status: "recorded",
            primary_tag: "slack",
            topic: "Retry error",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T12:30:00+03:00",
            session_dir: "/tmp/s5",
            has_transcript_text: true,
            has_summary_text: false,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s5",
          source: "slack",
          custom_tag: "",
          topic: "Retry error",
          participants: [],
        };
      }
      if (cmd === "run_summary") {
        throw new Error("Summary service is unavailable");
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_session_meta", { sessionId: "s5" });
    });

    await user.click(screen.getByRole("button", { name: "Get Summary" }));

    await waitFor(() => {
      expect(screen.getByText("Get summary failed: Summary service is unavailable")).toBeInTheDocument();
    });
  });

  it("opens transcript and summary artifacts by clicking labels", async () => {
    const user = userEvent.setup();

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_ui_sync_state") {
        return { source: "slack", topic: "", is_recording: false, active_session_id: null };
      }
      if (cmd === "set_ui_sync_state") {
        return "updated";
      }
      if (cmd === "list_sessions") {
        return [
          {
            session_id: "s6",
            status: "done",
            primary_tag: "slack",
            topic: "With artifacts",
            display_date_ru: "11.03.2026",
            started_at_iso: "2026-03-11T13:00:00+03:00",
            session_dir: "/tmp/s6",
            has_transcript_text: true,
            has_summary_text: true,
          },
        ];
      }
      if (cmd === "get_session_meta") {
        return {
          session_id: "s6",
          source: "slack",
          custom_tag: "",
          topic: "With artifacts",
          participants: [],
        };
      }
      if (cmd === "open_session_artifact") {
        return "opened";
      }
      return null;
    });

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Refresh sessions" }));
    await waitFor(() => {
      expect(screen.getByText("текст")).toBeInTheDocument();
      expect(screen.getByText("саммари")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "текст" }));
    await user.click(screen.getByRole("button", { name: "саммари" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("open_session_artifact", {
        sessionId: "s6",
        artifactKind: "transcript",
      });
      expect(invokeMock).toHaveBeenCalledWith("open_session_artifact", {
        sessionId: "s6",
        artifactKind: "summary",
      });
    });
  });
});
