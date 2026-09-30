// src/renderer/analytics/__tests__/pricing.test.ts
//
// pricing.ts 순수 함수 검증: 패턴 매칭 우선순위(구체 패턴이 먼저), 미지/부재
// 모델 → null, 캐시 단가가 반영된 총액, 표시 포맷 경계(자릿수·K/M 축약).
import { describe, expect, it } from "vitest";
import {
  estimateCostBreakdown,
  estimateCostUsd,
  formatTokenPair,
  formatTokens,
  formatUsd,
  rateFor,
} from "../pricing";

describe("rateFor", () => {
  it("Claude 기존 모델과 날짜가 붙은 ID를 구분한다", () => {
    expect(rateFor("claude-opus-4-5-20250929")).toEqual({
      input: 5,
      output: 25,
      cacheRead: 0.5,
      cacheWrite: 6.25,
    });
    expect(rateFor("claude-fable-5")?.input).toBe(10);
    expect(rateFor("claude-sonnet-4-5")?.output).toBe(15);
    expect(rateFor("claude-3-5-haiku-20241022")).toEqual({
      input: 0.8, output: 4, cacheRead: 0.08, cacheWrite: 1,
    });
  });

  it.each([
    ["claude-opus-5-5", { input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5 }],
    ["claude-sonnet-5-5", { input: 2, output: 10, cacheRead: 0.2, cacheWrite: 2.5 }],
    ["claude-sonnet-5", { input: 2, output: 10, cacheRead: 0.2, cacheWrite: 2.5 }],
    ["claude-fable-5-1", { input: 10, output: 50, cacheRead: 0.25, cacheWrite: 12.5 }],
    ["claude-mythos-5-1", { input: 10, output: 50, cacheRead: 0.25, cacheWrite: 12.5 }],
    ["claude-opus-4-1", { input: 15, output: 75, cacheRead: 1.5, cacheWrite: 18.75 }],
  ])("Claude %s의 버전별 단가를 적용한다", (model, rate) => {
    expect(rateFor(model)).toEqual(rate);
  });

  it("같은 계열의 이전 버전에 새 모델 가격을 적용하지 않는다", () => {
    expect(rateFor("claude-opus-5")?.input).toBe(5);
    expect(rateFor("claude-opus-4-8")?.input).toBe(5);
    expect(rateFor("claude-sonnet-4-6")?.input).toBe(3);
    expect(rateFor("claude-fable-5")?.cacheRead).toBe(1);
    expect(rateFor("claude-mythos-5")?.cacheRead).toBe(1);
    expect(rateFor("claude-haiku-4-5")?.input).toBe(1);
  });

  it("공급자 접두사와 점 버전도 같은 Claude 모델로 연결한다", () => {
    expect(rateFor("anthropic/claude-opus-5.5")).toEqual(rateFor("claude-opus-5-5"));
    expect(rateFor("us.anthropic.claude-sonnet-5-5")).toEqual(rateFor("claude-sonnet-5-5"));
    expect(rateFor("anthropic.claude-opus-4-6-v1:0")).toEqual(rateFor("claude-opus-4-6"));
    expect(rateFor("anthropic.claude-sonnet-4-5-20250929-v1:0")).toEqual(rateFor("claude-sonnet-4-5"));
    expect(rateFor("claude-haiku-4-5@20251001")).toEqual(rateFor("claude-haiku-4-5"));
  });

  it("대소문자를 가리지 않는다", () => {
    expect(rateFor("Claude-Opus-5")).toEqual(rateFor("claude-opus-5"));
    expect(rateFor("GPT-5.4")).toEqual(rateFor("gpt-5.4"));
  });

  it("OpenAI GPT 모델별 공식 단가와 날짜 스냅샷을 정확히 구분한다", () => {
    expect(rateFor("gpt-5.5-2026-04-23")).toEqual({
      input: 5,
      output: 30,
      cacheRead: 0.5,
      cacheWrite: 5,
    });
    expect(rateFor("gpt-5.6")).toEqual(rateFor("gpt-5.6-sol"));
    expect(rateFor("gpt-5.6-terra")).toEqual({
      input: 2,
      output: 12,
      cacheRead: 0.2,
      cacheWrite: 2.5,
    });
    expect(rateFor("gpt-5.6-luna")).toEqual({
      input: 0.2,
      output: 1.2,
      cacheRead: 0.02,
      cacheWrite: 0.25,
    });
    expect(rateFor("gpt-6-astra")).toEqual({
      input: 10,
      output: 50,
      cacheRead: 1,
      cacheWrite: 12.5,
    });
    expect(rateFor("gpt-5.4-mini-2026-03-17")?.output).toBe(4.5);
    expect(rateFor("gpt-5.4-nano")?.input).toBe(0.2);
    expect(rateFor("gpt-5.3-codex")?.output).toBe(14);
    expect(rateFor("gpt-5.2-codex")?.input).toBe(1.75);
    expect(rateFor("gpt-5.2-pro-2025-12-11")).toEqual({
      input: 21,
      output: 168,
      cacheRead: 21,
      cacheWrite: 21,
    });
    expect(rateFor("gpt-5.4-pro-2026-03-05")?.output).toBe(180);
    expect(rateFor("gpt-5.1-codex-mini")?.output).toBe(2);
    expect(rateFor("gpt-5-pro")?.output).toBe(120);
    expect(rateFor("gpt-5.5-pro")?.input).toBe(30);
  });

  it.each([
    ["gpt-6-sol", { input: 2, output: 10, cacheRead: 0.2, cacheWrite: 2.5 }],
    ["gpt-6.1-sol", { input: 2, output: 10, cacheRead: 0.1, cacheWrite: 2.5 }],
    ["gpt-6-luna", { input: 0.1, output: 0.5, cacheRead: 0.01, cacheWrite: 0.125 }],
  ])("Codex %s의 입력·출력·캐시 단가를 적용한다", (model, rate) => {
    expect(rateFor(model)).toEqual(rate);
    expect(rateFor(` ${model.toUpperCase()} `)).toEqual(rate);
  });

  it("더 구체적인 패턴이 일반 패턴보다 먼저 잡힌다", () => {
    // "gemini-2.5-pro"가 "gemini"보다 표에서 위에 있어야 한다.
    expect(rateFor("gemini-2.5-pro")?.input).toBe(1.25);
    expect(rateFor("gemini-2.0-flash")?.input).toBe(0.3);
  });

  it("agy 훅 modelName 꼴의 Gemini 3.x는 캐치올이 아닌 전용 요율을 쓴다", () => {
    const flashRate = {
      input: 0.75,
      output: 3.75,
      cacheRead: 0.075,
      cacheWrite: 0.75,
    };
    const proRate = { input: 2, output: 12, cacheRead: 0.2, cacheWrite: 2 };

    // 실측 modelName "gemini-3.8-flash-medium"(kbm #2se).
    expect(rateFor("gemini-3.8-flash-medium")).toEqual(flashRate);
    // 마이너 버전 없는 pro도, 마이너 버전이 낀 pro("gemini-3.1-pro-preview"
    // 같은 꼴)도 고정 패턴이 아니라 "gemini-3" + "pro" 포함 여부로 갈라야
    // 둘 다 잡힌다.
    expect(rateFor("gemini-3-pro-high")).toEqual(proRate);
    expect(rateFor("gemini-3.1-pro-preview")).toEqual(proRate);
    expect(rateFor("gemini-3.5-pro-low")).toEqual(proRate);
    // pro가 없는 나머지 3.x(마이너 버전 무관)는 전부 Flash 요율.
    expect(rateFor("gemini-3.5-flash-medium")).toEqual(flashRate);
    expect(rateFor("gemini-3-flash-high")).toEqual(flashRate);
  });

  it("모델 없음/미지 모델은 null", () => {
    expect(rateFor(undefined)).toBeNull();
    expect(rateFor("")).toBeNull();
    expect(rateFor("llama-3")).toBeNull();
    expect(rateFor("gpt-5.7")).toBeNull();
    expect(rateFor("gpt-6-astra-2026-08-01")).toBeNull();
    expect(rateFor("gpt-5.4-experimental")).toBeNull();
    expect(rateFor("my-gpt-5.4-wrapper")).toBeNull();
    expect(rateFor("gpt-6.2-sol")).toBeNull();
    expect(rateFor("gpt-6.1-sol-experimental")).toBeNull();
    expect(rateFor("gpt-6-sol-2026-09-01")).toBeNull();
    expect(rateFor("claude-opus-5-6")).toBeNull();
    expect(rateFor("claude-sonnet-6")).toBeNull();
    expect(rateFor("claude-fable-5-10")).toBeNull();
    expect(rateFor("claude-opus-5-5-experimental")).toBeNull();
    expect(rateFor("my-opus-wrapper")).toBeNull();
    expect(rateFor("constructor")).toBeNull();
    expect(rateFor("toString")).toBeNull();
  });
});

