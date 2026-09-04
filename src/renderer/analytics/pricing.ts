// src/renderer/analytics/pricing.ts
//
// 모델 ID → 공개 API 요율($/Mtok) 단가표와 비용 환산 순수 함수.
// aggregate.ts와 같은 결의 순수 모듈이라 vitest로 값싸게 경계를 검증한다.
//
// ⚠️ 여기 숫자는 **대표값**이며 실제 청구액이 아니다. 구간별 장문(long-context)
// 할증, 배치/프로모션 할인, 계약 단가, 구독제(정액) 사용은 반영하지 않는다.
// 화면에도 "추정치"라고 못박아 표시한다. 요율이 바뀌면 아래 RATES 표만 고치면
// 되고, 집계·표시 코드는 손댈 필요가 없다.
//
// 표 기준: 2026-09-05 시점 Anthropic 공식 API 요율 + OpenAI/Google 공개 요율.
// Anthropic 계열의 캐시 단가는 규칙이 일정하다 —
//   cacheRead = input × 0.1, cacheWrite(5분 TTL) = input × 1.25.
// 그 규칙으로 계산한 값을 상수로 박아 둔다(런타임 곱셈 없이 표만 읽으면 되게).
import type { SessionEventTokens, SessionModelTokens } from "@shared/types";

