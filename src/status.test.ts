import { describe, expect, it } from "vitest";

import { formatAppStatus, formatSessionStatus } from "./status";

describe("status formatters", () => {
  it("formats app statuses to russian", () => {
    expect(formatAppStatus("idle")).toBe("ожидание");
    expect(formatAppStatus("recording")).toBe("идет запись");
    expect(formatAppStatus("recorded")).toBe("запись завершена");
    expect(formatAppStatus("settings_saved")).toBe("настройки сохранены");
    expect(formatAppStatus("system_source_detected:BlackHole 2ch")).toBe(
      "системный источник: BlackHole 2ch",
    );
    expect(formatAppStatus("error: Topic is too long (max 200 chars)")).toBe(
      "ошибка: тема слишком длинная (максимум 200 символов)",
    );
  });

  it("formats session statuses to russian", () => {
    expect(formatSessionStatus("done")).toBe("готово");
    expect(formatSessionStatus("failed")).toBe("ошибка");
    expect(formatSessionStatus("unknown_status")).toBe("unknown_status");
  });
});
