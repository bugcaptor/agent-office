// src/renderer/usage/__tests__/usageView.test.ts
//
// 사용량 표시 순수 함수 테스트(docs/usage-limits-design.md §4): 절박 윈도
// 선택, 임계 색상, 카운트다운·신선도 포맷, stale 판정.

import { describe, expect, it } from "vitest";
import type { ProviderUsage, UsageWindow } from "@shared/types";
import {
  STALE_THRESHOLD_MS,
  formatCountdown,
  formatFreshness,
  isStale,
  mostUrgentWindow,
  usageLevel,
  windowLabel,
} from "../usageView";

function win(partial: Partial<UsageWindow>): UsageWindow {
  return {
    kind: "session",
    label: null,
    usedPercent: 0,
    resetsAtMs: null,
    windowMinutes: null,
    ...partial,
  };
}

function provider(windows: UsageWindow[]): ProviderUsage {
  return { provider: "claude", fetchedAtMs: 0, planLabel: null, windows };
}

describe("usageLevel 임계 70/90", () => {
  it("<70 = normal", () => {
    expect(usageLevel(0)).toBe("normal");
    expect(usageLevel(69.9)).toBe("normal");
  });
  it(">=70 = warn", () => {
    expect(usageLevel(70)).toBe("warn");
    expect(usageLevel(89.9)).toBe("warn");
  });
  it(">=90 = danger", () => {
    expect(usageLevel(90)).toBe("danger");
    expect(usageLevel(100)).toBe("danger");
  });
});

describe("mostUrgentWindow", () => {
  it("usedPercent 최대 윈도를 고른다", () => {
    const u = provider([
      win({ kind: "session", usedPercent: 61 }),
      win({ kind: "weekly", usedPercent: 18 }),
      win({ kind: "weekly_model", usedPercent: 24 }),
    ]);
    expect(mostUrgentWindow(u)?.kind).toBe("session");
  });

  it("동률이면 먼저 나온 윈도를 유지한다", () => {
    const u = provider([
      win({ kind: "weekly", usedPercent: 50 }),
      win({ kind: "session", usedPercent: 50 }),
    ]);
    expect(mostUrgentWindow(u)?.kind).toBe("weekly");
  });

  it("null/빈 윈도는 null", () => {
    expect(mostUrgentWindow(null)).toBeNull();
    expect(mostUrgentWindow(provider([]))).toBeNull();
  });
});

describe("windowLabel", () => {
  it("종류별 한국어 라벨", () => {
    expect(windowLabel(win({ kind: "session" }))).toBe("5시간");
    expect(windowLabel(win({ kind: "weekly" }))).toBe("주간");
    expect(windowLabel(win({ kind: "weekly_model", label: "Fable" }))).toBe("주간 · Fable");
    expect(windowLabel(win({ kind: "weekly_model", label: null }))).toBe("주간 (모델별)");
    expect(windowLabel(win({ kind: "unknown", windowMinutes: 1440 }))).toBe("1440분 창");
    expect(windowLabel(win({ kind: "unknown", windowMinutes: null }))).toBe("기타");
  });
});

describe("formatCountdown", () => {
  const NOW = 1_784_000_000_000;
  it("resetsAtMs null이면 빈 문자열", () => {
    expect(formatCountdown(null, NOW)).toBe("");
  });
  it("이미 지났으면 리셋 대기 중", () => {
    expect(formatCountdown(NOW - 1000, NOW)).toBe("리셋 대기 중");
    expect(formatCountdown(NOW, NOW)).toBe("리셋 대기 중");
  });
  it("분 단위", () => {
    expect(formatCountdown(NOW + 45 * 60000, NOW)).toBe("45분 후 리셋");
  });
  it("시간+분", () => {
    expect(formatCountdown(NOW + (3 * 60 + 12) * 60000, NOW)).toBe("3시간 12분 후 리셋");
  });
  it("하루 이상은 일+시간", () => {
    expect(formatCountdown(NOW + (2 * 24 * 60 + 5 * 60) * 60000, NOW)).toBe("2일 5시간 후 리셋");
  });
});

describe("formatFreshness / isStale", () => {
  const NOW = 1_784_000_000_000;
  it("1분 미만은 방금 기준", () => {
    expect(formatFreshness(NOW - 30_000, NOW)).toBe("방금 기준");
  });
  it("분 단위", () => {
    expect(formatFreshness(NOW - 14 * 60000, NOW)).toBe("14분 전 기준");
  });
  it("시간+분", () => {
    expect(formatFreshness(NOW - (2 * 60 + 3) * 60000, NOW)).toBe("2시간 3분 전 기준");
  });
  it("하루 이상", () => {
    expect(formatFreshness(NOW - 3 * 24 * 60 * 60000, NOW)).toBe("3일 전 기준");
  });
  it("미래 신선도(시계 밀림)는 방금 기준으로 클램프", () => {
    expect(formatFreshness(NOW + 60_000, NOW)).toBe("방금 기준");
  });
  it("stale 임계는 30분 초과", () => {
    expect(isStale(NOW - STALE_THRESHOLD_MS, NOW)).toBe(false);
    expect(isStale(NOW - STALE_THRESHOLD_MS - 1, NOW)).toBe(true);
  });
});
