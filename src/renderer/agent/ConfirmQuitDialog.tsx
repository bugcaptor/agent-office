// src/renderer/agent/ConfirmQuitDialog.tsx
//
// 앱 종료 확인 다이얼로그 (ConfirmRestartDialog와 동일 패턴). ModalState가
// confirm-quit일 때만 렌더한다 — `quitGuard`가 `CloseRequested`를 가로채
// 진행 중인 작업(어떤 에이전트든 열린 턴, `timeTracking[agentId].phase !==
// "idle"`)이 있을 때 이 모달을 연다. 종료 확인 시 모달을 닫고
// `getCurrentWindow().destroy()`를 호출한다 — `destroy()`는 `CloseRequested`를
// 재발행하지 않으므로(재진입 가드 불필요) 여기서 별도 처리가 필요 없고, 백엔드
// `ExitRequested`(dispose_all 정리)는 그대로 트리거된다. 취소 시 모달만 닫는다.
// CSS는 ProfileDialog와 동일한 전역 클래스(modal-backdrop / pixel-panel /
// pixel-btn / dialog-actions)를 재사용 — layout.css가 App 부팅 시 로드되어
// 있어 별도 import 불필요.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";

export function ConfirmQuitDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);

  if (modal.kind !== "confirm-quit") return null;

  const onConfirm = () => {
    closeModal();
    void getCurrentWindow().destroy();
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
        <p>진행 중인 작업이 있습니다. 지금 종료하면 실행 중인 세션이 모두 중단됩니다.</p>
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={onConfirm}>
            종료
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}
