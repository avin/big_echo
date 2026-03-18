import { PublicSettings } from "../appTypes";

function isValidHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

export function validateSettings(settings: PublicSettings | null): string[] {
  if (!settings) return [];
  const errors: string[] = [];
  if (settings.transcription_url.trim() && !isValidHttpUrl(settings.transcription_url.trim())) {
    errors.push("Неверный URL транскрибации");
  }
  if (settings.summary_url.trim() && !isValidHttpUrl(settings.summary_url.trim())) {
    errors.push("Неверный URL саммари");
  }
  if (settings.opus_bitrate_kbps < 12 || settings.opus_bitrate_kbps > 128) {
    errors.push("Битрейт Opus должен быть от 12 до 128 kbps");
  }
  return errors;
}
