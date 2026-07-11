// src/renderer/agent/ConfirmTerminateDialog.tsx
//
// 터미널 종료 확인 다이얼로그 (ConfirmRestartDialog와 동일 패턴). ModalState가
// confirm-terminate일 때만 렌더한다. 재시작과 달리 PTY만 죽이고 재생성하지
// 않으므로, 확인 시 terminateAgentSession(오케스트레이터)을 호출한다 —
// 캐릭터는 FSM 규칙대로 탕비실로 이동하고, 캐릭터 클릭으로 재소환된다.
// CSS는 전역 클래스(modal-backdrop / pixel-panel / pixel-btn / dialog-actions)
// 재사용 — layout.css가 App 부팅 시 로드되어 있어 별도 import 불필요.
import { useAppStore } from "../store/appStore";
import { terminateAgentSession } from "./terminateSession";

export function ConfirmTerminateDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agentId = modal.kind === "confirm-terminate" ? modal.agentId : undefined;
  const agent = useAppStore((s) => (agentId ? s.agents[agentId] : undefined));
  const running = useAppStore((s) => {
    if (!agentId) return false;
    const status = s.sessions[agentId]?.status;
    return status === "starting" || status === "running";
  });

  if (modal.kind !== "confirm-terminate") return null;

  const name = agent?.name ?? agentId;

  const onConfirm = () => {
    if (agentId) void terminateAgentSession(agentId);
    closeModal();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-terminate-dialog">
        <h2 className="pixel-title">터미널 종료</h2>
        <p>
          정말 <strong>{name}</strong>의 터미널을 종료할까요?
        </p>
        {running && (
          <p className="confirm-terminate-warning" style={{ color: "#e0574a" }}>
            실행 중인 세션이 종료됩니다. 캐릭터는 탕비실에서 대기하며, 캐릭터를
            클릭하면 새 세션이 시작됩니다.
          </p>
        )}
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
