import { SecretSaveState } from "../appTypes";

export function formatSecretSaveState(state: SecretSaveState): string {
  if (state === "updated") return "обновлён";
  if (state === "unchanged") return "не изменён";
  if (state === "error") return "ошибка";
  return "";
}

export function splitParticipants(value: string): string[] {
  return value
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);
}

export function parseEventPayload<T>(event: unknown): T | null {
  if (!event || typeof event !== "object") return null;
  const candidate = event as { payload?: unknown };
  const payload = candidate.payload ?? event;
  if (typeof payload === "string") {
    try {
      return JSON.parse(payload) as T;
    } catch {
      return null;
    }
  }
  if (payload && typeof payload === "object") {
    return payload as T;
  }
  return null;
}

export function getErrorMessage(value: unknown): string {
  if (value instanceof Error) return value.message;
  return String(value);
}

export function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}