/** 1M 토큰당 달러 단가 4종. */
export interface ModelRate {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

/**
 * 모델 ID 부분문자열 패턴 → 요율. **위에서 아래로 첫 일치**를 쓰므로 더
 * 구체적인 패턴을 위에 둔다(예: "gemini-2.5-pro"가 "gemini"보다 위).
 * 매칭은 소문자화한 모델 ID에 대한 `includes`다.
 */
const RATES: ReadonlyArray<readonly [pattern: string, rate: ModelRate]> = [
  ["fable", { input: 10, output: 50, cacheRead: 1.0, cacheWrite: 12.5 }],
  ["mythos", { input: 10, output: 50, cacheRead: 1.0, cacheWrite: 12.5 }],
  ["opus", { input: 5, output: 25, cacheRead: 0.5, cacheWrite: 6.25 }],
  ["sonnet", { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 }],
  ["haiku", { input: 1, output: 5, cacheRead: 0.1, cacheWrite: 1.25 }],
  ["gpt-4.1", { input: 2, output: 8, cacheRead: 0.5, cacheWrite: 2 }],
  ["gemini-2.5-pro", { input: 1.25, output: 10, cacheRead: 0.31, cacheWrite: 1.25 }],
  ["gemini", { input: 0.3, output: 2.5, cacheRead: 0.075, cacheWrite: 0.3 }],
];

/**
 * OpenAI GPT-5 계열은 이름만 비슷해도 모델별 단가가 다르다. 따라서 일반
 * `gpt-5` 부분문자열은 쓰지 않고, 공식 모델 ID(및 날짜 스냅샷)만 정확히
 * 허용한다. 출처(2026-09-05):
 * https://developers.openai.com/api/docs/models/gpt-5,
 * https://developers.openai.com/api/docs/models/gpt-5.4,
 * https://developers.openai.com/api/docs/models/gpt-5.5,
 * https://developers.openai.com/api/docs/models/gpt-5.6-sol,
 * https://developers.openai.com/api/docs/models/compare.
 * 캐시 기록은 5.6/6 Astra 문서에서 명시한 1.25배만 적용하고, 이전 모델은
 * 별도 단가가 공개되지 않아 입력 단가를 보수적으로 쓴다. Pro는 캐시 할인을
 * 제공하지 않으므로 `cacheRead`도 입력 단가다.
 */
const OPENAI_RATES: Readonly<Record<string, ModelRate>> = {
  "gpt-5": { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 },
  "gpt-5-codex": { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 },
  "gpt-5-pro": { input: 15, output: 120, cacheRead: 15, cacheWrite: 15 },
  "gpt-5-mini": { input: 0.25, output: 2, cacheRead: 0.025, cacheWrite: 0.25 },
  "gpt-5-nano": { input: 0.05, output: 0.4, cacheRead: 0.005, cacheWrite: 0.05 },
  "gpt-5.1": { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 },
  "gpt-5.1-codex": { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 },
  "gpt-5.1-codex-max": { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 },
  "gpt-5.1-codex-mini": { input: 0.25, output: 2, cacheRead: 0.025, cacheWrite: 0.25 },
  "gpt-5.2": { input: 1.75, output: 14, cacheRead: 0.175, cacheWrite: 1.75 },
  "gpt-5.2-codex": { input: 1.75, output: 14, cacheRead: 0.175, cacheWrite: 1.75 },
  "gpt-5.2-pro": { input: 21, output: 168, cacheRead: 21, cacheWrite: 21 },
  "gpt-5.3-codex": { input: 1.75, output: 14, cacheRead: 0.175, cacheWrite: 1.75 },
  "gpt-5.4": { input: 2.5, output: 15, cacheRead: 0.25, cacheWrite: 2.5 },
  "gpt-5.4-pro": { input: 30, output: 180, cacheRead: 30, cacheWrite: 30 },
  "gpt-5.4-mini": { input: 0.75, output: 4.5, cacheRead: 0.075, cacheWrite: 0.75 },
  "gpt-5.4-nano": { input: 0.2, output: 1.25, cacheRead: 0.02, cacheWrite: 0.2 },
  "gpt-5.5": { input: 5, output: 30, cacheRead: 0.5, cacheWrite: 5 },
  "gpt-5.5-pro": { input: 30, output: 180, cacheRead: 30, cacheWrite: 30 },
  "gpt-5.6": { input: 4, output: 20, cacheRead: 0.4, cacheWrite: 5 },
  "gpt-5.6-sol": { input: 4, output: 20, cacheRead: 0.4, cacheWrite: 5 },
  "gpt-5.6-terra": { input: 2, output: 12, cacheRead: 0.2, cacheWrite: 2.5 },
  "gpt-5.6-luna": { input: 0.2, output: 1.2, cacheRead: 0.02, cacheWrite: 0.25 },
  "gpt-6-astra": { input: 10, output: 50, cacheRead: 1, cacheWrite: 12.5 },
};

/** 공식 문서에 명시된 날짜 스냅샷만 위의 모델 단가에 연결한다. */
const OPENAI_SNAPSHOT_BASES: Readonly<Record<string, keyof typeof OPENAI_RATES>> = {
  "gpt-5-2025-08-07": "gpt-5",
  "gpt-5-mini-2025-08-07": "gpt-5-mini",
  "gpt-5-nano-2025-08-07": "gpt-5-nano",
  "gpt-5-pro-2025-10-06": "gpt-5-pro",
  "gpt-5.1-2025-11-13": "gpt-5.1",
  "gpt-5.2-2025-12-11": "gpt-5.2",
  "gpt-5.2-pro-2025-12-11": "gpt-5.2-pro",
  "gpt-5.4-2026-03-05": "gpt-5.4",
  "gpt-5.4-pro-2026-03-05": "gpt-5.4-pro",
  "gpt-5.4-mini-2026-03-17": "gpt-5.4-mini",
  "gpt-5.4-nano-2026-03-17": "gpt-5.4-nano",
  "gpt-5.5-2026-04-23": "gpt-5.5",
  "gpt-5.5-pro-2026-04-23": "gpt-5.5-pro",
};

// TS target lacks Object.hasOwn, so retain the same own-property semantics.
function hasOwn(object: object, key: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(object, key);
}

function openAiRateFor(model: string): ModelRate | null {
  const base = hasOwn(OPENAI_SNAPSHOT_BASES, model)
    ? OPENAI_SNAPSHOT_BASES[model]
    : model;
  return hasOwn(OPENAI_RATES, base) ? OPENAI_RATES[base] : null;
}

/**
 * 모델 ID에 맞는 요율. 모델이 없거나 표에 없는 모델이면 `null`
 * (→ 비용 집계에서 제외하고 "단가를 모르는 사용량"으로 따로 센다).
 */
export function rateFor(model: string | undefined): ModelRate | null {
  if (!model) return null;
  const key = model.toLowerCase().trim();
  const openAiRate = openAiRateFor(key);
  if (openAiRate) return openAiRate;
  for (const [pattern, rate] of RATES) {
    if (key.includes(pattern)) return rate;
  }
  return null;
}

/** 비용을 일부 계산할 수 있어도, 단가를 모르는 모델이 있었는지 함께 나타낸다. */
export interface CostBreakdown {
  costUsd: number;
  hasUnknown: boolean;
}

function costForSingleModel(tokens: SessionModelTokens): number | null {
  const r = rateFor(tokens.model);
  if (!r) return null;
  const cost =
    (tokens.input ?? 0) * r.input +
    (tokens.output ?? 0) * r.output +
    (tokens.cacheRead ?? 0) * r.cacheRead +
    (tokens.cacheWrite ?? 0) * r.cacheWrite;
  return cost / 1_000_000;
}

/**
 * 한 턴의 비용과 단가를 모르는 구성 모델 존재 여부를 함께 반환한다. 모델별
 * 내역이 있으면 그 합계만 쓴다. 최상위 토큰은 구형 소비자용 총계이므로
 * 이 경우 다시 더하면 이중 집계가 된다.
 */
export function estimateCostBreakdown(
  tokens: SessionEventTokens | undefined,
): CostBreakdown | null {
  if (!tokens) return null;
  const byModel = tokens.byModel;
  if (byModel?.length) {
    let costUsd = 0;
    let hasUnknown = false;
    for (const component of byModel) {
      const cost = costForSingleModel(component);
      if (cost === null) hasUnknown = true;
      else costUsd += cost;
    }
    return { costUsd, hasUnknown };
  }
  const costUsd = costForSingleModel(tokens);
  return costUsd === null ? { costUsd: 0, hasUnknown: true } : { costUsd, hasUnknown: false };
}

/**
 * 한 턴의 토큰 사용량 → 완전한 추정 비용(USD). 토큰이 없거나 하나라도 단가를
 * 모르면 `null`이다. 일부 비용이 필요하면 `estimateCostBreakdown`을 쓴다.
 */
export function estimateCostUsd(tokens: SessionEventTokens | undefined): number | null {
  const breakdown = estimateCostBreakdown(tokens);
  return !breakdown || breakdown.hasUnknown ? null : breakdown.costUsd;
}

/**
 * 표 셀용 달러 표기. 아주 작은 값이 `$0.00`으로 뭉개지지 않게 자릿수를
 * 크기에 따라 늘린다: ≥1 → 2자리, ≥0.01 → 3자리, 그 외 4자리.
 */
export function formatUsd(usd: number): string {
  const abs = Math.abs(usd);
  const digits = abs >= 1 ? 2 : abs >= 0.01 ? 3 : 4;
  return `$${usd.toFixed(digits)}`;
}

/** 토큰 수 축약 표기(한국어 UI지만 K/M은 관용대로 유지). 1234567 → "1.2M". */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(Math.round(n));
}

