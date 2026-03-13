const APP_STATUS_RU: Record<string, string> = {
  idle: "ожидание",
  recording: "идет запись",
  recorded: "запись завершена",
  done: "обработка завершена",
  session_deleted: "сессия удалена",
  session_details_autosaved: "детали сессии автосохранены",
  settings_saved: "настройки сохранены",
  keys_saved: "ключи сохранены",
  system_source_not_detected: "системный источник не найден",
};

const SESSION_STATUS_RU: Record<string, string> = {
  idle: "ожидание",
  recording: "идет запись",
  recorded: "записано",
  transcribed: "транскрибировано",
  done: "готово",
  failed: "ошибка",
};

export function formatAppStatus(status: string): string {
  if (status.startsWith("system_source_detected:")) {
    const name = status.slice("system_source_detected:".length).trim();
    return name ? `системный источник: ${name}` : "системный источник найден";
  }
  if (status.startsWith("error:")) {
    const message = status.slice("error:".length).trim();
    return message ? `ошибка: ${message}` : "ошибка";
  }
  return APP_STATUS_RU[status] ?? status;
}

export function formatSessionStatus(status: string): string {
  return SESSION_STATUS_RU[status] ?? status;
}
