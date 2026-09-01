// src/renderer/agent/summonSuppress.ts
//
// "자리로" 일괄 소환(summonToDesk.ts) 직후의 짧은 알림 억제 창.
//
// 탕비실에 있던 에이전트를 한꺼번에 깨우면, 셸이 뜨자마자 직전 턴의 잔여
// Stop/알림이 밀려 들어와 캐릭터가 자리에 앉기도 전에 알림 대기(pending)로
// 바뀐다 — 사용자가 방금 스스로 부른 것이라 알릴 내용이 아닌데도 머리 위
// 표시와 보스 줄서기가 켜지는 게 문제였다. 그래서 소환 시각부터 잠깐
// (`SUMMON_NOTIFY_SUPPRESS_MS`) 그 에이전트의 알림을 스토어에 넣지 않는다.
//
// 억제는 렌더러 쪽 게이트다(`ipc/sessionBridge.ts`의 onNotification). 억제된
// 알림은 백엔드에서도 해당 id만 지워 두 쪽 상태가 어긋나지 않게 한다.
// 시간 집계(applyNotificationTiming)는 억제와 무관하게 그대로 흐른다 —
// 알림을 감추는 것이지 턴이 없던 일이 되는 게 아니다.
//
// 타이머를 쓰지 않고 만료 시각만 들고 있다가 조회 시점에 비교한다(테스트가
// 가짜 시계를 쓰지 않아도 되고, 앱 수명 동안 정리할 핸들도 없다).

/** 소환 직후 알림을 감춰 두는 시간(ms). */
export const SUMMON_NOTIFY_SUPPRESS_MS = 3000;

/** agentId -> 억제 만료 시각(epoch ms). */
const suppressedUntil = new Map<string, number>();

/** `agentId`의 알림을 지금부터 `ms` 동안 억제한다. */
export function suppressNotifications(
  agentId: string,
  ms: number = SUMMON_NOTIFY_SUPPRESS_MS,
  now: number = Date.now(),
): void {
  // 이미 걸린 창이 더 길면 그대로 둔다(연달아 소환해도 창이 줄지 않게).
  const prev = suppressedUntil.get(agentId) ?? 0;
  suppressedUntil.set(agentId, Math.max(prev, now + ms));
}

/** 지금 `agentId`의 알림이 억제 중인가. 만료된 항목은 조회하면서 정리한다. */
export function isNotifySuppressed(agentId: string, now: number = Date.now()): boolean {
  const until = suppressedUntil.get(agentId);
  if (until === undefined) return false;
  if (until <= now) {
    suppressedUntil.delete(agentId);
    return false;
  }
  return true;
}

/** 테스트 격리용 — 억제 창 전부 해제. */
export function resetNotifySuppression(): void {
  suppressedUntil.clear();
}
