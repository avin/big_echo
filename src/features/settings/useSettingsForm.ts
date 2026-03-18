import { useEffect, useMemo, useState } from "react";
import { PublicSettings, SecretSaveState, SettingsTab } from "../../appTypes";
import { validateSettings } from "../../lib/validation";
import { tauriInvoke } from "../../lib/tauri";

type UseSettingsFormOptions = {
  isTrayWindow: boolean;
  setStatus: (status: string) => void;
};

export function useSettingsForm({ isTrayWindow, setStatus }: UseSettingsFormOptions) {
  const [settings, setSettings] = useState<PublicSettings | null>(null);
  const [savedSettingsSnapshot, setSavedSettingsSnapshot] = useState<PublicSettings | null>(null);
  const [nexaraKey, setNexaraKey] = useState("");
  const [openaiKey, setOpenaiKey] = useState("");
  const [nexaraSecretState, setNexaraSecretState] = useState<SecretSaveState>("unknown");
  const [openaiSecretState, setOpenaiSecretState] = useState<SecretSaveState>("unknown");
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("audiototext");

  const settingsErrors = useMemo(() => validateSettings(settings), [settings]);
  const canSaveSettings = Boolean(settings) && settingsErrors.length === 0;

  async function loadSettings() {
    const data = await tauriInvoke<PublicSettings>("get_settings");
    setSettings(data);
    setSavedSettingsSnapshot(data);
  }

  async function loadAudioDevices() {
    const list = await tauriInvoke<string[]>("list_audio_input_devices");
    setAudioDevices(list);
  }

  async function autoDetectSystemSource() {
    const detected = await tauriInvoke<string | null>("detect_system_source_device");
    if (!detected) {
      setStatus("system_source_not_detected");
      return;
    }
    setSettings((prev) => (prev ? { ...prev, system_device_name: detected } : prev));
    setStatus(`system_source_detected:${detected}`);
  }

  async function saveApiKeys() {
    let hasSecretError = false;
    let nextNexaraState: SecretSaveState = "unchanged";
    let nextOpenAiState: SecretSaveState = "unchanged";

    if (nexaraKey.trim()) {
      try {
        await tauriInvoke("set_api_secret", { name: "NEXARA_API_KEY", value: nexaraKey.trim() });
        nextNexaraState = "updated";
      } catch {
        nextNexaraState = "error";
        hasSecretError = true;
      }
    }

    if (openaiKey.trim()) {
      try {
        await tauriInvoke("set_api_secret", { name: "OPENAI_API_KEY", value: openaiKey.trim() });
        nextOpenAiState = "updated";
      } catch {
        nextOpenAiState = "error";
        hasSecretError = true;
      }
    }

    setNexaraSecretState(nextNexaraState);
    setOpenaiSecretState(nextOpenAiState);

    if (hasSecretError) {
      setStatus("error: не удалось сохранить один или несколько ключей");
      return;
    }

    if (nextNexaraState === "updated") setNexaraKey("");
    if (nextOpenAiState === "updated") setOpenaiKey("");
    setStatus("keys_saved");
  }

  async function saveSettings() {
    if (!settings) return;
    if (settingsErrors.length === 0) {
      await tauriInvoke("save_public_settings", { payload: settings });
      setSavedSettingsSnapshot(settings);
    }
    await saveApiKeys();
    setStatus(settingsErrors.length > 0 ? "error: исправьте настройки перед сохранением" : "settings_saved");
  }

  async function saveSettingsPatch(patch: Partial<PublicSettings>) {
    const base = settings ?? (await tauriInvoke<PublicSettings>("get_settings"));
    const next = { ...base, ...patch };
    setSettings(next);
    await tauriInvoke("save_public_settings", { payload: next });
  }

  useEffect(() => {
    void loadSettings().catch(() => undefined);
  }, []);

  useEffect(() => {
    if (isTrayWindow) {
      void loadAudioDevices().catch(() => undefined);
      return;
    }
    if (settingsTab !== "audio" || audioDevices.length > 0) return;
    void loadAudioDevices().catch(() => undefined);
  }, [audioDevices.length, isTrayWindow, settingsTab]);

  return {
    audioDevices,
    autoDetectSystemSource,
    canSaveSettings,
    loadAudioDevices,
    loadSettings,
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
  };
}
