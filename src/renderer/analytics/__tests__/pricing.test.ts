// src/renderer/analytics/__tests__/pricing.test.ts
//
// pricing.ts 순수 함수 검증: 패턴 매칭 우선순위(구체 패턴이 먼저), 미지/부재
// 모델 → null, 캐시 단가가 반영된 총액, 표시 포맷 경계(자릿수·K/M 축약).
import { describe, expect, it } from "vitest";
import { estimateCostUsd, formatTokens, formatUsd, rateFor } from "../pricing";

describe("rateFor", () => {
  it("부분문자열로 실제 모델 ID를 잡는다", () => {
    expect(rateFor("claude-opus-4-5-20250929")).toEqual({
      input: 5,
      output: 25,
      cacheRead: 0.5,
      cacheWrite: 6.25,
    });
    expect(rateFor("claude-fable-5")?.input).toBe(10);
    expect(rateFor("claude-sonnet-4-5")?.output).toBe(15);
    expect(rateFor("claude-3-5-haiku-20241022")?.input).toBe(1);
  });

  it("대소문자를 가리지 않는다", () => {
    expect(rateFor("Claude-Opus-5")).toEqual(rateFor("claude-opus-5"));
    expect(rateFor("GPT-5.4")).toEqual(rateFor("gpt-5.4"));
  });

  it("더 구체적인 패턴이 일반 패턴보다 먼저 잡힌다", () => {
    // "gemini-2.5-pro"가 "gemini"보다 표에서 위에 있어야 한다.
    expect(rateFor("gemini-2.5-pro")?.input).toBe(1.25);
    expect(rateFor("gemini-2.0-flash")?.input).toBe(0.3);
  });

  it("모델 없음/미지 모델은 null", () => {
    expect(rateFor(undefined)).toBeNull();
    expect(rateFor("")).toBeNull();
    expect(rateFor("llama-3")).toBeNull();
  });
});

describe("estimateCostUsd", () => {
  it("캐시 단가까지 반영해 합산한다", () => {
    // opus: in 5 / out 25 / cacheRead 0.5 / cacheWrite 6.25 ($/Mtok)
    const usd = estimateCostUsd({
      input: 1_000_000,
      output: 1_000_000,
      cacheRead: 1_000_000,
      cacheWrite: 1_000_000,
      model: "claude-opus-5",
    });
    expect(usd).toBeCloseTo(5 + 25 + 0.5 + 6.25, 10);
  });

  it("없는 필드는 0으로 친다", () => {
    const usd = estimateCostUsd({ output: 2_000, model: "claude-haiku-4-5" });
    expect(usd).toBeCloseTo((2_000 * 5) / 1_000_000, 12);
  });

  it("tokens 부재/미지 모델/모델 없음은 null", () => {
    expect(estimateCostUsd(undefined)).toBeNull();
    expect(estimateCostUsd({ input: 100, model: "llama-3" })).toBeNull();
    expect(estimateCostUsd({ input: 100 })).toBeNull();
  });

  it("모든 토큰이 0이면 0(모름이 아니다)", () => {
    expect(estimateCostUsd({ model: "claude-opus-5" })).toBe(0);
  });
});

describe("formatUsd", () => {
  it("크기에 따라 자릿수를 늘려 작은 값을 뭉개지 않는다", () => {
    expect(formatUsd(12.3456)).toBe("$12.35");
    expect(formatUsd(1)).toBe("$1.00");
    expect(formatUsd(0.5)).toBe("$0.500");
    expect(formatUsd(0.01)).toBe("$0.010");
    expect(formatUsd(0.0099)).toBe("$0.0099");
    expect(formatUsd(0.00004)).toBe("$0.0000");
    expect(formatUsd(0)).toBe("$0.0000");
  });
});

describe("formatTokens", () => {
  it("K/M로 축약한다", () => {
    expect(formatTokens(1_234_567)).toBe("1.2M");
    expect(formatTokens(12_345)).toBe("12.3K");
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1_000)).toBe("1.0K");
    expect(formatTokens(999_999)).toBe("1000.0K"); // 경계: 1M 미만은 K 유지
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(0)).toBe("0");
  });
});
