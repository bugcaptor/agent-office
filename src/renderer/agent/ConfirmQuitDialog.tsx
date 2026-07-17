// src/renderer/agent/ConfirmQuitDialog.tsx
//
// 앱 종료 확인 다이얼로그 (ConfirmRestartDialog와 동일 패턴). ModalState가
// confirm-quit일 때만 렌더한다 — `quitGuard`가 `CloseRequested`를 가로채
// 아직 퇴근하지 않은(on-duty, `agents[id].clockedOut`이 아닌) 에이전트가
// 하나라도 있을 때 이 모달을 연다. CSS는 ProfileDialog와 동일한 전역 클래스
// (modal-backdrop / pixel-panel / pixel-btn / dialog-actions)를 재사용 —
// layout.css가 App 부팅 시 로드되어 있어 별도 import 불필요.
//
// 세션 핸드오프(docs/session-handoff-design.md §핵심 6): 핸드오프 지원
// (`quitGuard.isHandoffSupported()`, 부팅 시 캐시) && 실행 중(Running) 세션이
// 하나라도 있으면 3버튼[터미널 유지하고 종료 / 모두 종료하고 종료 / 취소],
// 아니면 기존 2버튼[종료 / 취소]. "유지"는 `handoffSessions()`가 실패해도
// (구버전 데몬 기동 실패 등) 종료 자체는 진행한다 — 핸드오프는 best-effort.
// 두 "종료" 경로 모두 `destroy()`로 수렴한다: `destroy()`는 `CloseRequested`를
// 재발행하지 않으므로(재진입 가드 불필요) 여기서 별도 처리가 필요 없고, 백엔드
// `ExitRequested`(dispose_all 정리 — handed-off 세션은 스킵)는 그대로 트리거된다.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { isHandoffSupported } from "../quitGuard";

export function ConfirmQuitDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const hasRunningSession = useAppStore((s) =>
    s.agentOrder.some((id) => s.sessions[id]?.status === "running")
  );

  if (modal.kind !== "confirm-quit") return null;

  const showHandoffOptions = isHandoffSupported() && hasRunningSession;

  const destroyWindow = () => {
    closeModal();
    void getCurrentWindow().destroy();
  };

  const onKeepAndQuit = async () => {
    try {
      await tauriApi.handoffSessions();
    } catch (err) {
      console.warn("종료 확인: 세션 핸드오프 실패 — 터미널 유지 없이 종료 진행", err);
    }
    destroyWindow();
  };

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
              <button className="pixel-btn primary" onClick={() => void onKeepAndQuit()}>
                터미널 유지하고 종료
              </button>
              <button className="pixel-btn" onClick={destroyWindow}>
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
              <button className="pixel-btn primary" onClick={destroyWindow}>
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