describe("estimateCostUsd", () => {
  it("신구 Codex·Claude가 섞여도 모델별 캐시 비용을 계산하고 최상위를 중복 합산하지 않는다", () => {
    const tokens = {
      input: 5_000_000,
      model: "gpt-6-astra",
      byModel: [
        { model: "gpt-6.1-sol", input: 1_000_000, output: 1_000_000, cacheRead: 1_000_000, cacheWrite: 1_000_000 },
        { model: "gpt-6-sol", cacheRead: 1_000_000 },
        { model: "gpt-6-luna", input: 1_000_000, output: 1_000_000, cacheRead: 1_000_000, cacheWrite: 1_000_000 },
        { model: "claude-opus-5-5", input: 1_000_000, output: 1_000_000, cacheRead: 1_000_000, cacheWrite: 1_000_000 },
        { model: "claude-sonnet-5-5", input: 1_000_000, output: 1_000_000, cacheRead: 1_000_000, cacheWrite: 1_000_000 },
        { model: "claude-fable-5-1", cacheRead: 1_000_000 },
        { model: "claude-fable-5", cacheRead: 1_000_000 },
      ],
    };
    // 14.6 + 0.2 + 0.735 + 29.2 + 14.7 + 0.25 + 1
    expect(estimateCostUsd(tokens)).toBeCloseTo(60.685, 10);
    expect(estimateCostBreakdown(tokens)?.hasUnknown).toBe(false);
  });

  it("새 모델과 미지 Claude 버전이 섞이면 알려진 비용만 보존한다", () => {
    const tokens = { byModel: [
      { model: "gpt-6.1-sol", input: 1_000_000 },
      { model: "claude-opus-5-6", input: 1_000_000 },
    ] };
    expect(estimateCostBreakdown(tokens)).toEqual({ costUsd: 2, hasUnknown: true });
    expect(estimateCostUsd(tokens)).toBeNull();
  });

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

  it("모델별 내역이 있으면 최상위 합계를 더하지 않는다", () => {
    const tokens = {
      input: 1_000_000,
      output: 1_000_000,
      model: "gpt-6-astra",
      byModel: [
        { input: 1_000_000, model: "gpt-5.6-sol" },
        { output: 1_000_000, model: "gpt-5.6-terra" },
      ],
    };
    expect(estimateCostBreakdown(tokens)).toEqual({ costUsd: 16, hasUnknown: false });
    expect(estimateCostUsd(tokens)).toBe(16);
  });

  it("알 수 없는 모델이 섞여도 알려진 모델 비용과 불명 상태를 모두 보존한다", () => {
    const tokens = {
      byModel: [
        { input: 1_000_000, model: "gpt-5.6-sol" },
        { output: 1_000_000, model: "gpt-5.7" },
      ],
    };
    expect(estimateCostBreakdown(tokens)).toEqual({ costUsd: 4, hasUnknown: true });
    expect(estimateCostUsd(tokens)).toBeNull();
  });

  it("0 토큰인 알려진 구성 모델은 비용 0으로 완료 처리한다", () => {
    expect(estimateCostBreakdown({ byModel: [{ model: "gpt-6-astra" }] })).toEqual({
      costUsd: 0,
      hasUnknown: false,
    });
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

describe("formatTokenPair", () => {
  it("캐시가 있으면 순수 입출력 뒤에 공백·CH 표식·괄호로 전체를 병기한다", () => {
    expect(formatTokenPair(11_700, 1_680_000)).toBe("11.7K CH(1.7M)");
  });

  it("전체가 0이면 —", () => {
    expect(formatTokenPair(0, 0)).toBe("—");
  });

  it("캐시가 없어 순수·전체가 같으면 괄호 없이 단독 표기", () => {
    expect(formatTokenPair(500, 500)).toBe(formatTokens(500));
    expect(formatTokenPair(500, 500)).not.toContain("CH");
  });

  it("캐시만 있는 턴(순수 입출력 0)은 0 CH(전체)로 정직하게 그린다", () => {
    expect(formatTokenPair(0, 500)).toBe("0 CH(500)");
  });
});
