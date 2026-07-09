// src/renderer/office/types.ts
//
// Office-subsystem-local type contract.
//
// Deliberately looser/narrower than the authoritative `AgentProfile` in
// `src/shared/types.ts` (single definition owned by A) — this keeps
// subsystem B decoupled from A's exact shape. B only ever reads `id` and
// `seed`; the index signature lets any richer profile object (including the
// real shared `AgentProfile`) satisfy this structurally.
export interface AgentProfile {
  id: string;
  name: string;
  role: string;
  seed: string; // 생성 시드 (없으면 id로 대체)
  /** 사용자가 수동 지정한 책상 인덱스. 없으면 자동(해시) 배정. */
  assignedDeskIndex?: number;
  [k: string]: unknown;
}
