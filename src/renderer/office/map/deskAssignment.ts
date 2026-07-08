// src/renderer/office/map/deskAssignment.ts
//
// agentId -> deskSlot 결정적 배정.
//
// Note: AgentProfile.deskIndex (in src/shared/types.ts) is reference-only;
// actual seating is this function's deterministic hash assignment, which is
// order-independent and collision-free given the same set of agent ids.

import { hashStringToSeed } from '../gen/prng';
import type { OfficeMap, DeskSlot } from './mapData';

/** agentId → deskSlot. 입력 순서 무관, 결정적, 충돌은 선형 탐사로 해결. */
export function assignDesks(map: OfficeMap, agentIds: readonly string[]): Map<string, DeskSlot> {
  const n = map.desks.length;
  const taken = new Array<string | null>(n).fill(null);
  const result = new Map<string, DeskSlot>();
  if (n === 0) return result;
  // id 정렬로 순서 독립성 확보
  const ids = [...agentIds].sort();
  for (const id of ids) {
    const start = hashStringToSeed(id) % n;
    for (let k = 0; k < n; k++) {
      const s = (start + k) % n;
      if (taken[s] === null) {
        taken[s] = id;
        result.set(id, map.desks[s]);
        break;
      }
    }
  }
  return result; // 슬롯 부족 시 초과 에이전트는 미배정(=자유 배회 상태로 표시)
}
