// src/renderer/office/awardee.ts
//
// "이 달의 우수사원"(docs/employee-of-the-month-design.md §7) 오피스 씬 연출을
// 뒷받침하는 순수 로직. Pixi/DOM 의존 없음 — gen/ 디렉터리 관례를 따라 씬
// 코드에서 분리해 node 환경에서 바로 테스트한다.
//
// - `latestAwardee`: `AwardRecord[]`에서 "확정 수상자가 있는 가장 최근 달"을
//   고른다(진행 중인 이번 달의 잠정 1위는 대상이 아니다 — awardsStore.awards는
//   확정 레코드만 담는다). 배열이 month 오름차순이라는 계약이 깨져도 안전하도록
//   "YYYY-MM" 문자열 비교로 최댓값을 직접 찾는다(사전식 비교 = 날짜 비교).
// - `resolveAwardeeSeat`: 수상자 agentId → 오피스 좌석(그리드 좌표). 책상 배정은
//   `map/deskAssignment.ts`의 `assignDesks`(결정적) 그대로 재사용한다 — B는
//   자기만의 배정 규칙을 새로 만들지 않는다. 프로필이 사라졌거나(deleted) 책상이
//   부족해 배정을 못 받은 경우 null(트로피 숨김 판정과 동일 신호).
import type { AwardRecord } from "@shared/types";
import type { OfficeAwardee } from "./bus";
import { assignDesks } from "./map/deskAssignment";
import type { OfficeMap } from "./map/mapData";
import type { GridPos } from "./world/pathing";
import type { AgentProfile } from "./types";

/** `AwardRecord[]`(순서 무관) 중 `winner`가 있는 가장 최근 달의 수상자. 없으면 null. */
export function latestAwardee(awards: readonly AwardRecord[]): OfficeAwardee | null {
  let best: AwardRecord | undefined;
  for (const rec of awards) {
    if (!rec.winner) continue;
    if (!best || rec.month > best.month) best = rec;
  }
  if (!best?.winner) return null;
  return {
    agentId: best.winner.agentId,
    name: best.winner.name,
    month: best.month,
    hasPortrait: best.winner.hasPortrait,
  };
}

/** 값 동등성(참조가 달라도 내용이 같으면 true) — 재발화 억제에 쓴다. */
export function awardeeEquals(a: OfficeAwardee | null, b: OfficeAwardee | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.agentId === b.agentId && a.name === b.name && a.month === b.month && a.hasPortrait === b.hasPortrait;
}

/**
 * 수상자의 현재 오피스 좌석. `profiles`(현재 씬 로스터)에 수상자가 없거나
 * (프로필 삭제) 책상 수 부족으로 배정을 못 받으면 null — 트로피를 숨긴다.
 * `assignDesks`는 순서 무관·결정적이라 OfficeWorld.syncAgents가 같은 입력으로
 * 내부에서 계산하는 배정과 항상 같은 결과를 낸다.
 */
export function resolveAwardeeSeat(
  map: OfficeMap,
  awardeeAgentId: string | null,
  profiles: readonly AgentProfile[],
): GridPos | null {
  if (!awardeeAgentId) return null;
  const manual = new Map<string, number>();
  for (const p of profiles) {
    if (typeof p.assignedDeskIndex === "number") manual.set(p.id, p.assignedDeskIndex);
  }
  const desks = assignDesks(map, profiles.map((p) => p.id), manual);
  return desks.get(awardeeAgentId)?.seat ?? null;
}

/** 액자(벽 액자 내부 콘텐츠) 표시 여부 — 확정 수상자가 있으면 true. */
export function shouldShowAwardFrame(awardee: OfficeAwardee | null): boolean {
  return awardee !== null;
}
