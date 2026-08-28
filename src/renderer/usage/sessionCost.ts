// src/renderer/usage/sessionCost.ts
//
// 터미널 요약 바에 그릴 "현재 세션" 토큰·비용 누계 — 순수 집계 상태와 함수.
// 단가표/환산은 `renderer/analytics/pricing.ts`를 재사용한다(분석 패널과 같은
// 규칙으로 같은 숫자가 나오게 — 단가표를 여기 다시 만들지 않는다).
//
// 왜 누계를 두 갈래(실시간 + 시드)로 만드는가: 앱을 켠 순간부터의 알림만
// 누적하면 방금 재시작한 세션의 누계가 0으로 보인다(실제로는 계속 진행
// 중이었는데도). 그래서 부팅 직후 `useSessionUsageSeed`가 과거 세션 이벤트
// 기록에서 `aggregateSeed`로 세션별 누계를 한 번 씨딩하고, 스토어가 그 뒤로는
// `addTurn`으로 들어오는 실시간 알림만 더한다. 이 파일은 그 셋(누계 상태·
// 실시간 누적·과거 시딩)의 순수 함수만 갖고, 세션 경계 판정·이중 계산 방지
// (`at > seedAt`)는 스토어(`appStore.ts`) 몫이다.
import type { SessionEventRecord, SessionEventTokens, SessionId } from "@shared/types";
import { estimateCostUsd } from "../analytics/pricing";

/** 세션 하나의 토큰·비용 누계. 전부 불변값 — 갱신은 항상 새 객체를 만든다. */
export interface SessionUsageTotals {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  /** 단가를 아는 턴만 누적한 추정 비용(USD). */
  costUsd: number;
  /** 단가를 몰라 비용에서 빠진 턴 수 — 0보다 크면 표시에 `~`를 붙인다. */
  costUnknownTurns: number;
  /** 토큰이 실린 턴 수. */
  turns: number;
  /** 가장 최근 턴의 대표 모델(툴팁용). */
  model?: string;
}

/** 빈 누계(세션이 새로 열렸을 때의 초기값). */
export function emptyTotals(): SessionUsageTotals {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    costUsd: 0,
    costUnknownTurns: 0,
    turns: 0,
  };
}

/**
 * 턴 하나(`tokens`)를 누계에 더한 **새 객체**를 반환한다(불변 — `totals`는
 * 안 건드림). 비용을 모르는 모델(`estimateCostUsd` → `null`)은 `costUsd`에
 * 더하지 않고 `costUnknownTurns`만 올린다. `model`은 이 턴에 실려 있으면
 * 그 값으로 갱신한다(가장 최근 턴이 대표 모델).
 */
export function addTurn(totals: SessionUsageTotals, tokens: SessionEventTokens): SessionUsageTotals {
  const cost = estimateCostUsd(tokens);
  return {
    input: totals.input + (tokens.input ?? 0),
    output: totals.output + (tokens.output ?? 0),
    cacheRead: totals.cacheRead + (tokens.cacheRead ?? 0),
    cacheWrite: totals.cacheWrite + (tokens.cacheWrite ?? 0),
    costUsd: totals.costUsd + (cost ?? 0),
    costUnknownTurns: totals.costUnknownTurns + (cost === null ? 1 : 0),
    turns: totals.turns + 1,
    model: tokens.model ?? totals.model,
  };
}

/**
 * 두 누계를 더한 **새 객체**(시드 + 실시간 누계 합산용). `model`은 `b`(대개
 * 더 최근 쪽)가 있으면 그것을 우선한다.
 */
export function mergeTotals(a: SessionUsageTotals, b: SessionUsageTotals): SessionUsageTotals {
  return {
    input: a.input + b.input,
    output: a.output + b.output,
    cacheRead: a.cacheRead + b.cacheRead,
    cacheWrite: a.cacheWrite + b.cacheWrite,
    costUsd: a.costUsd + b.costUsd,
    costUnknownTurns: a.costUnknownTurns + b.costUnknownTurns,
    turns: a.turns + b.turns,
    model: b.model ?? a.model,
  };
}

/**
 * 과거 세션 이벤트 기록에서 세션별 누계를 시딩한다. `kind === "stop"`이고
 * `tokens`가 있고 `at <= maxAt`인 레코드만 `sessionId`별로 합산한다 — 뒤 두
 * 조건은 실시간 누적과의 이중 계산을 막는 경계선이고, 스토어의
 * `applyNotificationUsage`가 세우는 `e.at > sessionUsageSeed.at` 판정과 짝을
 * 이룬다(이 함수가 `maxAt`으로 자른 시점 이후 알림만 실시간이 더한다).
 */
export function aggregateSeed(
  records: SessionEventRecord[],
  maxAt: number,
): Record<SessionId, SessionUsageTotals> {
  const out: Record<SessionId, SessionUsageTotals> = {};
  for (const r of records) {
    if (r.kind !== "stop" || !r.tokens || r.at > maxAt) continue;
    const prev = out[r.sessionId] ?? emptyTotals();
    out[r.sessionId] = addTurn(prev, r.tokens);
  }
  return out;
}
