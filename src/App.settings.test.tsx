import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(async (cmd: string) => {
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
    if (cmd === "detect_system_source_device") {
      return "BlackHole 2ch";
    }
    if (cmd === "list_audio_input_devices") {
      return ["Built-in Microphone", "BlackHole 2ch"];
    }
    if (cmd === "list_text_editor_apps") {
      return {
        apps: [
          { id: "textedit", name: "TextEdit", icon_fallback: "📝", icon_data_url: null },
          { id: "visual_studio_code", name: "Visual Studio Code", icon_fallback: "💠", icon_data_url: null },
        ],
        default_app_id: "textedit",
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
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "settings" }),
}));

import { App } from "./App";

describe("App settings window", () => {
  beforeEach(() => {
    invokeMock.mockClear();
  });

  it("loads settings and auto-detects system source", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getByRole("main")).toHaveClass("mac-window");
    expect(screen.getByRole("main")).toHaveClass("settings-layout");
    expect(screen.getByText("BigEcho Settings")).toBeInTheDocument();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    await user.click(await screen.findByRole("tab", { name: "Audio" }));
    await user.click(screen.getByRole("button", { name: "Auto-detect system source" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_system_source_device");
    });

    const input = screen.getByDisplayValue("BlackHole 2ch");
    expect(input).toBeInTheDocument();
  });

  it("disables saving when settings are invalid", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    const saveButton = screen.getByRole("button", { name: "Save settings" });
    expect(saveButton).toBeEnabled();

    await user.clear(screen.getByLabelText("Transcription URL"));
    await user.type(screen.getByLabelText("Transcription URL"), "not-url");

    expect(screen.getByText("Неверный URL транскрибации")).toBeInTheDocument();
    expect(saveButton).toBeDisabled();

    await user.clear(screen.getByLabelText("Transcription URL"));
    await user.type(screen.getByLabelText("Transcription URL"), "https://example.com/transcribe");

    expect(screen.queryByText("Неверный URL транскрибации")).not.toBeInTheDocument();
    expect(saveButton).toBeEnabled();
  });

  it("treats non-http urls as invalid", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    const saveButton = screen.getByRole("button", { name: "Save settings" });
    await user.clear(screen.getByLabelText("Transcription URL"));
    await user.type(screen.getByLabelText("Transcription URL"), "file:///tmp/transcribe");

    expect(screen.getByText("Неверный URL транскрибации")).toBeInTheDocument();
    expect(saveButton).toBeDisabled();
  });

  it("saves settings and api keys", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    const keyInputs = screen.getAllByPlaceholderText("Stored in OS secure storage");
    await user.selectOptions(screen.getByLabelText("Task"), "diarize");
    await user.selectOptions(screen.getByLabelText("Diarization setting"), "meeting");
    await user.type(screen.getByLabelText("Summary prompt"), "Сделай саммари блоками: решения, риски, action items");
    await user.type(keyInputs[0], "nexara-secret");
    await user.type(keyInputs[1], "openai-secret");

    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_public_settings", expect.any(Object));
      expect(invokeMock).toHaveBeenCalledWith("save_public_settings", {
        payload: expect.objectContaining({
          transcription_task: "diarize",
          transcription_diarization_setting: "meeting",
          summary_prompt: "Сделай саммари блоками: решения, риски, action items",
        }),
      });
      expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
        name: "NEXARA_API_KEY",
        value: "nexara-secret",
      });
      expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
        name: "OPENAI_API_KEY",
        value: "openai-secret",
      });
    });

    expect(screen.getByText("Статус: настройки сохранены")).toBeInTheDocument();
    expect(screen.getByText("Nexara API key: обновлён")).toBeInTheDocument();
    expect(screen.getByText("OpenAI API key: обновлён")).toBeInTheDocument();
  });

  it("saves auto pipeline setting", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    await user.click(screen.getByRole("tab", { name: "Generals" }));
    const checkbox = screen.getByLabelText("Auto-run pipeline on Stop") as HTMLInputElement;
    const apiLoggingCheckbox = screen.getByLabelText("Enable API call logging") as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
    expect(apiLoggingCheckbox.checked).toBe(false);
    await user.click(checkbox);
    await user.click(apiLoggingCheckbox);
    expect(checkbox.checked).toBe(true);
    expect(apiLoggingCheckbox.checked).toBe(true);

    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_public_settings", {
        payload: expect.objectContaining({
          auto_run_pipeline_on_stop: true,
          api_call_logging_enabled: true,
        }),
      });
    });
  });

  it("saves artifact opener app from generals tab", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    await user.click(screen.getByRole("tab", { name: "Generals" }));
    await user.click(screen.getByRole("button", { name: "Artifact opener app (optional)" }));
    await user.click(screen.getByRole("button", { name: "Visual Studio Code" }));
    await user.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_public_settings", {
        payload: expect.objectContaining({
          artifact_open_app: "visual_studio_code",
        }),
      });
    });
  });

  it("marks api keys as unchanged when they are empty", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_public_settings", expect.any(Object));
    });

    expect(screen.getByText("Nexara API key: не изменён")).toBeInTheDocument();
    expect(screen.getByText("OpenAI API key: не изменён")).toBeInTheDocument();
  });

  it("saves api keys even when settings are invalid", async () => {
    const user = userEvent.setup();
    render(<App />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_settings");
    });

    await user.clear(screen.getByLabelText("Transcription URL"));
    await user.type(screen.getByLabelText("Transcription URL"), "not-url");
    expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled();

    const keyInputs = screen.getAllByPlaceholderText("Stored in OS secure storage");
    await user.type(keyInputs[0], "nexara-secret-only");

    await user.click(screen.getByRole("button", { name: "Save API keys" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
        name: "NEXARA_API_KEY",
        value: "nexara-secret-only",
      });
    });

    expect(screen.getByText("Статус: ключи сохранены")).toBeInTheDocument();
    expect(screen.getByText("Nexara API key: обновлён")).toBeInTheDocument();
  });
});
