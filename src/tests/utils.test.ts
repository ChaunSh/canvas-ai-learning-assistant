import { describe, expect, it } from "vitest";

import {
  errorMessage,
  formatBytes,
  formatDate,
  formatDateTime,
  fromDateTimeInput,
  percent,
  toDateInput,
  truncate,
} from "@/lib/utils";

describe("formatBytes", () => {
  it("handles zero and negatives", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(-5)).toBe("0 B");
  });

  it("scales units", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1024 * 1024 * 3)).toBe("3.0 MB");
  });
});

describe("percent", () => {
  it("clamps and rounds", () => {
    expect(percent(0)).toBe(0);
    expect(percent(0.456)).toBe(46);
    expect(percent(1.5)).toBe(100);
    expect(percent(-1)).toBe(0);
  });
});

describe("truncate", () => {
  it("keeps short text and ellipsizes long text", () => {
    expect(truncate("abc", 10)).toBe("abc");
    expect(truncate("abcdefghij", 5)).toBe("abcde…");
  });
});

describe("date helpers", () => {
  it("formats ISO instants in the requested locale", () => {
    const value = "2026-11-01T02:00:00Z";
    expect(formatDate(value, "zh")).toContain("2026");
    expect(formatDateTime(value, "zh")).toMatch(/\d{2}:\d{2}/);
  });

  it("returns a dash for invalid input instead of throwing", () => {
    expect(formatDate(null)).toBe("—");
    expect(formatDate("not-a-date")).toBe("—");
  });

  it("round-trips datetime-local values through UTC", () => {
    const iso = fromDateTimeInput("2026-11-01T09:30");
    const back = toDateInput(iso);
    expect(back).toBe("2026-11-01");
  });
});

describe("errorMessage", () => {
  it("unwraps Error, string and object shapes", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage("plain")).toBe("plain");
    expect(errorMessage({ message: "obj" })).toBe("obj");
    expect(errorMessage(undefined)).toBe("未知错误");
  });
});
