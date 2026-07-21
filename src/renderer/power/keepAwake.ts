// src/renderer/power/keepAwake.ts
//
// 작업 중 시스템 잠자기 방지(이슈 #68)의 렌더러 측 컨트롤러. 렌더러가 권위
// 있게 아는 per-agent 턴 상태(timeline/turnReducer)에서 "지금 일하는 캐릭터가
// 하나 이상"인지 집계해 백엔드 웨이크락을 켜고 끈다.
//
// 정책:
//  - acquire(잠자기 방지 켜기)는 rising-edge에 **즉시**. 이후 lease(백엔드 180초
//    TTL)를 갱신하려고 renewInterval마다 재통지한다.
//  - release는 **지연**한다(releaseDelay). 턴 사이 짧은 유휴(오토모드 자동 승인,
//    큐잉된 다음 프롬프트)에 OS assertion을 flap시키지 않기 위해서다. 그 사이
//    다시 working으로 돌아오면 지연 타이머를 취소한다.
//  - 설정이 꺼지면(enabled=false) 지연 없이 **즉시** 해제한다.
//
// 백엔드도 설정으로 게이트하고 lease로 backstop하므로, 이 컨트롤러가 통지를
// 놓쳐도 안전 측(잠자기 허용)으로 수렴한다.

import type { AgentTurnState } from "../timeline/turnReducer";

/** release 지연(ms). OS 잠자기 타임아웃(분 단위)보다 충분히 짧다. */
export const KEEP_AWAKE_RELEASE_DELAY_MS = 60_000;
/** lease 갱신 재통지 주기(ms). 백엔드 TTL 180초의 1/3 — 백그라운드 webview
 * 타이머 스로틀링에도 여유가 있다. */
export const KEEP_AWAKE_RENEW_INTERVAL_MS = 60_000;

/** 일하는 캐릭터가 하나라도 있는지 — 순수 집계. waiting/idle은 제외한다
 * (waiting은 사용자 질문 대기라 무기한일 수 있어 잠자기를 막지 않는다). */
export function computeAnyWorking(timeTracking: Record<string, AgentTurnState>): boolean {
  return Object.values(timeTracking).some((t) => t.phase === "working");
}

export interface KeepAwakeController {
  /** 설정 on/off와 "일하는 캐릭터 있음" 여부로 상태를 재계산해 백엔드를 구동한다. */
  update(enabled: boolean, anyWorking: boolean): void;
  /** 타이머 정리(테스트 teardown / 앱 종료). 통지는 보내지 않는다. */
  dispose(): void;
}

/**
 * @param notify 백엔드 통지 콜백(true=일하는 중, false=해제). rising-edge와
 *   갱신 주기에 true로, 지연/즉시 해제에 false로 호출된다.
 */
export function createKeepAwakeController(
  notify: (active: boolean) => void,
  opts?: { releaseDelayMs?: number; renewIntervalMs?: number },
): KeepAwakeController {
  const releaseDelayMs = opts?.releaseDelayMs ?? KEEP_AWAKE_RELEASE_DELAY_MS;
  const renewIntervalMs = opts?.renewIntervalMs ?? KEEP_AWAKE_RENEW_INTERVAL_MS;

  let held = false;
  let releaseTimer: ReturnType<typeof setTimeout> | null = null;
  let renewTimer: ReturnType<typeof setInterval> | null = null;

  const clearRelease = () => {
    if (releaseTimer !== null) {
      clearTimeout(releaseTimer);
      releaseTimer = null;
    }
  };
  const clearRenew = () => {
    if (renewTimer !== null) {
      clearInterval(renewTimer);
      renewTimer = null;
    }
  };

  const startRenew = () => {
    if (renewTimer === null) renewTimer = setInterval(() => notify(true), renewIntervalMs);
  };

  const acquire = () => {
    clearRelease();
    if (!held) {
      held = true;
      notify(true);
    }
    startRenew(); // 지연 해제 창에서 멈춘 갱신을 재개하는 경우도 포함(멱등).
  };

  const releaseNow = () => {
    clearRelease();
    if (held) {
      held = false;
      clearRenew();
      notify(false);
    }
  };

  const releaseSoon = () => {
    if (!held || releaseTimer !== null) return;
    // 유예 창(releaseDelay < 백엔드 lease TTL) 동안은 갱신을 멈춘다 — 그래야
    // 해제 순간과 갱신 통지가 겹치지 않는다. 이 창 안에 다시 acquire되면
    // startRenew가 갱신을 되살린다.
    clearRenew();
    releaseTimer = setTimeout(() => {
      releaseTimer = null;
      releaseNow();
    }, releaseDelayMs);
  };

  return {
    update(enabled, anyWorking) {
      if (!enabled) releaseNow();
      else if (anyWorking) acquire();
      else releaseSoon();
    },
    dispose() {
      clearRelease();
      clearRenew();
    },
  };
}
