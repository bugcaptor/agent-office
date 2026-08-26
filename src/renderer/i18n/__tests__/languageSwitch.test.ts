// i18next 인스턴스가 카탈로그의 모든 언어 코드를 실제로 물 수 있는지 본다.
// 파리티 테스트(shared)는 JSON끼리만 대조하므로 **런타임에서 코드가 안 먹는**
// 경우를 못 잡는다 — `zh-Hans`처럼 문자 부표가 붙은 코드는 i18next의
// supportedLngs·언어 정규화를 통과해야 비로소 화면에 나온다.
import { afterEach, describe, expect, it } from "vitest";

import { SOURCE_LANGUAGE, SUPPORTED_LANGUAGES, i18n, initI18nForTest } from "..";

afterEach(async () => {
  await initI18nForTest(SOURCE_LANGUAGE);
});

describe("언어 전환", () => {
  it("모든 지원 언어로 전환되고 그 언어의 문구가 나온다", async () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      await i18n.changeLanguage(lang);
      const label = i18n.t("common:_meta.label");
      const value = i18n.t("common:bottomBar.settingsAria");
      // 언어가 안 먹으면 정본(ko)으로 폴백돼 `_meta.label`이 "한국어"로 나온다.
      expect({ lang, resolved: i18n.resolvedLanguage, label }).toEqual({
        lang,
        resolved: lang,
        label: i18n.getResource(lang, "common", "_meta.label"),
      });
      expect({ lang, missing: value === "common:bottomBar.settingsAria" }).toEqual({
        lang,
        missing: false,
      });
    }
  });
});
