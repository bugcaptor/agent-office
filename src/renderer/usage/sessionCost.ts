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
 *
 * `partial`(기본 false): 이 사용량이 턴이 끝나기 **전**의 중간 관측인지
 * (`TurnUsageEvent.partial`, claude 어댑터의 PostToolUse 중간 갱신 —
 * docs/session-analytics-design.md §11.9). 토큰·비용은 partial이어도 그대로
 * 더한다(그만큼 실제로 썼으므로), 하지만 **`turns`는 올리지 않는다** — 한
 * 턴 안에서 도구를 여러 번 부르면 그만큼 partial 이벤트가 여러 번 오는데,
 * 매번 턴을 하나씩 잡으면 "지금까지 3번 도구를 썼다"가 "턴이 3개였다"로
 * 둔갑한다. 그 턴의 최종 Stop이 `partial:false`로 다시 잡히면서 그때
 * `turns`가 실제로 오른다. `costUnknownTurns`도 같은 이유로 partial에서는
 * 올리지 않는다 — 단가를 모르는 턴이라도 그 턴의 Stop 델타가 같은 모델로
 * 다시 잡히므로 거기서 표시된다(드물게 그 턴의 마지막 도구 이후 usage 줄이
 * 하나도 없어 Stop 델타가 비면, 그 턴은 애초에 tokens가 없어 addTurn 자체가
 * 안 불리므로 단가 미상 표시가 통째로 빠질 수 있다 — 수용된 한계).
 */
export function addTurn(
  totals: SessionUsageTotals,
  tokens: SessionEventTokens,
  partial = false,
): SessionUsageTotals {
  const cost = estimateCostUsd(tokens);
  return {
    input: totals.input + (tokens.input ?? 0),
    output: totals.output + (tokens.output ?? 0),
    cacheRead: totals.cacheRead + (tokens.cacheRead ?? 0),
    cacheWrite: totals.cacheWrite + (tokens.cacheWrite ?? 0),
    costUsd: totals.costUsd + (cost ?? 0),
    costUnknownTurns: totals.costUnknownTurns + (!partial && cost === null ? 1 : 0),
    turns: totals.turns + (partial ? 0 : 1),
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
 * 과거 세션 이벤트 기록에서 세션별 누계를 시딩한다. `tokens`가 있고
 * `at <= maxAt`인 레코드만 `sessionId`별로 합산한다 — **kind는 안 본다**.
 * 과거 파일은 `kind="stop"`에, 신규 파일은 `kind="usage"`에 토큰이 실리므로
 * kind로 걸러내면 한쪽 세대의 파일이 통째로 빠진다. `tokens` 유무·`at<=maxAt`
 * 두 조건이 실시간 누적과의 이중 계산을 막는 경계선이고, 스토어의
 * `applyTurnUsage`가 세우는 `e.at > sessionUsageSeed.at` 판정과 짝을 이룬다
 * (이 함수가 `maxAt`으로 자른 시점 이후 사용량만 실시간이 더한다). 신규
 * stop 레코드는 애초에 tokens가 없으므로, 전환기에도 같은 턴이 usage와
 * stop 양쪽에 실려 두 번 잡히는 일은 구조적으로 없다.
 *
 * `r.partial`도 `addTurn`에 그대로 넘긴다(§11.9) — 이 필드가 생기기 전의
 * 과거 레코드는 `undefined`이고, 전부 Stop 유래(=턴이 끝난 것)이므로
 * `?? false`로 강등한다. 그렇지 않고 partial 유무를 무시하면 재부팅 시드가
 * PostToolUse 중간 갱신 레코드까지 턴으로 세어 `turns`가 실제보다 부푼다.
 */
export function aggregateSeed(
  records: SessionEventRecord[],
  maxAt: number,
): Record<SessionId, SessionUsageTotals> {
  const out: Record<SessionId, SessionUsageTotals> = {};
  for (const r of records) {
    if (!r.tokens || r.at > maxAt) continue;
    const prev = out[r.sessionId] ?? emptyTotals();
    out[r.sessionId] = addTurn(prev, r.tokens, r.partial ?? false);
  }
  return out;
}
