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
  });

  it("formats session statuses to russian", () => {
    expect(formatSessionStatus("done")).toBe("готово");
    expect(formatSessionStatus("failed")).toBe("ошибка");
    expect(formatSessionStatus("unknown_status")).toBe("unknown_status");
  });
});
