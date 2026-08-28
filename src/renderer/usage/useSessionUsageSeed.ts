// src/renderer/usage/useSessionUsageSeed.ts
//
// 터미널 요약 바 사용량 표시용 과거 시드 로더(docs/session-analytics-design.md
// §11). 방금 재시작한 앱에서 계속 진행 중이던 세션의 누계가 0으로 보이지
// 않도록, 부팅 뒤 한 번 최근 `SEED_WINDOW_MS` 구간의 세션 이벤트를 읽어
// `aggregateSeed`로 세션별 누계를 스토어에 심는다. 그 뒤로는 실시간 알림
// (`applyNotificationUsage`)이 `at > seed.at`인 턴만 더해 이중 계산을 막는다.
//
// 설정 `sessionCostEnabled`가 꺼져 있으면 아무것도 하지 않는다 — 표시 자체를
// 안 쓰는데 세션 이벤트 파일을 읽을 이유가 없다. 모듈 스코프 플래그
// `attempted`로 앱 수명당 정확히 1회만 시도한다(성공/실패 무관 — 실패해도
// 재시도하지 않는다. 시드 없이 실시간 누계만 보여도 기능은 동작한다).
//
// 하이드레이션 게이트(§11.5): `appStore`의 `appSettings`는 스토어 생성
// 시점에는 `DEFAULT_APP_SETTINGS`(sessionCostEnabled=true) 플레이스홀더다.
// `bootApp`이 `getAppSettings` IPC 왕복을 마친 뒤에야 `hydrateSettings`로
// 실제 값을 심고 `settingsHydrated`를 true로 세운다. 이 훅은 그 전에는(즉
// `settingsHydrated`가 false인 동안은) `enabled` 값을 신뢰하지 않고 아무것도
// 하지 않는다 — `attempted`도 세우지 않는다. 그래서 설정을 꺼 놓은 사용자도
// 하이드레이트 전 첫 렌더 시점에 잘못 시딩되지 않는다. 하이드레이션이 실패해
// `hydrateSettings`가 끝내 안 불리면(부팅 IPC 실패 등) 이 훅은 영영 시도하지
// 않는다 — 시드 없이 실시간 누계만 쓰는 것이 의도된 강등이다.
import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { aggregateSeed } from "./sessionCost";

/** 시드 조회 창(3일). 그보다 오래 전에 시작해 계속 열려 있는 세션의 사용량은
 * 시드에서 빠져 화면 누계가 실제보다 적을 수 있다 — 받아들인 한계. */
export const SEED_WINDOW_MS = 3 * 24 * 60 * 60 * 1000;

let attempted = false;

export function useSessionUsageSeed(): void {
  const hydrated = useAppStore((s) => s.settingsHydrated);
  const enabled = useAppStore((s) => s.appSettings.sessionCostEnabled);
  const seeded = useAppStore((s) => s.sessionUsageSeed !== null);

  useEffect(() => {
    if (!hydrated || !enabled || seeded || attempted) return;
    attempted = true;

    // 부팅 크리티컬 패스에서 뺀다 — JSONL을 읽어 파싱하는 IPC 왕복이라 부팅
    // 시퀀스(요약기·git 브랜치 워처 등 이후 단계)를 지연시킬 이유가 없다.
    // 아래 firstAt 기반 컷오프 덕에 이 시딩이 늦게 끝나도(실시간 알림이
    // 먼저 들어와도) 이중 계산이 생기지 않는다 — 안전하게 미룰 수 있다.
    const timer = setTimeout(() => {
      // firstAt은 스토어에서 **실행 시점에** 직접 읽는다(훅 인자로 받은
      // 클로저 값이 아니다) — setTimeout이 대기하는 동안 실시간 알림이
      // 먼저 도착해 firstAt이 막 세워질 수 있어, 그 최신값을 놓치면 컷오프가
      // 너무 늦게 잡혀 그 알림이 시드에도 다시 잡히는 이중 계산이 생긴다.
      //
      // 컷오프는 "훅이 도는 시각"이 아니라 "실시간이 실제로 처음 센 턴의
      // 시각"에 묶는다(§11.3, B 리뷰 수정). 실시간이 먼저 턴을 반영하면
      // firstAt이 그 시각으로 잡혀 있고, 시드는 그 직전(firstAt - 1)까지만
      // 긁으므로 실시간과 겹치는 구간이 구조적으로 없다. 아직 실시간이 한
      // 턴도 못 봤으면(firstAt === null) 지금(Date.now())까지 긁는다 — 그
      // 뒤에 오는 실시간 알림은 이 시각보다 뒤에 온다(§11.3 원래 경계와 동일).
      const firstAt = useAppStore.getState().sessionUsageFirstAt;
      const cut = firstAt !== null ? firstAt - 1 : Date.now();
      void tauriApi
        .loadSessionEvents(cut - SEED_WINDOW_MS, cut, ["stop"])
        .then((records) => {
          useAppStore.getState().setSessionUsageSeed({ at: cut, bySession: aggregateSeed(records, cut) });
        })
        .catch((err) => {
          // 조용히 삼킨다 — 시드 없이 실시간 누계만 보여도 된다.
          console.warn("useSessionUsageSeed: loadSessionEvents failed", err);
        });
    }, 0);

    return () => clearTimeout(timer);
  }, [hydrated, enabled, seeded]);
}
