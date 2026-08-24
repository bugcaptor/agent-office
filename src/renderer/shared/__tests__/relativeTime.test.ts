// src/renderer/shared/__tests__/relativeTime.test.ts
//
// `formatRelativeTime`의 임계 경계 명세. 이 함수는 문구를
// `Intl.RelativeTimeFormat(currentLocale())`에 맡기므로 검증할 것이 둘이다:
//
//  1. **임계값**(5초 / 60초 / 60분 / 24시간)이 언어와 무관하게 같은 지점에서
//     단위를 바꾼다 — 손조립 시절부터 이어진 계약이라 언어를 늘려도 안 바뀐다.
//  2. **언어를 바꾸면 문구가 따라온다** — 포매터를 모듈 최상위에서 한 번만
//     만들면 이 부분이 조용히 깨진다(그래서 캐시 키가 currentLocale()이다).
//
// 정본(ko)과 en 양쪽을 다 본다. 테스트 UI 언어는 test-setup이 ko로 못박으므로
// en 블록은 스스로 언어를 바꾸고 afterAll에서 되돌린다(파일 간 누수 방지).
import { afterAll, describe, expect, it } from "vitest";

import { initI18nForTest } from "@renderer/i18n";
import { formatRelativeTime } from "../relativeTime";

const NOW = 1_700_000_000_000;
const SEC = 1000;
const MIN = 60 * SEC;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** `ago`밀리초 전에 받아온 데이터의 표시 문구. */
function ago(ms: number): string {
  return formatRelativeTime(NOW - ms, NOW);
}

afterAll(async () => {
  await initI18nForTest(); // 정본(ko)으로 복구
});

describe("formatRelativeTime — 임계 경계(ko)", () => {
  it("5초 미만은 '방금'", () => {
    expect(ago(0)).toBe("방금");
    expect(ago(4 * SEC + 999)).toBe("방금");
  });

  it("5초부터 60초 직전까지는 초 단위", () => {
    expect(ago(5 * SEC)).toBe("5초 전");
    expect(ago(59 * SEC)).toBe("59초 전");
  });

  it("60초부터 60분 직전까지는 분 단위", () => {
    expect(ago(1 * MIN)).toBe("1분 전");
    expect(ago(59 * MIN + 59 * SEC)).toBe("59분 전");
  });

  it("60분부터 24시간 직전까지는 시간 단위", () => {
    expect(ago(1 * HOUR)).toBe("1시간 전");
    expect(ago(23 * HOUR + 59 * MIN)).toBe("23시간 전");
  });

  it("24시간부터는 일 단위", () => {
    // numeric:"auto"라 ko는 하루/이틀 전을 관용 표현으로 부른다.
    expect(ago(1 * DAY)).toBe("1일 전");
    expect(ago(45 * DAY)).toBe("45일 전");
  });

  it("미래 시각(음수 차이)은 '방금'으로 클램프한다", () => {
    expect(formatRelativeTime(NOW + 10 * MIN, NOW)).toBe("방금");
  });
});

describe("formatRelativeTime — 같은 임계값, 영어 문구", () => {
  it("언어를 바꾸면 임계 구간 문구와 Intl 문구가 함께 따라온다", async () => {
    await initI18nForTest("en");

    expect(ago(0)).toBe("just now");
    expect(ago(4 * SEC + 999)).toBe("just now");
    expect(ago(5 * SEC)).toBe("5 seconds ago");
    expect(ago(59 * SEC)).toBe("59 seconds ago");
    expect(ago(1 * MIN)).toBe("1 minute ago");
    expect(ago(59 * MIN + 59 * SEC)).toBe("59 minutes ago");
    expect(ago(1 * HOUR)).toBe("1 hour ago");
    expect(ago(23 * HOUR + 59 * MIN)).toBe("23 hours ago");
    expect(ago(1 * DAY)).toBe("1 day ago");
    expect(ago(45 * DAY)).toBe("45 days ago");
  });
});
