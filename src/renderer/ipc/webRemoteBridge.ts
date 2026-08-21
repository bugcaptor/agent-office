// src/renderer/ipc/webRemoteBridge.ts
//
// 웹 원격(docs/web-remote-design.md)의 렌더러 브리지. 백엔드 이벤트 둘을 받아
// 스토어/터미널에 반영한다.
//
//   web-remote-snapshot-request  화면 직렬화 요청 → web_remote_submit_snapshot
//   web-remote-pair-request      승인 대기 페어링 → 스토어(승인 다이얼로그가 표시)
//
// 브라우저로 나가는 출력/입력 자체는 여기를 거치지 않는다 — 백엔드의
// `OutputSink` tap과 WS가 직접 나른다.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { useAppStore } from "../store/appStore";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { webRemoteApi } from "./webRemoteApi";
import type { PendingPairing } from "./webRemoteApi";

interface SnapshotRequest {
  agentId: string;
  requestId: string;
}

/**
 * 백엔드가 요청한 화면 스냅샷에 답한다. 아직 터미널을 한 번도 연 적 없는
 * 캐릭터(=xterm 없음)면 빈 문자열을 돌려준다 — 호스트는 그걸 "스냅샷 없음"과
 * 같게 취급해 링버퍼 리플레이로 복원한다. 어떤 경우에도 **응답은 한다**:
 * 침묵하면 브라우저가 타임아웃(2초)만큼 기다리게 된다.
 */
async function answerSnapshot(req: SnapshotRequest): Promise<void> {
  const snapshot = (await terminalRegistry.flushAndSerialize(req.agentId)) ?? "";
  await webRemoteApi.submitSnapshot(req.requestId, snapshot);
}

/**
 * 브리지를 설치한다. 반환값은 teardown(테스트 전용 — 앱은 수명 내내 유지).
 * `listen()`은 비동기로 UnlistenFn을 주므로, teardown이 먼저 와도 새는 일이
 * 없도록 해소 시점에 즉시 정리한다(tauriApi.wrapListen과 같은 관례).
 */
export function installWebRemoteBridge(): () => void {
  let disposed = false;
  const unlisteners: UnlistenFn[] = [];

  const track = (p: Promise<UnlistenFn>) => {
    void p
      .then((un) => {
        if (disposed) {
          void un();
          return;
        }
        unlisteners.push(un);
      })
      .catch((err) => {
        console.warn("webRemoteBridge: 이벤트 구독 실패", err);
      });
  };

  track(
    listen<SnapshotRequest>(Events.webRemoteSnapshotRequest, (e) => {
      void answerSnapshot(e.payload).catch((err) => {
        console.warn("webRemoteBridge: 스냅샷 응답 실패", err);
      });
    })
  );

  track(
    listen<PendingPairing>(Events.webRemotePairRequest, (e) => {
      const prev = useAppStore.getState().webRemotePending;
      const next = [...prev.filter((p) => p.pairingId !== e.payload.pairingId), e.payload];
      useAppStore.getState().setWebRemotePending(next);
    })
  );

  // 부팅 직후 1회 동기화 — 이벤트는 새 요청에만 오므로, 앱이 뜨기 전에
  // 시작된 페어링을 여기서 한 번 끌어온다.
  void webRemoteApi
    .hostStatus()
    .then((status) => {
      if (!disposed) useAppStore.getState().setWebRemotePending(status.pending ?? []);
    })
    .catch(() => {
      /* 무시 — 웹 원격이 꺼져 있을 뿐 */
    });

  return () => {
    disposed = true;
    for (const un of unlisteners) void un();
    unlisteners.length = 0;
  };
}
