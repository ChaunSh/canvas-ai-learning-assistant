import { describe, expect, it } from "vitest";

import { translate, translationKeys } from "@/i18n";

describe("i18n dictionaries", () => {
  it("keeps Chinese and English keys in sync", () => {
    const zh = translationKeys("zh");
    const en = translationKeys("en");
    const missingInEn = zh.filter((key) => !en.includes(key));
    const missingInZh = en.filter((key) => !zh.includes(key));
    expect(missingInEn).toEqual([]);
    expect(missingInZh).toEqual([]);
  });

  it("has no empty translations", () => {
    for (const locale of ["zh", "en"] as const) {
      for (const key of translationKeys(locale)) {
        expect(translate(locale, key).trim().length, `${locale}:${key}`).toBeGreaterThan(0);
      }
    }
  });
});

describe("translate", () => {
  it("interpolates named variables", () => {
    expect(translate("zh", "reader.page", { n: 7 })).toBe("第 7 页");
    expect(translate("en", "reader.slide", { n: 3 })).toBe("Slide 3");
  });

  it("falls back to the key when missing", () => {
    expect(translate("en", "does.not.exist")).toBe("does.not.exist");
  });

  it("falls back to Chinese for an unknown locale", () => {
    // Guards against a corrupted persisted locale value.
    expect(translate("fr" as "zh", "nav.home")).toBe("首页");
  });
});
