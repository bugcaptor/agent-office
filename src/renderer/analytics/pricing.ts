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
// 표 기준: 2026-06 시점 Anthropic 공식 API 요율 + OpenAI/Google 공개 요율.
// Anthropic 계열의 캐시 단가는 규칙이 일정하다 —
//   cacheRead = input × 0.1, cacheWrite(5분 TTL) = input × 1.25.
// 그 규칙으로 계산한 값을 상수로 박아 둔다(런타임 곱셈 없이 표만 읽으면 되게).
import type { SessionEventTokens } from "@shared/types";

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
  ["gpt-5", { input: 1.25, output: 10, cacheRead: 0.125, cacheWrite: 1.25 }],
  ["gpt-4.1", { input: 2, output: 8, cacheRead: 0.5, cacheWrite: 2 }],
  ["gemini-2.5-pro", { input: 1.25, output: 10, cacheRead: 0.31, cacheWrite: 1.25 }],
  ["gemini", { input: 0.3, output: 2.5, cacheRead: 0.075, cacheWrite: 0.3 }],
];

/**
 * 모델 ID에 맞는 요율. 모델이 없거나 표에 없는 모델이면 `null`
 * (→ 비용 집계에서 제외하고 "단가를 모르는 턴"으로 따로 센다).
 */
export function rateFor(model: string | undefined): ModelRate | null {
  if (!model) return null;
  const key = model.toLowerCase();
  for (const [pattern, rate] of RATES) {
    if (key.includes(pattern)) return rate;
  }
  return null;
}

/**
 * 한 턴의 토큰 사용량 → 추정 비용(USD). 토큰이 없거나 단가를 모르면 `null`
 * (0이 아니다 — "0원"과 "모름"을 구분해야 화면에서 `~` 표시를 붙일 수 있다).
 * 없는 항목은 0으로 친다.
 */
export function estimateCostUsd(tokens: SessionEventTokens | undefined): number | null {
  if (!tokens) return null;
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
