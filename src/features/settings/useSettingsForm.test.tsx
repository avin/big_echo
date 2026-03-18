import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(async (cmd: string, args?: unknown) => {
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
    if (cmd === "set_api_secret") {
      return null;
    }
    if (cmd === "save_public_settings") {
      return null;
    }
    if (cmd === "list_audio_input_devices") {
      return ["Built-in Microphone"];
    }
    if (cmd === "detect_system_source_device") {
      return "BlackHole 2ch";
    }
    return args ?? null;
  }),
}));

vi.mock("../../lib/tauri", () => ({
  tauriInvoke: invokeMock,
}));

import { useSettingsForm } from "./useSettingsForm";

describe("useSettingsForm", () => {
  it("loads settings and saves api keys through the tauri adapter", async () => {
    const setStatus = vi.fn();
    const { result } = renderHook(() =>
      useSettingsForm({ isTrayWindow: false, setStatus })
    );

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
      expect(result.current.settings?.openai_model).toBe("gpt-4.1-mini");
    });

    act(() => {
      result.current.setNexaraKey("nexara-secret");
    });

    await act(async () => {
      await result.current.saveApiKeys();
    });

    expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
      name: "NEXARA_API_KEY",
      value: "nexara-secret",
    });
  });
});
