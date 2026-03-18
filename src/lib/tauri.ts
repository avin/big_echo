import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  if (typeof args === "undefined") {
    return invoke<T>(cmd);
  }
  return invoke<T>(cmd, args as Record<string, unknown>);
}

export function tauriEmit(event: string, payload?: unknown): Promise<void> {
  return emit(event, payload);
}

export function tauriListen(
  event: string,
  handler: (event: unknown) => void | Promise<void>
): Promise<() => void> {
  return listen(event, handler as Parameters<typeof listen>[1]);
}

export function getCurrentWindowLabel(): string {
  return getCurrentWindow().label;
}
