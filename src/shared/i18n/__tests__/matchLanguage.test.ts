// 로케일 → 보유 카탈로그 해석. 중국어를 간체·번체 둘 다 갖게 되면서 생긴
// 함정을 못 박는다: 프리픽스 일치만으로는 `zh-*`가 전부 정렬상 앞에 오는
// `zh-Hans`로 붙어 대만·홍콩 사용자에게 간체가 나간다.
import { describe, expect, it } from "vitest";

import { FALLBACK_LANGUAGE, matchLanguage } from "../catalog";

describe("matchLanguage", () => {
  it("정확히 일치하는 코드는 그대로 쓴다", () => {
    expect(matchLanguage("ko")).toBe("ko");
    expect(matchLanguage("zh-Hant")).toBe("zh-Hant");
    expect(matchLanguage("zh-hans")).toBe("zh-Hans"); // 대소문자 무시
  });

  it("지역 코드를 문자(script)로 넓혀 중국어를 갈라 준다", () => {
    expect(matchLanguage("zh-TW")).toBe("zh-Hant");
    expect(matchLanguage("zh-HK")).toBe("zh-Hant");
    expect(matchLanguage("zh-MO")).toBe("zh-Hant");
    expect(matchLanguage("zh-CN")).toBe("zh-Hans");
    expect(matchLanguage("zh-SG")).toBe("zh-Hans");
    expect(matchLanguage("zh")).toBe("zh-Hans"); // CLDR 기본 문자
  });

  it("지역만 다른 코드는 프리픽스로 좁힌다", () => {
    expect(matchLanguage("ko-KR")).toBe("ko");
    expect(matchLanguage("en-GB")).toBe("en");
    expect(matchLanguage("ja-JP")).toBe("ja");
    expect(matchLanguage("fr-CA")).toBe("fr");
  });

  it("모르는 언어와 빈 값은 폴백으로 떨어진다", () => {
    expect(matchLanguage("de-DE")).toBe(FALLBACK_LANGUAGE);
    expect(matchLanguage("")).toBe(FALLBACK_LANGUAGE);
    expect(matchLanguage(null)).toBe(FALLBACK_LANGUAGE);
    expect(matchLanguage("!!not-a-locale!!")).toBe(FALLBACK_LANGUAGE);
  });
});