/**
 * 토큰 셀 표기 — `11.7K CH(1.7M)`처럼 **순수 입출력과 캐시 포함 전체를
 * 병기**한다: 순수 입출력 뒤에 공백, 캐시(Cache)를 뜻하는 `CH` 표식, 그
 * 뒤 괄호 안에 캐시까지 넣은 전체. 요약 바와 분석 패널이 같은 함수를 쓴다
 * (두 화면이 같은 숫자를 다르게 그리지 않게).
 *
 * 왜 병기인가: 예전에는 두 화면 모두 `input + output`만 그렸는데, 프롬프트
 * 캐시가 켜진 요즘 세션에서는 그 둘이 전체의 1% 남짓이라(실측: 11.7K /
 * 1.68M = 0.7%) 셀이 사실상 "거의 0"으로 보였다. 게다가 바로 옆 비용 셀은
 * `estimateCostUsd`가 캐시까지 넣어 계산하므로 두 셀의 기준이 서로 어긋나
 * 있었다 — 괄호 안 숫자가 그 비용과 같은 기준이다.
 *
 * 처음엔 `11.7K(1.7M)`처럼 괄호를 숫자에 바로 붙였는데, 그러면 두 숫자가
 * 시각적으로 뭉쳐 읽기 어렵다는 지적을 받아 공백과 `CH` 표식을 더했다(가독성
 * 개선, 병기 자체의 뜻은 그대로).
 *
 * - 전체가 0이면 `—`(집계된 토큰이 없다).
 * - 캐시가 없어 둘이 같으면 괄호(와 `CH` 표식)를 붙이지 않는다 — 같은 숫자를
 *   두 번 쓰는 셈이라 폭만 먹는다.
 * - 캐시만 있는 턴은 `0 CH(500)`으로 정직하게 그린다. 예전에 이걸 `—`로
 *   떨궜던 건 캐시가 셀에서 통째로 안 보였기 때문이고, 지금은 괄호가 그
 *   값을 보여준다.
 *
 * 괄호의 뜻은 호출부가 툴팁에 `tokenPairHint`로 붙인다(괄호가 붙었을 때만).
 */
export function formatTokenPair(net: number, gross: number): string {
  if (gross <= 0) return "—";
  if (net === gross) return formatTokens(net);
  return `${formatTokens(net)} CH(${formatTokens(gross)})`;
}
