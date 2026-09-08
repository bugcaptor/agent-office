import { describe, expect, it } from "vitest";
import { resources, SOURCE_LANGUAGE, SUPPORTED_LANGUAGES } from "@shared/i18n/catalog";

function keys(value: unknown, prefix = ""): string[] {
  if (!value || typeof value !== "object") return [prefix];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    keys(child, prefix ? `${prefix}.${key}` : key),
  );
}

describe("terminal automation locale integrity", () => {
  it("keeps every automation key available in all supported terminal catalogs", () => {
    const source = keys(resources[SOURCE_LANGUAGE].terminal.automation).sort();
    for (const language of SUPPORTED_LANGUAGES) {
      expect(keys(resources[language].terminal.automation).sort()).toEqual(source);
    }
  });
});
