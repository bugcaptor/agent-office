// src/renderer/office/map/deskAssignment.ts
//
// agentId -> deskSlot 배정: 수동 지정 우선 + 나머지는 결정적 해시 배정.
//
// Note: AgentProfile.assignedDeskIndex (in src/shared/types.ts)가 수동 지정의
// 원천이고, 지정이 없는 에이전트는 이 함수의 해시 배정(순서 독립·충돌 없음)을
// 따른다. 지정된 책상은 자동 배정 풀에서 제외된다 — "지정된 적 없는 책상"만
// 자리 없는 에이전트가 자동 선점할 수 있다.

import { hashStringToSeed } from '../gen/prng';
import type { OfficeMap, DeskSlot } from './mapData';

/**
 * agentId → deskSlot. 입력 순서 무관, 결정적, 충돌은 선형 탐사로 해결.
 *
 * `manualByAgent`(agentId → deskIndex)가 있으면 그 지정을 먼저 배정한다.
 * 범위 밖 인덱스는 무시(자동 폴백), 중복 지정은 id 정렬 순 선착 1명만
 * 인정하고 나머지는 자동 폴백. 지정된 책상 전부(승자 유무 무관)는 자동
 * 배정에서 건너뛴다.
 */
export function assignDesks(
  map: OfficeMap,
  agentIds: readonly string[],
  manualByAgent?: ReadonlyMap<string, number>,
): Map<string, DeskSlot> {
  const n = map.desks.length;
  const taken = new Array<string | null>(n).fill(null);
  const result = new Map<string, DeskSlot>();
  if (n === 0) return result;
  // id 정렬로 순서 독립성 확보
  const ids = [...agentIds].sort();

  // 1차: 수동 지정 배정 + 지정 책상 예약(자동 풀에서 제외).
  const reserved = new Set<number>();
  if (manualByAgent) {
    for (const [, idx] of manualByAgent) {
      if (Number.isInteger(idx) && idx >= 0 && idx < n) reserved.add(idx);
    }
    for (const id of ids) {
      const idx = manualByAgent.get(id);
      if (idx === undefined || !reserved.has(idx)) continue;
      if (taken[idx] !== null) continue; // 중복 지정: 정렬 순 선착이 이미 차지
      taken[idx] = id;
      result.set(id, map.desks[idx]);
    }
  }

  // 2차: 미지정 에이전트를 지정된 적 없는 책상에만 해시 배정.
  for (const id of ids) {
    if (result.has(id)) continue;
    const start = hashStringToSeed(id) % n;
    for (let k = 0; k < n; k++) {
      const s = (start + k) % n;
      if (taken[s] === null && !reserved.has(s)) {
        taken[s] = id;
        result.set(id, map.desks[s]);
        break;
      }
    }
  }
  return result; // 슬롯 부족 시 초과 에이전트는 미배정(=자유 배회 상태로 표시)
}
