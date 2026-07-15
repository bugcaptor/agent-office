// src/renderer/ipc/subagentCounts.ts
//
// 부모 agentId별 "활성 서브에이전트 수"를 소유하는 순수 렌더러 모듈.
// sub-start/sub-stop activity의 증감과 sub-count 스냅샷의 절대값을 반영하고,
// 세션 종료에서 reset한다. 카운트는 순수 시각 효과라 휘발이 정답.
// zustand가 아닌 plain 콜백 릴레이(리렌더 불필요, Pixi 전용 신호).

export type SubagentCountCb = (agentId: string, count: number) => void;

export class SubagentCountTracker {
  private counts = new Map<string, number>();
  private cbs = new Set<SubagentCountCb>();

  subscribe(cb: SubagentCountCb): () => void {
    this.cbs.add(cb);
    return () => this.cbs.delete(cb);
  }

  get(agentId: string): number {
    return this.counts.get(agentId) ?? 0;
  }

  bump(agentId: string, delta: number): void {
    this.set(agentId, this.get(agentId) + delta);
  }

  setAbsolute(agentId: string, count: number): void {
    this.set(agentId, Math.floor(count));
  }

  reset(agentId: string): void {
    this.set(agentId, 0);
  }

  private set(agentId: string, next: number): void {
    const clamped = next < 0 ? 0 : next;
    if (clamped === this.get(agentId)) return; // 변화 없으면 통지 생략
    this.counts.set(agentId, clamped);
    this.cbs.forEach((cb) => cb(agentId, clamped));
  }
}
