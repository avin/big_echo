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
      return { session_id: "tray-session", session_dir: "/tmp/tray-session", status: "recording" };
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
    if (cmd === "list_audio_input_devices") {
      return ["Built-in Microphone", "BlackHole 2ch"];
    }
    if (cmd === "save_public_settings") {
      return null;
    }
    if (cmd === "get_live_input_levels") {
      return { mic: 0.42, system: 0.73 };
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
  getCurrentWindow: () => ({ label: "tray", hide: vi.fn() }),
}));

import { App } from "./App";

describe("Tray window", () => {
  it("applies shared ui sync updates", async () => {
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_ui_sync_state");
      expect(listeners.has("ui:sync")).toBe(true);
      expect(listeners.has("ui:recording")).toBe(true);
    });

    await act(async () => {
      await listeners.get("ui:sync")?.({
        payload: JSON.stringify({ source: "facetime", topic: "1:1" }),
      });
    });

    await waitFor(() => {
      expect(screen.getByLabelText("Source")).toHaveValue("facetime");
      expect(screen.getByLabelText("Topic (optional)")).toHaveValue("1:1");
    });
  });

  it("renders mini recorder and starts with optional topic", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.queryByText("Recorder")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Source")).toBeInTheDocument();
    expect(screen.getByLabelText("Topic (optional)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Rec" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();

    await user.type(screen.getByLabelText("Topic (optional)"), "Daily sync");
    await user.click(screen.getByRole("button", { name: "Rec" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_recording", {
        payload: {
          tags: ["slack"],
          topic: "Daily sync",
          participants: [],
        },
      });
    });
  });

  it("saves topic edits to active tray recording session", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Rec" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_recording", {
        payload: {
          tags: ["slack"],
          topic: "",
          participants: [],
        },
      });
    });

    await user.type(screen.getByLabelText("Topic (optional)"), "Daily sync");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_session_details", {
        payload: {
          session_id: "tray-session",
          source: "slack",
          custom_tag: "",
          topic: "Daily sync",
          participants: [],
        },
      });
    });
  });

  it("polls live mic and system levels in tray popover even while idle", async () => {
    render(<App />);

    expect(screen.getByText("Mic")).toBeInTheDocument();
    expect(screen.getByText("System")).toBeInTheDocument();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_live_input_levels");
    });

    await waitFor(() => {
      const micTrack = screen.getByLabelText("Mic level");
      const sysTrack = screen.getByLabelText("System level");
      expect(micTrack.firstElementChild).toHaveStyle({ width: "42%" });
      expect(sysTrack.firstElementChild).toHaveStyle({ width: "73%" });
    });
  });

  it("shows audio device selectors near live levels and saves selected devices", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
      expect(invokeMock).toHaveBeenCalledWith("list_audio_input_devices");
    });

    const micSelect = screen.getByLabelText("Mic device");
    const systemSelect = screen.getByLabelText("System device");
    expect(micSelect).toBeInTheDocument();
    expect(systemSelect).toBeInTheDocument();

    await user.selectOptions(micSelect, "Built-in Microphone");
    await user.selectOptions(systemSelect, "BlackHole 2ch");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_public_settings",
        expect.objectContaining({
          payload: expect.objectContaining({
            mic_device_name: "Built-in Microphone",
            system_device_name: "BlackHole 2ch",
          }),
        })
      );
    });
  });
});
