// src/renderer/ipc/peerBridge.ts
//
// 피어 세션 공유(#7k, docs/peer-session-share-design.md)의 렌더러 브리지.
// 백엔드 이벤트 넷을 받아 스토어/터미널에 반영한다.
//
//   peer-status           (뷰어) 연결 상태 + 원격 캐릭터 목록 → 스토어
//   peer-snapshot-request (호스트) 화면 직렬화 요청 → submit_peer_snapshot
//   peer-pair-request     (호스트) 승인 대기 페어링 → 스토어(설정 UI가 표시)
//   peer-resized          (뷰어) 호스트가 정한 터미널 크기 → 스토어
//
// 원격 세션의 출력/입력 자체는 여기를 거치지 않는다 — 기존
// `subscribe_output`/`write_input`이 백엔드에서 `peer:` 접두사로 라우팅된다.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { useAppStore } from "../store/appStore";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { peerApi } from "./peerApi";
import type { PendingPairing, PeerViewerStatus } from "./peerApi";

interface SnapshotRequest {
  agentId: string;
  requestId: string;
}

interface ResizedEvent {
  agentId: string;
  cols: number;
  rows: number;
}

/**
 * 백엔드가 요청한 화면 스냅샷에 답한다. 아직 터미널을 한 번도 연 적 없는
 * 캐릭터(=xterm 없음)면 빈 문자열을 돌려준다 — 호스트는 그걸 "스냅샷 없음"과
 * 같게 취급해 링버퍼 리플레이로 복원한다. 어떤 경우에도 **응답은 한다**:
 * 침묵하면 뷰어가 타임아웃(2초)만큼 기다리게 된다.
 */
async function answerSnapshot(req: SnapshotRequest): Promise<void> {
  const snapshot = (await terminalRegistry.flushAndSerialize(req.agentId)) ?? "";
  await peerApi.submitSnapshot(req.requestId, snapshot);
}

/**
 * 브리지를 설치한다. 반환값은 teardown(테스트 전용 — 앱은 수명 내내 유지).
 * `listen()`은 비동기로 UnlistenFn을 주므로, teardown이 먼저 와도 새는 일이
 * 없도록 해소 시점에 즉시 정리한다(tauriApi.wrapListen과 같은 관례).
 */
export function installPeerBridge(): () => void {
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
        console.warn("peerBridge: 이벤트 구독 실패", err);
      });
  };

  track(
    listen<PeerViewerStatus[]>(Events.peerStatus, (e) => {
      useAppStore.getState().syncPeerViewers(e.payload ?? []);
    })
  );

  track(
    listen<SnapshotRequest>(Events.peerSnapshotRequest, (e) => {
      void answerSnapshot(e.payload).catch((err) => {
        console.warn("peerBridge: 스냅샷 응답 실패", err);
      });
    })
  );

  track(
    listen<PendingPairing>(Events.peerPairRequest, (e) => {
      const prev = useAppStore.getState().peerPending;
      const next = [...prev.filter((p) => p.pairingId !== e.payload.pairingId), e.payload];
      useAppStore.getState().setPeerPending(next);
    })
  );

  track(
    listen<ResizedEvent>(Events.peerResized, (e) => {
      const { agentId, cols, rows } = e.payload;
      // 리사이즈 소유자는 호스트다(§결정 6) — 뷰어는 통지받은 크기를 따른다.
      if (cols > 0 && rows > 0) {
        useAppStore.getState().setSessionSize(agentId, cols, rows);
      }
    })
  );

  // 부팅 직후 1회 동기화 — 이벤트는 변경 시에만 오므로, 이미 붙어 있는
  // 피어(자동 연결)를 여기서 한 번 끌어온다.
  void peerApi
    .viewerStatus()
    .then((list) => {
      if (!disposed) useAppStore.getState().syncPeerViewers(list);
    })
    .catch(() => {
      /* 구버전 백엔드/미지원 — 원격 없이 진행 */
    });
  void peerApi
    .hostStatus()
    .then((status) => {
      if (!disposed) useAppStore.getState().setPeerPending(status.pending ?? []);
    })
    .catch(() => {
      /* 무시 — 호스트 기능이 꺼져 있을 뿐 */
    });

  return () => {
    disposed = true;
    for (const un of unlisteners) void un();
    unlisteners.length = 0;
  };
}
