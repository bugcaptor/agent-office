// src/renderer/office/entities/behaviorFsm.ts
//
// Spec: Phase B office redesign — "sit at desk only while the session is
// active, otherwise hang out in the break room".
//
// Pure transition function — no clock, no RNG owned internally. The caller
// (movement/entity controller, `CharacterEntity`) injects `dtMs`, the
// accumulated per-state timer and this tick's random draw. That keeps the
// FSM itself fully deterministic and testable without Math.random or real
// timers.
//
// Flow: sitting -[session goes inactive, small linger]-> walking(break
// target) -> [arrival, owned by the movement controller] -> breakIdle ->
// [session/pending goes active] -> walking(return to desk) -> [arrival] ->
// sitting. While in breakIdle and still inactive, the character may stroll
// to another break-room tile (walking(break wander) -> [arrival] ->
// breakIdle) purely for flavor.

export type BehaviorState = "sitting" | "walking" | "breakIdle" | "queueing";

export interface FsmContext {
  hasPending: boolean; // 알림 대기 (있으면 자리 지킴/즉시 복귀)
  sessionActive: boolean; // 세션이 starting/running 인가 (있으면 자리 지킴/즉시 복귀)
  shouldQueue: boolean; // 보스 줄에 서야 하는가 — 월드가 계산(hasPending && !휴가 && 슬롯 배정)
  timerMs: number; // 현재 상태 경과
  rand: number; // [0,1) 이번 틱 난수
}

export interface FsmResult {
  next: BehaviorState;
  // walking 진입 시 목적지 요청 플래그
  requestBreakTarget?: boolean; // sitting -> walking: 탕비실로 이동
  requestBreakWander?: boolean; // breakIdle -> walking: 탕비실 내 다른 타일로 산책
  requestReturnToDesk?: boolean; // breakIdle -> walking: 자기 자리로 복귀
  requestQueueSlot?: boolean; // walking 진입: 보스 책상 줄 슬롯으로 이동
}

// 세션이 끝난 뒤 곧장 일어서지 않도록 두는 짧은 여유 시간.
const SIT_LINGER_MS = 2000;
const BREAK_WANDER_CHANCE_PER_SEC = 0.06;

export function stepBehavior(state: BehaviorState, c: FsmContext, dtMs: number): FsmResult {
  switch (state) {
    case "sitting": {
      if (c.shouldQueue) return { next: "walking", requestQueueSlot: true };
      // 세션이 활성 상태이거나 알림 대기 중이면 자리 고정 — 배회하지 않는다.
      if (c.sessionActive || c.hasPending) return { next: "sitting" };
      if (c.timerMs < SIT_LINGER_MS) return { next: "sitting" };
      return { next: "walking", requestBreakTarget: true };
    }
    case "walking": {
      // 도착 판정은 이동 컨트롤러가 하고 도착 시 상태 종료를 부른다.
      return { next: "walking" };
    }
    case "queueing": {
      if (!c.shouldQueue) return { next: "walking", requestReturnToDesk: true };
      return { next: "queueing" };
    }
    case "breakIdle": {
      if (c.shouldQueue) return { next: "walking", requestQueueSlot: true };
      // 세션이 다시 활성화되거나 알림이 오면 즉시 자리로 복귀.
      if (c.sessionActive || c.hasPending) {
        return { next: "walking", requestReturnToDesk: true };
      }
      // 초당 확률 -> 이번 틱 확률로 변환해 탕비실 내 산책 여부를 결정.
      const p = 1 - Math.pow(1 - BREAK_WANDER_CHANCE_PER_SEC, dtMs / 1000);
      if (c.rand < p) return { next: "walking", requestBreakWander: true };
      return { next: "breakIdle" };
    }
  }
}
