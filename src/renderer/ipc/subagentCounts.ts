// src/renderer/ipc/subagentCounts.ts
//
// 부모 agentId별 "활성 서브에이전트 수"를 소유하는 순수 렌더러 모듈.
// sub-start/sub-stop activity의 증감과 sub-count 스냅샷의 절대값을 반영하고,
// 세션 종료에서 reset한다. 카운트는 순수 시각 효과라 휘발이 정답.
// zustand가 아닌 plain 콜백 릴레이(리렌더 불필요, Pixi 전용 신호).

export type SubagentCountCb = (agentId: string, count: number) => void;

export class SubagentCountTracker {
  private counts = new Map<string, number>();
  // agentId별 "마지막으로 반영한 이벤트의 백엔드 at(ms)". 훅은 독립 HTTP+이벤트
  // 채널로 흘러 순서 보장이 없으므로, 이 워터마크보다 오래된 절대 스냅샷(sub-count)은
  // 무시해 재정렬 클로버링(오래된 스냅샷이 더 최신 델타를 덮어써 미니미 조기 소멸)을 막는다.
  // 델타(bump)는 확정 이벤트라 항상 적용하되 워터마크는 전진시킨다.
  private lastAt = new Map<string, number>();
  private cbs = new Set<SubagentCountCb>();

  subscribe(cb: SubagentCountCb): () => void {
    this.cbs.add(cb);
    // 구독 시점 replay — 씬 재마운트 시 진행 중인 미니 캐릭터 수 복원.
    for (const [id, n] of this.counts) cb(id, n);
    return () => this.cbs.delete(cb);
  }

  get(agentId: string): number {
    return this.counts.get(agentId) ?? 0;
  }

  bump(agentId: string, delta: number, at?: number): void {
    this.advance(agentId, at);
    this.set(agentId, this.get(agentId) + delta);
  }

  /** 절대 스냅샷 반영. `at`이 이미 반영한 이벤트보다 오래되면 스테일로 간주해 무시. */
  setAbsolute(agentId: string, count: number, at?: number): void {
    if (typeof at === "number" && at < (this.lastAt.get(agentId) ?? -Infinity)) {
      return; // 스테일 스냅샷 — 더 최신 이벤트가 이미 반영됨
    }
    this.advance(agentId, at);
    this.set(agentId, Math.floor(count));
  }

  reset(agentId: string): void {
    this.lastAt.delete(agentId); // 세션 경계 — 워터마크도 초기화(다음 세션 스냅샷이 반영되게)
    this.set(agentId, 0);
  }

  private advance(agentId: string, at?: number): void {
    if (typeof at !== "number") return;
    const prev = this.lastAt.get(agentId) ?? -Infinity;
    if (at > prev) this.lastAt.set(agentId, at);
  }

  private set(agentId: string, next: number): void {
    const clamped = next < 0 ? 0 : next;
    if (clamped === this.get(agentId)) return; // 변화 없으면 통지 생략
    this.counts.set(agentId, clamped);
    this.cbs.forEach((cb) => cb(agentId, clamped));
  }
}
