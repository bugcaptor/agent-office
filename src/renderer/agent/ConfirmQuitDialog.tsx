// src/renderer/agent/ConfirmQuitDialog.tsx
//
// 앱 종료 확인 다이얼로그. `quitGuard`가 `CloseRequested`를 가로채 아직 퇴근하지
// 않은(on-duty) 에이전트가 있을 때 이 모달을 연다. CSS는 ProfileDialog와 동일한
// 전역 클래스(modal-backdrop / pixel-panel / pixel-btn / dialog-actions)를 재사용.
//
// 세션 핸드오프(docs/session-handoff-design.md §핵심 6): 핸드오프 지원 && 실행 중
// 세션이 있으면 3버튼[터미널 유지하고 종료 / 모두 종료하고 종료 / 취소], 아니면
// 2버튼[종료 / 취소]. "유지"는 `handoffSessions()`가 실패해도 종료는 진행한다.
//
// 캐릭터 일기(#60): "종료" 확정 시, 밀린(종료된 미기록) 세션 일기가 있으면 곧바로
// destroy하지 않고 flushing 단계로 전환해 잠시 일기를 쓴다. 사용자는 [건너뛰고
// 종료]로 언제든 취소할 수 있고, 데드라인(QUIT_FLUSH_DEADLINE_MS)을 넘겨도 그냥
// 종료한다. 캔슬해도 작업 로그는 디스크에 남아 다음 실행에 이어진다. 밀린 게
// 없으면(대부분) 예전처럼 즉시 종료한다.
import { useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { isHandoffSupported } from "../quitGuard";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import {
  pendingDiaryAgents,
  runQuitDiaryFlush,
  QUIT_FLUSH_DEADLINE_MS,
} from "../diary/quitDiaryFlush";

type Phase = "confirm" | "flushing";

export function ConfirmQuitDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const hasRunningSession = useAppStore((s) =>
    s.agentOrder.some((id) => s.sessions[id]?.status === "running")
  );

  const [phase, setPhase] = useState<Phase>("confirm");
  const [progress, setProgress] = useState<{ done: number; total: number }>({ done: 0, total: 0 });
  // 캔슬 버튼이 resolve하는 프라미스 — flush 레이스의 한 갈래.
  const cancelRef = useRef<(() => void) | null>(null);
  // destroy를 한 번만 부르게 하는 가드.
  const destroyedRef = useRef(false);

  if (modal.kind !== "confirm-quit") return null;

  const showHandoffOptions = isHandoffSupported() && hasRunningSession;

  const destroyWindow = () => {
    if (destroyedRef.current) return;
    destroyedRef.current = true;
    closeModal();
    void getCurrentWindow().destroy();
  };

  // 종료 확정 공통 경로: (핸드오프면) 세션 이관 → 밀린 일기 flush(있으면 잠시 대기)
  // → destroy. 밀린 게 없으면 즉시 destroy.
  const beginQuit = async (handoff: boolean) => {
    if (handoff) {
      try {
        // 종료 직전 화면(스크롤백)을 실어 보낸다 — 데몬은 핸드오프 이후 출력만
        // 보관하므로. 직렬화 전에 xterm write 큐를 flush(§P1)하고, 렌더 완료된 raw
        // 바이트 누적치를 함께 실어 스냅샷 offset을 확정한다(§#49).
        const snapshots = await terminalRegistry.flushAndSerializeAll();
        await tauriApi.handoffSessions(snapshots, terminalRegistry.getRenderedBytes());
      } catch (err) {
        console.warn("종료 확인: 세션 핸드오프 실패 — 터미널 유지 없이 종료 진행", err);
      }
    }

    // 살아서 데몬으로 넘어간 세션은 "종료"가 아니므로 제외(includeLive=false).
    const targets = pendingDiaryAgents();
    if (targets.length === 0) {
      destroyWindow();
      return;
    }

    setProgress({ done: 0, total: targets.length });
    setPhase("flushing");

    const cancelled = new Promise<void>((resolve) => {
      cancelRef.current = resolve;
    });
    const deadline = new Promise<void>((resolve) => setTimeout(resolve, QUIT_FLUSH_DEADLINE_MS));
    const flushed = runQuitDiaryFlush(targets, {
      onProgress: (done, total) => setProgress({ done, total }),
    });

    // 완료·캔슬·데드라인 중 먼저 오는 것에서 종료.
    await Promise.race([flushed, cancelled, deadline]);
    destroyWindow();
  };

  if (phase === "flushing") {
    return (
      <div className="modal-backdrop">
        <div className="pixel-panel confirm-quit-dialog">
          <h2 className="pixel-title">일기 쓰는 중…</h2>
          <p>
            종료 전에 오늘 한 일을 일기로 남기는 중입니다
            {progress.total > 0 ? ` (${progress.done}/${progress.total})` : ""}.
          </p>
          <div className="dialog-actions">
            <button className="pixel-btn" onClick={() => cancelRef.current?.()}>
              건너뛰고 종료
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-quit-dialog">
        <h2 className="pixel-title">종료 확인</h2>
        {showHandoffOptions ? (
          <>
            <p>실행 중인 터미널이 있습니다. 터미널을 그대로 둔 채 종료할 수 있습니다.</p>
            <div className="dialog-actions">
              <button className="pixel-btn primary" onClick={() => void beginQuit(true)}>
                터미널 유지하고 종료
              </button>
              <button className="pixel-btn" onClick={() => void beginQuit(false)}>
                모두 종료하고 종료
              </button>
              <button className="pixel-btn" onClick={closeModal}>
                취소
              </button>
            </div>
          </>
        ) : (
          <>
            <p>아직 퇴근하지 않은 에이전트가 있습니다. 지금 종료하면 실행 중인 세션이 모두 중단됩니다.</p>
            <div className="dialog-actions">
              <button className="pixel-btn primary" onClick={() => void beginQuit(false)}>
                종료
              </button>
              <button className="pixel-btn" onClick={closeModal}>
                취소
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
